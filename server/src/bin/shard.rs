//! Binaire Shard (M0-M1 + manifeste A2). Usage :
//!   cargo run -p server --bin shard -- <listen_addr> --manifest <path/to/server.toml>
//! Le manifeste (voir server.example.toml) porte le rayon de visibilité AoI
//! ([runtime.aoi]) — c'est la seule partie du manifeste dont le Shard a besoin.

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();
    let v: Vec<String> = std::env::args().collect();
    let addr = v
        .get(1)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:27030".to_string());
    let manifest_path = v
        .iter()
        .position(|a| a == "--manifest")
        .and_then(|i| v.get(i + 1))
        .unwrap_or_else(|| {
            eprintln!("usage: shard <listen_addr> --manifest <path/to/server.toml>");
            std::process::exit(1);
        });

    let manifest = server::manifest::load(std::path::Path::new(manifest_path)).unwrap_or_else(|e| {
        eprintln!("manifeste invalide ({manifest_path}): {e}");
        std::process::exit(1);
    });
    let aoi_radius = server::manifest::shard_aoi_radius(&manifest);
    server::shard_main(&addr, aoi_radius).await
}
