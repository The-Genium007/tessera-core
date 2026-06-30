//! Le Router : décide quel shard possède un joueur. En M2, un seul shard → la décision
//! ignore la position et renvoie toujours l'unique shard. La logique de découpe (patterns
//! multi-shards) viendra en M3 sans toucher au Gateway ni au client.

use crate::framing::FrameReader;
use crate::internal_net::{decode_route_request, encode_route_reply};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Décision de routage. M2 : un shard unique, position ignorée.
pub fn route(shard_addr: &str, _client_id: u64, _x: f32, _y: f32, _z: f32) -> String {
    shard_addr.to_string()
}

/// Service Router : pour chaque `RouteRequest` reçu, répond un `RouteReply(shard_addr)`.
pub async fn router_main(listen_addr: &str, shard_addr: String) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;
    tracing::info!("Router en écoute sur {listen_addr} (shard unique = {shard_addr})");
    loop {
        let (mut sock, _peer) = listener.accept().await?;
        let shard_addr = shard_addr.clone();
        tokio::spawn(async move {
            let mut reader = FrameReader::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                reader.push(&buf[..n]);
                while let Some(body) = reader.next_frame() {
                    if let Some((cid, x, y, z)) = decode_route_request(&body) {
                        let reply = encode_route_reply(&route(&shard_addr, cid, x, y, z));
                        if sock.write_all(&reply).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_returns_the_single_shard_regardless_of_position() {
        // En M2, un seul shard : la position est ignorée.
        assert_eq!(route("127.0.0.1:27030", 1, 999.0, -999.0, 0.0), "127.0.0.1:27030");
        assert_eq!(route("10.0.0.5:27031", 42, 0.0, 0.0, 0.0), "10.0.0.5:27031");
    }
}
