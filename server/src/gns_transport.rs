//! `GnsTransport` — implémentation du trait `Transport` sur GameNetworkingSockets.
//!
//! Ce module est feature-gated : il n'est compilé que lorsque la feature `gns` est activée.
//! Build : cf. `docs/architecture/0003-gns-binding.md` pour les prérequis système (cmake,
//! protobuf@21, openssl@3).
//!
//! # Mapping GnsConnection → ClientId
//! `GnsConnection` est `Copy + Hash + Eq` ; on l'utilise directement comme clé de HashMap.
//! Le transport assigne son PROPRE `ClientId` monotone (`next_id`, démarré à 1) à chaque
//! connexion acceptée — pas de transmute, pas d'accès à un champ privé.

use crate::transport::{ClientId, Transport, TransportEvent};
use gns::{
    sys::ESteamNetworkingConnectionState, GnsConnection, GnsGlobal, GnsSocket, IsServer,
    MessageSlot, SendFlags, SendOutcome,
};
use std::{collections::HashMap, net::IpAddr};

/// Transport réseau basé sur GameNetworkingSockets (GNS).
///
/// Écoute sur une adresse IP/port donnée, accepte les connexions entrantes,
/// et expose les événements via le trait `Transport`.
///
/// Chaque connexion acceptée se voit attribuer un `ClientId` monotone propre au transport ;
/// deux maps (`id_to_conn` / `conn_to_id`) assurent la correspondance dans les deux sens.
pub struct GnsTransport {
    global: &'static GnsGlobal,
    socket: GnsSocket<IsServer>,
    /// Prochain `ClientId` à attribuer (démarré à 1 ; 0 réservé/inutilisé).
    next_id: ClientId,
    /// ClientId → GnsConnection (pour `send`).
    id_to_conn: HashMap<ClientId, GnsConnection>,
    /// GnsConnection → ClientId (pour `poll` : messages et déconnexions).
    conn_to_id: HashMap<GnsConnection, ClientId>,
    /// Buffer de réception réutilisé à chaque appel à `poll`.
    recv_buf: Box<[MessageSlot]>,
}

impl GnsTransport {
    /// Ouvre un endpoint d'écoute sur `addr:port`.
    ///
    /// # Panics / Errors
    /// Retourne une erreur si GNS ne peut pas être initialisé ou si `listen` échoue.
    pub fn listen(addr: IpAddr, port: u16) -> Result<Self, gns::GnsError> {
        let global = GnsGlobal::get()?;
        let socket = GnsSocket::new(global).listen(addr, port)?;
        // Buffer de 128 slots — suffisant pour les tests et les charges légères Phase 0-C.
        let recv_buf = vec![const { MessageSlot::uninit() }; 128].into_boxed_slice();
        Ok(Self {
            global,
            socket,
            next_id: 1,
            id_to_conn: HashMap::new(),
            conn_to_id: HashMap::new(),
            recv_buf,
        })
    }
}

impl Transport for GnsTransport {
    /// Draine tous les événements GNS en attente (connexions, messages) et les retourne
    /// sous forme de `Vec<TransportEvent>`. Non bloquant.
    fn poll(&mut self) -> Vec<TransportEvent> {
        let mut events = Vec::new();

        // 1. Callbacks bas-niveau GNS — OBLIGATOIRE à chaque tick (ADR 0003 §c-i).
        self.global.poll_callbacks();

        // 2. Événements de connexion (Connected / Disconnected) (ADR 0003 §c-ii).
        for event in self.socket.receive_events() {
            let old = event.old_state();
            let new = event.info().state();
            match (old, new) {
                // Connexion entrante — accepter et enregistrer.
                (
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
                ) => {
                    let conn = event.connection();
                    if self.socket.accept(conn).is_ok() {
                        let client_id = self.next_id;
                        self.next_id += 1;
                        self.id_to_conn.insert(client_id, conn);
                        self.conn_to_id.insert(conn, client_id);
                        events.push(TransportEvent::Connected(client_id));
                    }
                }

                // Fermeture de connexion.
                (
                    _,
                    ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ClosedByPeer
                    | ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ProblemDetectedLocally,
                ) => {
                    let conn = event.connection();
                    let _ = self.socket.close_connection(conn, 0, None, false);
                    // N'émettre Disconnected que si la connexion était bien connue
                    // (évite un faux event pour une connexion jamais acceptée).
                    if let Some(client_id) = self.conn_to_id.remove(&conn) {
                        self.id_to_conn.remove(&client_id);
                        events.push(TransportEvent::Disconnected(client_id));
                    }
                }

                _ => {}
            }
        }

        // 3. Messages entrants (ADR 0003 §c-iii) — variante zero-allocation avec buffer réutilisé.
        if let Ok(msgs) = self.socket.receive_messages_into(&mut self.recv_buf) {
            for msg in msgs {
                if let Some(&client_id) = self.conn_to_id.get(&msg.connection()) {
                    events.push(TransportEvent::Message {
                        from: client_id,
                        data: msg.payload().to_vec(),
                    });
                }
            }
        }

        events
    }

