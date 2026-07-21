//! Store Postgres pour la table `bans` (ban à 3 vecteurs : compte/IP/HWID). Suit le patron de
//! `postgres_store.rs` : pas de macro `sqlx::query!` (pas de base accessible à la compilation),
//! méthodes async natives (pas le trait sync `PlayerStore`), erreurs typées.
//!
//! **Ban Postgres-only** : aucun repli JSON pour serveur privé — le ban à 3 vecteurs suppose une
//! notion de compte stable, cohérente avec le fait que Postgres n'existe déjà que sur serveur
//! public (`identity.public = true`). Limitation assumée, pas un oubli.
//!
//! **Aucun HWID brut n'est jamais stocké** — seul `hwid_hash` (haché+salé per-serveur côté
//! launcher, cf. spec) est persisté ici.

use sqlx::PgPool;
use std::fmt;

/// Ligne brute renvoyée par `load_all_active_async` avant reconstruction en `BanRecord` —
/// alias pour éviter le tuple à 7 éléments inline (clippy::type_complexity).
type BanRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<chrono::DateTime<chrono::Utc>>,
    String,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BanScope {
    Temp,
    Perm,
}

impl BanScope {
    fn as_str(&self) -> &'static str {
        match self {
            BanScope::Temp => "temp",
            BanScope::Perm => "perm",
        }
    }
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "temp" => Some(BanScope::Temp),
            "perm" => Some(BanScope::Perm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BanRecord {
    pub subject: Option<String>,
    pub ip: Option<String>,
    pub hwid_hash: Option<String>,
    pub scope: BanScope,
    pub reason: String,
    /// Secondes Unix ; `None` = permanent.
    pub expires_at: Option<i64>,
    pub banned_by: String,
}

#[derive(Debug)]
pub enum BanStoreError {
    /// Aucun des 3 vecteurs n'est renseigné (contrainte `bans_at_least_one_vector`).
    NoVectorProvided,
    Database(String),
}

impl fmt::Display for BanStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BanStoreError::NoVectorProvided => {
                write!(f, "au moins un vecteur (subject/ip/hwid_hash) requis")
            }
            BanStoreError::Database(e) => write!(f, "erreur base de données: {e}"),
        }
    }
}
impl std::error::Error for BanStoreError {}

pub struct BanStore {
    pool: PgPool,
}

