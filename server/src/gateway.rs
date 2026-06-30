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

/// Point d'entrée du Gateway : interroge le Router pour obtenir l'adresse du Shard, ouvre
/// l'écoute GNS publique pour les clients, puis relaie en boucle à 20 Hz entre le client GNS
/// et le Shard TCP interne.
#[cfg(feature = "gns")]
pub async fn gateway_main(listen_addr: &str, router_addr: &str) -> std::io::Result<()> {
    use crate::framing::FrameReader;
    use crate::gns_transport::GnsTransport;
    use crate::internal_net::{decode_route_reply, encode_route_request};
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    // 1) Demander au Router l'adresse du shard (M2 : position factice, un seul shard).
    let mut router = TcpStream::connect(router_addr).await?;
    router.write_all(&encode_route_request(0, 0.0, 0.0, 0.0)).await?;
    let mut rbuf = [0u8; 1024];
    let mut rreader = FrameReader::new();
    let shard_addr = loop {
        let n = router.read(&mut rbuf).await?;
        if n == 0 {
            return Err(std::io::Error::other("router fermé"));
        }
        rreader.push(&rbuf[..n]);
        if let Some(body) = rreader.next_frame() {
            if let Some(addr) = decode_route_reply(&body) {
                break addr;
            }
        }
    };
    tracing::info!("Gateway : shard attribué = {shard_addr}");

    // 2) Connexion TCP au Shard.
    let mut shard = TcpStream::connect(&shard_addr).await?;

    // 3) Transport GNS côté client.
    let sock: SocketAddr = listen_addr.parse().expect("adresse GNS invalide");
    let mut client = GnsTransport::listen(sock.ip(), sock.port())
        .expect("GnsTransport::listen failed");
    tracing::info!("Gateway : écoute GNS sur {listen_addr}, relai vers le shard {shard_addr}");

    // 4) Boucle de relai à 20 Hz : client GNS ⇄ shard TCP.
    let mut sreader = FrameReader::new();
    let mut sbuf = [0u8; 8192];
    let mut ticker = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            read = shard.read(&mut sbuf) => {
                let n = match read { Ok(0) | Err(_) => break, Ok(n) => n };
                sreader.push(&sbuf[..n]);
                while let Some(body) = sreader.next_frame() {
                    apply_shard_frame_to_client(&body, &mut client);
                }
            }
            _ = ticker.tick() => {
                for frame in drain_client_to_shard(&mut client) {
                    if shard.write_all(&frame).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
    Ok(())
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
