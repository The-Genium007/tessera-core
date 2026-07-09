//! Binaire Gateway (handoff M4 + persistance + manifeste M6). Usage (feature gns) :
//!   cargo run -p server --features gns --bin gateway -- --manifest <path>
//! Le manifeste (voir server.example.toml) porte topologie/spawn/rayons/store/adresses — plus
//! aucun argument positionnel.

// TODO(gap): plus de source de spawn point dans le manifeste depuis le remplacement du schéma
// de topologie (G3) — l'ancien default_entry/spawn_points vivait dans l'arbre de splits/shards
// disparu. Placeholder en dur en attendant une vraie stratégie de spawn (hors scope de ce
// chantier §5.6 câblage runtime ; à traiter séparément).
#[cfg(feature = "gns")]
const PLACEHOLDER_SPAWN: [f32; 3] = [0.0, 0.0, 0.0];

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

    let manifest_path_buf = std::path::Path::new(manifest_path);
    let manifest = server::manifest::load(manifest_path_buf).unwrap_or_else(|e| {
        eprintln!("manifeste invalide ({manifest_path}): {e}");
        std::process::exit(1);
    });
    let manifest_dir = manifest_path_buf.parent().unwrap_or_else(|| {
        eprintln!("manifeste invalide ({manifest_path}): chemin sans répertoire parent");
        std::process::exit(1);
    });
    let zones = server::manifest::load_authority_topology(&manifest.runtime.topology, manifest_dir)
        .unwrap_or_else(|e| {
            eprintln!("topologie d'autorité invalide ({manifest_path}): {e}");
            std::process::exit(1);
        });
    let topology = server::handoff::ShardTopology { shards: zones };
    let radius = server::handoff::RadiusPolicy {
        base: manifest.runtime.radius.base,
        moderator: manifest.runtime.radius.moderator,
        game_master: manifest.runtime.radius.game_master,
    };
    let store_path = manifest.runtime.store_path.clone();
    let spawn = PLACEHOLDER_SPAWN;
    let listen = manifest.runtime.gateway.listen_addr.clone();
    let store = server::persistence::FileStore::open(&store_path);
    let admin_store = server::admin_store::AdminStore::open(
        std::path::Path::new(&store_path).with_file_name("permission_groups.json"),
        std::path::Path::new(&store_path).with_file_name("server_admins.json"),
    );
    let max_players = manifest.identity.max_players;
    server::gateway::gateway_main(
        &listen,
        topology,
        radius,
        store,
        admin_store,
        spawn,
        max_players,
    )
    .await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
