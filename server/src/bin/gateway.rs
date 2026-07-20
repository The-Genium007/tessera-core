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
    // Bannière de version, posée AVANT tout ce qui peut échouer (topologie, Postgres, bind UDP) :
    // quand le boot casse, elle est souvent la seule preuve de QUEL binaire tourne et de QUEL
    // modset il exige. Aurait tranché en une seconde l'incident Dokploy du 2026-07-14 (conteneur
    // réutilisé avec un mount périmé : on débuggait un vieux binaire sans le savoir).
    //
    // - server_version : injectée au build d'image (ARG TESSERA_RELEASE_VERSION -> option_env!,
    //   gravée à la compilation). PAS CARGO_PKG_VERSION : le crate est figé à 0.0.1 et ne suit pas
    //   les versions de release, il annoncerait « v0.0.1 » partout. `option_env!` (compile-time) et
    //   pas `std::env::var` (runtime) : même piège que TESSERASYNTH_ZITADEL_CLIENT_ID côté launcher
    //   — une var lue au runtime est vide en prod. Build hors CI => "dev-build (non publié)",
    //   un repli qui se voit au lieu d'un numéro faux.
    // - required_modset : version du modset CLIENT que ce serveur exige ; le launcher la lit via
    //   l'annuaire et résout le modset à installer. Les deux sortent en lockstep du même run de
    //   release.yml (--required-modset), donc server_version != required_modset sur un serveur
    //   publié = déploiement incohérent (image et bundle de deux runs différents).
    //
    // LIMITE ASSUMÉE — hollow re-tag : quand le côté serveur n'a pas changé, release.yml re-tague
    // l'image existante (`imagetools create`, même digest) au lieu de la rebuilder. server_version
    // reste donc celui du build d'origine et peut afficher moins que le tag de l'image. Ce n'est
    // pas un bug : le binaire EST bit-pour-bit celui de cette version-là, la bannière dit la vérité
    // sur le code qui tourne. required_modset, lui, est repatché à chaque run et reste exact.
    let server_version = option_env!("TESSERA_RELEASE_VERSION").filter(|v| !v.is_empty());
    let server_version_display = server_version
        .map(|v| format!("v{v}"))
        .unwrap_or_else(|| "dev-build (non publié)".to_string());
    tracing::info!(
        server_version = server_version.unwrap_or("dev-build"),
        required_modset = %manifest.identity.required_modset,
        channel = ?manifest.identity.channel,
        server_name = %manifest.identity.name,
        "Gateway TesseraSynth — serveur {} · modset client requis v{}",
        server_version_display,
        manifest.identity.required_modset,
    );

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
    if server::manifest::spawn_is_unconfigured(spawn) {
        tracing::warn!(
            "spawn non configuré ([0,0,0]) : les nouveaux joueurs naîtront à l'origine du monde — \
             régler `[runtime] spawn = [x, y, z]` dans le manifeste"
        );
    }
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

    // Store personnage (flux d'arrivée, palier 2) : construit UNIQUEMENT sur un serveur public,
    // depuis le même pool Postgres que `PostgresStore`. `None` sur un serveur privé (FileStore) —
    // le flux personnage y est alors inerte (voir `gateway_main`). Rempli dans la branche
    // `identity.public` ci-dessous, avant que `PostgresStore::new(pool)` ne consomme le pool.
    let mut character_store: Option<server::character_store::CharacterStore> = None;

    let store = if manifest.identity.public {
        // Dernier niveau de repli : composants séparés TESSERA_PG_* (voir docker-compose.yml,
        // service `postgres` colocalisé) — permet un déploiement standard sans jamais saisir
        // d'URL complète (ni sur Dokploy, ni dans le manifeste). N'est consulté que si aucune
        // URL complète n'a été fournie par les deux niveaux au-dessus.
        let postgres_url = postgres_url.or_else(|| {
            let host = std::env::var("TESSERA_PG_HOST").unwrap_or_default();
            let port = std::env::var("TESSERA_PG_PORT").unwrap_or_else(|_| "5432".to_string());
            let user = std::env::var("TESSERA_PG_USER").unwrap_or_else(|_| "tessera".to_string());
            let password = std::env::var("TESSERA_PG_PASSWORD").unwrap_or_default();
            let database =
                std::env::var("TESSERA_PG_DATABASE").unwrap_or_else(|_| "tessera".to_string());
            server::manifest::assemble_postgres_url_from_components(
                server::manifest::PostgresComponents {
                    host: &host,
                    port: &port,
                    user: &user,
                    password: &password,
                    database: &database,
                },
            )
        });
        let postgres_url = postgres_url.unwrap_or_else(|| {
            eprintln!(
                "manifeste invalide ({manifest_path}): identity.public = true nécessite runtime.postgres_url, {}, ou TESSERA_PG_HOST",
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
        // Exécutées à chaque boot, pas seulement au premier déploiement : idempotent (sqlx suit
        // les migrations déjà appliquées dans sa table `_sqlx_migrations`), donc sans effet sur
        // un Postgres déjà à jour. Nécessaire pour qu'un Postgres neuf (colocalisé, volume vide
        // au premier démarrage) obtienne son schéma sans étape manuelle — voir server/README.md.
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .unwrap_or_else(|e| {
                eprintln!("migrations Postgres échouées ({postgres_url}): {e}");
                std::process::exit(1);
            });
        // Même pool Postgres que le `PostgresStore` (le `PgPool` est un handle clonable, backé par
        // un Arc — pas une nouvelle connexion). Alimente le flux d'arrivée : liste/creation/
        // sélection/suppression de personnages, table `characters` (migrée juste au-dessus).
        character_store = Some(server::character_store::CharacterStore::new(pool.clone()));
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

    // Attestation « serveur officiel » (spec 2026-07-16) : le JWT signé par le CMS, collé en env.
    // C'est le VRAI binaire de prod (tessera-gateway, cf. docker-compose.yml) : sans ce câblage,
    // `directory publish` appelle l'endpoint d'attestation interne, tombe sur connection-refused
    // (rien n'écoute) et cape chaque serveur à "community" pour toujours — la feature entière est
    // inerte en prod. Absente = ce serveur ne sera jamais republié "official". Jamais un
    // prérequis dur.
    //
    // Contrairement à main.rs (fn main() non-async, d'où sa gymnastique thread+Runtime dédiés),
    // ce main() est déjà `#[tokio::main]` : on `tokio::spawn` directement sur le runtime existant,
    // comme le refresh JWKS ci-dessus et `metrics::serve` dans gateway_main. Échec = log + on
    // continue vers gateway_main : l'attestation N'EST PAS un prérequis dur (à l'inverse de
    // Postgres/Redis qui `std::process::exit(1)`), elle doit se dégrader sans jamais bloquer le
    // démarrage du serveur de jeu ni paniquer.
    if let Ok(attestation) = std::env::var("TESSERA_OFFICIAL_ATTESTATION") {
        let listen_addr = std::env::var("TESSERA_INTERNAL_ATTESTATION_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:9090".to_string());
        match server::attestation_display::describe(&attestation) {
            Ok((sub, exp)) => {
                tracing::info!(slug = %sub, exp_epoch = exp, "attestation officielle active")
            }
            Err(e) => tracing::warn!(
                "TESSERA_OFFICIAL_ATTESTATION illisible : {e:?} \
                 (attendu : JWT 3 segments, payload base64url no-pad avec sub+exp)"
            ),
        }
        let token = attestation.clone();
        tokio::spawn(async move {
            if let Err(e) =
                server::internal_attestation_http::serve(&listen_addr, Some(token)).await
            {
                tracing::error!(error = %e, "serveur d'attestation interne arrêté");
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
        character_store,
    )
    .await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
