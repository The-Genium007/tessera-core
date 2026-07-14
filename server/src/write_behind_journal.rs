//! Journal local append-only (JSONL) pour le mécanisme write-behind (données partagées,
//! design 2026-07-14) — chaque entrée porte un numéro de séquence local monotone, jamais un
//! UUID (le journal est local à un seul process Gateway, pas distribué). Voir `write_behind.rs`
//! pour la vidange vers Postgres et la garantie idempotente qui en découle.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JournalEntry {
    pub seq: u64,
    pub payload: serde_json::Value,
}

pub struct WriteBehindJournal {
    file: File,
    next_seq: u64,
}

impl WriteBehindJournal {
    /// Ouvre (ou crée) le journal à `path`. Si le fichier existe déjà (redémarrage après
    /// crash), relit la dernière entrée pour reprendre la numérotation de séquence là où elle
    /// s'était arrêtée — sans ça, un redémarrage réutiliserait des numéros déjà pris, cassant
    /// la monotonie dont dépend `read_since`.
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let next_seq = Self::read_since(path, 0)?
            .last()
            .map(|entry| entry.seq + 1)
            .unwrap_or(0);
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file, next_seq })
    }

    /// Ajoute une entrée, lui assigne le prochain numéro de séquence, retourne ce numéro.
    /// Écriture bufferisée (pas de `fsync` ici, voir `flush`) — appelée dans le chemin
    /// d'action, avant la réponse au joueur (design 2026-07-14, "ordre précis dans le chemin
    /// d'action").
    pub fn append(&mut self, payload: serde_json::Value) -> io::Result<u64> {
        let seq = self.next_seq;
        let entry = JournalEntry { seq, payload };
        let line = serde_json::to_string(&entry).expect("JournalEntry serialization cannot fail");
        writeln!(self.file, "{line}")?;
        self.next_seq += 1;
        Ok(seq)
    }

    /// `fsync` explicite — appelé par lot vidé (pas par entrée), voir `write_behind.rs`.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()
    }

    /// Relit les entrées du journal à `path` dont le numéro de séquence est `>= since_seq`.
    /// Fonction associée (pas besoin d'un journal ouvert en écriture) : utilisée en interne par
    /// `open` (reprise de numérotation) et par la récupération après crash (`write_behind.rs`,
    /// `entries_to_replay`).
    pub fn read_since(path: &Path, since_seq: u64) -> io::Result<Vec<JournalEntry>> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            let entry: JournalEntry = serde_json::from_str(&line).map_err(io::Error::other)?;
            if entry.seq >= since_seq {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_assigns_sequential_seq_numbers_starting_at_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = WriteBehindJournal::open(&path).unwrap();

        let seq0 = journal.append(serde_json::json!({"a": 1})).unwrap();
        let seq1 = journal.append(serde_json::json!({"a": 2})).unwrap();

        assert_eq!(seq0, 0);
        assert_eq!(seq1, 1);
    }

    #[test]
    fn append_then_read_since_zero_roundtrips_all_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = WriteBehindJournal::open(&path).unwrap();
        journal.append(serde_json::json!({"a": 1})).unwrap();
        journal.append(serde_json::json!({"a": 2})).unwrap();
        journal.flush().unwrap();

        let entries = WriteBehindJournal::read_since(&path, 0).unwrap();

        assert_eq!(
            entries,
            vec![
                JournalEntry {
                    seq: 0,
                    payload: serde_json::json!({"a": 1})
                },
                JournalEntry {
                    seq: 1,
                    payload: serde_json::json!({"a": 2})
                },
            ]
        );
    }

    #[test]
    fn read_since_only_returns_entries_at_or_after_the_given_seq() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let mut journal = WriteBehindJournal::open(&path).unwrap();
        journal.append(serde_json::json!({"a": 0})).unwrap();
        journal.append(serde_json::json!({"a": 1})).unwrap();
        journal.append(serde_json::json!({"a": 2})).unwrap();
        journal.flush().unwrap();

        let entries = WriteBehindJournal::read_since(&path, 1).unwrap();

        assert_eq!(
            entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn read_since_returns_empty_vec_when_file_does_not_exist() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.jsonl");

        let entries = WriteBehindJournal::read_since(&path, 0).unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn open_after_existing_entries_resumes_seq_numbering_after_the_last_one() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        {
            let mut journal = WriteBehindJournal::open(&path).unwrap();
            journal.append(serde_json::json!({"a": 0})).unwrap();
            journal.append(serde_json::json!({"a": 1})).unwrap();
            journal.flush().unwrap();
        } // le journal est "fermé" (drop) ici, simulant un redémarrage du process

        let mut reopened = WriteBehindJournal::open(&path).unwrap();
        let seq = reopened.append(serde_json::json!({"a": 2})).unwrap();

        assert_eq!(
            seq, 2,
            "la numérotation doit reprendre après la dernière entrée existante, pas repartir de 0"
        );
    }

    #[test]
    fn open_on_missing_file_starts_seq_at_zero() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("fresh.jsonl");

        let mut journal = WriteBehindJournal::open(&path).unwrap();
        let seq = journal.append(serde_json::json!({"a": 0})).unwrap();

        assert_eq!(seq, 0);
    }
}
