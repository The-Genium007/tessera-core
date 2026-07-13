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
    let spawn = manifest.runtime.spawn;
    let listen = manifest.runtime.gateway.listen_addr.clone();

    // Bascule FileStore/PostgresStore selon identity.public (câblage runtime, design stockage
    // 2026-07-09) : un serveur privé (défaut) garde exactement le comportement historique
    // (FileStore local) ; un serveur public route vers Postgres, indexé par sub OIDC vérifié.
    // `TESSERA_POSTGRES_URL`/`TESSERA_REDIS_URL` (Dokploy) l'emportent sur le manifeste quand
    // présentes — voir `manifest::resolve_postgres_url`/`resolve_redis_url` — pour changer
    // d'instance sans rebuild d'image ni édition du manifeste monté.
    let postgres_url_env = std::env::var(server::manifest::TESSERA_POSTGRES_URL_ENV).ok();
    let postgres_url = server::manifest::resolve_postgres_url(
        manifest.runtime.postgres_url.as_deref(),
        postgres_url_env.as_deref(),
    );
    let redis_url_env = std::env::var(server::manifest::TESSERA_REDIS_URL_ENV).ok();
    let redis_url =
        server::manifest::resolve_redis_url(&manifest.runtime.redis_url, redis_url_env.as_deref());

    let store = if manifest.identity.public {
        let postgres_url = postgres_url.unwrap_or_else(|| {
            eprintln!(
                "manifeste invalide ({manifest_path}): identity.public = true nécessite runtime.postgres_url ou {}",
                server::manifest::TESSERA_POSTGRES_URL_ENV
            );
            std::process::exit(1);
        });
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&postgres_url)
            .await
            .unwrap_or_else(|e| {
                eprintln!("connexion Postgres échouée ({postgres_url}): {e}");
                std::process::exit(1);
            });
        server::player_store_impl::PlayerStoreImpl::Postgres {
            store: server::postgres_store::PostgresStore::new(pool),
            display_names: std::collections::HashMap::new(),
        }
    } else {
        server::player_store_impl::PlayerStoreImpl::File(server::persistence::FileStore::open(
            &store_path,
        ))
    };

    let hot_state = server::hot_state_cache::HotStateCache::connect(&redis_url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("connexion Redis (hot state) échouée ({redis_url}): {e:?}");
            std::process::exit(1);
        });

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
    let whitelist_enabled = manifest.runtime.whitelist;
    let whitelist_names: std::collections::HashSet<String> =
        manifest.runtime.whitelist_names.iter().cloned().collect();
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
        whitelist_enabled,
        whitelist_names,
        hot_state,
    )
    .await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
