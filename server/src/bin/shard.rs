//! Binaire Shard. Usage : `cargo run -p server --bin shard -- [addr]` (défaut 127.0.0.1:27030).

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:27030".to_string());
    server::shard_main(&addr).await
}
