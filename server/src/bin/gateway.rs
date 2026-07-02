//! Binaire Gateway (handoff M4 + persistance + manifeste M6). Usage (feature gns) :
//!   cargo run -p server --features gns --bin gateway -- --manifest <path>
//! Le manifeste (voir server.example.toml) porte topologie/spawn/rayons/store/adresses — plus
//! aucun argument positionnel.

#[cfg(feature = "gns")]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();
    let v: Vec<String> = std::env::args().collect();
    let manifest_path = v
        .iter()
        .position(|a| a == "--manifest")
        .and_then(|i| v.get(i + 1))
        .unwrap_or_else(|| {
            eprintln!("usage: gateway --manifest <path/to/server.toml>");
            std::process::exit(1);
        });

    let manifest = server::manifest::load(std::path::Path::new(manifest_path)).unwrap_or_else(|e| {
        eprintln!("manifeste invalide ({manifest_path}): {e}");
        std::process::exit(1);
    });
    let (topology, radius, spawn, store_path) =
        server::manifest::to_runtime(&manifest).unwrap_or_else(|e| {
            eprintln!("manifeste invalide ({manifest_path}): {e}");
            std::process::exit(1);
        });
    let listen = manifest.runtime.gateway.listen_addr.clone();
    let store = server::persistence::FileStore::open(&store_path);
    server::gateway::gateway_main(&listen, topology, radius, store, spawn).await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
