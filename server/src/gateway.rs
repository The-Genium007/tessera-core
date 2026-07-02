//! Cœur de relai du Gateway : traduit les événements client (transport GNS) en frames
//! internes vers le Shard, et les `ServerSend` du Shard en envois client. Générique sur le
//! transport client → testable avec `InMemoryTransport`, branché sur `GnsTransport` en prod.

use crate::framing::FrameReader;
use crate::internal_net::{decode_server_send, event_to_client_event_frame};
use crate::transport::Transport;
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
pub async fn write_to_shard(
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
    let result: std::io::Result<()> = async {
        let link = shards.get_mut(shard_addr).unwrap();
        for f in frames {
            link.sock.write_all(f).await?;
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        shards.remove(shard_addr);
    }
    result
}

/// Lit tout ce qui est disponible sur chaque shard connecté (non bloquant, timeout court par
/// shard) et alimente `latest[client][shard_addr]` avec le dernier `ServerSend` reçu. Une
/// lecture EOF (`n == 0`) ou en erreur évacue l'entrée du shard concerné — connexion morte,
/// sera recréée au prochain `write_to_shard` pour cette adresse.
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
    for addr in dead {
        shards.remove(&addr);
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
    let mut residence: HashMap<u64, Option<[f32; 3]>> = HashMap::new();

    let sock: SocketAddr = listen_addr.parse().expect("adresse GNS invalide");
    let mut client =
        GnsTransport::listen(sock.ip(), sock.port()).expect("GnsTransport::listen failed");
    tracing::info!(
        "Gateway handoff : écoute GNS sur {listen_addr} ({} shards)",
        topology.shards.len()
    );

    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    loop {
        // 1) Lire chaque shard connecté (évacue et laisse reconnecter les connexions mortes).
        read_from_shards(&mut shards, &mut latest).await;

        // 2) Tick : événements clients → placement → charge/décharge/relai.
        ticker.tick().await;
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
                    last_pos.insert(cid, [x, y, z]);
                    let r = radius.radius_for(*ranks.get(&cid).unwrap_or(&Rank::Player));
                    placement = Some(topology.locate(x, y, r));
                }
            }

            for LoadAction::Forward { shard, frames } in loader.feed(ev, placement) {
                let _ = write_to_shard(&mut shards, &shard, &frames).await;
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
}
