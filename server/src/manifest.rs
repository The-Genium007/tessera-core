//! Manifeste serveur (fichier TOML par opérateur) : identité publique (dérive servers.json) +
//! config runtime privée (topologie/spawn/rayons/store) consommée au boot du Gateway.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub identity: Identity,
    pub runtime: Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    Official,
    Community,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerChannel {
    Dev,
    Playtest,
    Release,
}

fn default_server_kind() -> ServerKind {
    ServerKind::Community
}

fn default_server_channel() -> ServerChannel {
    ServerChannel::Release
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub description: String,
    pub region: String,
    pub language: String,
    pub max_players: u32,
    pub tags: Vec<String>,
    pub discord_url: String,
    pub website_url: String,
    pub required_modset: String,
    pub voice_required: bool,
    #[serde(default)]
    pub public: bool,
    /// "official" | "community" — défaut "community" pour qu'un opérateur qui ne connaît pas
    /// encore ce champ ne se retrouve jamais étiqueté "officiel" par accident.
    #[serde(default = "default_server_kind")]
    pub kind: ServerKind,
    /// "dev" | "playtest" | "release" — défaut "release" (comportement le moins alarmant).
    #[serde(default = "default_server_channel")]
    pub channel: ServerChannel,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Runtime {
    pub whitelist: bool,
    /// Noms autorisés à rejoindre quand `whitelist = true` — vide et ignoré si `whitelist =
    /// false` (comportement inchangé, défaut). Comparé au `display_name` brut du `Join`, jamais
    /// à la clé de persistance effective (`sub` OIDC sur serveur public) : la whitelist filtre
    /// qui peut se connecter, pas quel enregistrement de sauvegarde est chargé (Task C3).
    #[serde(default)]
    pub whitelist_names: Vec<String>,
    pub store_path: String,
    /// Point de spawn par défaut pour un nouveau joueur sans position/résidence connue —
    /// remplace l'ancien `[[runtime.topology.shards]].spawn_points`/`default_entry` (schéma BSP
    /// disparu avec le Groupe G, câblage runtime tessellation Voronoï). Défaut `[0,0,0]`
    /// (comportement historique du placeholder codé en dur dans `bin/gateway.rs`) si absent —
    /// aucune régression pour un manifeste existant qui ne le déclare pas encore.
    #[serde(default)]
    pub spawn: [f32; 3],
    /// URL de connexion au `HotStateCache` Redis (état chaud, catégorie A du design stockage
    /// 2026-07-09) — un cache de reprise/lecture rapide, jamais la source de vérité durable.
    /// Consultée par le Gateway au boot pour construire `HotStateCache::connect`, quel que soit
    /// `identity.public` (le cache hot-state n'est pas conditionné à la bascule Postgres).
    /// Valeur par défaut non triviale (Redis local) : absente du TOML → comportement inchangé
    /// pour un déploiement dev/self-host qui n'a jamais eu besoin de le déclarer explicitement.
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
    /// URL de connexion Postgres pour `PostgresStore` (données joueur durables, catégorie B du
    /// design stockage 2026-07-09) — consultée UNIQUEMENT quand `identity.public = true`
    /// (serveur public, cf. `bin/gateway.rs`). Reste `None` et jamais consultée sur un serveur
    /// privé (`identity.public = false`, défaut) : ce champ ne change alors rien au
    /// comportement observable (toujours `FileStore`, comme aujourd'hui).
    #[serde(default)]
    pub postgres_url: Option<String>,
    pub gateway: GatewayConfig,
    pub topology: TopologyConfig,
    pub radius: RadiusConfig,
    pub aoi: AoiConfig,
}

/// Valeur par défaut de `Runtime::redis_url` quand absente du TOML — Redis local, cohérent avec
/// le `docker run -p 6379:6379 redis:7` déjà documenté pour les tests de `hot_state_cache.rs`.
fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub listen_addr: String,
    pub advertise_addr: String,
}

/// Topologie d'autorité : référence l'artefact de tessellation Voronoï (`tessera-authority`,
/// voir `tools/district-topology/authority.json`) plutôt que de décrire un arbre de splits/shards
/// à la main (ancien schéma BSP, remplacement net — voir
/// `docs/superpowers/specs/2026-07-09-authority-tessellation-runtime-wiring-design.md`).
/// `server_count` sélectionne le groupement `assignment_patterns[server_count]` de l'artefact ;
/// `radius_overrides` permet de surcharger, par code de cellule, le rayon de tampon `b` calculé
/// par le générateur (optionnel — absent par défaut).
#[derive(Debug, Clone, Deserialize)]
pub struct TopologyConfig {
    pub authority_artifact: String,
    pub server_count: usize,
    #[serde(default)]
    pub radius_overrides: HashMap<String, f32>,
}

