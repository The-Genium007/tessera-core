//! Store Postgres pour la table `characters` (modèle multi-personnage). Suit le patron de
//! `postgres_store.rs` : pas de macro `sqlx::query!`, méthodes async natives (pas le trait sync
//! `PlayerStore`), erreurs typées avec détection explicite de violation de contrainte unique.

use sqlx::PgPool;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    pub id: i64,
    pub owner: String,
    pub pseudonym: String,
    pub appearance: Vec<u8>,
    pub last_position: [f32; 3],
    pub residence: Option<[f32; 3]>,
}

#[derive(Debug)]
pub enum CharacterStoreError {
    /// Le `pseudonym` demandé est déjà utilisé par un autre personnage (contrainte unique
    /// `idx_characters_pseudonym`) — premier arrivé, premier servi.
    PseudonymTaken,
    NotFound,
    /// Le personnage existe mais n'appartient pas à l'`owner` qui tente l'action.
    NotOwner,
    Database(String),
}

impl fmt::Display for CharacterStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CharacterStoreError::PseudonymTaken => write!(f, "pseudonyme déjà utilisé"),
            CharacterStoreError::NotFound => write!(f, "personnage introuvable"),
            CharacterStoreError::NotOwner => {
                write!(f, "ce personnage n'appartient pas à ce compte")
            }
            CharacterStoreError::Database(e) => write!(f, "erreur base de données: {e}"),
        }
    }
}
impl std::error::Error for CharacterStoreError {}

pub struct CharacterStore {
    pool: PgPool,
}

impl CharacterStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::type_complexity)]
    pub async fn list_by_owner_async(
        &self,
        owner: &str,
    ) -> Result<Vec<Character>, CharacterStoreError> {
        let rows: Vec<(i64, String, String, Vec<u8>, Vec<f32>, Option<Vec<f32>>)> = sqlx::query_as(
            "SELECT id, owner, pseudonym, appearance, last_position, residence
             FROM characters WHERE owner = $1 ORDER BY id",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CharacterStoreError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(
                |(id, owner, pseudonym, appearance, last_position, residence)| Character {
                    id,
                    owner,
                    pseudonym,
                    appearance,
                    last_position: [last_position[0], last_position[1], last_position[2]],
                    residence: residence.map(|r| [r[0], r[1], r[2]]),
                },
            )
            .collect())
    }

    /// Crée un personnage. Refuse proprement (`PseudonymTaken`) plutôt que d'écraser silencieusement.
    pub async fn create_async(
        &mut self,
        owner: &str,
        pseudonym: &str,
        appearance: &[u8],
    ) -> Result<Character, CharacterStoreError> {
        let last_position: [f32; 3] = [0.0, 0.0, 0.0];
        let result: Result<(i64,), _> = sqlx::query_as(
            "INSERT INTO characters (owner, pseudonym, appearance, last_position)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(owner)
        .bind(pseudonym)
        .bind(appearance)
        .bind(&last_position[..])
        .fetch_one(&self.pool)
        .await;
        match result {
            Ok((id,)) => Ok(Character {
                id,
                owner: owner.to_string(),
                pseudonym: pseudonym.to_string(),
                appearance: appearance.to_vec(),
                last_position,
                residence: None,
            }),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                Err(CharacterStoreError::PseudonymTaken)
            }
            Err(e) => Err(CharacterStoreError::Database(e.to_string())),
        }
    }

    /// Supprime un personnage — refuse si `owner` ne correspond pas (anti-triche basique : un
    /// joueur ne peut pas supprimer le personnage d'un autre).
    pub async fn delete_async(
        &mut self,
        owner: &str,
        character_id: i64,
    ) -> Result<(), CharacterStoreError> {
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT owner FROM characters WHERE id = $1")
                .bind(character_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CharacterStoreError::Database(e.to_string()))?;
        match existing {
            None => Err(CharacterStoreError::NotFound),
            Some((existing_owner,)) if existing_owner != owner => {
                Err(CharacterStoreError::NotOwner)
            }
            Some(_) => {
                sqlx::query("DELETE FROM characters WHERE id = $1")
                    .bind(character_id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| CharacterStoreError::Database(e.to_string()))?;
                Ok(())
            }
        }
    }

    pub async fn save_progress_async(
        &mut self,
        character_id: i64,
        position: [f32; 3],
    ) -> Result<(), CharacterStoreError> {
        let result = sqlx::query(
            "UPDATE characters SET last_position = $1, updated_at = now() WHERE id = $2",
        )
        .bind(&position[..])
        .bind(character_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CharacterStoreError::Database(e.to_string()))?;
        if result.rows_affected() == 0 {
            return Err(CharacterStoreError::NotFound);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_then_list_by_owner_roundtrips_a_character(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        let created = store
            .create_async("owner-1", "Nyx", b"appearance-blob")
            .await
            .unwrap();
        assert_eq!(created.pseudonym, "Nyx");
        let listed = store.list_by_owner_async("owner-1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pseudonym, "Nyx");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_a_duplicate_pseudonym_across_different_owners(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        store.create_async("owner-1", "Nyx", b"").await.unwrap();
        let err = store.create_async("owner-2", "Nyx", b"").await.unwrap_err();
        assert!(matches!(err, CharacterStoreError::PseudonymTaken));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_by_owner_returns_empty_for_an_owner_with_no_characters(pool: PgPool) {
        let store = CharacterStore::new(pool);
        let listed = store.list_by_owner_async("nobody").await.unwrap();
        assert!(listed.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_removes_a_character_owned_by_the_caller(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        let created = store.create_async("owner-1", "Nyx", b"").await.unwrap();
        store.delete_async("owner-1", created.id).await.unwrap();
        let listed = store.list_by_owner_async("owner-1").await.unwrap();
        assert!(listed.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_refuses_when_caller_is_not_the_owner(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        let created = store.create_async("owner-1", "Nyx", b"").await.unwrap();
        let err = store.delete_async("owner-2", created.id).await.unwrap_err();
        assert!(matches!(err, CharacterStoreError::NotOwner));
        // Le personnage doit toujours exister.
        let listed = store.list_by_owner_async("owner-1").await.unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_returns_not_found_for_an_unknown_id(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        let err = store.delete_async("owner-1", 999_999).await.unwrap_err();
        assert!(matches!(err, CharacterStoreError::NotFound));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_progress_updates_the_stored_position(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        let created = store.create_async("owner-1", "Nyx", b"").await.unwrap();
        store
            .save_progress_async(created.id, [1.0, 2.0, 3.0])
            .await
            .unwrap();
        let listed = store.list_by_owner_async("owner-1").await.unwrap();
        assert_eq!(listed[0].last_position, [1.0, 2.0, 3.0]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_progress_returns_not_found_for_an_unknown_id(pool: PgPool) {
        let mut store = CharacterStore::new(pool);
        let err = store
            .save_progress_async(999_999, [0.0, 0.0, 0.0])
            .await
            .unwrap_err();
        assert!(matches!(err, CharacterStoreError::NotFound));
    }
}
