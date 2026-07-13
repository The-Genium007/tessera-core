//! Binaire Shard (M0-M1 + manifeste A2 + métriques B). Usage :
//!   cargo run -p server --bin shard -- <listen_addr> --manifest <path/to/server.toml> --group-id <n> [--metrics-addr <addr>]
//! Le manifeste porte le rayon de visibilité AoI ([runtime.aoi]) — seule partie du manifeste
//! dont le Shard a besoin. `--metrics-addr` est optionnel (défaut `127.0.0.1:9100`).
//! `--group-id <n>` identifie, pour la traçabilité/logging uniquement, le groupe
//! `assignment_patterns[server_count][n]` de la tessellation d'autorité que ce process incarne
//! (un process Shard par groupe — voir §5.6). Le Shard reste un simulateur pur sans notion de
//! géométrie de zone : le routage/la frontière restent une responsabilité exclusive du Gateway
//! (`ShardTopology::locate`, câblé côté `bin/gateway.rs`).

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
            eprintln!("usage: shard <listen_addr> --manifest <path/to/server.toml> [--group-id <n>] [--metrics-addr <addr>]");
            std::process::exit(1);
        });
    let metrics_addr = v
        .iter()
        .position(|a| a == "--metrics-addr")
        .and_then(|i| v.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:9100".to_string());
    let group_id: usize = v
        .iter()
        .position(|a| a == "--group-id")
        .and_then(|i| v.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            eprintln!(
                "usage: shard <listen_addr> --manifest <path> --group-id <n> [--metrics-addr <addr>]"
            );
            std::process::exit(1);
        });

    let manifest =
        server::manifest::load(std::path::Path::new(manifest_path)).unwrap_or_else(|e| {
            eprintln!("manifeste invalide ({manifest_path}): {e}");
            std::process::exit(1);
        });
    let aoi_radius = server::manifest::shard_aoi_radius(&manifest);
    tracing::info!("Shard démarré pour le groupe {group_id} (écoute {addr})");
    server::shard_main(&addr, aoi_radius, &metrics_addr).await
}