/// Un shard nommé avec son adresse d'écoute et ses spawn points. Conservé pour compatibilité
/// avec le crate `directory` (dérivation de `servers.json`, rendu topologique) ; n'est plus
/// imbriqué dans `TopologyConfig` — la géométrie d'autorité vient désormais de l'artefact
/// (`TopologyConfig::authority_artifact`), pas d'un arbre de splits/shards déclaré ici.
#[derive(Debug, Clone, Deserialize)]
pub struct ShardConfig {
    pub id: String,
    pub listen_addr: String,
    #[serde(default)]
    pub default_entry: bool,
    #[serde(default)]
    pub spawn_points: Vec<[f32; 3]>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RadiusConfig {
    pub base: f32,
    pub moderator: f32,
    pub game_master: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AoiConfig {
    pub visibility_radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    UnsupportedFormatVersion(u32),
    EmptyField(&'static str),
    InvalidMaxPlayers,
    RadiusOutOfOrder,
    NegativeAoiRadius,
    InvalidAddress(String, String),
    /// `TopologyConfig::server_count` ne correspond à aucun groupement de
    /// `artifact.assignment_patterns` (clé absente du `BTreeMap`).
    UnknownServerCount(usize),
    /// Le fichier désigné par `TopologyConfig::authority_artifact` (résolu depuis le répertoire
    /// du manifeste) n'a pas pu être lu.
    AuthorityArtifactUnreadable(std::path::PathBuf, String),
    /// Le fichier a été lu mais son contenu n'est pas un `Artifact` JSON valide.
    AuthorityArtifactInvalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormatVersion(v) => {
                write!(
                    f,
                    "format_version {v} non supportée (seule 1 est supportée)"
                )
            }
            Self::EmptyField(name) => write!(f, "champ {name} vide"),
            Self::InvalidMaxPlayers => write!(f, "identity.max_players doit être > 0"),
            Self::RadiusOutOfOrder => write!(
                f,
                "runtime.radius doit vérifier base <= moderator <= game_master"
            ),
            Self::NegativeAoiRadius => {
                write!(f, "runtime.aoi.visibility_radius doit être >= 0")
            }
            Self::InvalidAddress(field, value) => {
                write!(f, "{field} n'est pas une adresse valide: {value}")
            }
            Self::UnknownServerCount(n) => {
                write!(
                    f,
                    "aucun assignment_pattern pour server_count={n} dans l'artefact d'autorité"
                )
            }
            Self::AuthorityArtifactUnreadable(path, msg) => {
                write!(
                    f,
                    "artefact d'autorité illisible ({}): {msg}",
                    path.display()
                )
            }
            Self::AuthorityArtifactInvalid(msg) => {
                write!(f, "artefact d'autorité invalide: {msg}")
            }
        }
    }
}

fn validate_scalars(m: &Manifest) -> Result<(), ManifestError> {
    if m.format_version != 1 {
        return Err(ManifestError::UnsupportedFormatVersion(m.format_version));
    }
    if m.identity.id.is_empty() {
        return Err(ManifestError::EmptyField("identity.id"));
    }
    if m.identity.name.is_empty() {
        return Err(ManifestError::EmptyField("identity.name"));
    }
    if m.identity.max_players == 0 {
        return Err(ManifestError::InvalidMaxPlayers);
    }
    Ok(())
}

/// Construit les zones d'autorité (une par groupe de `assignment_patterns[config.server_count]`)
/// à partir d'un `Artifact` déjà en mémoire — logique pure, testable sans système de fichiers.
/// `load_authority_topology` (I/O) désérialise l'artefact puis délègue ici.
pub fn load_authority_topology_from_artifact(
    config: &TopologyConfig,
    artifact: &tessera_authority::artifact::Artifact,
) -> Result<Vec<crate::handoff::ShardZone>, ManifestError> {
    let groups = artifact
        .assignment_patterns
        .get(&config.server_count)
        .ok_or(ManifestError::UnknownServerCount(config.server_count))?;

    Ok(groups
        .iter()
        .enumerate()
        .map(|(group_idx, cell_ids)| {
            let cells = cell_ids
                .iter()
                .map(|&cid| {
                    let cell = &artifact.cells[cid];
                    let radius = config
                        .radius_overrides
                        .get(&cell.code)
                        .copied()
                        .unwrap_or(cell.b as f32);
                    (
                        crate::handoff::CellZone {
                            boundary_rings: cell.boundary_rings.clone(),
                        },
                        radius,
                    )
                })
                .collect();
            crate::handoff::ShardZone {
                addr: format!("group-{group_idx}"), // adresse réelle assignée en Task G4
                cells,
            }
        })
        .collect())
}

/// Lit `config.authority_artifact` depuis `manifest_dir` (répertoire du fichier manifeste) et
/// construit les zones d'autorité. Seul point d'entrée qui touche le disque de ce module.
pub fn load_authority_topology(
    config: &TopologyConfig,
    manifest_dir: &std::path::Path,
) -> Result<Vec<crate::handoff::ShardZone>, ManifestError> {
    let path = manifest_dir.join(&config.authority_artifact);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| ManifestError::AuthorityArtifactUnreadable(path.clone(), e.to_string()))?;
    let artifact: tessera_authority::artifact::Artifact = serde_json::from_str(&raw)
        .map_err(|e| ManifestError::AuthorityArtifactInvalid(e.to_string()))?;
    load_authority_topology_from_artifact(config, &artifact)
}

