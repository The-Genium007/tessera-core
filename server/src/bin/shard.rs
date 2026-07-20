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
            eprintln!("usage: shard <listen_addr> --manifest <path/to/server.toml> --group-id <n> [--metrics-addr <addr>]");
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

    let manifest_path_buf = std::path::Path::new(manifest_path);
    let manifest = server::manifest::load(manifest_path_buf).unwrap_or_else(|e| {
        eprintln!("manifeste invalide ({manifest_path}): {e}");
        std::process::exit(1);
    });
    let aoi_radius = server::manifest::shard_aoi_radius(&manifest);

    // Comportement historique préservé : `[runtime.population.target_density]` absent ou vide (le
    // défaut sûr, cf. manifest.rs) = pas de PNJ sur ce Shard, exactement comme avant l'existence de
    // cette fonctionnalité (fondation nav-indépendante, sous-projet PNJ, palier 2).
    let population = if manifest.runtime.population.target_density.is_empty() {
        None
    } else {
        // Résolu depuis le répertoire du manifeste, même convention que
        // `TopologyConfig::authority_artifact` (cf. `manifest.rs::load_authority_topology`) : le
        // catalogue PNJ est un fichier local versionné à côté du manifeste, pas un chemin absolu
        // codé en dur.
        let manifest_dir = manifest_path_buf.parent().unwrap_or_else(|| {
            eprintln!("manifeste invalide ({manifest_path}): chemin sans répertoire parent");
            std::process::exit(1);
        });
        let catalog_path = manifest_dir.join("npc-catalog.toml");
        let catalog = server::npc_catalog::load(&catalog_path).unwrap_or_else(|e| {
            eprintln!("catalogue PNJ invalide ({}): {e}", catalog_path.display());
            std::process::exit(1);
        });
        let director = server::population_director::PopulationDirector::new(
            manifest.runtime.population.target_density.clone(),
        );
        Some((catalog, director))
    };

    tracing::info!("Shard démarré pour le groupe {group_id} (écoute {addr})");
    server::shard_main(&addr, aoi_radius, &metrics_addr, population).await
}
