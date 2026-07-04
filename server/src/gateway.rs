//! Cœur de relai du Gateway : traduit les événements client (transport GNS) en frames
//! internes vers le Shard, et les `ServerSend` du Shard en envois client. Générique sur le
//! transport client → testable avec `InMemoryTransport`, branché sur `GnsTransport` en prod.

use crate::framing::FrameReader;
use crate::internal_net::{decode_server_send, event_to_client_event_frame};
use crate::transport::{Transport, TransportEvent};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Une connexion TCP interne vers un Shard, avec son `FrameReader` de lecture persistant.
pub struct ShardLink {
    sock: TcpStream,
    reader: FrameReader,
}

/// Écrit `frames` vers le shard à `shard_addr`, en connectant si besoin. Une connexion déjà
/// présente dans `shards` mais dont l'écriture échoue est évacuée avant de renvoyer l'erreur —
/// une entrée morte ne doit jamais bloquer une reconnexion au prochain appel.
///
/// Renvoie `true` si cet appel vient de créer la connexion (1re connexion, ou reconnexion après
/// une entrée morte évacuée) — signal utilisé par l'appelant pour re-semer l'état des clients
/// déjà chargés sur ce shard (cf. `reseed_frames_for_reconnected_shard`), puisqu'un shard qui
/// vient d'accepter une nouvelle connexion a perdu tout son état précédent.
pub async fn write_to_shard(
    shards: &mut HashMap<String, ShardLink>,
    shard_addr: &str,
    frames: &[Vec<u8>],
) -> std::io::Result<bool> {
    let created = if !shards.contains_key(shard_addr) {
        let sock = TcpStream::connect(shard_addr).await?;
        shards.insert(
            shard_addr.to_string(),
            ShardLink {
                sock,
                reader: FrameReader::new(),
            },
        );
        true
    } else {
        false
    };
    let result: std::io::Result<()> = async {
        let link = shards.get_mut(shard_addr).unwrap();
        for f in frames {
            link.sock.write_all(f).await?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        shards.remove(shard_addr);
        return Err(e);
    }
    Ok(created)
}

/// Lit tout ce qui est disponible sur chaque shard connecté (non bloquant, timeout court par
/// shard) et alimente `latest[client][shard_addr]` avec le dernier `ServerSend` reçu. Une
/// lecture EOF (`n == 0`) ou en erreur évacue l'entrée du shard concerné — connexion morte,
/// sera recréée au prochain `write_to_shard` pour cette adresse — et purge de `latest`, pour
/// tous les clients, tout snapshot associé à cette adresse : un snapshot laissé en place y
/// serait rediffusé à chaque tick jusqu'à la reconnexion, comme s'il était encore à jour (bug
/// A.1 de l'audit prod du 2026-07-03 : la zone paraît figée au lieu de disparaître).
pub async fn read_from_shards(
    shards: &mut HashMap<String, ShardLink>,
    latest: &mut HashMap<u64, HashMap<String, Vec<u8>>>,
) {
    use crate::internal_net::decode_server_send;

    let addrs: Vec<String> = shards.keys().cloned().collect();
    let mut dead = Vec::new();
    let mut sbuf = [0u8; 8192];
    for addr in addrs {
        let link = shards.get_mut(&addr).unwrap();
        match tokio::time::timeout(
            std::time::Duration::from_millis(1),
            link.sock.read(&mut sbuf),
        )
        .await
        {
            Ok(Ok(0)) => dead.push(addr), // EOF : le shard a fermé la connexion
            Ok(Ok(n)) => {
                link.reader.push(&sbuf[..n]);
                if link
                    .reader
                    .declared_len_exceeds(crate::framing::MAX_FRAME_LEN)
                {
                    dead.push(addr);
                    continue;
                }
                while let Some(body) = link.reader.next_frame() {
                    if let Some((cid, payload)) = decode_server_send(&body) {
                        latest.entry(cid).or_default().insert(addr.clone(), payload);
                    }
                }
            }
            Ok(Err(_)) => dead.push(addr), // erreur de lecture : connexion morte
            Err(_) => {}                   // timeout : rien à lire pour l'instant
        }
    }
    for addr in &dead {
        shards.remove(addr);
        for per_shard in latest.values_mut() {
            per_shard.remove(addr);
        }
    }
}

/// Poll le transport client et renvoie les frames `ClientEvent` à écrire au Shard.
pub fn drain_client_to_shard<T: Transport>(client: &mut T) -> Vec<Vec<u8>> {
    client
        .poll()
        .iter()
        .map(event_to_client_event_frame)
        .collect()
}

/// Reconstruit, pour chaque client que le Gateway sait chargé sur `shard_addr`, les frames à
/// rejouer vers ce shard après une reconnexion. Le shard vient de perdre tout son état (nouveau
/// `Server::new()` recréé côté Shard à chaque connexion acceptée, cf. `shard_main`) et ne connaît
/// plus aucun de ces clients tant qu'on ne les re-sème pas : sans ça, ils restent invisibles pour
/// les autres joueurs du shard, indéfiniment (bug A.1 de l'audit prod du 2026-07-03). Un client
/// chargé mais sans position connue du Gateway (ne devrait pas arriver : `loaded` n'est peuplé
/// qu'après une 1re position) est ignoré plutôt que de semer une position inventée.
pub fn reseed_frames_for_reconnected_shard(
    loader: &crate::handoff::ShardLoader,
    shard_addr: &str,
    last_pos: &HashMap<u64, [f32; 3]>,
) -> Vec<(u64, Vec<Vec<u8>>)> {
    loader
        .clients_loaded_on(shard_addr)
        .into_iter()
        .filter_map(|cid| {
            let pos = *last_pos.get(&cid)?;
            let mut frames = loader.preamble_frames(cid);
            frames.push(event_to_client_event_frame(&TransportEvent::Message {
                from: cid,
                data: crate::gateway_routing::encode_position_update(pos),
            }));
            Some((cid, frames))
        })
        .collect()
}

/// Sauve la position actuelle de tous les clients connus par nom (rejoints via `keys`) — utilisé
/// à la fois par l'autosave périodique et le flush d'arrêt propre. Un client sans position
/// connue (jamais reçu de `PositionUpdate` depuis son `Join`) n'est pas sauvé.
pub fn save_all_known(
    store: &mut impl crate::persistence::PlayerStore,
    keys: &HashMap<u64, String>,
    last_pos: &HashMap<u64, [f32; 3]>,
    residence: &HashMap<u64, Option<[f32; 3]>>,
) {
    for (cid, name) in keys.iter() {
        if let Some(pos) = last_pos.get(cid).copied() {
            store.save(
                name,
                crate::persistence::PlayerRecord {
                    last_position: pos,
                    residence: residence.get(cid).copied().flatten(),
                },
            );
        }
    }
}

/// Décode un corps `ServerSend` (déjà déframé) reçu du Shard et l'envoie au bon client.
pub fn apply_shard_frame_to_client<T: Transport>(body: &[u8], client: &mut T) {
    if let Some((client_id, payload)) = decode_server_send(body) {
        client.send(client_id, &payload);
    }
}

/// Point d'entrée du Gateway (M4, handoff) : ouvre l'écoute GNS publique et, pour chaque client,
/// calcule à chaque position — via la `ShardTopology` locale + le rayon selon le rang — l'ensemble
/// de shards où le charger (autoritaire + zones tampon). Il diffuse les événements du client à tous
/// ses shards chargés, et **fusionne** les snapshots reçus de ces shards en un seul avant de les
/// renvoyer au client. Le double-chargement près d'une frontière élimine les saccades au transfert.
#[cfg(feature = "gns")]
pub async fn gateway_main(
    listen_addr: &str,
    topology: crate::handoff::ShardTopology,
    radius: crate::handoff::RadiusPolicy,
    mut store: crate::persistence::FileStore,
    spawn: [f32; 3],
) -> std::io::Result<()> {
    use crate::gateway_routing::{extract_join_name, extract_position};
    use crate::gns_transport::GnsTransport;
    use crate::handoff::{LoadAction, Rank, ShardLoader};
    use crate::persistence::{resolve_spawn, PlayerRecord, PlayerStore};
    use crate::snapshot_merge::merge_snapshots;
    use crate::transport::{Transport, TransportEvent};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::time::Duration;

    let mut shards: HashMap<String, ShardLink> = HashMap::new();
    let mut loader = ShardLoader::new();
    // Dernier snapshot reçu de chaque shard, par client : latest[client][shard_addr] = payload.
    let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
    // Rang par client (stub M4 : tout le monde Player ; surchargeable quand l'auth existera).
    let ranks: HashMap<u64, Rank> = HashMap::new();
    // Persistance : clé (display_name), dernière position, et résidence chargée — par client.
    let mut keys: HashMap<u64, String> = HashMap::new();
    let mut last_pos: HashMap<u64, [f32; 3]> = HashMap::new();
    // Horodatage de la dernière PositionUpdate ACCEPTÉE par client (absent tant qu'aucune
    // position n'a encore été acceptée depuis le Join — sert de garde anti-triche).
    let mut last_pos_at: HashMap<u64, std::time::Instant> = HashMap::new();
    let mut residence: HashMap<u64, Option<[f32; 3]>> = HashMap::new();

    let sock: SocketAddr = listen_addr.parse().expect("adresse GNS invalide");
    let mut client =
        GnsTransport::listen(sock.ip(), sock.port()).expect("GnsTransport::listen failed");
    tracing::info!(
        "Gateway handoff : écoute GNS sur {listen_addr} ({} shards)",
        topology.shards.len()
    );

    let metrics = crate::metrics::Metrics::new();
    {
        let metrics = metrics.clone();
        let metrics_addr = std::env::var("TESSERA_GATEWAY_METRICS_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".to_string());
        tokio::spawn(async move {
            if let Err(e) = crate::metrics::serve(&metrics_addr, metrics).await {
                tracing::warn!("endpoint métriques indisponible ({metrics_addr}): {e}");
            }
        });
    }

    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    // Cf. shard.rs : Skip plutôt que le Burst par défaut — sauter un tick manqué au lieu de
    // rattraper en rafale, pour ne pas dépenser plus de CPU/réseau juste après un pic de charge.
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_autosave = std::time::Instant::now();
    let autosave_interval = Duration::from_secs(30);
    // Enregistré UNE SEULE FOIS avant la boucle : sur Unix, `tokio::signal::unix::signal(...)`
    // installe une registration OS qui ne bufferise rien tant qu'aucun récepteur n'existe — la
    // recréer à chaque itération (comme le faisait l'ancien `shutdown_signal()` appelé depuis le
    // `select!` ci-dessous) crée une fenêtre où un SIGTERM reçu pendant la partie synchrone de
    // l'itération (après le drop de l'ancien flux, avant la création du nouveau) est perdu — le
    // process attend alors un 2e signal. Le flux persistant survit à toutes les itérations.
    let mut shutdown = ShutdownSignal::new();
    loop {
        // 1) Lire chaque shard connecté (évacue et laisse reconnecter les connexions mortes).
        read_from_shards(&mut shards, &mut latest).await;

        // 2) Tick, avec une course contre le signal d'arrêt propre (SIGTERM/SIGINT).
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.recv() => {
                save_all_known(&mut store, &keys, &last_pos, &residence);
                tracing::info!("Arrêt propre : positions sauvegardées, extinction du Gateway");
                return Ok(());
            }
        }
        for ev in client.poll() {
            let cid = match &ev {
                TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
                TransportEvent::Message { from, .. } => *from,
            };
            let is_disconnect = matches!(ev, TransportEvent::Disconnected(_));

            // Décoder ce que porte un message client : Join → identité + résolution de spawn ;
            // PositionUpdate → placement (topologie + rang) et mémorisation de la dernière position.
            let mut placement = None;
            if let TransportEvent::Message { data, .. } = &ev {
                if let Some(name) = extract_join_name(data) {
                    if !name.is_empty() {
                        let record = store.load(&name);
                        let (pos, source) = resolve_spawn(record.as_ref(), spawn);
                        tracing::info!(
                            "Connexion de {name} : placement décidé {pos:?} (source: {source:?})"
                        );
                        residence.insert(cid, record.and_then(|r| r.residence));
                        last_pos.insert(cid, pos); // départ tant qu'aucune position réelle
                        keys.insert(cid, name);
                    }
                } else if let Some((x, y, z)) = extract_position(data) {
                    let now = std::time::Instant::now();
                    let plausible =
                        match (last_pos.get(&cid).copied(), last_pos_at.get(&cid).copied()) {
                            (Some(prev), Some(at)) => crate::anticheat::is_plausible_move(
                                prev,
                                [x, y, z],
                                now.duration_since(at),
                                crate::anticheat::MAX_PLAYER_SPEED_MPS,
                            ),
                            // Pas encore de référence temporelle (1re position après Join) : accepté.
                            _ => true,
                        };
                    if !plausible {
                        tracing::warn!(client = cid, "PositionUpdate rejeté (vitesse implausible)");
                        continue;
                    }
                    last_pos.insert(cid, [x, y, z]);
                    last_pos_at.insert(cid, now);
                    let r = radius.radius_for(*ranks.get(&cid).unwrap_or(&Rank::Player));
                    placement = Some(topology.locate(x, y, r));
                }
            }

            for LoadAction::Forward { shard, frames } in loader.feed(ev, placement) {
                if let Ok(true) = write_to_shard(&mut shards, &shard, &frames).await {
                    // Le shard vient de (re)connecter : côté Shard, `Server::new()` est recréé à
                    // chaque connexion acceptée (cf. `shard_main`) — tout son état précédent est
                    // perdu. Re-semer le préambule + dernière position connue de chaque client que
                    // le Gateway sait chargé sur ce shard, sinon ils y restent invisibles pour
                    // toujours (bug A.1, audit prod 2026-07-03). Idempotent : `World::add_player`
                    // (`or_default`) et `set_pose` tolèrent un double envoi sans effet de bord.
                    for (_, seed_frames) in
                        reseed_frames_for_reconnected_shard(&loader, &shard, &last_pos)
                    {
                        let _ = write_to_shard(&mut shards, &shard, &seed_frames).await;
                    }
                }
            }

            if is_disconnect {
                // Sauver la dernière position connue avant d'oublier le client.
                if let Some(name) = keys.remove(&cid) {
                    if let Some(pos) = last_pos.get(&cid).copied() {
                        store.save(
                            &name,
                            PlayerRecord {
                                last_position: pos,
                                residence: residence.get(&cid).copied().flatten(),
                            },
                        );
                        tracing::info!("Sauvegarde de {name} à {pos:?}");
                    }
                }
                last_pos.remove(&cid);
                last_pos_at.remove(&cid);
                residence.remove(&cid);
                loader.forget(cid);
                latest.remove(&cid);
            } else if let Some(per_shard) = latest.get_mut(&cid) {
                // Élaguer les snapshots des shards qui ne sont plus chargés pour ce client.
                let loaded = loader.loaded_shards(cid);
                per_shard.retain(|s, _| loaded.contains(s));
            }
        }

        // 3) Pour chaque client, fusionner les derniers snapshots de ses shards chargés → envoi.
        for (cid, per_shard) in latest.iter() {
            let snaps: Vec<Vec<u8>> = per_shard.values().cloned().collect();
            if let Some(merged) = merge_snapshots(&snaps) {
                client.send(*cid, &merged);
            }
        }
        metrics
            .players
            .store(latest.len() as u64, std::sync::atomic::Ordering::Relaxed);
        metrics
            .shards_loaded
            .store(shards.len() as u64, std::sync::atomic::Ordering::Relaxed);

        // 4) Autosave périodique — ne dépend pas d'une déconnexion propre.
        if last_autosave.elapsed() >= autosave_interval {
            save_all_known(&mut store, &keys, &last_pos, &residence);
            last_autosave = std::time::Instant::now();
        }
    }
}