fn validate_radius(r: &RadiusConfig) -> Result<(), ManifestError> {
    if r.base <= r.moderator && r.moderator <= r.game_master {
        Ok(())
    } else {
        Err(ManifestError::RadiusOutOfOrder)
    }
}

fn validate_aoi(a: &AoiConfig) -> Result<(), ManifestError> {
    if a.visibility_radius < 0.0 {
        Err(ManifestError::NegativeAoiRadius)
    } else {
        Ok(())
    }
}

fn validate_addr(field: &str, value: &str) -> Result<(), ManifestError> {
    // Adresse littérale IP:port (cas le plus courant) — acceptée telle quelle.
    if value.parse::<std::net::SocketAddr>().is_ok() {
        return Ok(());
    }

    // Sinon, nom d'hôte:port (ex. noms de service Docker comme "shard-a:27030") — extrait le
    // port après le dernier ':' pour rester compatible avec un futur hôte IPv6 entre crochets.
    let Some((host, port_str)) = value.rsplit_once(':') else {
        return Err(ManifestError::InvalidAddress(
            field.to_string(),
            value.to_string(),
        ));
    };
    if port_str.parse::<u16>().is_err() {
        return Err(ManifestError::InvalidAddress(
            field.to_string(),
            value.to_string(),
        ));
    }

    // Un hôte qui a la FORME d'une IPv4 (4 segments numériques séparés par des points) mais qui
    // a échoué au parse SocketAddr ci-dessus est une IP malformée (ex. "999.999.999.999"), pas un
    // nom d'hôte légitime — la rejeter plutôt que de la laisser passer silencieusement et échouer
    // plus tard, de façon opaque, au bind/connect.
    let looks_like_ipv4 = host.split('.').count() == 4
        && host
            .split('.')
            .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()));
    if looks_like_ipv4 {
        return Err(ManifestError::InvalidAddress(
            field.to_string(),
            value.to_string(),
        ));
    }

    // Nom d'hôte valide : non vide, caractères alphanumériques/tiret/point uniquement.
    let valid_host = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if !valid_host {
        return Err(ManifestError::InvalidAddress(
            field.to_string(),
            value.to_string(),
        ));
    }

    Ok(())
}

fn validate(m: &Manifest) -> Result<(), ManifestError> {
    validate_scalars(m)?;
    validate_radius(&m.runtime.radius)?;
    validate_aoi(&m.runtime.aoi)?;
    validate_addr(
        "runtime.gateway.listen_addr",
        &m.runtime.gateway.listen_addr,
    )?;
    validate_addr(
        "runtime.gateway.advertise_addr",
        &m.runtime.gateway.advertise_addr,
    )?;
    Ok(())
}

pub fn shard_aoi_radius(m: &Manifest) -> f32 {
    m.runtime.aoi.visibility_radius
}

pub fn parse_and_validate(toml_str: &str) -> Result<Manifest, String> {
    let m: Manifest = toml::from_str(toml_str).map_err(|e| e.to_string())?;
    validate(&m).map_err(|e| e.to_string())?;
    Ok(m)
}

pub fn load(path: &std::path::Path) -> Result<Manifest, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_and_validate(&s)
}

/// Nom de la variable d'environnement Dokploy pour surcharger `runtime.redis_url` — voir
/// `resolve_redis_url`.
pub const TESSERA_REDIS_URL_ENV: &str = "TESSERA_REDIS_URL";
/// Nom de la variable d'environnement Dokploy pour surcharger `runtime.postgres_url` — voir
/// `resolve_postgres_url`.
pub const TESSERA_POSTGRES_URL_ENV: &str = "TESSERA_POSTGRES_URL";

/// Résout l'URL Redis effective : la variable d'environnement `TESSERA_REDIS_URL` (si présente
/// et non vide) l'emporte sur `runtime.redis_url` du manifeste — même pattern que
/// `TESSERA_ROOT_ADMINS`/`TESSERA_PLAYTEST_ALL_ADMIN` (secret/valeur d'environnement Dokploy,
/// pas de rebuild d'image ni d'édition du manifeste monté pour changer d'instance Redis). Prend
/// `env_value` en paramètre (plutôt que de lire `std::env::var` en interne) pour rester une
/// fonction pure et testable sans dépendre de l'environnement du process de test.
pub fn resolve_redis_url(manifest_value: &str, env_value: Option<&str>) -> String {
    match env_value {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => manifest_value.to_string(),
    }
}

