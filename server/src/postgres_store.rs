//! Store Postgres pour les données joueur durables (catégorie B, design stockage
//! 2026-07-09) — clé = subject OIDC ZITADEL, écriture directe et synchrone au moment
//! de l'action (pas de tampon intermédiaire ; le débit d'événements durables est
//! largement absorbable par Postgres à cette échelle — voir
//! `docs/superpowers/specs/2026-07-09-game-data-storage-strategy-design.md`).
//!
//! **Décision sync/async** : le trait `PlayerStore` existant (`persistence.rs`) est
//! synchrone — conçu pour `MemoryStore`/`FileStore`, qui n'ont pas d'I/O réseau. `sqlx`
//! est async de bout en bout. Plutôt que de forcer `PostgresStore` dans ce trait via un
//! `tokio::runtime::Handle::block_on` (bloque un thread de l'executor tokio pour une
//! opération réseau — piège classique, en particulier sous charge), `PostgresStore`
//! n'implémente PAS `PlayerStore` : il expose ses propres méthodes async
//! (`save_async`/`load_async`), appelées directement par l'appelant (le Gateway, qui
//! tourne déjà dans un contexte tokio).
//!
//! **Pas de macro `sqlx::query!`/`query_as!`** (cohérence avec `platform-api/src/db.rs` :
//! ces macros compile-time nécessitent soit une base accessible à la compilation, soit
//! un cache `.sqlx/` committé — évité ici comme dans `platform-api`). Requêtes construites
//! avec `sqlx::query`/`query_as` + `.bind(..)`, vérifiées uniquement à l'exécution (tests).
//!
//! **Contrainte unique sur `display_name`** : parade structurelle au bug d'identité du
//! playtest 1 (deux comptes distincts pouvant écraser silencieusement le même
//! `display_name` sous `FileStore`). Ici, un conflit de pseudo échoue proprement avec
//! `StoreError::DisplayNameConflict` au lieu d'écraser silencieusement — premier arrivé,
//! premier servi.

use crate::persistence::PlayerRecord;
use sqlx::PgPool;
use std::fmt;

/// Store Postgres pour `player_records`. Ne implémente pas `PlayerStore` (voir doc de
/// module) — méthodes async natives uniquement.
pub struct PostgresStore {
    pool: PgPool,
}

/// Erreurs possibles lors d'un accès `PostgresStore`.
#[derive(Debug)]
pub enum StoreError {
    /// Le `display_name` demandé est déjà utilisé par un autre `subject` (contrainte
    /// unique `idx_player_records_display_name`) — premier arrivé, premier servi.
    DisplayNameConflict,
    /// Toute autre erreur Postgres/sqlx, avec son message d'origine.
    Database(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::DisplayNameConflict => {
                write!(f, "display_name déjà utilisé par un autre compte")
            }
            StoreError::Database(e) => write!(f, "erreur base de données: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Sauvegarde (insert ou update) l'enregistrement d'un joueur, indexé par `subject`
    /// OIDC. Un conflit sur `display_name` avec un *autre* `subject` échoue proprement
    /// (`StoreError::DisplayNameConflict`) plutôt que d'écraser silencieusement.
    pub async fn save_async(
        &mut self,
        subject: &str,
        display_name: &str,
        record: PlayerRecord,
    ) -> Result<(), StoreError> {
        let residence: Option<&[f32]> = record.residence.as_ref().map(|r| &r[..]);
        let result = sqlx::query(
            "INSERT INTO player_records (subject, display_name, last_position, residence)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (subject) DO UPDATE SET
                 last_position = EXCLUDED.last_position,
                 residence = EXCLUDED.residence,
                 updated_at = now()",
        )
        .bind(subject)
        .bind(display_name)
        .bind(&record.last_position[..])
        .bind(residence)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                Err(StoreError::DisplayNameConflict)
            }
            Err(e) => Err(StoreError::Database(e.to_string())),
        }
    }

    /// Charge l'enregistrement d'un joueur par `subject` OIDC. `None` si inconnu.
    pub async fn load_async(&self, subject: &str) -> Result<Option<PlayerRecord>, StoreError> {
        let row: Option<(Vec<f32>, Option<Vec<f32>>)> = sqlx::query_as(
            "SELECT last_position, residence FROM player_records WHERE subject = $1",
        )
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(row.map(|(last_position, residence)| PlayerRecord {
            last_position: last_position.try_into().unwrap_or([0.0; 3]),
            residence: residence.map(|v| v.try_into().unwrap_or([0.0; 3])),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn save_then_load_roundtrips_record(pool: PgPool) {
        let mut store = PostgresStore::new(pool);
        let record = PlayerRecord {
            last_position: [1.0, 2.0, 3.0],
            residence: None,
        };
        store
            .save_async("oidc-sub-abc", "display_name_1", record.clone())
            .await
            .unwrap();

        let loaded = store.load_async("oidc-sub-abc").await.unwrap();
        assert_eq!(loaded, Some(record));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_then_load_roundtrips_record_with_residence(pool: PgPool) {
        let mut store = PostgresStore::new(pool);
        let record = PlayerRecord {
            last_position: [1.0, 2.0, 3.0],
            residence: Some([4.0, 5.0, 6.0]),
        };
        store
            .save_async("oidc-sub-res", "display_name_res", record.clone())
            .await
            .unwrap();

        let loaded = store.load_async("oidc-sub-res").await.unwrap();
        assert_eq!(loaded, Some(record));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_returns_none_for_unknown_subject(pool: PgPool) {
        let store = PostgresStore::new(pool);
        assert_eq!(store.load_async("unknown-sub").await.unwrap(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn saving_again_for_same_subject_updates_the_record(pool: PgPool) {
        let mut store = PostgresStore::new(pool);
        store
            .save_async(
                "sub-a",
                "Lucas",
                PlayerRecord {
                    last_position: [0.0; 3],
                    residence: None,
                },
            )
            .await
            .unwrap();
        store
            .save_async(
                "sub-a",
                "Lucas",
                PlayerRecord {
                    last_position: [9.0, 8.0, 7.0],
                    residence: Some([1.0, 1.0, 1.0]),
                },
            )
            .await
            .unwrap();

        let loaded = store.load_async("sub-a").await.unwrap();
        assert_eq!(
            loaded,
            Some(PlayerRecord {
                last_position: [9.0, 8.0, 7.0],
                residence: Some([1.0, 1.0, 1.0]),
            })
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn display_name_conflict_is_first_come_first_served(pool: PgPool) {
        let mut store = PostgresStore::new(pool);
        store
            .save_async(
                "sub-a",
                "Lucas",
                PlayerRecord {
                    last_position: [0.0; 3],
                    residence: None,
                },
            )
            .await
            .unwrap();

        // Un deuxième subject tentant le même display_name doit échouer proprement
        // (contrainte unique), pas silencieusement écraser — parade exacte au bug
        // playtest 1.
        let result = store
            .save_async(
                "sub-b",
                "Lucas",
                PlayerRecord {
                    last_position: [1.0; 3],
                    residence: None,
                },
            )
            .await;
        assert!(matches!(result, Err(StoreError::DisplayNameConflict)));

        // Le premier enregistrement n'a pas été altéré.
        let loaded = store.load_async("sub-a").await.unwrap();
        assert_eq!(
            loaded,
            Some(PlayerRecord {
                last_position: [0.0; 3],
                residence: None,
            })
        );
        // Le deuxième subject n'a jamais été inséré.
        assert_eq!(store.load_async("sub-b").await.unwrap(), None);
    }
}
