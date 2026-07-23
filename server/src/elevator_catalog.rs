//! Catalogue d'ascenseurs, TOML versionné (spec ascenseurs §8). Suit exactement le patron de
//! `npc_catalog.rs` : struct `Deserialize` + `format_version` + `parse_and_validate` PUR séparé de
//! `load` (I/O), et un enum d'erreur dédié.
//!
//! Décision embarquée n°2 de la spec : la liste d'étages vient d'ICI, jamais d'une donnée client —
//! un client ne peut pas faire exister un ascenseur ni inventer ses étages.

use crate::elevator::{ElevatorState, FloorSpec};
use serde::Deserialize;
use std::fmt;

#[derive(Debug, Deserialize)]
struct RawCatalog {
    format_version: u32,
    elevator: Vec<RawElevator>,
}

#[derive(Debug, Deserialize)]
struct RawElevator {
    id: String,
    name: String,
    start_floor: i32,
    start_delay_ms: u32,
    travel_time_ms: u32,
    floors: Vec<RawFloor>,
}

#[derive(Debug, Deserialize)]
struct RawFloor {
    index: i32,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    inactive: bool,
    /// Position monde de la porte d'étage — TROIS champs TOML séparés (pas un tableau) pour rester
    /// facile à écrire à la main dans un catalogue d'opérateur. `Option` : un catalogue existant
    /// sans ces champs continue de parser exactement comme avant ce plan (`serde(default)` produit
    /// `None` pour chacun, combinés en `None` par `parse_and_validate`, Step 4).
    #[serde(default)]
    position_x: Option<f32>,
    #[serde(default)]
    position_y: Option<f32>,
    #[serde(default)]
    position_z: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct ElevatorCatalog {
    entries: Vec<(String, ElevatorState)>,
}

impl ElevatorCatalog {
    /// Nom lisible d'un ascenseur (logs/debug uniquement — jamais sur le fil).
    pub fn name_of(&self, elevator_id: u64) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, s)| s.elevator_id == elevator_id)
            .map(|(n, _)| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Consomme le catalogue pour produire les états initiaux, prêts à peupler le registre.
    pub fn into_states(self) -> Vec<ElevatorState> {
        self.entries.into_iter().map(|(_, s)| s).collect()
    }
}

#[derive(Debug)]
pub enum ElevatorCatalogError {
    Parse(String),
    UnsupportedFormatVersion(u32),
    InvalidElevatorId(String),
    DuplicateElevatorId(u64),
    EmptyFloorList { elevator_id: u64 },
    DuplicateFloorIndex { elevator_id: u64, index: i32 },
    StartFloorNotInFloorList { elevator_id: u64, start_floor: i32 },
}

impl fmt::Display for ElevatorCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ElevatorCatalogError::Parse(e) => {
                write!(f, "catalogue ascenseurs invalide (TOML) : {e}")
            }
            ElevatorCatalogError::UnsupportedFormatVersion(v) => {
                write!(f, "format_version {v} non supporté (attendu : 1)")
            }
            ElevatorCatalogError::InvalidElevatorId(id) => {
                write!(f, "id d'ascenseur {id:?} n'est pas un entier u64 valide")
            }
            ElevatorCatalogError::DuplicateElevatorId(id) => {
                write!(f, "ascenseur {id} déclaré plusieurs fois")
            }
            ElevatorCatalogError::EmptyFloorList { elevator_id } => {
                write!(
                    f,
                    "ascenseur {elevator_id} n'a aucun étage — au moins un requis"
                )
            }
            ElevatorCatalogError::DuplicateFloorIndex { elevator_id, index } => {
                write!(
                    f,
                    "ascenseur {elevator_id} : étage {index} déclaré plusieurs fois"
                )
            }
            ElevatorCatalogError::StartFloorNotInFloorList {
                elevator_id,
                start_floor,
            } => {
                write!(
                    f,
                    "ascenseur {elevator_id} : start_floor {start_floor} absent de sa liste d'étages"
                )
            }
        }
    }
}
impl std::error::Error for ElevatorCatalogError {}

