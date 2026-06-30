//! Binaire Router. Usage : `cargo run -p server --bin router -- [listen_addr] [shard_addr]`
//! Défauts : 127.0.0.1:27040 (écoute) et 127.0.0.1:27030 (shard).

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();
    let listen = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:27040".to_string());
    let shard = std::env::args().nth(2).unwrap_or_else(|| "127.0.0.1:27030".to_string());
    server::router::router_main(&listen, shard).await
}