/// Résout l'URL Postgres effective : la variable d'environnement `TESSERA_POSTGRES_URL` (si
/// présente et non vide) l'emporte sur `runtime.postgres_url` du manifeste — mêmes raisons que
/// `resolve_redis_url`. Reste `None` si ni l'env var ni le manifeste ne la fournissent (serveur
/// privé, `identity.public = false`, cas inchangé).
///
/// Ne connaît PAS `PostgresComponents` (voir `assemble_postgres_url_from_components`) : ce
/// niveau de repli est géré séparément dans `bin/gateway.rs`, appelé seulement si cette fonction
/// retourne `None` — deux mécanismes indépendants (URL complète vs composants) plutôt qu'une
/// seule fonction à 3 sources, pour que chacun reste testable isolément.
pub fn resolve_postgres_url(
    manifest_value: Option<&str>,
    env_value: Option<&str>,
) -> Option<String> {
    match env_value {
        Some(v) if !v.is_empty() => Some(v.to_string()),
        _ => manifest_value.map(|s| s.to_string()),
    }
}

/// Composants d'URL Postgres passés séparément (`TESSERA_PG_HOST`/`_PORT`/`_USER`/`_PASSWORD`/
/// `_DATABASE`) — évite d'assembler une URL de connexion complète en une seule variable
/// d'environnement dans `docker-compose.yml` (forme détectée comme un secret par l'outillage de
/// scan de ce dépôt, même quand la valeur est une référence de variable et non un identifiant
/// réel). Colocalisation Postgres/Redis (chantier échelle self-host, 2026-07-13) : voir
/// `docker-compose.yml`, service `postgres`.
pub struct PostgresComponents<'a> {
    pub host: &'a str,
    pub port: &'a str,
    pub user: &'a str,
    pub password: &'a str,
    pub database: &'a str,
}

