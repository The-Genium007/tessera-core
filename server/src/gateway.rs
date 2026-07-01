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
) -> std::io::Result<()> {
    use crate::framing::FrameReader;
    use crate::gateway_routing::extract_position;
    use crate::gns_transport::GnsTransport;
    use crate::handoff::{LoadAction, Rank, ShardLoader};
    use crate::internal_net::decode_server_send;
    use crate::snapshot_merge::merge_snapshots;
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
    let mut loader = ShardLoader::new();
    // Dernier snapshot reçu de chaque shard, par client : latest[client][shard_addr] = payload.
    let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
    // Rang par client (stub M4 : tout le monde Player ; surchargeable quand l'auth existera).
    let ranks: HashMap<u64, Rank> = HashMap::new();

    // Connecte un shard si nécessaire et lui écrit des frames.
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
    tracing::info!(
        "Gateway handoff : écoute GNS sur {listen_addr} ({} shards)",
        topology.shards.len()
    );

    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    let mut sbuf = [0u8; 8192];
    loop {
        // 1) Lire chaque shard connecté : on mémorise le DERNIER snapshot par (client, shard).
        let addrs: Vec<String> = shards.keys().cloned().collect();
        for addr in addrs {
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
                    if let Some((cid, payload)) = decode_server_send(&body) {
                        latest.entry(cid).or_default().insert(addr.clone(), payload);
                    }
                }
            }
        }

        // 2) Tick : événements clients → placement → charge/décharge/relai.
        ticker.tick().await;
        for ev in client.poll() {
            let cid = match &ev {
                TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
                TransportEvent::Message { from, .. } => *from,
            };
            let is_disconnect = matches!(ev, TransportEvent::Disconnected(_));

            // Si c'est une position, calculer le placement via la topologie + rayon selon le rang.
            let placement = if let TransportEvent::Message { data, .. } = &ev {
                extract_position(data).map(|(x, y, _z)| {
                    let r = radius.radius_for(*ranks.get(&cid).unwrap_or(&Rank::Player));
                    topology.locate(x, y, r)
                })
            } else {
                None
            };

            for LoadAction::Forward { shard, frames } in loader.feed(ev, placement) {
                let _ = write_to_shard(&mut shards, &shard, &frames).await;
            }

            if is_disconnect {
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
}
