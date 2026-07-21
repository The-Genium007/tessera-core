//! Cache de lecture/tampon de reprise pour l'état chaud (position, catégorie A du design
//! stockage 2026-07-09) — **un seul Redis PARTAGÉ, Gateway-central** (jamais un par shard : la
//! clé est le joueur qui migre entre shards au handoff, un cache par-shard casserait la
//! continuité cross-shard — décision explicite, cf. discussion 10 du 2026-07-18). Écrit/lu
//! uniquement par le Gateway ; les Shards n'y touchent jamais. Jamais la source de vérité pour
//! la durabilité : TTL systématique, pas de garantie de persistance au-delà de la fenêtre de
//! reprise visée.

use redis::aio::ConnectionManager;
use redis::AsyncCommands;

const HOT_STATE_TTL_SECS: i64 = 120; // fenêtre de reprise après crash — valeur à affiner en charge

pub struct HotStateCache {
    conn: ConnectionManager,
}

#[derive(Debug)]
pub enum CacheError {
    Connection(String),
}

impl HotStateCache {
    pub async fn connect(redis_url: &str) -> Result<Self, CacheError> {
        let client =
            redis::Client::open(redis_url).map_err(|e| CacheError::Connection(e.to_string()))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| CacheError::Connection(e.to_string()))?;
        Ok(Self { conn })
    }

    fn key(subject: &str) -> String {
        format!("hot_state:{subject}")
    }

    pub async fn write(&self, subject: &str, position: [f32; 3]) -> Result<(), CacheError> {
        let mut conn = self.conn.clone();
        let encoded = format!("{},{},{}", position[0], position[1], position[2]);
        conn.set_ex::<_, _, ()>(Self::key(subject), encoded, HOT_STATE_TTL_SECS as u64)
            .await
            .map_err(|e| CacheError::Connection(e.to_string()))
    }

    pub async fn read(&self, subject: &str) -> Result<Option<[f32; 3]>, CacheError> {
        let mut conn = self.conn.clone();
        let raw: Option<String> = conn
            .get(Self::key(subject))
            .await
            .map_err(|e| CacheError::Connection(e.to_string()))?;
        Ok(raw.and_then(|s| {
            let parts: Vec<f32> = s.split(',').filter_map(|p| p.parse().ok()).collect();
            if parts.len() == 3 {
                Some([parts[0], parts[1], parts[2]])
            } else {
                None
            }
        }))
    }
}

/// Décide si une écriture hot-state doit avoir lieu MAINTENANT pour ce client, étant donné la
/// dernière écriture connue et l'intervalle minimal configuré. Pure — aucune I/O.
pub fn should_write_now(
    last_write: Option<std::time::Instant>,
    min_interval: std::time::Duration,
) -> bool {
    match last_write {
        None => true,
        Some(t) => t.elapsed() >= min_interval,
    }
}

#[cfg(test)]
mod throttle_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn should_write_now_is_true_when_never_written_before() {
        assert!(should_write_now(None, Duration::from_secs(2)));
    }

    #[test]
    fn should_write_now_is_false_immediately_after_a_write() {
        let now = std::time::Instant::now();
        assert!(!should_write_now(Some(now), Duration::from_secs(5)));
    }

    #[test]
    fn should_write_now_is_true_once_the_interval_has_elapsed() {
        let past = std::time::Instant::now() - Duration::from_secs(10);
        assert!(should_write_now(Some(past), Duration::from_secs(5)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nécessite un Redis accessible en local pour les tests — documenter la commande
    // (`docker run -p 6379:6379 redis:7`) dans le message de commit ou un README serveur.

    #[tokio::test]
    async fn write_then_read_roundtrips_position() {
        let cache = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .unwrap();
        cache.write("oidc-sub-abc", [1.0, 2.0, 3.0]).await.unwrap();

        let read = cache.read("oidc-sub-abc").await.unwrap();
        assert_eq!(read, Some([1.0, 2.0, 3.0]));
    }

    #[tokio::test]
    async fn read_returns_none_for_unknown_subject() {
        let cache = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .unwrap();
        let read = cache.read("never-written-subject").await.unwrap();
        assert_eq!(read, None);
    }

    #[tokio::test]
    async fn write_sets_ttl_so_stale_entries_expire() {
        let cache = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .unwrap();
        cache
            .write("oidc-sub-ttl-test", [0.0, 0.0, 0.0])
            .await
            .unwrap();

        // Vérifier via une commande Redis directe (TTL key) que l'entrée a bien une expiration
        // définie, pas une durée de vie infinie — cohérent avec le design (Redis = cache, jamais
        // source de vérité durable ; une entrée orpheline doit finir par disparaître).
        let ttl: i64 = redis::cmd("TTL")
            .arg("hot_state:oidc-sub-ttl-test")
            .query_async(&mut cache.conn.clone())
            .await
            .unwrap();
        assert!(ttl > 0);
    }
}