/// Parse + valide, sans I/O — testable sans fichier sur disque.
pub fn parse_and_validate(toml_str: &str) -> Result<ElevatorCatalog, ElevatorCatalogError> {
    let raw: RawCatalog =
        toml::from_str(toml_str).map_err(|e| ElevatorCatalogError::Parse(e.to_string()))?;
    if raw.format_version != 1 {
        return Err(ElevatorCatalogError::UnsupportedFormatVersion(
            raw.format_version,
        ));
    }
    let mut entries: Vec<(String, ElevatorState)> = Vec::new();
    for e in raw.elevator {
        let id: u64 =
            e.id.parse()
                .map_err(|_| ElevatorCatalogError::InvalidElevatorId(e.id.clone()))?;
        if entries.iter().any(|(_, s)| s.elevator_id == id) {
            return Err(ElevatorCatalogError::DuplicateElevatorId(id));
        }
        if e.floors.is_empty() {
            return Err(ElevatorCatalogError::EmptyFloorList { elevator_id: id });
        }
        let mut floors: Vec<FloorSpec> = Vec::new();
        for f in e.floors {
            if floors.iter().any(|x| x.index == f.index) {
                return Err(ElevatorCatalogError::DuplicateFloorIndex {
                    elevator_id: id,
                    index: f.index,
                });
            }
            floors.push(FloorSpec {
                index: f.index,
                hidden: f.hidden,
                inactive: f.inactive,
                position: match (f.position_x, f.position_y, f.position_z) {
                    (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                    _ => None,
                },
            });
        }
        if !floors.iter().any(|f| f.index == e.start_floor) {
            return Err(ElevatorCatalogError::StartFloorNotInFloorList {
                elevator_id: id,
                start_floor: e.start_floor,
            });
        }
        entries.push((
            e.name,
            ElevatorState::new(
                id,
                e.start_floor,
                floors,
                e.start_delay_ms,
                e.travel_time_ms,
            ),
        ));
    }
    Ok(ElevatorCatalog { entries })
}

/// Charge depuis un chemin sur disque (câblage boot, cf. Task 6).
pub fn load(path: &std::path::Path) -> Result<ElevatorCatalog, ElevatorCatalogError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ElevatorCatalogError::Parse(format!("lecture {path:?} échouée : {e}")))?;
    parse_and_validate(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
        format_version = 1
        [[elevator]]
        id = "9939278384122899325"
        name = "megabuilding-h10-little-china"
        start_floor = 1
        start_delay_ms = 1000
        travel_time_ms = 4000
        floors = [
          { index = 0, hidden = false, inactive = false },
          { index = 1, hidden = false, inactive = false },
        ]
    "#;

    #[test]
    fn valid_catalog_parses_and_builds_an_initial_state() {
        let cat = parse_and_validate(VALID).unwrap();
        assert_eq!(cat.len(), 1);
        assert_eq!(
            cat.name_of(9939278384122899325),
            Some("megabuilding-h10-little-china")
        );
        let states = cat.into_states();
        assert_eq!(states[0].active_floor, 1);
        assert_eq!(states[0].start_delay_ms, 1000);
        assert_eq!(states[0].travel_time_ms, 4000);
        assert_eq!(states[0].floors.len(), 2);
    }

    #[test]
    fn unsupported_format_version_is_rejected() {
        let toml = VALID.replace("format_version = 1", "format_version = 2");
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(
            err,
            ElevatorCatalogError::UnsupportedFormatVersion(2)
        ));
    }

    #[test]
    fn duplicate_elevator_id_is_rejected() {
        let toml = format!("{VALID}\n{}", VALID.replace("format_version = 1", ""));
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, ElevatorCatalogError::DuplicateElevatorId(_)));
    }

    #[test]
    fn an_elevator_with_no_floor_is_rejected() {
        let toml = VALID.replace(
            "floors = [\n          { index = 0, hidden = false, inactive = false },\n          { index = 1, hidden = false, inactive = false },\n        ]",
            "floors = []",
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(err, ElevatorCatalogError::EmptyFloorList { .. }));
    }

    #[test]
    fn a_duplicate_floor_index_is_rejected() {
        let toml = VALID.replace(
            "{ index = 1, hidden = false, inactive = false },",
            "{ index = 0, hidden = false, inactive = false },",
        );
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(
            err,
            ElevatorCatalogError::DuplicateFloorIndex { index: 0, .. }
        ));
    }

    #[test]
    fn a_start_floor_outside_the_floor_list_is_rejected() {
        let toml = VALID.replace("start_floor = 1", "start_floor = 7");
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(matches!(
            err,
            ElevatorCatalogError::StartFloorNotInFloorList { start_floor: 7, .. }
        ));
    }

    #[test]
    fn a_non_numeric_elevator_id_is_rejected() {
        let toml = VALID.replace(r#"id = "9939278384122899325""#, r#"id = "not-a-number""#);
        let err = parse_and_validate(&toml).unwrap_err();
        assert!(
            matches!(err, ElevatorCatalogError::InvalidElevatorId(ref id) if id == "not-a-number")
        );
    }

    #[test]
    fn malformed_toml_is_rejected_with_a_parse_error() {
        let err = parse_and_validate("not valid toml {{{").unwrap_err();
        assert!(matches!(err, ElevatorCatalogError::Parse(_)));
    }

    #[test]
    fn the_example_toml_file_on_disk_parses_successfully() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("elevator-catalog.example.toml");
        let cat = load(&path).expect("elevator-catalog.example.toml doit être valide");
        assert_eq!(cat.len(), 1);
        assert_eq!(
            cat.name_of(9939278384122899325),
            Some("megabuilding-h10-little-china"),
            "le vrai EntityID mesuré en jeu doit survivre à l'aller-retour depuis le fichier livré"
        );
    }

    #[test]
    fn a_floor_with_all_three_position_fields_parses_a_known_position() {
        let toml = r#"
            format_version = 1
            [[elevator]]
            id = "100"
            name = "Test"
            start_floor = 0
            start_delay_ms = 1000
            travel_time_ms = 4000
            [[elevator.floors]]
            index = 0
            position_x = 10.0
            position_y = 20.0
            position_z = 30.0
        "#;
        let catalog = parse_and_validate(toml).expect("catalogue valide");
        let states = catalog.into_states();
        assert_eq!(states[0].floors[0].position, Some([10.0, 20.0, 30.0]));
    }

    #[test]
    fn a_floor_missing_one_position_field_has_no_known_position() {
        let toml = r#"
            format_version = 1
            [[elevator]]
            id = "100"
            name = "Test"
            start_floor = 0
            start_delay_ms = 1000
            travel_time_ms = 4000
            [[elevator.floors]]
            index = 0
            position_x = 10.0
            position_y = 20.0
        "#;
        let catalog = parse_and_validate(toml).expect("catalogue valide");
        let states = catalog.into_states();
        assert_eq!(
            states[0].floors[0].position, None,
            "position_z manquant => aucune position connue, jamais une position à moitié renseignée"
        );
    }

    #[test]
    fn a_floor_with_no_position_fields_has_no_known_position() {
        let toml = r#"
            format_version = 1
            [[elevator]]
            id = "100"
            name = "Test"
            start_floor = 0
            start_delay_ms = 1000
            travel_time_ms = 4000
            [[elevator.floors]]
            index = 0
        "#;
        let catalog = parse_and_validate(toml).expect("catalogue valide");
        let states = catalog.into_states();
        assert_eq!(states[0].floors[0].position, None);
    }
}
