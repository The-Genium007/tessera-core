//! Bascule `FileStore`/`PostgresStore` selon `identity.public` (câblage runtime, design stockage
//! 2026-07-09) — unifie les deux stores concrets derrière le trait synchrone `PlayerStore`
//! existant (`persistence.rs`), pour que `gateway_main` reste NON générique et que
//! `cleanup_client_state`/`save_all_known` (génériques sur `impl PlayerStore`) fonctionnent sans
//! modification.
//!
//! **Choix (a) du brief `store-hotcache-task-brief.md`** : `PostgresStore` n'expose que des
//! méthodes async natives (`save_async`/`load_async`, voir doc de module `postgres_store.rs` —
//! qui explique pourquoi ce module a délibérément évité `block_on` comme piège classique).
//! Plutôt que d'éclater `PlayerStore` en une variante async (option (b), qui aurait forcé un
//! changement de signature de `cleanup_client_state`/`save_all_known` et donc de leurs tests
//! existants basés sur `MemoryStore`), la branche `Postgres` de l'impl `PlayerStore` bride l'appel
//! async avec `tokio::task::block_in_place` + `Handle::current().block_on(...)` — PAS un
//! `block_on` nu (un bug de la première version de ce fichier, trouvé par la revue finale
//! whole-branch : un `block_on` nu appelé depuis une tâche async du runtime multi-thread — le
//! cas réel de `gateway_main` — panique avec "Cannot start a runtime from within a runtime",
//! faisant planter tout serveur `identity.public=true` au premier `Join`). `block_in_place`
//! déplace explicitement le thread hors du pool de workers avant de bloquer dessus, ce qui rend
//! le blocage sûr — voir `postgres_variant_load_from_a_genuine_async_task_does_not_panic`
//! (module de tests) pour la reproduction exacte du bug et la preuve du correctif. Ce blocage
//! reste accepté (pas éliminé) car ce n'est PAS le chemin chaud identifié comme sensible dans le
//! design (l'avertissement de `postgres_store.rs` porte sur un blocage RÉPÉTÉ à haute fréquence
//! dans l'executor) : un save/load Postgres n'a lieu qu'au Join, à la déconnexion/Leave, au
//! kick-flood, ou à l'autosave périodique (30s) — des événements peu fréquents comparés au tick
//! réseau à 20 Hz. Risque documenté : sous une charge de connexions/déconnexions simultanées
//! inhabituellement élevée, `block_in_place` retire un thread du pool de workers tokio pour toute
//! la durée du blocage — à surveiller si le nombre de Join/Leave par seconde devient significatif
//! (hors échelle self-host visée à ce jour).
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
                // `block_in_place` déplace CE thread hors du pool de workers tokio avant de
                // bloquer dessus — indispensable sur le runtime multi-thread : un `block_on` nu
                // appelé depuis une tâche async (le cas réel ici, gateway_main) panique avec
                // "Cannot start a runtime from within a runtime" (bug trouvé par la revue finale
                // whole-branch, reproduit par
                // postgres_variant_load_from_a_genuine_async_task_does_not_panic).
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(store.load_async(key))
                })
                .unwrap_or_else(|e| {
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
                let display_name = display_names.get(key).cloned().unwrap_or_else(|| {
                    // Ne devrait pas arriver après un Join réussi (note_display_name est
                    // toujours appelée avant tout save pour ce subject) — rendu observable
                    // plutôt que silencieux, cf. Minor de la revue finale whole-branch.
                    tracing::warn!(
                        "PlayerStoreImpl::save : aucun display_name noté pour subject={key}, \
                         repli sur le subject lui-même — invariant Join→note_display_name violé ?"
                    );
                    key.to_string()
                });
                let key = key.to_string();
                let result = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(store.save_async(
                        &key,
                        &display_name,
                        record,
                    ))
                });
                if let Err(e) = result {
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

        // Prouve que save()/load() fonctionnent aussi depuis un thread bloquant dédié
        // (spawn_blocking), un contexte d'appel légitime mais DIFFÉRENT de celui de
        // gateway_main — voir postgres_variant_load_from_a_genuine_async_task_does_not_panic
        // ci-dessous pour le call-site réel (tâche async directe, pas spawn_blocking).
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

    // Régression : gateway_main appelle store.load()/save() DIRECTEMENT depuis son propre
    // corps async (pas depuis un spawn_blocking), sur le runtime MULTI-THREAD par défaut de
    // `#[tokio::main]` (bin/gateway.rs:7). `#[sqlx::test]` ne convient PAS pour reproduire ce
    // contexte : `sqlx::test_block_on` construit en interne un runtime `new_current_thread()`
    // (sqlx-core/src/rt/mod.rs) — `block_in_place` y panique avec "can call blocking only when
    // running on the multi-threaded runtime", un message différent du bug original mais tout
    // aussi peu représentatif de la prod. Ce test construit donc son propre pool + runtime
    // multi-thread explicite, pour reproduire fidèlement l'environnement réel de gateway_main.
    // Avant le fix (Handle::current().block_on nu), ce test panique avec "Cannot start a
    // runtime from within a runtime" ; c'est le bug trouvé par la revue finale whole-branch
    // (tout serveur identity.public=true plantait au premier Join).
    #[test]
    fn postgres_variant_load_from_a_genuine_async_task_does_not_panic() {
        let database_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "DATABASE_URL absent — test sauté (nécessite un Postgres réel + migrations, \
                     voir README/CI pour le service Postgres de test)"
                );
                return;
            }
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime multi-thread");

        runtime.block_on(async {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .connect(&database_url)
                .await
                .expect("connexion Postgres de test");
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .expect("migrations");

            let mut store = PlayerStoreImpl::Postgres {
                store: PostgresStore::new(pool),
                display_names: HashMap::new(),
            };
            store.note_display_name("oidc-sub-impl-2", "Diane");
            let rec = PlayerRecord {
                last_position: [7.0, 8.0, 9.0],
                residence: None,
            };

            // Appel direct, comme le fait gateway_main (gateway.rs:466,526,962,1301) — cette
            // tâche `async {}` tourne sur le runtime multi-thread construit ci-dessus,
            // exactement le contexte qui fait paniquer un `block_on` nu.
            store.save("oidc-sub-impl-2", rec.clone());
            assert_eq!(store.load("oidc-sub-impl-2"), Some(rec));
        });
    }
}
