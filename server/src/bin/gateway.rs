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

    let manifest =
        server::manifest::load(std::path::Path::new(manifest_path)).unwrap_or_else(|e| {
            eprintln!("manifeste invalide ({manifest_path}): {e}");
            std::process::exit(1);
        });
    let (topology, radius, spawn, store_path) = server::manifest::to_runtime(&manifest)
        .unwrap_or_else(|e| {
            eprintln!("manifeste invalide ({manifest_path}): {e}");
            std::process::exit(1);
        });
    let listen = manifest.runtime.gateway.listen_addr.clone();
    let store = server::persistence::FileStore::open(&store_path);
    let admin_store = server::admin_store::AdminStore::open(
        std::path::Path::new(&store_path).with_file_name("permission_groups.json"),
        std::path::Path::new(&store_path).with_file_name("server_admins.json"),
    );
    let max_players = manifest.identity.max_players;

    // ZITADEL est l'IdP unique de la plateforme (auth.tesserasynth.net, cf. design
    // 2026-07-09 launcher-server-auth §1 : « un seul compte pour tout ») — pas un service
    // self-hosté par opérateur, donc pas (encore) un champ manifeste par serveur. À revisiter
    // si un opérateur demande un jour son propre IdP (hors périmètre connu à ce jour).
    const ZITADEL_JWKS_URL: &str = "https://auth.tesserasynth.net/oauth/v2/keys";
    const JWKS_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

    let identity_public = manifest.identity.public;
    let jwks_cache = std::sync::Arc::new(server::jwks::JwksCache::new());
    if identity_public {
        let jwks_cache = jwks_cache.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(JWKS_REFRESH_INTERVAL);
            loop {
                interval.tick().await;
                if let Err(e) = jwks_cache.refresh(ZITADEL_JWKS_URL).await {
                    tracing::warn!(
                        "refresh JWKS échoué ({ZITADEL_JWKS_URL}): {e:?} — clés en cache conservées"
                    );
                }
            }
        });
    }

    server::gateway::gateway_main(
        &listen,
        topology,
        radius,
        store,
        admin_store,
        spawn,
        max_players,
        jwks_cache,
        identity_public,
    )
    .await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
