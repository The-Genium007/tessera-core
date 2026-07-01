//! Persistance & reconnexion : sauver la dernière position d'un joueur à la déconnexion, la
//! restituer à la reconnexion. Store abstrait (`PlayerStore`) + impls mémoire et fichier JSON.
//! L'application effective du spawn côté jeu est M5 ; ici on décide et on stocke.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Ce qu'on retient d'un joueur entre deux sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerRecord {
    /// Dernière position connue (x, y, z).
    pub last_position: [f32; 3],
    /// Résidence (domicile) si définie — sert de repli. Non modifiable pour l'instant.
    pub residence: Option<[f32; 3]>,
}

/// Stockage abstrait des enregistrements joueurs, indexé par clé (le `display_name` pour l'instant).
pub trait PlayerStore {
    fn load(&self, key: &str) -> Option<PlayerRecord>;
    fn save(&mut self, key: &str, record: PlayerRecord);
}

/// Store en mémoire (tests, ou serveur éphémère).
#[derive(Default)]
pub struct MemoryStore {
    records: HashMap<String, PlayerRecord>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PlayerStore for MemoryStore {
    fn load(&self, key: &str) -> Option<PlayerRecord> {
        self.records.get(key).cloned()
    }
    fn save(&mut self, key: &str, record: PlayerRecord) {
        self.records.insert(key.to_string(), record);
    }
}

/// D'où vient la position de spawn résolue (pour journalisation/décision).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpawnSource {
    LastPosition,
    Residence,
    Spawn,
}

fn is_valid(p: [f32; 3]) -> bool {
    p.iter().all(|v| v.is_finite())
}

/// Résout où replacer un joueur : dernière position (si valide) → résidence (si valide) → spawn.
pub fn resolve_spawn(record: Option<&PlayerRecord>, spawn: [f32; 3]) -> ([f32; 3], SpawnSource) {
    if let Some(r) = record {
        if is_valid(r.last_position) {
            return (r.last_position, SpawnSource::LastPosition);
        }
        if let Some(res) = r.residence {
            if is_valid(res) {
                return (res, SpawnSource::Residence);
            }
        }
    }
    (spawn, SpawnSource::Spawn)
}

/// Store persistant sur disque (JSON). Charge tout en mémoire à l'ouverture ; réécrit le fichier
/// entier (atomiquement) à chaque sauvegarde. Suffisant à l'échelle d'un serveur self-host.
pub struct FileStore {
    path: PathBuf,
    records: HashMap<String, PlayerRecord>,
}

impl FileStore {
    /// Ouvre le store. Fichier absent → store vide ; fichier illisible/corrompu → store vide + warn.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let records = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("store joueurs illisible ({path:?}: {e}) — démarrage à vide");
                HashMap::new()
            }),
            Err(_) => HashMap::new(),
        };
        Self { path, records }
    }

    /// Réécrit le fichier atomiquement : écrit dans un temporaire voisin puis `rename`.
    fn flush(&self) {
        let Ok(json) = serde_json::to_string_pretty(&self.records) else {
            return;
        };
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }
}

impl PlayerStore for FileStore {
    fn load(&self, key: &str) -> Option<PlayerRecord> {
        self.records.get(key).cloned()
    }
    fn save(&mut self, key: &str, record: PlayerRecord) {
        self.records.insert(key.to_string(), record);
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_saves_and_loads() {
        let mut s = MemoryStore::new();
        assert_eq!(s.load("V"), None);
        let rec = PlayerRecord {
            last_position: [1500.0, -1295.0, 63.0],
            residence: None,
        };
        s.save("V", rec.clone());
        assert_eq!(s.load("V"), Some(rec));
        // Une 2e sauvegarde écrase.
        let rec2 = PlayerRecord {
            last_position: [10.0, 20.0, 30.0],
            residence: Some([1.0, 2.0, 3.0]),
        };
        s.save("V", rec2.clone());
        assert_eq!(s.load("V"), Some(rec2));
    }

    const SPAWN: [f32; 3] = [2387.0, -1295.0, 63.0];

    #[test]
    fn resolve_prefers_last_then_residence_then_spawn() {
        // Dernière position valide → utilisée.
        let r = PlayerRecord {
            last_position: [1500.0, -1295.0, 63.0],
            residence: Some([1.0, 2.0, 3.0]),
        };
        assert_eq!(
            resolve_spawn(Some(&r), SPAWN),
            ([1500.0, -1295.0, 63.0], SpawnSource::LastPosition)
        );

        // Dernière position invalide (NaN) mais résidence valide → résidence.
        let r2 = PlayerRecord {
            last_position: [f32::NAN, 0.0, 0.0],
            residence: Some([1.0, 2.0, 3.0]),
        };
        assert_eq!(
            resolve_spawn(Some(&r2), SPAWN),
            ([1.0, 2.0, 3.0], SpawnSource::Residence)
        );

        // Dernière invalide + pas de résidence → spawn.
        let r3 = PlayerRecord {
            last_position: [f32::INFINITY, 0.0, 0.0],
            residence: None,
        };
        assert_eq!(resolve_spawn(Some(&r3), SPAWN), (SPAWN, SpawnSource::Spawn));

        // Aucun enregistrement → spawn.
        assert_eq!(resolve_spawn(None, SPAWN), (SPAWN, SpawnSource::Spawn));
    }

    use tempfile::tempdir;

    #[test]
    fn file_store_persists_across_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("players.json");
        {
            let mut s = FileStore::open(&path);
            s.save(
                "V",
                PlayerRecord {
                    last_position: [1500.0, -1295.0, 63.0],
                    residence: None,
                },
            );
        }
        // Réouverture : la donnée est relue depuis le disque.
        let s2 = FileStore::open(&path);
        assert_eq!(
            s2.load("V"),
            Some(PlayerRecord {
                last_position: [1500.0, -1295.0, 63.0],
                residence: None,
            })
        );
    }

    #[test]
    fn file_store_starts_empty_on_corrupt_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("players.json");
        std::fs::write(&path, b"{ this is not json").unwrap();
        let s = FileStore::open(&path); // ne panique pas
        assert_eq!(s.load("V"), None);
    }
}
