//! Catalogue de briques comportementales PNJ, TOML versionné (spec fondation PNJ §4, modèle
//! serveur §5 : « data-driven TOML versionné... l'opérateur peut tuner sans recompiler »). Suit
//! le patron `manifest.rs` : struct Deserialize + format_version + parse_and_validate/load séparés.

use serde::Deserialize;
use std::fmt;

#[derive(Debug, Deserialize)]
struct RawCatalog {
    format_version: u32,
    archetype: Vec<RawArchetype>,
}

#[derive(Debug, Deserialize)]
struct RawArchetype {
    id: u32,
    name: String,
    briques: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NpcArchetypeConfig {
    pub name: String,
    pub briques: Vec<String>,
}

/// `Clone` : `shard_main` (Task 7) reconstruit un `Server::new_with_npcs` frais à chaque
/// connexion Gateway acceptée (même patron que `Server::new`/`new_with_metrics`) — il lui faut
/// donc pouvoir cloner le catalogue chargé une seule fois au boot plutôt que de le recharger
/// depuis disque à chaque reconnexion.
#[derive(Debug, Clone, Default)]
pub struct NpcCatalog {
    archetypes: std::collections::HashMap<u32, NpcArchetypeConfig>,
}

impl NpcCatalog {
    pub fn archetype(&self, id: u32) -> Option<&NpcArchetypeConfig> {
        self.archetypes.get(&id)
    }

    pub fn archetype_ids(&self) -> Vec<u32> {
        self.archetypes.keys().copied().collect()
    }
}

#[derive(Debug)]
pub enum NpcCatalogError {
    Parse(String),
    UnsupportedFormatVersion(u32),
    DuplicateArchetypeId(u32),
    EmptyBriquesList { archetype_id: u32 },
}

impl fmt::Display for NpcCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NpcCatalogError::Parse(e) => write!(f, "catalogue PNJ invalide (TOML) : {e}"),
            NpcCatalogError::UnsupportedFormatVersion(v) => {
                write!(f, "format_version {v} non supporté (attendu : 1)")
            }
            NpcCatalogError::DuplicateArchetypeId(id) => {
                write!(f, "archétype {id} déclaré plusieurs fois")
            }
            NpcCatalogError::EmptyBriquesList { archetype_id } => {
                write!(f, "archétype {archetype_id} n'a aucune brique — au moins une requise")
            }
        }
    }
}
impl std::error::Error for NpcCatalogError {}

/// Parse + valide, sans I/O — testable sans fichier sur disque.
pub fn parse_and_validate(toml_str: &str) -> Result<NpcCatalog, NpcCatalogError> {
    let raw: RawCatalog =
        toml::from_str(toml_str).map_err(|e| NpcCatalogError::Parse(e.to_string()))?;
    if raw.format_version != 1 {
        return Err(NpcCatalogError::UnsupportedFormatVersion(raw.format_version));
    }
    let mut archetypes = std::collections::HashMap::new();
    for a in raw.archetype {
        if a.briques.is_empty() {
            return Err(NpcCatalogError::EmptyBriquesList { archetype_id: a.id });
        }
        if archetypes
            .insert(
                a.id,
                NpcArchetypeConfig {
                    name: a.name,
                    briques: a.briques,
                },
            )
            .is_some()
        {
            return Err(NpcCatalogError::DuplicateArchetypeId(a.id));
        }
    }
    Ok(NpcCatalog { archetypes })
}

/// Charge depuis un chemin sur disque (câblage boot, cf. Task 7).
pub fn load(path: &std::path::Path) -> Result<NpcCatalog, NpcCatalogError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| NpcCatalogError::Parse(format!("lecture {path:?} échouée : {e}")))?;
    parse_and_validate(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
        format_version = 1
        [[archetype]]
        id = 1
        name = "marcheur-de-rue"
        briques = ["flaner-sur-place", "fuir-si-menace"]
    "#;

    #[test]
    fn valid_catalog_parses_and_exposes_the_archetype() {
        let cat = parse_and_validate(VALID).unwrap();
        let a = cat.archetype(1).unwrap();
        assert_eq!(a.name, "marcheur-de-rue");
        assert_eq!(a.briques, vec!["flaner-sur-place", "fuir-si-menace"]);
    }

    #[test]
    fn unsupported_format_version_is_rejected() {
        let toml = VALID.replace("format_version = 1", "format_version = 2");
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, NpcCatalogError::UnsupportedFormatVersion(2)));
    }

    #[test]
    fn duplicate_archetype_id_is_rejected() {
        let toml = format!(
            "{VALID}\n[[archetype]]\nid = 1\nname = \"doublon\"\nbriques = [\"rester-statique\"]\n"
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, NpcCatalogError::DuplicateArchetypeId(1)));
    }

    #[test]
    fn archetype_with_no_briques_is_rejected() {
        let toml = VALID.replace(
            r#"briques = ["flaner-sur-place", "fuir-si-menace"]"#,
            "briques = []",
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, NpcCatalogError::EmptyBriquesList { archetype_id: 1 }));
    }

    #[test]
    fn malformed_toml_is_rejected_with_a_parse_error() {
        let err = parse_and_validate("not valid toml {{{").unwrap_err();
        assert!(matches!(err, NpcCatalogError::Parse(_)));
    }

    #[test]
    fn unknown_archetype_id_returns_none() {
        let cat = parse_and_validate(VALID).unwrap();
        assert!(cat.archetype(999).is_none());
    }

    #[test]
    fn the_example_toml_file_on_disk_parses_successfully() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("npc-catalog.example.toml");
        let cat = load(&path).expect("npc-catalog.example.toml doit être valide");
        assert_eq!(cat.archetype_ids().len(), 3);
    }
}
