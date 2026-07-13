//! Bascule `FileStore`/`PostgresStore` selon `identity.public` (câblage runtime, design stockage
//! 2026-07-09) — unifie les deux stores concrets derrière le trait synchrone `PlayerStore`
//! existant (`persistence.rs`), pour que `gateway_main` reste NON générique et que
//! `cleanup_client_state`/`save_all_known` (génériques sur `impl PlayerStore`) fonctionnent sans
//! modification.
//!
//! **Choix (a) du brief `store-hotcache-task-brief.md`** : `PostgresStore` n'expose que des
//! méthodes async natives (`save_async`/`load_async`, voir doc de module `postgres_store.rs`).
//! Plutôt que d'éclater `PlayerStore` en une variante async (option (b), qui aurait forcé un
//! changement de signature de `cleanup_client_state`/`save_all_known` et donc de leurs tests
//! existants basés sur `MemoryStore`), la branche `Postgres` de l'impl `PlayerStore` bride l'appel
//! async avec `tokio::runtime::Handle::current().block_on(...)`. Ce blocage est accepté ici parce
//! que ce n'est PAS le chemin chaud identifié comme sensible dans le design (l'avertissement de
//! `postgres_store.rs` porte sur un blocage RÉPÉTÉ à haute fréquence dans l'executor) : un
//! save/load Postgres n'a lieu qu'au Join, à la déconnexion/Leave, au kick-flood, ou à l'autosave
//! périodique (30s) — des événements peu fréquents comparés au tick réseau à 20 Hz. Risque
//! documenté : sous une charge de connexions/déconnexions simultanées inhabituellement élevée, ce
//! blocage pourrait retarder d'autres tâches du même runtime tokio ; à surveiller si le nombre de
//! Join/Leave par seconde devient significatif (hors échelle self-host visée à ce jour).
//!
//! **`display_name` pour `PostgresStore::save_async`** : le trait `PlayerStore::save(key, record)`
//! ne porte qu'une seule clé (le `subject` OIDC vérifié pour la branche Postgres), alors que
//! `PostgresStore::save_async` a aussi besoin du `display_name` brut (contrainte unique en base,
//! parade au bug playtest 1). `PlayerStoreImpl` mémorise ce mapping subject→display_name via
//! `note_display_name`, appelé par `gateway_main` au Join — seul endroit où les deux sont connus
//! simultanément. Un `save` Postgres pour un `subject` jamais noté (ne devrait pas arriver après
//! un Join réussi) retombe sur le `subject` lui-même comme `display_name` plutôt que de paniquer
//! ou d'échouer silencieusement l'écriture.

use crate::persistence::{FileStore, PlayerRecord, PlayerStore};
use crate::postgres_store::PostgresStore;
use std::collections::HashMap;

/// Unifie `FileStore` (serveur privé) et `PostgresStore` (serveur public, `identity.public =
/// true`) derrière le trait `PlayerStore` — voir doc de module pour le choix (a) vs (b).
pub enum PlayerStoreImpl {
    File(FileStore),
    Postgres {
        store: PostgresStore,
        /// subject OIDC → display_name brut, alimenté par `note_display_name` au Join — voir
        /// doc de module.
        display_names: HashMap<String, String>,
    },
}

impl PlayerStoreImpl {
    /// Mémorise le `display_name` associé à un `subject` — à appeler au Join, avant toute
    /// éventuelle sauvegarde ultérieure pour ce `subject`. Sans effet sur la branche `File`
    /// (n'a jamais besoin de cette information : `key` porte déjà le display_name brut).
    pub fn note_display_name(&mut self, subject: &str, display_name: &str) {
        if let PlayerStoreImpl::Postgres { display_names, .. } = self {
            display_names.insert(subject.to_string(), display_name.to_string());
        }
    }
}

impl PlayerStore for PlayerStoreImpl {
    fn load(&self, key: &str) -> Option<PlayerRecord> {
        match self {
            PlayerStoreImpl::File(store) => store.load(key),
            PlayerStoreImpl::Postgres { store, .. } => {
                tokio::runtime::Handle::current().block_on(store.load_async(key)).unwrap_or_else(|e| {
                    tracing::warn!("PostgresStore::load_async échoué (subject={key}): {e} — traité comme absent");
                    None
                })
            }
        }
    }

    fn save(&mut self, key: &str, record: PlayerRecord) {
        match self {
            PlayerStoreImpl::File(store) => store.save(key, record),
            PlayerStoreImpl::Postgres {
                store,
                display_names,
            } => {
                let display_name = display_names
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| key.to_string());
                let key = key.to_string();
                if let Err(e) = tokio::runtime::Handle::current()
                    .block_on(store.save_async(&key, &display_name, record))
                {
                    tracing::warn!("PostgresStore::save_async échoué (subject={key}): {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::PlayerRecord;

    #[test]
    fn file_variant_saves_and_loads_like_plain_filestore() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("players.json");
        let mut store = PlayerStoreImpl::File(FileStore::open(&path));
        assert_eq!(store.load("Alice"), None);
        let rec = PlayerRecord {
            last_position: [1.0, 2.0, 3.0],
            residence: None,
        };
        store.save("Alice", rec.clone());
        assert_eq!(store.load("Alice"), Some(rec));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn postgres_variant_saves_and_loads_using_noted_display_name(pool: sqlx::PgPool) {
        let mut store = PlayerStoreImpl::Postgres {
            store: PostgresStore::new(pool),
            display_names: HashMap::new(),
        };
        store.note_display_name("oidc-sub-impl-1", "Vincent");
        let rec = PlayerRecord {
            last_position: [4.0, 5.0, 6.0],
            residence: Some([1.0, 1.0, 1.0]),
        };

        // save()/load() du trait PlayerStore (synchrone) doivent fonctionner même appelés
        // depuis un contexte async (comme le ferait un test #[sqlx::test]) : la branche
        // Postgres utilise Handle::current().block_on, qui exige justement d'être appelé
        // depuis un thread du runtime, pas d'en être lui-même la seule tâche active — on
        // vérifie ici que ça ne deadlock pas et renvoie bien la valeur.
        let handle = tokio::runtime::Handle::current();
        let cloned_key = "oidc-sub-impl-1".to_string();
        let cloned_rec = rec.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = handle.enter();
            store.save(&cloned_key, cloned_rec);
            assert_eq!(store.load(&cloned_key), Some(rec));
        })
        .await
        .unwrap();
    }
}
