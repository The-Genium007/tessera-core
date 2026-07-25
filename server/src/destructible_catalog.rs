//! Catalogue de destructibles (Classe A, spec 2026-07-23 §2 + plan 2026-07-25 §2.1). Suit
//! exactement le patron de `elevator_catalog.rs` : struct `Deserialize` + `format_version` +
//! `parse_and_validate` PUR séparé de `load` (I/O), enum d'erreur dédié.
//!
//! Décision embarquée : la liste de devices destructibles vient d'ICI, jamais d'une donnée
//! client — un client ne peut pas faire exister un destructible ni changer sa nature.

use serde::Deserialize;
use std::fmt;

#[derive(Debug, Deserialize)]
struct RawCatalog {
    format_version: u32,
    destructible: Vec<RawDestructible>,
}

#[derive(Debug, Deserialize)]
struct RawDestructible {
    persistent_id: String,
    name: String,
    kind: String, // "device" | "explosive"
    #[serde(default = "default_durability")]
    durability: u32,
    #[serde(default)]
    master: Option<String>,
}

fn default_durability() -> u32 {
    100 // BaseStats.DeviceHealth, spec 2026-07-23 §2 Classe A
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructibleKind {
    Device,
    Explosive,
}

#[derive(Debug, Clone)]
pub struct DestructibleRecord {
    pub persistent_id: u64,
    pub kind: DestructibleKind,
    pub durability: u32,
    pub destroyed: bool,
    pub exploded: bool,
    pub master: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DestructibleCatalog {
    entries: Vec<(String, DestructibleRecord)>,
}

impl DestructibleCatalog {
    pub fn name_of(&self, persistent_id: u64) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, r)| r.persistent_id == persistent_id)
            .map(|(n, _)| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn into_states(self) -> Vec<DestructibleRecord> {
        self.entries.into_iter().map(|(_, r)| r).collect()
    }
}

#[derive(Debug)]
pub enum DestructibleCatalogError {
    Parse(String),
    UnsupportedFormatVersion(u32),
    InvalidPersistentId(String),
    DuplicatePersistentId(u64),
    UnknownKind { persistent_id: u64, kind: String },
    UnknownMaster { persistent_id: u64, master: String },
}

impl fmt::Display for DestructibleCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DestructibleCatalogError::Parse(e) => {
                write!(f, "catalogue destructibles invalide (TOML) : {e}")
            }
            DestructibleCatalogError::UnsupportedFormatVersion(v) => {
                write!(f, "format_version {v} non supporté (attendu : 1)")
            }
            DestructibleCatalogError::InvalidPersistentId(id) => {
                write!(f, "persistent_id {id:?} n'est pas un entier u64 valide")
            }
            DestructibleCatalogError::DuplicatePersistentId(id) => {
                write!(f, "destructible {id} déclaré plusieurs fois")
            }
            DestructibleCatalogError::UnknownKind {
                persistent_id,
                kind,
            } => {
                write!(f, "destructible {persistent_id} : kind {kind:?} inconnu (attendu device|explosive)")
            }
            DestructibleCatalogError::UnknownMaster {
                persistent_id,
                master,
            } => {
                write!(f, "destructible {persistent_id} : master {master:?} ne référence aucun destructible du catalogue")
            }
        }
    }
}
impl std::error::Error for DestructibleCatalogError {}

pub fn parse_and_validate(toml_str: &str) -> Result<DestructibleCatalog, DestructibleCatalogError> {
    let raw: RawCatalog =
        toml::from_str(toml_str).map_err(|e| DestructibleCatalogError::Parse(e.to_string()))?;
    if raw.format_version != 1 {
        return Err(DestructibleCatalogError::UnsupportedFormatVersion(
            raw.format_version,
        ));
    }
    let mut entries: Vec<(String, DestructibleRecord)> = Vec::new();
    for d in &raw.destructible {
        let id: u64 = d
            .persistent_id
            .parse()
            .map_err(|_| DestructibleCatalogError::InvalidPersistentId(d.persistent_id.clone()))?;
        if entries.iter().any(|(_, r)| r.persistent_id == id) {
            return Err(DestructibleCatalogError::DuplicatePersistentId(id));
        }
        let kind = match d.kind.as_str() {
            "device" => DestructibleKind::Device,
            "explosive" => DestructibleKind::Explosive,
            other => {
                return Err(DestructibleCatalogError::UnknownKind {
                    persistent_id: id,
                    kind: other.to_string(),
                })
            }
        };
        entries.push((
            d.name.clone(),
            DestructibleRecord {
                persistent_id: id,
                kind,
                durability: d.durability,
                destroyed: false,
                exploded: false,
                master: None, // résolu dans une deuxième passe ci-dessous
            },
        ));
    }
    // Deuxième passe : résoudre `master` (String id -> u64) une fois tous les ids connus, pour
    // qu'un master déclaré APRÈS son esclave dans le fichier reste valide (ordre TOML non garanti
    // par l'utilisateur).
    for (i, d) in raw.destructible.iter().enumerate() {
        if let Some(master_str) = &d.master {
            let master_id: u64 = master_str
                .parse()
                .map_err(|_| DestructibleCatalogError::InvalidPersistentId(master_str.clone()))?;
            if !entries.iter().any(|(_, r)| r.persistent_id == master_id) {
                return Err(DestructibleCatalogError::UnknownMaster {
                    persistent_id: entries[i].1.persistent_id,
                    master: master_str.clone(),
                });
            }
            entries[i].1.master = Some(master_id);
        }
    }
    Ok(DestructibleCatalog { entries })
}