/// Détecteur d'arrêt propre : SIGINT (Ctrl+C) partout, plus SIGTERM (docker stop) sous Unix.
/// Utilisé uniquement depuis `gateway_main`, qui est déjà gns-gated.
///
/// Doit être construit UNE SEULE FOIS avant la boucle principale et réutilisé (via `&mut`) à
/// chaque itération. Sur Unix, `recv()` réutilise le même flux `tokio::signal::unix::Signal` —
/// reconstruire ce flux à chaque itération (comme le faisait un appel `shutdown_signal()` frais
/// dans le `select!` de la boucle) rouvre une fenêtre où un signal arrivé entre deux itérations
/// (après le drop de l'ancien flux, avant la création du nouveau) n'est délivré à personne et est
/// silencieusement perdu — tokio ne bufferise pas un signal pour un récepteur qui n'existe pas
/// encore. `tokio::signal::ctrl_c()`, lui, n'a pas ce problème (canal partagé installé une seule
/// fois en interne par tokio dès le premier appel) : il reste donc appelé frais à chaque `recv()`.
#[cfg(feature = "gns")]
struct ShutdownSignal {
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
}

#[cfg(feature = "gns")]
impl ShutdownSignal {
    #[cfg(unix)]
    fn new() -> Self {
        use tokio::signal::unix::{signal, SignalKind};
        Self {
            sigterm: signal(SignalKind::terminate()).expect("SIGTERM handler"),
        }
    }

