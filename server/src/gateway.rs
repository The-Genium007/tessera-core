//! Cœur de relai du Gateway : traduit les événements client (transport GNS) en frames
//! internes vers le Shard, et les `ServerSend` du Shard en envois client. Générique sur le
//! transport client → testable avec `InMemoryTransport`, branché sur `GnsTransport` en prod.

use crate::internal_net::{decode_server_send, event_to_client_event_frame};
use crate::transport::Transport;

/// Poll le transport client et renvoie les frames `ClientEvent` à écrire au Shard.
pub fn drain_client_to_shard<T: Transport>(client: &mut T) -> Vec<Vec<u8>> {
    client
        .poll()
        .iter()
        .map(event_to_client_event_frame)
        .collect()
}

/// Décode un corps `ServerSend` (déjà déframé) reçu du Shard et l'envoie au bon client.
pub fn apply_shard_frame_to_client<T: Transport>(body: &[u8], client: &mut T) {
    if let Some((client_id, payload)) = decode_server_send(body) {
        client.send(client_id, &payload);
    }
}

/// Point d'entrée du Gateway (M3, multi-shard) : ouvre l'écoute GNS publique, et pour chaque
/// client interroge le Router selon sa 1re position afin de l'assigner au bon shard, puis relaie
/// ce client ⇄ son shard. Plusieurs shards peuvent être connectés simultanément (un par zone).
#[cfg(feature = "gns")]
pub async fn gateway_main(listen_addr: &str, router_addr: &str) -> std::io::Result<()> {
    use crate::framing::FrameReader;
    use crate::gateway_routing::{AssignAction, ShardAssigner};
    use crate::gns_transport::GnsTransport;
    use crate::internal_net::{decode_route_reply, encode_route_request};
    use crate::transport::{Transport, TransportEvent};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    // Une connexion par shard, créée à la demande, avec son FrameReader de lecture.
    struct ShardLink {
        sock: TcpStream,
        reader: FrameReader,
    }
    let mut shards: HashMap<String, ShardLink> = HashMap::new();
    let mut assigner = ShardAssigner::new();

    // Connexion persistante au Router (requête/réponse par position).
    let mut router = TcpStream::connect(router_addr).await?;
    let mut router_reader = FrameReader::new();

    // Interroge le Router avec une position et bloque jusqu'à la réponse. M3 : volontairement
    // sans timeout (boucle séquentielle ; le Router est local et fiable). Si le Router ne répond
    // pas, la boucle gèle pour tous les clients — à durcir en M4 (timeout + retry ou select!).
    async fn ask_router(
        router: &mut TcpStream,
        reader: &mut FrameReader,
        client_id: u64,
        x: f32,
        y: f32,
        z: f32,
    ) -> std::io::Result<String> {
        router
            .write_all(&encode_route_request(client_id, x, y, z))
            .await?;
        let mut buf = [0u8; 1024];
        loop {
            let n = router.read(&mut buf).await?;
            if n == 0 {
                return Err(std::io::Error::other("router fermé"));
            }
            reader.push(&buf[..n]);
            if let Some(body) = reader.next_frame() {
                if let Some(addr) = decode_route_reply(&body) {
                    return Ok(addr);
                }
            }
        }
    }

    // Écrit des frames vers le shard donné (le connecte si nécessaire).
    async fn write_to_shard(
        shards: &mut HashMap<String, ShardLink>,
        shard_addr: &str,
        frames: &[Vec<u8>],
    ) -> std::io::Result<()> {
        if !shards.contains_key(shard_addr) {
            let sock = TcpStream::connect(shard_addr).await?;
            shards.insert(
                shard_addr.to_string(),
                ShardLink {
                    sock,
                    reader: FrameReader::new(),
                },
            );
        }
        let link = shards.get_mut(shard_addr).unwrap();
        for f in frames {
            link.sock.write_all(f).await?;
        }
        Ok(())
    }

    let sock: SocketAddr = listen_addr.parse().expect("adresse GNS invalide");
    let mut client =
        GnsTransport::listen(sock.ip(), sock.port()).expect("GnsTransport::listen failed");
    tracing::info!("Gateway multi-shard : écoute GNS sur {listen_addr}, router {router_addr}");

    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    let mut sbuf = [0u8; 8192];
    loop {
        // 1) Lire tous les shards connectés (snapshots → clients).
        let addrs: Vec<String> = shards.keys().cloned().collect();
        for addr in addrs {
            // Lecture non bloquante : on tente un read court sur chaque shard.
            if let Ok(Ok(n)) = tokio::time::timeout(
                Duration::from_millis(1),
                shards.get_mut(&addr).unwrap().sock.read(&mut sbuf),
            )
            .await
            {
                if n == 0 {
                    continue;
                }
                let link = shards.get_mut(&addr).unwrap();
                link.reader.push(&sbuf[..n]);
                while let Some(body) = link.reader.next_frame() {
                    apply_shard_frame_to_client(&body, &mut client);
                }
            }
        }

        // 2) Tick : poll GNS et router chaque événement client via l'assigneur.
        ticker.tick().await;
        for ev in client.poll() {
            let cid = match &ev {
                TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
                TransportEvent::Message { from, .. } => *from,
            };
            let is_disconnect = matches!(ev, TransportEvent::Disconnected(_));
            // Traitement séquentiel : la 1re position d'un client est routée et assignée avant
            // l'événement suivant, donc jamais deux `NeedRoute` en vol pour le même client.
            match assigner.feed(ev) {
                AssignAction::Buffered => {}
                AssignAction::Forward { shard, frames } => {
                    let _ = write_to_shard(&mut shards, &shard, &frames).await;
                }
                AssignAction::NeedRoute { client_id, x, y, z } => {
                    if let Ok(shard) =
                        ask_router(&mut router, &mut router_reader, client_id, x, y, z).await
                    {
                        if let AssignAction::Forward { shard, frames } =
                            assigner.assign(client_id, shard)
                        {
                            let _ = write_to_shard(&mut shards, &shard, &frames).await;
                        }
                    }
                }
            }
            // Libère l'état local à la déconnexion (après que `feed` ait relayé le Disconnected
            // au shard si le client était assigné) — sinon le buffer du client fuirait.
            if is_disconnect {
                assigner.forget(cid);
            }
        }
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
        client.inject(TransportEvent::Message { from: 1, data: vec![4, 2] });

        let frames = drain_client_to_shard(&mut client);
        assert_eq!(frames.len(), 2);

        // Chaque frame est un ClientEvent décodable.
        let mut r = FrameReader::new();
        for f in &frames { r.push(f); }
        assert_eq!(decode_client_event(&r.next_frame().unwrap()), Some(TransportEvent::Connected(1)));
        assert_eq!(decode_client_event(&r.next_frame().unwrap()), Some(TransportEvent::Message { from: 1, data: vec![4, 2] }));
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
}