    /// Envoie `data` en unreliable à `to`. Sans effet si `to` n'est pas connecté.
    ///
    /// Seul trafic sortant du Gateway aujourd'hui : les snapshots (`ServerMsg::Snapshot`), dont
    /// chacun rend le précédent obsolète — pas la peine de les retransmettre en cas de perte, le
    /// suivant arrive 50 ms après. En RELIABLE (avant ce fix, audit prod 2026-07-03 §4.3), une
    /// perte de paquet chez un client au réseau faible mettait tous ses snapshots suivants en
    /// attente de retransmission (head-of-line blocking) au lieu de simplement en recevoir un
    /// plus récent. Si un futur message a besoin de fiabilité (chat, transaction), lui donner son
    /// propre appel avec `SendFlags::RELIABLE` plutôt que de changer ce chemin partagé.
    fn send(&mut self, to: ClientId, data: &[u8]) {
        if let Some(&conn) = self.id_to_conn.get(&to) {
            let msg =
                self.global
                    .utils()
                    .allocate_message(conn, SendFlags::UNRELIABLE, data.to_vec());
            let outcomes = self.socket.send_messages(std::iter::once(msg));
            for outcome in outcomes {
                match outcome {
                    SendOutcome::Failed(result, _) => {
                        tracing::warn!(client_id = to, ?result, "GNS send failed");
                    }
                    SendOutcome::Skipped(_) => {
                        tracing::warn!(client_id = to, "GNS send skipped (earlier batch failure)");
                    }
                    SendOutcome::Sent(_) => {}
                }
            }
        }
    }

    /// Ferme la connexion de `to` (kick — serveur plein, flood soutenu). `linger: true` laisse
    /// le temps aux messages déjà en file par un `send()` précédent (ex. `Kicked`) de partir
    /// avant la fermeture effective — mais `send()` est UNRELIABLE (cf. sa doc), donc sous
    /// perte de paquet le client peut se retrouver déconnecté sans avoir reçu le motif. Sans
    /// effet si `to` n'est pas connecté.
    fn disconnect(&mut self, to: ClientId) {
        if let Some(conn) = self.id_to_conn.remove(&to) {
            self.conn_to_id.remove(&conn);
            if let Err(e) = self.socket.close_connection(conn, 0, None, true) {
                tracing::warn!(client_id = to, ?e, "GNS close_connection failed");
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;
    use protocol::*;
    use std::{
        net::Ipv4Addr,
        thread,
        time::{Duration, Instant},
    };

    /// Encode un `PositionUpdate` dans un `ClientEnvelope` FlatBuffers.
    fn encode_position(x: f32, y: f32, z: f32, yaw: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = QVec3::new(
            crate::quant::q_pos(x),
            crate::quant::q_pos(y),
            crate::quant::q_pos(z),
        );
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw: crate::quant::q_yaw(yaw),
                locomotion: 0,
                move_dir: 0,
                flags: 0,
                frame: 0,
                slot: 0,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    /// Smoke test loopback GNS : démarre un listener, connecte un client local, envoie une
    /// `PositionUpdate`, vérifie qu'un `TransportEvent::Message` arrive via `poll()`.
    ///
    /// Marqué `#[ignore]` car il exige le réseau (loopback) et les binaires GNS compilés —
    /// lancer explicitement :
    ///   `cargo test -p server --features gns -- --ignored gns_loopback --nocapture`
    #[test]
    #[ignore]
    fn gns_loopback() {
        let port: u16 = 27025;

        // ── Serveur : écoute en background ───────────────────────────────────
        let mut server_transport =
            GnsTransport::listen(Ipv4Addr::LOCALHOST.into(), port).expect("listen failed");

        // ── Client : connecte, attend Connected, envoie un message ───────────
        let gns_global = GnsGlobal::get().expect("GnsGlobal::get (client)");
        let client = GnsSocket::new(gns_global)
            .connect(Ipv4Addr::LOCALHOST.into(), port)
            .expect("connect failed");

        // Attendre la confirmation de connexion côté client (timeout 5 s).
        let mut connected = false;
        let start = Instant::now();
        while !connected && start.elapsed() < Duration::from_secs(5) {
            gns_global.poll_callbacks();
            for event in client.receive_events() {
                if event.old_state()
                    == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting
                    && event.info().state()
                        == ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected
                {
                    connected = true;
                    println!("[test/client] connected");
                }
            }
            // Laisser le serveur accepter aussi.
            server_transport.poll();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(connected, "le client ne s'est pas connecté dans les 5 s");

        // Envoyer une PositionUpdate via le client GNS.
        let payload = encode_position(1.0, 2.0, 3.0, 0.5);
        let msg = gns_global.utils().allocate_message(
            client.connection(),
            SendFlags::RELIABLE,
            payload.clone(),
        );
        client.send_messages(std::iter::once(msg));
        println!("[test/client] PositionUpdate envoyée");

        // ── Attendre que le serveur reçoive le message (timeout 5 s) ─────────
        let start = Instant::now();
        loop {
            gns_global.poll_callbacks();
            let events = server_transport.poll();
            for ev in &events {
                if let TransportEvent::Message { from, data } = ev {
                    println!(
                        "[test/server] Message reçu de client_id={from}, {} octets",
                        data.len()
                    );
                    // Vérifier le contenu FlatBuffers.
                    let env =
                        flatbuffers::root::<ClientEnvelope>(data).expect("FlatBuffers invalid");
                    assert_eq!(env.msg_type(), ClientMsg::PositionUpdate);
                    let pu = env
                        .msg_as_position_update()
                        .expect("PositionUpdate manquante");
                    assert_eq!(pu.yaw(), 0.5);
                    let p = pu.position().expect("position manquante");
                    assert_eq!((p.x(), p.y(), p.z()), (1.0, 2.0, 3.0));
                    println!("[test] gns_loopback OK — TransportEvent::Message reçu et vérifié");
                    return;
                }
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("le serveur n'a pas reçu le message dans les 5 s");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