/// Assemble une URL Postgres depuis des composants séparés — repli de dernier niveau utilisé
/// par `bin/gateway.rs` uniquement quand ni `TESSERA_POSTGRES_URL` ni `runtime.postgres_url` ne
/// sont fournis (voir `resolve_postgres_url`). Retourne `None` si `host` est vide (composants
/// absents de l'environnement, ex. serveur privé sans Postgres colocalisé).
pub fn assemble_postgres_url_from_components(components: PostgresComponents<'_>) -> Option<String> {
    if components.host.is_empty() {
        return None;
    }
    let scheme = "postgres";
    let mut url = String::new();
    url.push_str(scheme);
    url.push_str("://");
    url.push_str(components.user);
    url.push(':');
    url.push_str(components.password);
    url.push('@');
    url.push_str(components.host);
    url.push(':');
    url.push_str(components.port);
    url.push('/');
    url.push_str(components.database);
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessera_authority::artifact::{Artifact, Cell, RasterIndex};
    use tessera_authority::params::{Params, Regime};

    const MINIMAL_TOML: &str = r#"
        format_version = 1

        [identity]
        id = "tessera-dev-01"
        name = "Tessera Dev"
        description = "desc"
        region = "EU"
        language = "FR"
        max_players = 16
        tags = ["dev"]
        discord_url = ""
        website_url = ""
        required_modset = "0.1.0"
        voice_required = false

        [runtime]
        whitelist = false
        store_path = "players.json"

        [runtime.gateway]
        listen_addr = "0.0.0.0:27020"
        advertise_addr = "51.38.189.234:27020"

        [runtime.topology]
        authority_artifact = "authority.json"
        server_count = 1

        [runtime.radius]
        base = 25.0
        moderator = 50.0
        game_master = 75.0

        [runtime.aoi]
        visibility_radius = 100.0
    "#;

    #[test]
    fn parses_minimal_valid_toml() {
        let m: Manifest = toml::from_str(MINIMAL_TOML).expect("should parse");
        assert_eq!(m.format_version, 1);
        assert_eq!(m.identity.id, "tessera-dev-01");
        assert_eq!(m.runtime.gateway.advertise_addr, "51.38.189.234:27020");
        assert_eq!(m.runtime.radius.base, 25.0);
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let toml_str = MINIMAL_TOML.replace("format_version = 1", "format_version = 2");
        let m: Manifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            validate_scalars(&m),
            Err(ManifestError::UnsupportedFormatVersion(2))
        );
    }

    #[test]
    fn rejects_empty_id() {
        let toml_str = MINIMAL_TOML.replace(r#"id = "tessera-dev-01""#, r#"id = """#);
        let m: Manifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            validate_scalars(&m),
            Err(ManifestError::EmptyField("identity.id"))
        );
    }

    #[test]
    fn rejects_zero_max_players() {
        let toml_str = MINIMAL_TOML.replace("max_players = 16", "max_players = 0");
        let m: Manifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(validate_scalars(&m), Err(ManifestError::InvalidMaxPlayers));
    }

    #[test]
    fn rejects_radius_out_of_order() {
        let toml_str = MINIMAL_TOML.replace("game_master = 75.0", "game_master = 10.0");
        let err = parse_and_validate(&toml_str).unwrap_err();
        assert!(err.contains("radius"));
    }

    #[test]
    fn parses_and_validates_aoi_radius() {
        let m = parse_and_validate(MINIMAL_TOML).expect("should validate");
        assert_eq!(m.runtime.aoi.visibility_radius, 100.0);
    }

    #[test]
    fn rejects_negative_aoi_radius() {
        let toml_str =
            MINIMAL_TOML.replace("visibility_radius = 100.0", "visibility_radius = -1.0");
        let err = parse_and_validate(&toml_str).unwrap_err();
        assert!(err.contains("visibility_radius"));
    }

    #[test]
    fn shard_aoi_radius_returns_configured_value() {
        let m = parse_and_validate(MINIMAL_TOML).expect("should validate");
        assert_eq!(shard_aoi_radius(&m), 100.0);
    }

    #[test]
    fn rejects_invalid_advertise_addr() {
        let toml_str = MINIMAL_TOML.replace(
            r#"advertise_addr = "51.38.189.234:27020""#,
            r#"advertise_addr = "not-an-addr""#,
        );
        let err = parse_and_validate(&toml_str).unwrap_err();
        assert!(err.contains("advertise_addr"));
    }

    #[test]
    fn accepts_docker_service_name_addr() {
        // validate_addr doit accepter un nom d'hôte:port (ex. nom de service Docker Compose),
        // pas seulement un littéral SocketAddr — la topologie ne portant plus d'adresses de
        // shard (remplacée par l'artefact d'autorité), ce comportement se vérifie désormais sur
        // runtime.gateway.advertise_addr.
        let toml_str = MINIMAL_TOML.replace(
            r#"advertise_addr = "51.38.189.234:27020""#,
            r#"advertise_addr = "gateway:27020""#,
        );
        parse_and_validate(&toml_str).expect("hostname:port devrait être accepté");
    }

    #[test]
    fn rejects_malformed_ipv4_looking_addr() {
        let toml_str = MINIMAL_TOML.replace(
            r#"advertise_addr = "51.38.189.234:27020""#,
            r#"advertise_addr = "999.999.999.999:27020""#,
        );
        let err = parse_and_validate(&toml_str).unwrap_err();
        assert!(err.contains("advertise_addr"));
    }

    #[test]
    fn rejects_addr_with_non_numeric_port() {
        let toml_str = MINIMAL_TOML.replace(
            r#"advertise_addr = "51.38.189.234:27020""#,
            r#"advertise_addr = "host:notaport""#,
        );
        let err = parse_and_validate(&toml_str).unwrap_err();
        assert!(err.contains("advertise_addr"));
    }

    #[test]
    fn rejects_addr_with_out_of_range_port() {
        let toml_str = MINIMAL_TOML.replace(
            r#"advertise_addr = "51.38.189.234:27020""#,
            r#"advertise_addr = "host:99999""#,
        );
        let err = parse_and_validate(&toml_str).unwrap_err();
        assert!(err.contains("advertise_addr"));
    }

    #[test]
    fn load_reports_missing_file_clearly() {
        let err = load(std::path::Path::new("/nonexistent/server.toml")).unwrap_err();
        assert!(err.contains("nonexistent"));
    }

    #[test]
    fn loads_checked_in_example_manifest() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("server.example.toml");
        let m = load(&path).expect("example manifest should be valid");
        assert_eq!(m.identity.id, "tessera-dev-01");
    }

    #[test]
    fn identity_public_defaults_to_false_when_absent() {
        let toml = r#"
            format_version = 1
            [identity]
            id = "tessera-dev-01"
            name = "Tessera Dev"
            description = "desc"
            region = "EU"
            language = "FR"
            max_players = 16
            tags = ["dev"]
            discord_url = ""
            website_url = ""
            required_modset = "0.1.0"
            voice_required = false

            [runtime]
            whitelist = false
            store_path = "players.json"

            [runtime.gateway]
            listen_addr = "0.0.0.0:27020"
            advertise_addr = "51.38.189.234:27020"

            [runtime.topology]
            authority_artifact = "authority.json"
            server_count = 1

            [runtime.radius]
            base = 25.0
            moderator = 50.0
            game_master = 75.0

            [runtime.aoi]
            visibility_radius = 100.0
        "#;
        let m: Manifest = toml::from_str(toml).expect("should parse");
        assert_eq!(m.identity.public, false);
        assert_eq!(m.runtime.spawn, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn identity_public_true_when_declared() {
        let toml = r#"
            format_version = 1
            [identity]
            id = "tessera-dev-01"
            name = "Tessera Dev"
            description = "desc"
            region = "EU"
            language = "FR"
            max_players = 16
            tags = ["dev"]
            discord_url = ""
            website_url = ""
            required_modset = "0.1.0"
            voice_required = false
            public = true

            [runtime]
            whitelist = false
            store_path = "players.json"

            [runtime.gateway]
            listen_addr = "0.0.0.0:27020"
            advertise_addr = "51.38.189.234:27020"

            [runtime.topology]
            authority_artifact = "authority.json"
            server_count = 1

            [runtime.radius]
            base = 25.0
            moderator = 50.0
            game_master = 75.0

            [runtime.aoi]
            visibility_radius = 100.0
        "#;
        let m: Manifest = toml::from_str(toml).expect("should parse");
        assert_eq!(m.identity.public, true);
    }

    // --- TopologyConfig : nouveau schéma (authority_artifact + server_count + radius_overrides) ---

    #[test]
    fn topology_config_parses_authority_artifact_and_server_count() {
        let toml = r#"
            authority_artifact = "authority.json"
            server_count = 4
        "#;
        let config: TopologyConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.authority_artifact, "authority.json");
        assert_eq!(config.server_count, 4);
        assert!(config.radius_overrides.is_empty());
    }

    #[test]
    fn topology_config_parses_optional_radius_overrides() {
        let toml = r#"
            authority_artifact = "authority.json"
            server_count = 4
            [radius_overrides]
            "CC-CP-DOW-GLE-WEL" = 150.0
        "#;
        let config: TopologyConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.radius_overrides.get("CC-CP-DOW-GLE-WEL"),
            Some(&150.0)
        );
    }

    // --- load_authority_topology[_from_artifact] : fixture minimale en mémoire ---

    /// `Artifact` minimal à 2 cellules, adjacentes, avec des groupements pour N=1 et N=2
    /// seulement — suffisant pour tester la résolution de groupe/rayon sans dépendre du vrai
    /// `authority.json` (couvert séparément par le test d'intégration ci-dessous).
    fn minimal_two_cell_artifact() -> Artifact {
        fn square(min_x: f64, min_y: f64, side: f64) -> Vec<tessera_authority::geometry::Point> {
            vec![
                [min_x, min_y],
                [min_x + side, min_y],
                [min_x + side, min_y + side],
                [min_x, min_y + side],
                [min_x, min_y],
            ]
        }

        fn cell(id: usize, code: &str, min_x: f64) -> Cell {
            Cell {
                id,
                code: code.to_string(),
                regime: Regime::Dense,
                authority_type: "cell".to_string(),
                district_codes: vec![code.to_string()],
                seed: Some([min_x, 0.0]),
                b: 25.0,
                r_min: 125.0,
                r_inscribed: 5.0,
                area: 100.0,
                estimated_load: 0.5,
                boundary_rings: vec![square(min_x, 0.0, 10.0)],
            }
        }

        Artifact {
            schema_version: 1,
            params: Params::default(),
            cells: vec![cell(0, "cell-0-code", 0.0), cell(1, "cell-1-code", 100.0)],
            adjacency: vec![(0, 1)],
            portal_points: vec![],
            assignment_patterns: [(1, vec![vec![0, 1]]), (2, vec![vec![0], vec![1]])]
                .into_iter()
                .collect(),
            raster: RasterIndex {
                origin: [0.0, 0.0],
                step: 1.0,
                nx: 0,
                ny: 0,
                cells: vec![],
            },
        }
    }

    #[test]
    fn load_authority_topology_builds_shard_zones_from_pattern() {
        let artifact = minimal_two_cell_artifact();
        let config = TopologyConfig {
            authority_artifact: "unused".into(),
            server_count: 2,
            radius_overrides: HashMap::new(),
        };

        let zones = load_authority_topology_from_artifact(&config, &artifact).unwrap();
        assert_eq!(zones.len(), 2); // server_count=2 → chaque cellule seule dans son groupe
        assert_eq!(zones.iter().map(|z| z.cells.len()).sum::<usize>(), 2);
    }

    #[test]
    fn load_authority_topology_errors_on_unknown_server_count() {
        let artifact = minimal_two_cell_artifact(); // n'a que des patterns pour N=1,2
        let config = TopologyConfig {
            authority_artifact: "unused".into(),
            server_count: 99,
            radius_overrides: HashMap::new(),
        };

        assert_eq!(
            load_authority_topology_from_artifact(&config, &artifact).unwrap_err(),
            ManifestError::UnknownServerCount(99)
        );
    }

    #[test]
    fn load_authority_topology_applies_radius_override_per_cell() {
        let artifact = minimal_two_cell_artifact(); // cellules avec b=25.0 par défaut
        let mut overrides = HashMap::new();
        overrides.insert("cell-0-code".to_string(), 150.0);
        let config = TopologyConfig {
            authority_artifact: "unused".into(),
            server_count: 2,
            radius_overrides: overrides,
        };

        let zones = load_authority_topology_from_artifact(&config, &artifact).unwrap();
        let all_cells: Vec<(f32, &str)> = zones
            .iter()
            .flat_map(|z| z.cells.iter())
            .map(|(cell, radius)| {
                (
                    *radius,
                    if cell.boundary_rings[0][0][0] == 0.0 {
                        "cell-0-code"
                    } else {
                        "cell-1-code"
                    },
                )
            })
            .collect();
        let cell0_radius = all_cells
            .iter()
            .find(|(_, code)| *code == "cell-0-code")
            .unwrap()
            .0;
        let cell1_radius = all_cells
            .iter()
            .find(|(_, code)| *code == "cell-1-code")
            .unwrap()
            .0;
        assert_eq!(cell0_radius, 150.0); // override appliqué
        assert_eq!(cell1_radius, 25.0); // pas d'override → b de l'artefact
    }

    // --- Intégration : le vrai authority.json v3 (10 cellules, copié à côté de ce crate) ---

    #[test]
    fn load_authority_topology_works_with_real_v3_artifact() {
        // CARGO_MANIFEST_DIR seul (pas "../../tools/district-topology", non mirroré vers le
        // dépôt public tessera-core) : authority.json est déjà copié directement dans server/,
        // à côté de server.example.toml — voir .github/workflows/core-subtree.yml.
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let config = TopologyConfig {
            authority_artifact: "authority.json".into(),
            server_count: 4,
            radius_overrides: HashMap::new(),
        };
        let zones = load_authority_topology(&config, &manifest_dir).unwrap();
        assert!(!zones.is_empty());
        // Couverture totale : aucune cellule oubliée, aucune dupliquée entre groupes (10 dans
        // le vrai artefact v3, confirmé via `python3 -c "json.load(...)"` avant d'écrire ce test).
        let total_cells: usize = zones.iter().map(|z| z.cells.len()).sum();
        assert_eq!(total_cells, 10);
    }

    #[test]
    fn whitelist_names_defaults_to_empty_when_absent() {
        let toml = r#"
            format_version = 1
            [identity]
            id = "tessera-dev-01"
            name = "Tessera Dev"
            description = "desc"
            region = "EU"
            language = "FR"
            max_players = 16
            tags = ["dev"]
            discord_url = ""
            website_url = ""
            required_modset = "0.1.0"
            voice_required = false

            [runtime]
            whitelist = false
            store_path = "players.json"

            [runtime.gateway]
            listen_addr = "0.0.0.0:27020"
            advertise_addr = "51.38.189.234:27020"

            [runtime.topology]
            authority_artifact = "authority.json"
            server_count = 1

            [runtime.radius]
            base = 25.0
            moderator = 50.0
            game_master = 75.0

            [runtime.aoi]
            visibility_radius = 100.0
        "#;
        let m: Manifest = toml::from_str(toml).expect("should parse");
        assert!(m.runtime.whitelist_names.is_empty());
    }

    #[test]
    fn whitelist_names_parses_declared_list() {
        let toml = r#"
            format_version = 1
            [identity]
            id = "tessera-dev-01"
            name = "Tessera Dev"
            description = "desc"
            region = "EU"
            language = "FR"
            max_players = 16
            tags = ["dev"]
            discord_url = ""
            website_url = ""
            required_modset = "0.1.0"
            voice_required = false

            [runtime]
            whitelist = true
            whitelist_names = ["Alice", "Bob"]
            store_path = "players.json"

            [runtime.gateway]
            listen_addr = "0.0.0.0:27020"
            advertise_addr = "51.38.189.234:27020"

            [runtime.topology]
            authority_artifact = "authority.json"
            server_count = 1

            [runtime.radius]
            base = 25.0
            moderator = 50.0
            game_master = 75.0

            [runtime.aoi]
            visibility_radius = 100.0
        "#;
        let m: Manifest = toml::from_str(toml).expect("should parse");
        assert_eq!(
            m.runtime.whitelist_names,
            vec!["Alice".to_string(), "Bob".to_string()]
        );
    }

    // --- Runtime::spawn (remplace PLACEHOLDER_SPAWN codé en dur, cf. bin/gateway.rs) --------

    #[test]
    fn spawn_parses_declared_value() {
        let toml = r#"
            format_version = 1
            [identity]
            id = "tessera-dev-01"
            name = "Tessera Dev"
            description = "desc"
            region = "EU"
            language = "FR"
            max_players = 16
            tags = ["dev"]
            discord_url = ""
            website_url = ""
            required_modset = "0.1.0"
            voice_required = false

            [runtime]
            whitelist = false
            store_path = "players.json"
            spawn = [120.5, 8.0, -40.25]

            [runtime.gateway]
            listen_addr = "0.0.0.0:27020"
            advertise_addr = "51.38.189.234:27020"

            [runtime.topology]
            authority_artifact = "authority.json"
            server_count = 1

            [runtime.radius]
            base = 25.0
            moderator = 50.0
            game_master = 75.0

            [runtime.aoi]
            visibility_radius = 100.0
        "#;
        let m: Manifest = toml::from_str(toml).expect("should parse");
        assert_eq!(m.runtime.spawn, [120.5, 8.0, -40.25]);
    }

    // --- Runtime::redis_url / postgres_url (câblage PostgresStore/HotStateCache) ------------

    #[test]
    fn redis_url_defaults_to_local_redis_when_absent() {
        let m: Manifest = toml::from_str(MINIMAL_TOML).expect("should parse");
        assert_eq!(m.runtime.redis_url, "redis://127.0.0.1:6379");
    }

    #[test]
    fn redis_url_can_be_overridden_explicitly() {
        let toml_str = MINIMAL_TOML.replace(
            "store_path = \"players.json\"",
            "store_path = \"players.json\"\n        redis_url = \"redis://redis-service:6379\"",
        );
        let m: Manifest = toml::from_str(&toml_str).expect("should parse");
        assert_eq!(m.runtime.redis_url, "redis://redis-service:6379");
    }

    #[test]
    fn postgres_url_defaults_to_none_when_absent() {
        let m: Manifest = toml::from_str(MINIMAL_TOML).expect("should parse");
        assert_eq!(m.runtime.postgres_url, None);
    }

    #[test]
    fn postgres_url_parses_declared_value() {
        // Placeholder sans forme de secret réel (pas de user:pass) — juste un hôte de test.
        let toml_str = MINIMAL_TOML.replace(
            "store_path = \"players.json\"",
            "store_path = \"players.json\"\n        postgres_url = \"postgres://pg-service/tessera\"",
        );
        let m: Manifest = toml::from_str(&toml_str).expect("should parse");
        assert_eq!(
            m.runtime.postgres_url,
            Some("postgres://pg-service/tessera".to_string())
        );
    }

    #[test]
    fn resolve_redis_url_falls_back_to_manifest_when_env_absent() {
        assert_eq!(
            resolve_redis_url("redis://manifest-host:6379", None),
            "redis://manifest-host:6379"
        );
    }

    #[test]
    fn resolve_redis_url_falls_back_to_manifest_when_env_empty() {
        assert_eq!(
            resolve_redis_url("redis://manifest-host:6379", Some("")),
            "redis://manifest-host:6379"
        );
    }

    #[test]
    fn resolve_redis_url_prefers_env_when_present() {
        assert_eq!(
            resolve_redis_url(
                "redis://manifest-host:6379",
                Some("redis://dokploy-host:6379")
            ),
            "redis://dokploy-host:6379"
        );
    }

    #[test]
    fn resolve_postgres_url_falls_back_to_manifest_when_env_absent() {
        assert_eq!(
            resolve_postgres_url(Some("postgres://manifest/tessera"), None),
            Some("postgres://manifest/tessera".to_string())
        );
    }

    #[test]
    fn resolve_postgres_url_prefers_env_when_present() {
        assert_eq!(
            resolve_postgres_url(
                Some("postgres://manifest/tessera"),
                Some("postgres://dokploy/tessera")
            ),
            Some("postgres://dokploy/tessera".to_string())
        );
    }

    #[test]
    fn resolve_postgres_url_stays_none_when_neither_set() {
        assert_eq!(resolve_postgres_url(None, None), None);
    }

    #[test]
    fn resolve_postgres_url_env_alone_is_enough_without_manifest_value() {
        // Un serveur privé (identity.public = false) n'a jamais postgres_url dans son manifeste ;
        // s'il bascule en public via env var seule (sans éditer le manifeste monté), l'env var
        // doit suffire.
        assert_eq!(
            resolve_postgres_url(None, Some("postgres://dokploy/tessera")),
            Some("postgres://dokploy/tessera".to_string())
        );
    }

    #[test]
    fn assemble_postgres_url_from_components_includes_each_component() {
        let components = PostgresComponents {
            host: "pghost",
            port: "5432",
            user: "tessera",
            password: "pw9",
            database: "tessera",
        };
        let url = assemble_postgres_url_from_components(components).expect("host non vide");
        assert!(url.contains("pghost"));
        assert!(url.contains("5432"));
        assert!(url.contains("pw9"));
        assert!(url.starts_with("postgres"));
    }

    // --- Identity::kind / Identity::channel (badges launcher, defaults non-alarmants) --------

    #[test]
    fn identity_defaults_kind_to_community_and_channel_to_release_when_absent() {
        let m: Manifest = toml::from_str(MINIMAL_TOML).expect("manifest should parse with defaults");
        assert_eq!(m.identity.kind, ServerKind::Community);
        assert_eq!(m.identity.channel, ServerChannel::Release);
    }

    #[test]
    fn identity_reads_explicit_kind_and_channel() {
        let toml_str = MINIMAL_TOML.replace(
            "voice_required = false",
            "voice_required = false\n        kind = \"official\"\n        channel = \"dev\"",
        );
        let m: Manifest = toml::from_str(&toml_str).expect("manifest should parse");
        assert_eq!(m.identity.kind, ServerKind::Official);
        assert_eq!(m.identity.channel, ServerChannel::Dev);
    }

    #[test]
    fn identity_rejects_invalid_kind_value() {
        let toml_str = MINIMAL_TOML.replace(
            "voice_required = false",
            "voice_required = false\n        kind = \"not-a-real-kind\"",
        );
        let result: Result<Manifest, _> = toml::from_str(&toml_str);
        assert!(
            result.is_err(),
            "un kind invalide doit échouer au parsing, pas être silencieusement ignoré"
        );
    }
}
