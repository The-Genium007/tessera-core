//! Persistance & reconnexion : sauver la dernière position d'un joueur à la déconnexion, la
//! restituer à la reconnexion. Store abstrait (`PlayerStore`) + impls mémoire et fichier JSON.
//! L'application effective du spawn côté jeu est M5 ; ici on décide et on stocke.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}
