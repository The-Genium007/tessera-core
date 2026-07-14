//! Vidange Postgres du journal local write-behind (design 2026-07-14, données partagées) —
//! applique un lot d'entrées de `write_behind_journal::WriteBehindJournal` et avance la marque
//! haute (`write_behind_progress`) dans UNE SEULE transaction Postgres : soit les deux
//! réussissent ensemble, soit aucun des deux (jamais de marque avancée sans l'application
//! correspondante, jamais l'inverse) — c'est cette atomicité qui rend le mécanisme idempotent
//! sous rejeu après crash, sans avoir besoin de clé d'idempotence par entrée.
//!
//! Aucun domaine (inventaire/économie/progression/social) n'existe encore dans ce dépôt — ce
//! module reste volontairement générique, le futur domaine fournira sa propre implémentation
//! de `BatchApplier` plutôt que d'être câblé en dur ici (voir design 2026-07-14, "Ce que cette
//! spec ne couvre pas").

use crate::write_behind_journal::JournalEntry;
use sqlx::{PgConnection, PgPool};

/// Applique un lot d'entrées dans la transaction fournie — implémenté par le futur domaine
/// (pas encore écrit). `drain_batch` appelle `apply` puis avance la marque haute dans la MÊME
/// transaction.
// `async fn` dans un trait public est déconseillé par rustc car `Send` ne peut pas être
// exprimé sur le `Future` résultant — sans conséquence ici : ce trait n'est appelé que depuis
// `drain_batch` dans ce même crate (pas de borne `Send` requise par un exécuteur externe).
#[allow(async_fn_in_trait)]
pub trait BatchApplier {
    async fn apply(
        &self,
        tx: &mut PgConnection,
        entries: &[JournalEntry],
    ) -> Result<(), sqlx::Error>;
}

#[derive(Debug)]
pub enum DrainError {
    Sqlx(sqlx::Error),
}

impl std::fmt::Display for DrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrainError::Sqlx(e) => write!(f, "write-behind drain échoué: {e}"),
        }
    }
}

impl std::error::Error for DrainError {}

/// Lit la marque haute Postgres pour `stream_id` — `None` si ce flux n'a jamais été drainé
/// (premier boot, aucune entrée appliquée encore).
pub async fn read_progress(pool: &PgPool, stream_id: &str) -> Result<Option<u64>, DrainError> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT last_applied_seq FROM write_behind_progress WHERE stream_id = $1")
            .bind(stream_id)
            .fetch_optional(pool)
            .await
            .map_err(DrainError::Sqlx)?;
    Ok(row.map(|(seq,)| seq as u64))
}

/// Applique `entries` (non vide) via `applier` et avance la marque haute à
/// `entries.last().seq`, dans une seule transaction — voir doc de module pour la garantie
/// d'atomicité que ça procure.
pub async fn drain_batch<A: BatchApplier>(
    pool: &PgPool,
    stream_id: &str,
    entries: &[JournalEntry],
    applier: &A,
) -> Result<u64, DrainError> {
    let last_seq = entries
        .last()
        .expect("drain_batch appelé avec un lot vide")
        .seq;
    let mut tx = pool.begin().await.map_err(DrainError::Sqlx)?;
    applier
        .apply(&mut tx, entries)
        .await
        .map_err(DrainError::Sqlx)?;
    sqlx::query(
        "INSERT INTO write_behind_progress (stream_id, last_applied_seq) VALUES ($1, $2)
         ON CONFLICT (stream_id) DO UPDATE SET last_applied_seq = EXCLUDED.last_applied_seq",
    )
    .bind(stream_id)
    .bind(last_seq as i64)
    .execute(&mut *tx)
    .await
    .map_err(DrainError::Sqlx)?;
    tx.commit().await.map_err(DrainError::Sqlx)?;
    Ok(last_seq)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    struct NoopApplier;
    impl BatchApplier for NoopApplier {
        async fn apply(
            &self,
            _tx: &mut PgConnection,
            _entries: &[JournalEntry],
        ) -> Result<(), sqlx::Error> {
            Ok(())
        }
    }

    struct FailingApplier;
    impl BatchApplier for FailingApplier {
        async fn apply(
            &self,
            _tx: &mut PgConnection,
            _entries: &[JournalEntry],
        ) -> Result<(), sqlx::Error> {
            Err(sqlx::Error::RowNotFound)
        }
    }

    fn sample_entries(seqs: &[u64]) -> Vec<JournalEntry> {
        seqs.iter()
            .map(|&seq| JournalEntry {
                seq,
                payload: serde_json::json!({}),
            })
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn read_progress_returns_none_when_stream_never_drained(pool: PgPool) {
        let progress = read_progress(&pool, "never-touched").await.unwrap();
        assert_eq!(progress, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn drain_batch_applies_entries_and_records_the_last_seq_as_progress(pool: PgPool) {
        let entries = sample_entries(&[0, 1]);

        let last_seq = drain_batch(&pool, "test-stream", &entries, &NoopApplier)
            .await
            .unwrap();

        assert_eq!(last_seq, 1);
        assert_eq!(read_progress(&pool, "test-stream").await.unwrap(), Some(1));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn drain_batch_called_again_with_a_later_batch_advances_progress_further(pool: PgPool) {
        let first = sample_entries(&[0]);
        let second = sample_entries(&[1]);

        drain_batch(&pool, "test-stream", &first, &NoopApplier)
            .await
            .unwrap();
        drain_batch(&pool, "test-stream", &second, &NoopApplier)
            .await
            .unwrap();

        assert_eq!(read_progress(&pool, "test-stream").await.unwrap(), Some(1));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn drain_batch_does_not_advance_progress_when_apply_fails(pool: PgPool) {
        let entries = sample_entries(&[0]);

        let result = drain_batch(&pool, "test-stream", &entries, &FailingApplier).await;

        assert!(result.is_err());
        assert_eq!(
            read_progress(&pool, "test-stream").await.unwrap(),
            None,
            "la marque haute ne doit jamais avancer si l'application du domaine échoue"
        );
    }
}
