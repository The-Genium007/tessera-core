//! Journal local append-only (JSONL) pour le mécanisme write-behind (données partagées,
//! design 2026-07-14) — chaque entrée porte un numéro de séquence local monotone, jamais un
//! UUID (le journal est local à un seul process Gateway, pas distribué). Voir `write_behind.rs`
//! pour la vidange vers Postgres et la garantie idempotente qui en découle.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
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

/// Résultat interne du scan d'un journal : les entrées valides (respectant `since_seq`), et la
/// longueur en octets du fichier jusqu'à la fin de la dernière entrée valide complète — ignore
/// les octets d'une éventuelle ligne finale tronquée par un crash. `open()` utilise cette
/// longueur pour tronquer physiquement le fichier (un seul appel `set_len`, atomique) avant de
/// reprendre l'écriture — voir sa doc pour pourquoi une réécriture complète a été écartée.
struct ScanOutcome {
    entries: Vec<JournalEntry>,
    clean_len: u64,
}

/// Lit `path` en octets bruts (pas ligne par ligne) pour pouvoir suivre précisément la position
/// en octets de chaque ligne — nécessaire à `ScanOutcome::clean_len`. Une ligne complète (`\n`
/// terminale présente) qui échoue à parser est une vraie corruption (erreur immédiate). Un
/// reste final SANS `\n` est traité comme une écriture tronquée par un crash : ignoré
/// silencieusement, jamais compté dans `clean_len` ni dans les entrées retournées.
fn scan(path: &Path, since_seq: u64) -> io::Result<ScanOutcome> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(ScanOutcome {
                entries: Vec::new(),
                clean_len: 0,
            })
        }
        Err(e) => return Err(e),
    };

    let mut entries = Vec::new();
    let mut clean_len: u64 = 0;
    let mut pos: usize = 0;

    while pos < bytes.len() {
        let rest = &bytes[pos..];
        match rest.iter().position(|&b| b == b'\n') {
            Some(newline_index) => {
                let line = &rest[..newline_index];
                let consumed = newline_index + 1;
                if !line.is_empty() {
                    let text = std::str::from_utf8(line).map_err(io::Error::other)?;
                    let entry: JournalEntry =
                        serde_json::from_str(text).map_err(io::Error::other)?;
                    if entry.seq >= since_seq {
                        entries.push(entry);
                    }
                }
                pos += consumed;
                clean_len = pos as u64;
            }
            None => {
                // Reste final sans `\n` : soit vide, soit une écriture tronquée par un crash —
                // dans les deux cas on s'arrête ici sans avancer clean_len, sans erreur (voir
                // doc de fonction).
                break;
            }
        }
    }

    Ok(ScanOutcome { entries, clean_len })
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
        let outcome = scan(path, 0)?;
        let next_seq = outcome
            .entries
            .last()
            .map(|entry| entry.seq + 1)
            .unwrap_or(0);

        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let current_len = file.metadata()?.len();
        if outcome.clean_len < current_len {
            // Une ligne tronquée par un crash traîne en fin de fichier (voir `scan`) — retirée
            // par un seul appel `set_len`, atomique : contrairement à une réécriture complète
            // du fichier (essayée dans un correctif précédent, écartée en revue), `set_len` ne
            // déplace aucune donnée et n'a pas d'état intermédiaire interrompable — soit il
            // aboutit, soit il n'a aucun effet, jamais une perte partielle d'entrées déjà
            // durcies. Appelé seulement quand nécessaire : un redémarrage propre (pas de ligne
            // tronquée) ne touche pas le fichier du tout.
            file.set_len(outcome.clean_len)?;
        }
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
        Ok(scan(path, since_seq)?.entries)
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

    #[test]
    fn read_since_discards_a_truncated_trailing_line_without_erroring() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        {
            let mut journal = WriteBehindJournal::open(&path).unwrap();
            journal.append(serde_json::json!({"a": 0})).unwrap();
            journal.append(serde_json::json!({"a": 1})).unwrap();
            journal.flush().unwrap();
        }
        // Simule une écriture interrompue par un crash (SIGKILL/coupure) : une ligne finale
        // tronquée, ni valide JSON ni vide, ajoutée directement au fichier (pas via `append`,
        // qui écrirait toujours une ligne complète).
        use std::fs::OpenOptions;
        use std::io::Write as _;
        let mut raw = OpenOptions::new().append(true).open(&path).unwrap();
        write!(raw, "{{\"seq\":2,\"payloa").unwrap(); // coupé en plein milieu, pas de \n final

        let entries = WriteBehindJournal::read_since(&path, 0).unwrap();

        assert_eq!(
            entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![0, 1],
            "la ligne tronquée finale doit être ignorée silencieusement, pas faire échouer la lecture"
        );
    }

    #[test]
    fn read_since_still_errors_on_corruption_that_is_not_the_last_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        {
            let mut journal = WriteBehindJournal::open(&path).unwrap();
            journal.append(serde_json::json!({"a": 0})).unwrap();
            journal.flush().unwrap();
        }
        use std::fs::OpenOptions;
        use std::io::Write as _;
        let mut raw = OpenOptions::new().append(true).open(&path).unwrap();
        // Ligne du milieu corrompue (pas la dernière), suivie d'une ligne valide complète —
        // une vraie corruption, pas une écriture interrompue en fin de fichier.
        writeln!(raw, "not valid json at all").unwrap();
        {
            // Ouvrir un nouveau journal pour ajouter une entrée valide APRÈS la ligne corrompue
            // nécessiterait de connaître le prochain seq — on écrit directement la ligne JSON
            // valide pour ne pas dépendre de open() (qui échouerait déjà à cause de la ligne
            // corrompue avant la dernière).
            writeln!(raw, "{{\"seq\":2,\"payload\":{{}}}}").unwrap();
        }

        let result = WriteBehindJournal::read_since(&path, 0);

        assert!(
            result.is_err(),
            "une ligne corrompue qui N'EST PAS la dernière doit toujours faire échouer la lecture"
        );
    }

    #[test]
    fn open_after_a_torn_trailing_line_cleans_the_file_so_subsequent_appends_stay_valid() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        {
            let mut journal = WriteBehindJournal::open(&path).unwrap();
            journal.append(serde_json::json!({"a": 0})).unwrap();
            journal.flush().unwrap();
        }
        // Simule un crash en pleine écriture : ligne tronquée ajoutée directement au fichier
        // (pas via `append`, qui écrirait toujours une ligne complète avec son `\n` final).
        use std::fs::OpenOptions;
        use std::io::Write as _;
        {
            let mut raw = OpenOptions::new().append(true).open(&path).unwrap();
            write!(raw, "{{\"seq\":1,\"payloa").unwrap(); // pas de \n final
        }

        // Redémarrage : open() doit nettoyer le fichier (retirer les octets tronqués) avant de
        // reprendre l'écriture — sinon le prochain append() se fusionnerait avec ces octets.
        let mut reopened = WriteBehindJournal::open(&path).unwrap();
        let seq = reopened.append(serde_json::json!({"a": 1})).unwrap();
        reopened.flush().unwrap();

        assert_eq!(
            seq, 1,
            "la numérotation doit reprendre après l'entrée 0, la ligne tronquée ne comptant pas"
        );

        // Relire tout le journal depuis le disque : la nouvelle entrée doit être lisible et
        // correcte, pas fusionnée avec les octets tronqués laissés par le crash simulé.
        let entries = WriteBehindJournal::read_since(&path, 0).unwrap();
        assert_eq!(
            entries,
            vec![
                JournalEntry {
                    seq: 0,
                    payload: serde_json::json!({"a": 0})
                },
                JournalEntry {
                    seq: 1,
                    payload: serde_json::json!({"a": 1})
                },
            ]
        );
    }

    #[test]
    fn open_on_a_clean_journal_does_not_truncate_any_valid_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        {
            let mut journal = WriteBehindJournal::open(&path).unwrap();
            journal.append(serde_json::json!({"a": 0})).unwrap();
            journal.append(serde_json::json!({"a": 1})).unwrap();
            journal.flush().unwrap();
        }

        // Aucune ligne tronquée ici (arrêt propre) — un redémarrage ne doit rien perdre.
        let reopened_entries = {
            let _journal = WriteBehindJournal::open(&path).unwrap();
            WriteBehindJournal::read_since(&path, 0).unwrap()
        };

        assert_eq!(
            reopened_entries,
            vec![
                JournalEntry {
                    seq: 0,
                    payload: serde_json::json!({"a": 0})
                },
                JournalEntry {
                    seq: 1,
                    payload: serde_json::json!({"a": 1})
                },
            ],
            "un redémarrage propre (sans ligne tronquée) ne doit jamais perdre d'entrée valide"
        );
    }
}