impl BanStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insère un ban. Refuse en amont (avant toute requête) si aucun vecteur n'est renseigné —
    /// la contrainte SQL est un filet, pas le chemin normal d'erreur.
    pub async fn save_async(
        &mut self,
        record: &BanRecord,
        banned_by: &str,
    ) -> Result<(), BanStoreError> {
        if record.subject.is_none() && record.ip.is_none() && record.hwid_hash.is_none() {
            return Err(BanStoreError::NoVectorProvided);
        }
        let expires_at = record
            .expires_at
            .map(|secs| chrono::DateTime::from_timestamp(secs, 0).unwrap_or_default());
        sqlx::query(
            "INSERT INTO bans (subject, ip, hwid_hash, scope, reason, expires_at, banned_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&record.subject)
        .bind(&record.ip)
        .bind(&record.hwid_hash)
        .bind(record.scope.as_str())
        .bind(&record.reason)
        .bind(expires_at)
        .bind(banned_by)
        .execute(&self.pool)
        .await
        .map_err(|e| BanStoreError::Database(e.to_string()))?;
        Ok(())
    }

    /// Charge tous les bans actifs (non expirés) — appelé au boot pour peupler le cache RAM
    /// vérifié au Join, et après chaque `ban`/`unban` pour rester synchronisé.
    pub async fn load_all_active_async(&self) -> Result<Vec<BanRecord>, BanStoreError> {
        let rows: Vec<BanRow> = sqlx::query_as(
            "SELECT subject, ip, hwid_hash, scope, reason, expires_at, banned_by FROM bans
                 WHERE expires_at IS NULL OR expires_at > now()",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| BanStoreError::Database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(
                |(subject, ip, hwid_hash, scope, reason, expires_at, banned_by)| BanRecord {
                    subject,
                    ip,
                    hwid_hash,
                    scope: BanScope::from_str(&scope).unwrap_or(BanScope::Temp),
                    reason,
                    expires_at: expires_at.map(|dt| dt.timestamp()),
                    banned_by,
                },
            )
            .collect())
    }

    /// Supprime tous les bans matchant `subject` (unban par compte — le seul vecteur qu'un admin
    /// tape en clair, cohérent avec `/promote`/`/demote` qui prennent un `account`).
    pub async fn delete_by_subject_async(&mut self, subject: &str) -> Result<u64, BanStoreError> {
        let result = sqlx::query("DELETE FROM bans WHERE subject = $1")
            .bind(subject)
            .execute(&self.pool)
            .await
            .map_err(|e| BanStoreError::Database(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn save_then_load_active_roundtrips_a_temp_ban(pool: PgPool) {
        let mut store = BanStore::new(pool);
        let record = BanRecord {
            subject: Some("sub-1".to_string()),
            ip: Some("1.2.3.4".to_string()),
            hwid_hash: None,
            scope: BanScope::Temp,
            reason: "flood".to_string(),
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
            banned_by: "root".to_string(),
        };
        store.save_async(&record, "root").await.unwrap();
        let loaded = store.load_all_active_async().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].subject, Some("sub-1".to_string()));
        assert_eq!(loaded[0].scope, BanScope::Temp);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_rejects_a_record_with_no_vector(pool: PgPool) {
        let mut store = BanStore::new(pool);
        let record = BanRecord {
            subject: None,
            ip: None,
            hwid_hash: None,
            scope: BanScope::Perm,
            reason: "x".to_string(),
            expires_at: None,
            banned_by: "root".to_string(),
        };
        let err = store.save_async(&record, "root").await.unwrap_err();
        assert!(matches!(err, BanStoreError::NoVectorProvided));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn expired_temp_ban_is_excluded_from_active_load(pool: PgPool) {
        let mut store = BanStore::new(pool);
        let record = BanRecord {
            subject: Some("sub-expired".to_string()),
            ip: None,
            hwid_hash: None,
            scope: BanScope::Temp,
            reason: "x".to_string(),
            expires_at: Some(chrono::Utc::now().timestamp() - 3600), // déjà expiré
            banned_by: "root".to_string(),
        };
        store.save_async(&record, "root").await.unwrap();
        let loaded = store.load_all_active_async().await.unwrap();
        assert!(
            loaded.is_empty(),
            "un ban expiré ne doit pas apparaître dans le cache actif"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn perm_ban_has_no_expiry_and_stays_active(pool: PgPool) {
        let mut store = BanStore::new(pool);
        let record = BanRecord {
            subject: None,
            ip: None,
            hwid_hash: Some("hash-abc".to_string()),
            scope: BanScope::Perm,
            reason: "cheating".to_string(),
            expires_at: None,
            banned_by: "root".to_string(),
        };
        store.save_async(&record, "root").await.unwrap();
        let loaded = store.load_all_active_async().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].expires_at, None);
        assert_eq!(loaded[0].hwid_hash, Some("hash-abc".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_by_subject_removes_matching_bans_and_returns_count(pool: PgPool) {
        let mut store = BanStore::new(pool);
        let record = BanRecord {
            subject: Some("sub-to-unban".to_string()),
            ip: None,
            hwid_hash: None,
            scope: BanScope::Temp,
            reason: "x".to_string(),
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
            banned_by: "root".to_string(),
        };
        store.save_async(&record, "root").await.unwrap();
        let deleted = store.delete_by_subject_async("sub-to-unban").await.unwrap();
        assert_eq!(deleted, 1);
        let loaded = store.load_all_active_async().await.unwrap();
        assert!(loaded.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_by_subject_returns_zero_when_nothing_matches(pool: PgPool) {
        let mut store = BanStore::new(pool);
        let deleted = store.delete_by_subject_async("never-banned").await.unwrap();
        assert_eq!(deleted, 0);
    }
}
