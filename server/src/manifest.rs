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

#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    UnsupportedFormatVersion(u32),
    EmptyField(&'static str),
    InvalidMaxPlayers,
    DuplicateId(String),
    DanglingReference(String),
    TreeNotConnected(String),
    NoRootShardOrSplit,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormatVersion(v) => {
                write!(f, "format_version {v} non supportée (seule 1 est supportée)")
            }
            Self::EmptyField(name) => write!(f, "champ {name} vide"),
            Self::InvalidMaxPlayers => write!(f, "identity.max_players doit être > 0"),
            Self::DuplicateId(id) => write!(f, "id dupliqué dans la topologie: {id}"),
            Self::DanglingReference(id) => write!(f, "référence vers un id inconnu: {id}"),
            Self::TreeNotConnected(id) => {
                write!(f, "id non atteint depuis root ou référencé plus d'une fois: {id}")
            }
            Self::NoRootShardOrSplit => write!(f, "topologie vide : aucun shard ni split"),
        }
    }
}

#[allow(dead_code)]
// Wired into server::manifest::validate() in Task 4 of the M6 plan.
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

#[allow(dead_code)]
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
        active_preset = "2-shards"
        shards = []

        [runtime.radius]
        base = 25.0
        moderator = 50.0
        game_master = 75.0
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
        assert_eq!(validate_scalars(&m), Err(ManifestError::EmptyField("identity.id")));
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
}
