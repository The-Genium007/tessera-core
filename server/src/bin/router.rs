//! Binaire Router. Usage : `cargo run -p server --bin router -- [listen] [shard_a] [shard_b] [boundary_x]`
//! Défauts : 127.0.0.1:27040 · A=127.0.0.1:27030 · B=127.0.0.1:27031 · boundary_x=1000.0

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();
    let a = std::env::args();
    let v: Vec<String> = a.collect();
    let listen = v
        .get(1)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:27040".to_string());
    let shard_a = v
        .get(2)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:27030".to_string());
    let shard_b = v
        .get(3)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:27031".to_string());
    let boundary_x: f32 = v.get(4).and_then(|s| s.parse().ok()).unwrap_or(1000.0);
    server::router::router_main(&listen, shard_a, shard_b, boundary_x).await
}
