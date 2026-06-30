//! Vérifie que le Router route par position : x sous la frontière → shard A, au-dessus → shard B.
use server::framing::FrameReader;
use server::internal_net::{decode_route_reply, encode_route_request};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn ask(sock: &mut TcpStream, x: f32) -> String {
    sock.write_all(&encode_route_request(1, x, 0.0, 0.0))
        .await
        .unwrap();
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf))
            .await
            .expect("timeout")
            .unwrap();
        reader.push(&buf[..n]);
        if let Some(body) = reader.next_frame() {
            return decode_route_reply(&body).expect("reply");
        }
    }
}

#[tokio::test]
async fn router_routes_by_position() {
    let listen = "127.0.0.1:27140";
    tokio::spawn(async move {
        server::router::router_main(listen, "shardA:1".into(), "shardB:2".into(), 1000.0)
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut sock = TcpStream::connect(listen).await.unwrap();
    assert_eq!(ask(&mut sock, 500.0).await, "shardA:1"); // sous la frontière → A
    assert_eq!(ask(&mut sock, 2000.0).await, "shardB:2"); // au-dessus → B
}
