//! Manifeste de PNJ NOMINATIFS, TOML versionné (spec fondation d'interaction §6 : « entrées
//! nominatives... pas du director de population »). Fichier séparé de `npc_catalog.rs` : espace
//! d'ids différent (id-manifeste STABLE, string, distinct de l'id runtime `ClientId` attribué au
//! spawn — cf. `Server`/`World`). Suit le même patron que `npc_catalog.rs` : struct Deserialize +
//! format_version + parse_and_validate/load séparés, sans réutiliser son type d'erreur (les modes
//! d'échec diffèrent : id-manifeste dupliqué plutôt qu'id d'archétype numérique dupliqué).

use serde::Deserialize;
use std::fmt;

#[derive(Debug, Deserialize)]
struct RawCatalog {
    format_version: u32,
    pnj: Vec<RawNamedNpc>,
}

#[derive(Debug, Deserialize)]
struct RawNamedNpc {
    id: String,
    archetype: String,
    position: [f32; 3],
    briques: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NamedNpcConfig {
    pub archetype: String,
    pub position: [f32; 3],
    pub briques: Vec<String>,
}

#[derive(Debug, Default)]
pub struct NamedNpcCatalog {
    entries: std::collections::HashMap<String, NamedNpcConfig>,
}

impl NamedNpcCatalog {
    pub fn get(&self, id: &str) -> Option<&NamedNpcConfig> {
        self.entries.get(id)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug)]
pub enum NamedNpcCatalogError {
    Parse(String),
    UnsupportedFormatVersion(u32),
    DuplicateId(String),
    EmptyBriquesList { id: String },
}

impl fmt::Display for NamedNpcCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NamedNpcCatalogError::Parse(e) => write!(f, "manifeste PNJ nominatif invalide (TOML) : {e}"),
            NamedNpcCatalogError::UnsupportedFormatVersion(v) => {
                write!(f, "format_version {v} non supporté (attendu : 1)")
            }
            NamedNpcCatalogError::DuplicateId(id) => write!(f, "id-manifeste '{id}' déclaré plusieurs fois"),
            NamedNpcCatalogError::EmptyBriquesList { id } => {
                write!(f, "PNJ '{id}' n'a aucune brique — au moins une requise")
            }
        }
    }
}
impl std::error::Error for NamedNpcCatalogError {}

pub fn parse_and_validate(toml_str: &str) -> Result<NamedNpcCatalog, NamedNpcCatalogError> {
    let raw: RawCatalog =
        toml::from_str(toml_str).map_err(|e| NamedNpcCatalogError::Parse(e.to_string()))?;
    if raw.format_version != 1 {
        return Err(NamedNpcCatalogError::UnsupportedFormatVersion(raw.format_version));
    }
    let mut entries = std::collections::HashMap::new();
    for p in raw.pnj {
        if p.briques.is_empty() {
            return Err(NamedNpcCatalogError::EmptyBriquesList { id: p.id });
        }
        if entries
            .insert(
                p.id.clone(),
                NamedNpcConfig {
                    archetype: p.archetype,
                    position: p.position,
                    briques: p.briques,
                },
            )
            .is_some()
        {
            return Err(NamedNpcCatalogError::DuplicateId(p.id));
        }
    }
    Ok(NamedNpcCatalog { entries })
}

pub fn load(path: &std::path::Path) -> Result<NamedNpcCatalog, NamedNpcCatalogError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| NamedNpcCatalogError::Parse(format!("lecture {path:?} échouée : {e}")))?;
    parse_and_validate(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
        format_version = 1
        [[pnj]]
        id = "ripperdoc-watson-01"
        archetype = "ripperdoc_male_a"
        position = [1.0, 2.0, 3.0]
        briques = ["rester-statique", "vendre"]
    "#;

    #[test]
    fn valid_catalog_parses_and_exposes_the_entry() {
        let cat = parse_and_validate(VALID).unwrap();
        let e = cat.get("ripperdoc-watson-01").unwrap();
        assert_eq!(e.archetype, "ripperdoc_male_a");
        assert_eq!(e.position, [1.0, 2.0, 3.0]);
        assert_eq!(e.briques, vec!["rester-statique", "vendre"]);
    }

    #[test]
    fn unsupported_format_version_is_rejected() {
        let toml = VALID.replace("format_version = 1", "format_version = 2");
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, NamedNpcCatalogError::UnsupportedFormatVersion(2)));
    }

    #[test]
    fn duplicate_manifest_id_is_rejected() {
        let toml = format!(
            "{VALID}\n[[pnj]]\nid = \"ripperdoc-watson-01\"\narchetype = \"x\"\nposition = [0.0, 0.0, 0.0]\nbriques = [\"rester-statique\"]\n"
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, NamedNpcCatalogError::DuplicateId(id) if id == "ripperdoc-watson-01"));
    }

    #[test]
    fn entry_with_no_briques_is_rejected() {
        let toml = VALID.replace(
            r#"briques = ["rester-statique", "vendre"]"#,
            "briques = []",
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, NamedNpcCatalogError::EmptyBriquesList { id } if id == "ripperdoc-watson-01"));
    }

    #[test]
    fn malformed_toml_is_rejected_with_a_parse_error() {
        let err = parse_and_validate("not valid toml {{{").unwrap_err();
        assert!(matches!(err, NamedNpcCatalogError::Parse(_)));
    }

    #[test]
    fn unknown_id_returns_none() {
        let cat = parse_and_validate(VALID).unwrap();
        assert!(cat.get("does-not-exist").is_none());
    }

    #[test]
    fn the_example_toml_file_on_disk_parses_successfully() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("named-npc-catalog.example.toml");
        let cat = load(&path).expect("named-npc-catalog.example.toml doit être valide");
        assert_eq!(cat.len(), 2);
    }
}
