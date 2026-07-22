//! Télémétrie combat PNJ hostiles (spec §2 : "signal net, horodaté, archivable" pour la détection
//! god-mode/anomalies PAR REVUE HUMAINE — ce module n'analyse rien lui-même, il enregistre
//! seulement). JSONL append-only, même patron que `write_behind_journal.rs`
//! (`OpenOptions::new().create(true).append(true).open(path)`).
//!
//! Détection god-mode automatique (corrélation attaques-subies-sans-jamais-tomber) explicitement
//! HORS PÉRIMÈTRE de ce module et de ce plan : nécessiterait un état cross-tick de comptage par
//! joueur, une extension future si le besoin réel apparaît après un premier playtest — pas
//! construite à l'aveugle ici (spec §2 : "shadow-flag + revue humaine").

use serde::Serialize;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct CombatTelemetryEvent {
    pub npc_id: u64,
    pub archetype: u32,
    pub killer: u64,
    pub timestamp_ms: u64,
}

/// Ajoute une ligne JSON à `path` (créé si absent — le répertoire parent, lui, doit déjà exister,
/// cf. `std::fs::OpenOptions`, contrairement à `WriteBehindJournal::open` qui appelle
/// `create_dir_all` : ce module est plus simple, pas de reprise de séquence à recalculer).
/// Échec d'écriture = erreur remontée à l'appelant, PAS un panic (la télémétrie ne doit jamais
/// faire planter un tick serveur réel — l'appelant, `tick_npcs`, choisit de logger l'erreur plutôt
/// que de la propager plus loin).
pub fn append_combat_event(path: &Path, event: &CombatTelemetryEvent) -> std::io::Result<()> {
    let json = serde_json::to_string(event)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{json}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_combat_event_writes_one_json_line_per_call() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("combat.jsonl");
        let event = CombatTelemetryEvent {
            npc_id: 1_000_000,
            archetype: 4,
            killer: 42,
            timestamp_ms: 12345,
        };
        append_combat_event(&path, &event).unwrap();
        append_combat_event(&path, &event).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content.lines().count(),
            2,
            "un appel = une ligne JSON, append-only"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["npc_id"], 1_000_000);
        assert_eq!(parsed["killer"], 42);
    }

    #[test]
    fn append_combat_event_creates_the_file_if_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("combat.jsonl");
        // Note : le répertoire parent DOIT exister — ce module ne crée pas d'arborescence, cf.
        // std::fs::OpenOptions — vérifié ce comportement réel plutôt que de le supposer :
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let event = CombatTelemetryEvent {
            npc_id: 2,
            archetype: 1,
            killer: 1,
            timestamp_ms: 0,
        };
        append_combat_event(&path, &event).unwrap();
        assert!(path.exists());
    }
}
