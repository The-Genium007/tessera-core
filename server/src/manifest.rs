//! Manifeste serveur (fichier TOML par opérateur) : identité publique (dérive servers.json) +
//! config runtime privée (topologie/spawn/rayons/store) consommée au boot du Gateway.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub identity: Identity,
    pub runtime: Runtime,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct Runtime {
    pub whitelist: bool,
    pub store_path: String,
    pub gateway: GatewayConfig,
    pub topology: TopologyConfig,
    pub radius: RadiusConfig,
    pub aoi: AoiConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub listen_addr: String,
    pub advertise_addr: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopologyConfig {
    pub active_preset: String,
    #[serde(default)]
    pub splits: Vec<SplitConfig>,
    pub shards: Vec<ShardConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SplitConfig {
    pub id: String,
    pub axis: Axis,
    pub at: f32,
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
}

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
    DuplicateId(String),
    DanglingReference(String),
    TreeNotConnected(String),
    NoRootShardOrSplit,
    DefaultEntryCount(usize),
    NoSpawnPointForDefaultEntry,
    RadiusOutOfOrder,
    NegativeAoiRadius,
    InvalidAddress(String, String),
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
            Self::DuplicateId(id) => write!(f, "id dupliqué dans la topologie: {id}"),
            Self::DanglingReference(id) => write!(f, "référence vers un id inconnu: {id}"),
            Self::TreeNotConnected(id) => {
                write!(
                    f,
                    "id non atteint depuis root ou référencé plus d'une fois: {id}"
                )
            }
            Self::NoRootShardOrSplit => write!(f, "topologie vide : aucun shard ni split"),
            Self::DefaultEntryCount(n) => write!(
                f,
                "il doit y avoir exactement un shard default_entry=true (trouvé {n})"
            ),
            Self::NoSpawnPointForDefaultEntry => {
                write!(
                    f,
                    "le shard default_entry doit avoir au moins un spawn_point"
                )
            }
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

fn validate_topology_structure(topo: &TopologyConfig) -> Result<(), ManifestError> {
    let mut split_by_id: HashMap<&str, &SplitConfig> = HashMap::new();
    let mut shard_by_id: HashMap<&str, &ShardConfig> = HashMap::new();

    for s in &topo.splits {
        if split_by_id.insert(&s.id, s).is_some() {
            return Err(ManifestError::DuplicateId(s.id.clone()));
        }
    }
    for s in &topo.shards {
        if shard_by_id.contains_key(s.id.as_str()) || split_by_id.contains_key(s.id.as_str()) {
            return Err(ManifestError::DuplicateId(s.id.clone()));
        }
        shard_by_id.insert(&s.id, s);
    }

    if split_by_id.is_empty() && shard_by_id.is_empty() {
        return Err(ManifestError::NoRootShardOrSplit);
    }

    if split_by_id.is_empty() {
        if shard_by_id.len() != 1 {
            return Err(ManifestError::NoRootShardOrSplit);
        }
        return Ok(());
    }

    let mut visited: HashSet<String> = HashSet::new();
    fn walk(
        id: &str,
        split_by_id: &HashMap<&str, &SplitConfig>,
        shard_by_id: &HashMap<&str, &ShardConfig>,
        visited: &mut HashSet<String>,
    ) -> Result<(), ManifestError> {
        if !visited.insert(id.to_string()) {
            return Err(ManifestError::TreeNotConnected(id.to_string()));
        }
        if let Some(split) = split_by_id.get(id) {
            walk(&split.left, split_by_id, shard_by_id, visited)?;
            walk(&split.right, split_by_id, shard_by_id, visited)?;
            Ok(())
        } else if shard_by_id.contains_key(id) {
            Ok(())
        } else {
            Err(ManifestError::DanglingReference(id.to_string()))
        }
    }
    walk("root", &split_by_id, &shard_by_id, &mut visited)?;

    for id in split_by_id.keys().chain(shard_by_id.keys()) {
        if !visited.contains(*id) {
            return Err(ManifestError::TreeNotConnected(id.to_string()));
        }
    }
    Ok(())
}

pub fn flatten_topology(
    topo: &TopologyConfig,
) -> Result<Vec<crate::handoff::ShardZone>, ManifestError> {
    use crate::handoff::{Aabb, ShardZone};

    let mut split_by_id: HashMap<&str, &SplitConfig> = HashMap::new();
    let mut shard_by_id: HashMap<&str, &ShardConfig> = HashMap::new();
    for s in &topo.splits {
        split_by_id.insert(&s.id, s);
    }
    for s in &topo.shards {
        shard_by_id.insert(&s.id, s);
    }

    let whole = Aabb {
        min_x: f32::NEG_INFINITY,
        max_x: f32::INFINITY,
        min_y: f32::NEG_INFINITY,
        max_y: f32::INFINITY,
    };

    if split_by_id.is_empty() {
        let shard = topo
            .shards
            .first()
            .ok_or(ManifestError::NoRootShardOrSplit)?;
        return Ok(vec![ShardZone {
            addr: shard.listen_addr.clone(),
            zone: whole,
        }]);
    }

    fn walk(
        id: &str,
        bounds: crate::handoff::Aabb,
        split_by_id: &HashMap<&str, &SplitConfig>,
        shard_by_id: &HashMap<&str, &ShardConfig>,
        zones: &mut Vec<crate::handoff::ShardZone>,
    ) -> Result<(), ManifestError> {
        use crate::handoff::{Aabb, ShardZone};
        if let Some(split) = split_by_id.get(id) {
            let (left_bounds, right_bounds) = match split.axis {
                Axis::X => (
                    Aabb {
                        max_x: split.at,
                        ..bounds
                    },
                    Aabb {
                        min_x: split.at,
                        ..bounds
                    },
                ),
                Axis::Y => (
                    Aabb {
                        max_y: split.at,
                        ..bounds
                    },
                    Aabb {
                        min_y: split.at,
                        ..bounds
                    },
                ),
            };
            walk(&split.left, left_bounds, split_by_id, shard_by_id, zones)?;
            walk(&split.right, right_bounds, split_by_id, shard_by_id, zones)?;
            Ok(())
        } else if let Some(shard) = shard_by_id.get(id) {
            zones.push(ShardZone {
                addr: shard.listen_addr.clone(),
                zone: bounds,
            });
            Ok(())
        } else {
            Err(ManifestError::DanglingReference(id.to_string()))
        }
    }

    let mut zones = Vec::new();
    walk("root", whole, &split_by_id, &shard_by_id, &mut zones)?;
    Ok(zones)
}

fn validate_default_entry(topo: &TopologyConfig) -> Result<(), ManifestError> {
    let defaults: Vec<&ShardConfig> = topo.shards.iter().filter(|s| s.default_entry).collect();
    if defaults.len() != 1 {
        return Err(ManifestError::DefaultEntryCount(defaults.len()));
    }
    if defaults[0].spawn_points.is_empty() {
        return Err(ManifestError::NoSpawnPointForDefaultEntry);
    }
    Ok(())
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
    validate_topology_structure(&m.runtime.topology)?;
    flatten_topology(&m.runtime.topology)?;
    validate_default_entry(&m.runtime.topology)?;
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
    for s in &m.runtime.topology.shards {
        validate_addr(
            &format!("runtime.topology.shards[{}].listen_addr", s.id),
            &s.listen_addr,
        )?;
    }
    Ok(())
}

pub fn to_runtime(
    m: &Manifest,
) -> Result<
    (
        crate::handoff::ShardTopology,
        crate::handoff::RadiusPolicy,
        [f32; 3],
        String,
    ),
    ManifestError,
> {
    let zones = flatten_topology(&m.runtime.topology)?;
    let topology = crate::handoff::ShardTopology { shards: zones };
    let radius = crate::handoff::RadiusPolicy {
        base: m.runtime.radius.base,
        moderator: m.runtime.radius.moderator,
        game_master: m.runtime.radius.game_master,
    };
    let default_shard = m
        .runtime
        .topology
        .shards
        .iter()
        .find(|s| s.default_entry)
        .ok_or(ManifestError::DefaultEntryCount(0))?;
    let spawn = *default_shard
        .spawn_points
        .first()
        .ok_or(ManifestError::NoSpawnPointForDefaultEntry)?;
    Ok((topology, radius, spawn, m.runtime.store_path.clone()))
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

#[cfg(test)]
mod tests {
    use super::*;

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
        active_preset = "1-shard"

        [[runtime.topology.shards]]
        id = "shard-a"
        listen_addr = "127.0.0.1:27030"
        default_entry = true
        spawn_points = [[0.0, 0.0, 0.0]]

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

    fn two_shard_topology() -> TopologyConfig {
        TopologyConfig {
            active_preset: "2-shards".into(),
            splits: vec![SplitConfig {
                id: "root".into(),
                axis: Axis::X,
                at: 1000.0,
                left: "shard-a".into(),
                right: "shard-b".into(),
            }],
            shards: vec![
                ShardConfig {
                    id: "shard-a".into(),
                    listen_addr: "127.0.0.1:27030".into(),
                    default_entry: true,
                    spawn_points: vec![[2387.0, -1295.0, 63.0]],
                },
                ShardConfig {
                    id: "shard-b".into(),
                    listen_addr: "127.0.0.1:27031".into(),
                    default_entry: false,
                    spawn_points: vec![],
                },
            ],
        }
    }

    #[test]
    fn valid_two_shard_tree_passes_structural_validation() {
        assert_eq!(validate_topology_structure(&two_shard_topology()), Ok(()));
    }

    #[test]
    fn rejects_dangling_reference() {
        let mut topo = two_shard_topology();
        topo.splits[0].right = "shard-ghost".into();
        assert_eq!(
            validate_topology_structure(&topo),
            Err(ManifestError::DanglingReference("shard-ghost".into()))
        );
    }

    #[test]
    fn rejects_duplicate_id() {
        let mut topo = two_shard_topology();
        topo.shards[1].id = "shard-a".into();
        assert_eq!(
            validate_topology_structure(&topo),
            Err(ManifestError::DuplicateId("shard-a".into()))
        );
    }

    #[test]
    fn rejects_node_referenced_twice() {
        let mut topo = two_shard_topology();
        topo.splits.push(SplitConfig {
            id: "extra".into(),
            axis: Axis::Y,
            at: 0.0,
            left: "shard-a".into(),
            right: "shard-b".into(),
        });
        topo.splits[0].right = "extra".into();
        assert!(matches!(
            validate_topology_structure(&topo),
            Err(ManifestError::TreeNotConnected(_))
        ));
    }

    #[test]
    fn single_shard_topology_with_no_splits_is_valid() {
        let topo = TopologyConfig {
            active_preset: "1-shard".into(),
            splits: vec![],
            shards: vec![ShardConfig {
                id: "shard-a".into(),
                listen_addr: "127.0.0.1:27030".into(),
                default_entry: true,
                spawn_points: vec![[0.0, 0.0, 0.0]],
            }],
        };
        assert_eq!(validate_topology_structure(&topo), Ok(()));
    }

    #[test]
    fn flattens_two_shard_tree_to_expected_aabbs() {
        let zones = flatten_topology(&two_shard_topology()).unwrap();
        assert_eq!(zones.len(), 2);
        let a = zones.iter().find(|z| z.addr == "127.0.0.1:27030").unwrap();
        assert_eq!(a.zone.min_x, f32::NEG_INFINITY);
        assert_eq!(a.zone.max_x, 1000.0);
        assert_eq!(a.zone.min_y, f32::NEG_INFINITY);
        assert_eq!(a.zone.max_y, f32::INFINITY);
        let b = zones.iter().find(|z| z.addr == "127.0.0.1:27031").unwrap();
        assert_eq!(b.zone.min_x, 1000.0);
        assert_eq!(b.zone.max_x, f32::INFINITY);
    }

    #[test]
    fn merging_four_shard_tree_matches_two_shard_tree() {
        let four = TopologyConfig {
            active_preset: "4-shards".into(),
            splits: vec![
                SplitConfig {
                    id: "root".into(),
                    axis: Axis::X,
                    at: 1000.0,
                    left: "split-a".into(),
                    right: "split-b".into(),
                },
                SplitConfig {
                    id: "split-a".into(),
                    axis: Axis::Y,
                    at: 0.0,
                    left: "shard-a1".into(),
                    right: "shard-a2".into(),
                },
                SplitConfig {
                    id: "split-b".into(),
                    axis: Axis::Y,
                    at: 0.0,
                    left: "shard-b1".into(),
                    right: "shard-b2".into(),
                },
            ],
            shards: vec![
                ShardConfig {
                    id: "shard-a1".into(),
                    listen_addr: "a1".into(),
                    default_entry: true,
                    spawn_points: vec![[0.0, 0.0, 0.0]],
                },
                ShardConfig {
                    id: "shard-a2".into(),
                    listen_addr: "a2".into(),
                    default_entry: false,
                    spawn_points: vec![],
                },
                ShardConfig {
                    id: "shard-b1".into(),
                    listen_addr: "b1".into(),
                    default_entry: false,
                    spawn_points: vec![],
                },
                ShardConfig {
                    id: "shard-b2".into(),
                    listen_addr: "b2".into(),
                    default_entry: false,
                    spawn_points: vec![],
                },
            ],
        };
        let zones = flatten_topology(&four).unwrap();
        let a1 = zones.iter().find(|z| z.addr == "a1").unwrap();
        let a2 = zones.iter().find(|z| z.addr == "a2").unwrap();
        let b1 = zones.iter().find(|z| z.addr == "b1").unwrap();
        let b2 = zones.iter().find(|z| z.addr == "b2").unwrap();

        assert_eq!(a1.zone.min_x, f32::NEG_INFINITY);
        assert_eq!(a1.zone.max_x, 1000.0);
        assert_eq!(a2.zone.min_x, f32::NEG_INFINITY);
        assert_eq!(a2.zone.max_x, 1000.0);
        assert_eq!(a1.zone.min_y, f32::NEG_INFINITY);
        assert_eq!(a1.zone.max_y, 0.0);
        assert_eq!(a2.zone.min_y, 0.0);
        assert_eq!(a2.zone.max_y, f32::INFINITY);

        assert_eq!(b1.zone.min_x, 1000.0);
        assert_eq!(b1.zone.max_x, f32::INFINITY);
        assert_eq!(b2.zone.min_x, 1000.0);
        assert_eq!(b2.zone.max_x, f32::INFINITY);

        let two = flatten_topology(&two_shard_topology()).unwrap();
        let shard_a = two.iter().find(|z| z.addr == "127.0.0.1:27030").unwrap();
        assert_eq!(a1.zone.min_x, shard_a.zone.min_x);
        assert_eq!(a1.zone.max_x, shard_a.zone.max_x);
        assert_eq!(a1.zone.min_y, shard_a.zone.min_y);
        assert_eq!(a2.zone.max_y, shard_a.zone.max_y);
    }

    #[test]
    fn validates_exactly_one_default_entry() {
        let mut topo = two_shard_topology();
        topo.shards[1].default_entry = true;
        assert_eq!(
            validate_default_entry(&topo),
            Err(ManifestError::DefaultEntryCount(2))
        );

        topo.shards[0].default_entry = false;
        topo.shards[1].default_entry = false;
        assert_eq!(
            validate_default_entry(&topo),
            Err(ManifestError::DefaultEntryCount(0))
        );
    }

    #[test]
    fn rejects_default_entry_with_no_spawn_point() {
        let mut topo = two_shard_topology();
        topo.shards[0].spawn_points = vec![];
        assert_eq!(
            validate_default_entry(&topo),
            Err(ManifestError::NoSpawnPointForDefaultEntry)
        );
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
        // Valeur exacte utilisée par server/server.docker.toml pour le listen_addr du shard :
        // un nom de service Docker Compose n'est pas un littéral SocketAddr.
        let toml_str = MINIMAL_TOML.replace(
            r#"listen_addr = "127.0.0.1:27030""#,
            r#"listen_addr = "shard-a:27030""#,
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

    fn full_two_shard_toml() -> String {
        r#"
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
        active_preset = "2-shards"

        [[runtime.topology.splits]]
        id = "root"
        axis = "x"
        at = 1000.0
        left = "shard-a"
        right = "shard-b"

        [[runtime.topology.shards]]
        id = "shard-a"
        listen_addr = "127.0.0.1:27030"
        default_entry = true
        spawn_points = [[2387.0, -1295.0, 63.0]]

        [[runtime.topology.shards]]
        id = "shard-b"
        listen_addr = "127.0.0.1:27031"
        spawn_points = []

        [runtime.radius]
        base = 25.0
        moderator = 50.0
        game_master = 75.0

        [runtime.aoi]
        visibility_radius = 100.0
        "#
        .to_string()
    }

    #[test]
    fn parses_and_validates_full_two_shard_manifest() {
        let m = parse_and_validate(&full_two_shard_toml()).expect("should validate");
        let (topology, radius, spawn, store_path) = to_runtime(&m).expect("should translate");
        assert_eq!(topology.shards.len(), 2);
        assert_eq!(radius.base, 25.0);
        assert_eq!(spawn, [2387.0, -1295.0, 63.0]);
        assert_eq!(store_path, "players.json");
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
}
