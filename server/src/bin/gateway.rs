//! Binaire Gateway (handoff M4 + persistance). Usage (feature gns) :
//!   cargo run -p server --features gns --bin gateway -- [listen] [shard_a] [shard_b] [boundary_x] [radius] [gm_radius] [store_path]
//! Défauts : 0.0.0.0:27020 (GNS public) · A=127.0.0.1:27030 · B=127.0.0.1:27031 · x=1000 · r=25 · gm=75 · players.json
//! Topologie 2-shards : A = x<boundary, B = x>=boundary (Y plein). Valeurs destinées au fichier
//! serveur (M6) — en dur/args pour l'instant.

#[cfg(feature = "gns")]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use server::handoff::{Aabb, RadiusPolicy, ShardTopology, ShardZone};
    tracing_subscriber::fmt::init();
    let v: Vec<String> = std::env::args().collect();
    let listen = v
        .get(1)
        .cloned()
        .unwrap_or_else(|| "0.0.0.0:27020".to_string());
    let shard_a = v
        .get(2)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:27030".to_string());
    let shard_b = v
        .get(3)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:27031".to_string());
    let boundary_x: f32 = v.get(4).and_then(|s| s.parse().ok()).unwrap_or(1000.0);
    let base: f32 = v.get(5).and_then(|s| s.parse().ok()).unwrap_or(25.0);
    let gm: f32 = v.get(6).and_then(|s| s.parse().ok()).unwrap_or(75.0);
    let store_path = v
        .get(7)
        .cloned()
        .unwrap_or_else(|| "players.json".to_string());

    let topology = ShardTopology {
        shards: vec![
            ShardZone {
                addr: shard_a,
                zone: Aabb {
                    min_x: f32::NEG_INFINITY,
                    max_x: boundary_x,
                    min_y: f32::NEG_INFINITY,
                    max_y: f32::INFINITY,
                },
            },
            ShardZone {
                addr: shard_b,
                zone: Aabb {
                    min_x: boundary_x,
                    max_x: f32::INFINITY,
                    min_y: f32::NEG_INFINITY,
                    max_y: f32::INFINITY,
                },
            },
        ],
    };
    let radius = RadiusPolicy {
        base,
        moderator: (base + gm) / 2.0,
        game_master: gm,
    };
    // Store de persistance + point de spawn par défaut (Watson, devant l'appart de V).
    let store = server::persistence::FileStore::open(&store_path);
    let spawn = [2387.0_f32, -1295.0, 63.0];
    server::gateway::gateway_main(&listen, topology, radius, store, spawn).await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