pub fn load(path: &std::path::Path) -> Result<DestructibleCatalog, DestructibleCatalogError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| DestructibleCatalogError::Parse(format!("lecture {path:?} échouée : {e}")))?;
    parse_and_validate(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
        format_version = 1
        [[destructible]]
        persistent_id = "1001"
        name = "lampadaire-rue-jig-jig"
        kind = "device"
    "#;

    #[test]
    fn valid_catalog_parses_and_builds_an_initial_state() {
        let cat = parse_and_validate(VALID).unwrap();
        assert_eq!(cat.len(), 1);
        assert_eq!(cat.name_of(1001), Some("lampadaire-rue-jig-jig"));
        let states = cat.into_states();
        assert_eq!(states[0].kind, DestructibleKind::Device);
        assert_eq!(states[0].durability, 100, "défaut BaseStats.DeviceHealth");
        assert!(!states[0].destroyed);
        assert!(!states[0].exploded);
        assert_eq!(states[0].master, None);
    }

    #[test]
    fn unsupported_format_version_is_rejected() {
        let toml = VALID.replace("format_version = 1", "format_version = 2");
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(
            err,
            DestructibleCatalogError::UnsupportedFormatVersion(2)
        ));
    }

    #[test]
    fn duplicate_persistent_id_is_rejected() {
        let toml = format!("{VALID}\n{}", VALID.replace("format_version = 1", ""));
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(
            err,
            DestructibleCatalogError::DuplicatePersistentId(1001)
        ));
    }

    #[test]
    fn an_unknown_kind_is_rejected() {
        let toml = VALID.replace(r#"kind = "device""#, r#"kind = "spaceship""#);
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, DestructibleCatalogError::UnknownKind { .. }));
    }

    #[test]
    fn a_non_numeric_persistent_id_is_rejected() {
        let toml = VALID.replace(
            r#"persistent_id = "1001""#,
            r#"persistent_id = "not-a-number""#,
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(
            matches!(err, DestructibleCatalogError::InvalidPersistentId(ref id) if id == "not-a-number")
        );
    }

    #[test]
    fn explosive_kind_parses() {
        let toml = VALID.replace(r#"kind = "device""#, r#"kind = "explosive""#);
        let cat = parse_and_validate(&toml).unwrap();
        assert_eq!(cat.into_states()[0].kind, DestructibleKind::Explosive);
    }

    #[test]
    fn a_custom_durability_overrides_the_default() {
        let toml = format!("{VALID}\ndurability = 250");
        let cat = parse_and_validate(&toml).unwrap();
        assert_eq!(cat.into_states()[0].durability, 250);
    }

    #[test]
    fn a_master_reference_to_a_known_id_resolves() {
        let toml = r#"
            format_version = 1
            [[destructible]]
            persistent_id = "1001"
            name = "master-light"
            kind = "device"
            [[destructible]]
            persistent_id = "1002"
            name = "slave-light"
            kind = "device"
            master = "1001"
        "#;
        let cat = parse_and_validate(toml).unwrap();
        let states = cat.into_states();
        let slave = states.iter().find(|s| s.persistent_id == 1002).unwrap();
        assert_eq!(slave.master, Some(1001));
    }

    #[test]
    fn a_master_reference_to_an_unknown_id_is_rejected() {
        let toml = r#"
            format_version = 1
            [[destructible]]
            persistent_id = "1002"
            name = "slave-light"
            kind = "device"
            master = "9999"
        "#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(matches!(
            err,
            DestructibleCatalogError::UnknownMaster { .. }
        ));
    }

    #[test]
    fn malformed_toml_is_rejected_with_a_parse_error() {
        let err = parse_and_validate("not valid toml {{{").unwrap_err();
        assert!(matches!(err, DestructibleCatalogError::Parse(_)));
    }

    #[test]
    fn the_example_toml_file_on_disk_parses_successfully() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("destructible-catalog.example.toml");
        let cat = load(&path).expect("destructible-catalog.example.toml doit être valide");
        assert!(!cat.is_empty());
    }
}
