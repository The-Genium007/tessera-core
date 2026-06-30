//! Vérifie que le Router répond un RouteReply avec l'adresse du shard.
use server::framing::FrameReader;
use server::internal_net::{decode_route_reply, encode_route_request};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn router_replies_with_shard_address() {
    let listen = "127.0.0.1:27140";
    let shard = "127.0.0.1:27030".to_string();
    tokio::spawn(async move { server::router::router_main(listen, shard).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut sock = TcpStream::connect(listen).await.unwrap();
    sock.write_all(&encode_route_request(1, 0.0, 0.0, 0.0)).await.unwrap();

    let mut reader = FrameReader::new();
    let mut buf = [0u8; 1024];
    let n = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf))
        .await.expect("timeout").unwrap();
    reader.push(&buf[..n]);
    let body = reader.next_frame().expect("une réponse");
    assert_eq!(decode_route_reply(&body), Some("127.0.0.1:27030".to_string()));
}
