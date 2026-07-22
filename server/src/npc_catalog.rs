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
struct RawCombat {
    hp: u32,
    degats_max_par_rapport: u32,
    cadence_min_ms: u32,
}

#[derive(Debug, Deserialize)]
struct RawArchetype {
    id: u32,
    name: String,
    briques: Vec<String>,
    combat: Option<RawCombat>,
}

/// Paramètres de combat d'un archétype PNJ (spec PNJ hostiles §4.2) — `None` sur
/// `NpcArchetypeConfig::combat` (pas ce type) signifie "PNJ increvable", cf. doc de ce champ.
#[derive(Debug, Clone)]
pub struct NpcCombatConfig {
    pub hp: u32,
    /// Dégâts max acceptés en UN SEUL rapport `EntityInteraction{kind=5}` — clamp anti-triche
    /// (spec §2 : "rapports de dégâts gonflés"). Un rapport qui dépasse cette valeur est TRONQUÉ à
    /// cette valeur, pas rejeté entièrement (spec ne précise pas explicitement ce choix mais "clamp"
    /// implique une saturation, pas un rejet total — cohérent avec le principe "aucun enjeu
    /// économique" : tronquer plutôt que rejeter ne change rien à la trajectoire du combat, juste
    /// empêche un one-shot).
    pub degats_max_par_rapport: u32,
    /// Intervalle minimal entre deux rapports de dégâts ACCEPTÉS pour la MÊME paire (attaquant,
    /// cible) — anti-spam de rapports (spec §2 : "cadence/précision inhumaine"). Un rapport plus
    /// rapproché que ce délai est silencieusement ignoré (ni erreur, ni dégâts appliqués).
    pub cadence_min_ms: u32,
}

#[derive(Debug, Clone)]
pub struct NpcArchetypeConfig {
    pub name: String,
    pub briques: Vec<String>,
    pub combat: Option<NpcCombatConfig>,
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
    InvalidCombatConfig { archetype_id: u32, reason: String },
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
                write!(
                    f,
                    "archétype {archetype_id} n'a aucune brique — au moins une requise"
                )
            }
            NpcCatalogError::InvalidCombatConfig {
                archetype_id,
                reason,
            } => {
                write!(
                    f,
                    "archétype {archetype_id} : configuration combat invalide ({reason})"
                )
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
        return Err(NpcCatalogError::UnsupportedFormatVersion(
            raw.format_version,
        ));
    }
    let mut archetypes = std::collections::HashMap::new();
    for a in raw.archetype {
        if a.briques.is_empty() {
            return Err(NpcCatalogError::EmptyBriquesList { archetype_id: a.id });
        }
        let combat = match a.combat {
            None => None,
            Some(raw_combat) => {
                if raw_combat.hp == 0 {
                    return Err(NpcCatalogError::InvalidCombatConfig {
                        archetype_id: a.id,
                        reason: "hp doit être > 0".to_string(),
                    });
                }
                if raw_combat.degats_max_par_rapport == 0 {
                    return Err(NpcCatalogError::InvalidCombatConfig {
                        archetype_id: a.id,
                        reason: "degats_max_par_rapport doit être > 0".to_string(),
                    });
                }
                Some(NpcCombatConfig {
                    hp: raw_combat.hp,
                    degats_max_par_rapport: raw_combat.degats_max_par_rapport,
                    cadence_min_ms: raw_combat.cadence_min_ms,
                })
            }
        };
        if archetypes
            .insert(
                a.id,
                NpcArchetypeConfig {
                    name: a.name,
                    briques: a.briques,
                    combat,
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
        assert!(matches!(
            err,
            NpcCatalogError::EmptyBriquesList { archetype_id: 1 }
        ));
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
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("npc-catalog.example.toml");
        let cat = load(&path).expect("npc-catalog.example.toml doit être valide");
        assert_eq!(cat.archetype_ids().len(), 4);
    }

    #[test]
    fn an_archetype_without_a_combat_section_has_no_combat_config() {
        let cat = parse_and_validate(VALID).unwrap();
        let a = cat.archetype(1).unwrap();
        assert!(
            a.combat.is_none(),
            "un archétype sans section [archetype.combat] doit rester increvable"
        );
    }

    #[test]
    fn an_archetype_with_a_combat_section_parses_hp_and_clamps() {
        let toml = r#"
            format_version = 1
            [[archetype]]
            id = 10
            name = "gang-membre"
            briques = ["attaquer-cible"]
            [archetype.combat]
            hp = 100
            degats_max_par_rapport = 40
            cadence_min_ms = 500
        "#;
        let cat = parse_and_validate(toml).unwrap();
        let a = cat.archetype(10).unwrap();
        let combat = a.combat.as_ref().expect("combat doit être présent");
        assert_eq!(combat.hp, 100);
        assert_eq!(combat.degats_max_par_rapport, 40);
        assert_eq!(combat.cadence_min_ms, 500);
    }

    #[test]
    fn a_combat_section_with_zero_hp_is_rejected() {
        let toml = r#"
            format_version = 1
            [[archetype]]
            id = 10
            name = "gang-membre"
            briques = ["attaquer-cible"]
            [archetype.combat]
            hp = 0
            degats_max_par_rapport = 40
            cadence_min_ms = 500
        "#;
        let err = parse_and_validate(toml).unwrap_err();
        assert!(matches!(
            err,
            NpcCatalogError::InvalidCombatConfig {
                archetype_id: 10,
                ..
            }
        ));
    }
}
