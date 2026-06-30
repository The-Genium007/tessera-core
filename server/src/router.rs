//! Le Router : décide quel shard possède un joueur. En M3, **2 shards** séparés par une frontière
//! sur l'axe X (Shard A = Watson sous le seuil, Shard B = reste au-dessus). Pas de handoff (M4).

use crate::framing::FrameReader;
use crate::internal_net::{decode_route_request, encode_route_reply};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Décision de routage par seuil sur X. `x < boundary_x` → shard A ; sinon shard B.
pub fn route_by_x(shard_a: &str, shard_b: &str, boundary_x: f32, x: f32) -> String {
    if x < boundary_x {
        shard_a.to_string()
    } else {
        shard_b.to_string()
    }
}

/// Service Router : répond à chaque `RouteRequest` un `RouteReply(shard)` choisi par position.
pub async fn router_main(
    listen_addr: &str,
    shard_a: String,
    shard_b: String,
    boundary_x: f32,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;
    tracing::info!(
        "Router en écoute sur {listen_addr} (A={shard_a} | B={shard_b}, frontière x={boundary_x})"
    );
    loop {
        let (mut sock, _peer) = listener.accept().await?;
        let (shard_a, shard_b) = (shard_a.clone(), shard_b.clone());
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
                    if let Some((_cid, x, _y, _z)) = decode_route_request(&body) {
                        let shard = route_by_x(&shard_a, &shard_b, boundary_x, x);
                        if sock.write_all(&encode_route_reply(&shard)).await.is_err() {
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
    fn route_by_x_picks_shard_a_below_boundary_and_b_at_or_above() {
        assert_eq!(route_by_x("A:1", "B:2", 1000.0, 999.0), "A:1"); // sous la frontière → A
        assert_eq!(route_by_x("A:1", "B:2", 1000.0, 1000.0), "B:2"); // à la frontière → B
        assert_eq!(route_by_x("A:1", "B:2", 1000.0, 5000.0), "B:2"); // au-dessus → B
    }
}