    #[cfg(not(unix))]
    fn new() -> Self {
        Self {}
    }

    #[cfg(unix)]
    async fn recv(&mut self) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = self.sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    async fn recv(&mut self) {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::FrameReader;
    use crate::internal_net::decode_client_event;
    use crate::transport::{InMemoryTransport, Transport, TransportEvent};

    #[test]
    fn drains_client_events_into_shard_frames() {
        let mut client = InMemoryTransport::new();
        client.inject(TransportEvent::Connected(1));
        client.inject(TransportEvent::Message {
            from: 1,
            data: vec![4, 2],
        });

        let frames = drain_client_to_shard(&mut client);
        assert_eq!(frames.len(), 2);

        // Chaque frame est un ClientEvent décodable.
        let mut r = FrameReader::new();
        for f in &frames {
            r.push(f);
        }
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Connected(1))
        );
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Message {
                from: 1,
                data: vec![4, 2]
            })
        );
    }

    #[test]
    fn applies_shard_serversend_to_the_right_client() {
        // Un ServerSend{client 9, payload [7,7]} arrive du Shard ; il doit partir au client 9.
        let mut shard_side = InMemoryTransport::new(); // sert juste à produire un ServerSend framé
        use crate::internal_net::InternalTransport;
        let mut it = InternalTransport::new();
        it.send(9, &[7, 7]);
        let framed = it.take_outbound().remove(0);
        let mut r = FrameReader::new();
        r.push(&framed);
        let body = r.next_frame().unwrap();

        let mut client = InMemoryTransport::new();
        apply_shard_frame_to_client(&body, &mut client);
        assert_eq!(client.take_sent(9), vec![vec![7, 7]]);
        let _ = &mut shard_side;
    }

    fn join_payload() -> Vec<u8> {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let name = b.create_string("v");
        let join = protocol::Join::create(
            &mut b,
            &protocol::JoinArgs {
                display_name: Some(name),
            },
        );
        let env = protocol::ClientEnvelope::create(
            &mut b,
            &protocol::ClientEnvelopeArgs {
                msg_type: protocol::ClientMsg::Join,
                msg: Some(join.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn reseed_frames_reconstruct_preamble_and_last_position_for_every_loaded_client() {
        use crate::handoff::Placement;
        use crate::transport::TransportEvent;

        let mut loader = crate::handoff::ShardLoader::new();
        loader.feed(TransportEvent::Connected(1), None);
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: crate::gateway_routing::encode_position_update([500.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: "A".to_string(),
                overlaps: vec![],
            }),
        );

        let mut last_pos = HashMap::new();
        last_pos.insert(1u64, [500.0, 0.0, 0.0]);

        let seeded = reseed_frames_for_reconnected_shard(&loader, "A", &last_pos);
        assert_eq!(seeded.len(), 1);
        let (cid, frames) = &seeded[0];
        assert_eq!(*cid, 1);
        assert_eq!(frames.len(), 3); // Connected + Join + Position
    }

    #[test]
    fn reseed_frames_skips_a_loaded_client_with_no_known_position() {
        use crate::handoff::Placement;
        use crate::transport::TransportEvent;

        let mut loader = crate::handoff::ShardLoader::new();
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: crate::gateway_routing::encode_position_update([500.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: "A".to_string(),
                overlaps: vec![],
            }),
        );

        let last_pos: HashMap<u64, [f32; 3]> = HashMap::new(); // aucune position connue du Gateway
        assert!(reseed_frames_for_reconnected_shard(&loader, "A", &last_pos).is_empty());
    }

    #[tokio::test]
    async fn write_to_shard_reports_whether_it_created_a_new_connection() {
        use std::collections::HashMap;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 64];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
            }
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();

        let created_first = write_to_shard(&mut shards, &addr, &[b"a".to_vec()])
            .await
            .expect("1re écriture doit réussir");
        assert!(created_first, "la 1re écriture crée forcément la connexion");

        let created_second = write_to_shard(&mut shards, &addr, &[b"b".to_vec()])
            .await
            .expect("2e écriture doit réussir");
        assert!(
            !created_second,
            "une connexion déjà vivante ne doit pas être signalée comme nouvelle"
        );
    }

    #[tokio::test]
    async fn dead_shard_link_purges_its_stale_snapshots_from_latest_for_every_client() {
        use std::collections::HashMap;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            drop(sock); // ferme aussitôt : simule un shard qui vient de crasher
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        write_to_shard(&mut shards, &addr, &[b"a".to_vec()])
            .await
            .unwrap();

        // Deux clients ont chacun un snapshot périmé en attente pour ce shard, plus un snapshot
        // d'un AUTRE shard qui doit survivre à la purge (seule l'adresse morte est concernée).
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        latest
            .entry(1u64)
            .or_default()
            .insert(addr.clone(), b"perime-1".to_vec());
        latest
            .entry(1u64)
            .or_default()
            .insert("autre-shard".to_string(), b"toujours-valide".to_vec());
        latest
            .entry(2u64)
            .or_default()
            .insert(addr.clone(), b"perime-2".to_vec());

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        read_from_shards(&mut shards, &mut latest).await;

        assert!(
            !latest.get(&1).unwrap().contains_key(&addr),
            "le snapshot périmé du client 1 pour le shard mort doit être purgé"
        );
        assert!(
            latest.get(&1).unwrap().contains_key("autre-shard"),
            "le snapshot d'un shard toujours vivant ne doit pas être touché"
        );
        assert!(
            !latest.contains_key(&2) || !latest.get(&2).unwrap().contains_key(&addr),
            "le snapshot périmé du client 2 pour le shard mort doit être purgé"
        );
    }

    /// Reproduit le bug A.1 de bout en bout, sans GNS : un shard "crashe" (ferme sa connexion),
    /// redémarre sur la même adresse (comme un conteneur Docker relancé), et un 2e client
    /// déclenche une nouvelle écriture vers ce shard. Le shard frais ne connaît plus le 1er
    /// client — il doit être re-semé (Connected+Join+Position), sinon il reste invisible pour
    /// toujours pour les autres joueurs de ce shard, silencieusement.
    #[tokio::test]
    async fn shard_reconnect_reseeds_every_previously_loaded_client() {
        use crate::handoff::{LoadAction, Placement, ShardLoader};
        use crate::transport::TransportEvent;
        use std::collections::HashMap;
        use tokio::net::TcpListener;

        let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener1.local_addr().unwrap().to_string();
        let accept1 = tokio::spawn(async move {
            let (sock, _) = listener1.accept().await.unwrap();
            drop(sock); // le shard "crashe" aussitôt après avoir accepté le client 1
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        let mut loader = ShardLoader::new();
        let mut last_pos: HashMap<u64, [f32; 3]> = HashMap::new();

        // Client 1 rejoint et se place sur le shard "A" — écriture normale vers le shard n°1.
        loader.feed(TransportEvent::Connected(1), None);
        loader.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        last_pos.insert(1, [500.0, 0.0, 0.0]);
        for LoadAction::Forward { shard, frames } in loader.feed(
            TransportEvent::Message {
                from: 1,
                data: crate::gateway_routing::encode_position_update([500.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: addr.clone(),
                overlaps: vec![],
            }),
        ) {
            write_to_shard(&mut shards, &shard, &frames).await.unwrap();
        }
        accept1.await.unwrap();

        // Le shard n°1 meurt : EOF détecté, connexion évacuée.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
        read_from_shards(&mut shards, &mut latest).await;
        assert!(!shards.contains_key(&addr));

        // Le shard redémarre sur la MÊME adresse et capture tout ce qu'il reçoit. Le Gateway
        // écrit en 2 appels séparés (frames du client 2, puis re-seed du client 1) qui peuvent
        // arriver en 2 segments TCP distincts : accumuler jusqu'à 6 frames décodables (3+3) ou
        // un timeout, plutôt qu'un seul `read()` qui capturerait parfois seulement le 1er lot.
        let listener2 = TcpListener::bind(&addr).await.unwrap();
        let (recv_tx, recv_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut sock, _) = listener2.accept().await.unwrap();
            let mut reader = FrameReader::new();
            let mut events = Vec::new();
            let mut buf = [0u8; 4096];
            while events.len() < 6 {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                reader.push(&buf[..n]);
                while let Some(body) = reader.next_frame() {
                    if let Some(ev) = crate::internal_net::decode_client_event(&body) {
                        events.push(ev);
                    }
                }
            }
            let _ = recv_tx.send(events);
        });

        // Client 2 arrive et se place aussi sur le shard "A" — déclenche la reconnexion.
        loader.feed(TransportEvent::Connected(2), None);
        loader.feed(
            TransportEvent::Message {
                from: 2,
                data: join_payload(),
            },
            None,
        );
        last_pos.insert(2, [510.0, 0.0, 0.0]);
        for LoadAction::Forward { shard, frames } in loader.feed(
            TransportEvent::Message {
                from: 2,
                data: crate::gateway_routing::encode_position_update([510.0, 0.0, 0.0]),
            },
            Some(Placement {
                authoritative: addr.clone(),
                overlaps: vec![],
            }),
        ) {
            let reconnected = write_to_shard(&mut shards, &shard, &frames).await.unwrap();
            if reconnected {
                for (_, seed_frames) in
                    reseed_frames_for_reconnected_shard(&loader, &shard, &last_pos)
                {
                    write_to_shard(&mut shards, &shard, &seed_frames)
                        .await
                        .unwrap();
                }
            }
        }

        let events = tokio::time::timeout(std::time::Duration::from_secs(2), recv_rx)
            .await
            .expect("le shard frais doit recevoir les 6 frames attendues (3+3) sous 2s")
            .unwrap();
        assert!(
            events.contains(&TransportEvent::Connected(1)),
            "le client 1 (jamais revenu lui-même) doit être re-semé au shard frais par le Gateway ; reçu {events:?}"
        );
    }

    #[tokio::test]
    async fn evicts_dead_shard_link_and_reconnects_once_a_new_listener_is_up() {
        use std::collections::HashMap;
        use std::time::Duration;
        use tokio::net::TcpListener;

        let listener1 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener1.local_addr().unwrap().to_string();

        // "Shard" n°1 : accepte une connexion puis se ferme aussitôt (simule un crash).
        tokio::spawn(async move {
            let (sock, _) = listener1.accept().await.unwrap();
            drop(sock);
            drop(listener1); // libère le port pour le "redémarrage" ci-dessous
        });

        let mut shards: HashMap<String, ShardLink> = HashMap::new();
        let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();

        write_to_shard(&mut shards, &addr, &[b"a".to_vec()])
            .await
            .expect("1re connexion doit réussir");
        assert!(shards.contains_key(&addr));

        // Laisse le "shard" fermer, puis détecte l'EOF côté lecture.
        tokio::time::sleep(Duration::from_millis(100)).await;
        read_from_shards(&mut shards, &mut latest).await;
        assert!(
            !shards.contains_key(&addr),
            "la connexion morte doit être évacuée après EOF"
        );

        // "Shard" n°2 redémarre à la MÊME adresse (comme un conteneur Docker relancé).
        let listener2 = TcpListener::bind(&addr)
            .await
            .expect("le port doit être libre après le drop du 1er listener");
        let accept2 = tokio::spawn(async move {
            listener2.accept().await.unwrap();
        });

        // La prochaine écriture doit reconnecter automatiquement, sans intervention.
        write_to_shard(&mut shards, &addr, &[b"b".to_vec()])
            .await
            .expect("la reconnexion automatique doit réussir");
        assert!(shards.contains_key(&addr));
        accept2.await.unwrap();
    }

    #[test]
    fn save_all_known_saves_every_client_with_a_known_position() {
        use crate::persistence::{MemoryStore, PlayerRecord, PlayerStore};

        let mut store = MemoryStore::new();
        let mut keys = HashMap::new();
        keys.insert(1u64, "Alice".to_string());
        keys.insert(2u64, "Bob".to_string());
        let mut last_pos = HashMap::new();
        last_pos.insert(1u64, [10.0, 20.0, 30.0]);
        // Bob n'a jamais bougé depuis le Join : pas de position connue, pas sauvé.
        let residence: HashMap<u64, Option<[f32; 3]>> = HashMap::new();

        save_all_known(&mut store, &keys, &last_pos, &residence);

        assert_eq!(
            store.load("Alice"),
            Some(PlayerRecord {
                last_position: [10.0, 20.0, 30.0],
                residence: None,
            })
        );
        assert_eq!(
            store.load("Bob"),
            None,
            "un client sans position connue ne doit pas être sauvé"
        );
    }
}
