//! Stockage JSON des groupes de permissions et des comptes admin — même discipline que
//! `persistence::FileStore` (réécriture atomique, tolérant aux fichiers absents/corrompus).
//! Schéma volontairement "en forme de table" (enregistrements plats) pour qu'une migration
//! future vers une vraie base de données soit mécanique (spec admin-mode-permissions, Partie 2).

use crate::permissions::{AdminRecord, Group};
use std::path::PathBuf;

fn load_json<T: serde::de::DeserializeOwned + Default>(path: &PathBuf) -> T {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!("store admin illisible ({path:?}: {e}) — démarrage à vide");
            T::default()
        }),
        Err(_) => T::default(),
    }
}

fn save_json<T: serde::Serialize>(path: &PathBuf, value: &T) {
    let Ok(json) = serde_json::to_string_pretty(value) else {
        return;
    };
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

pub struct AdminStore {
    groups_path: PathBuf,
    admins_path: PathBuf,
    pub groups: Vec<Group>,
    pub admins: Vec<AdminRecord>,
}

impl AdminStore {
    /// Ouvre les deux stores. Fichier absent → vide ; fichier illisible/corrompu → vide + warn
    /// (même discipline que `persistence::FileStore::open`).
    pub fn open(groups_path: impl Into<PathBuf>, admins_path: impl Into<PathBuf>) -> Self {
        let groups_path = groups_path.into();
        let admins_path = admins_path.into();
        let groups = load_json(&groups_path);
        let admins = load_json(&admins_path);
        Self {
            groups_path,
            admins_path,
            groups,
            admins,
        }
    }

    /// Réécrit `permission_groups.json` atomiquement (write temp + rename).
    pub fn save_groups(&self) {
        save_json(&self.groups_path, &self.groups);
    }

    /// Réécrit `server_admins.json` atomiquement (write temp + rename).
    pub fn save_admins(&self) {
        save_json(&self.admins_path, &self.admins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn opens_empty_when_files_absent() {
        let dir = tempdir().unwrap();
        let store = AdminStore::open(
            dir.path().join("groups.json"),
            dir.path().join("admins.json"),
        );
        assert!(store.groups.is_empty());
        assert!(store.admins.is_empty());
    }

    #[test]
    fn opens_empty_on_corrupt_file_without_panicking() {
        let dir = tempdir().unwrap();
        let groups_path = dir.path().join("groups.json");
        std::fs::write(&groups_path, b"{ not valid json").unwrap();
        let store = AdminStore::open(groups_path, dir.path().join("admins.json"));
        assert!(store.groups.is_empty());
    }

    #[test]
    fn groups_persist_across_reopen() {
        let dir = tempdir().unwrap();
        let groups_path = dir.path().join("groups.json");
        let admins_path = dir.path().join("admins.json");
        {
            let mut store = AdminStore::open(&groups_path, &admins_path);
            store.groups.push(Group {
                name: "moderator".into(),
                permissions: vec!["admin.noclip".into()],
            });
            store.save_groups();
        }
        let reopened = AdminStore::open(&groups_path, &admins_path);
        assert_eq!(
            reopened.groups,
            vec![Group {
                name: "moderator".into(),
                permissions: vec!["admin.noclip".into()]
            }]
        );
    }

    #[test]
    fn admins_persist_across_reopen() {
        let dir = tempdir().unwrap();
        let groups_path = dir.path().join("groups.json");
        let admins_path = dir.path().join("admins.json");
        let record = AdminRecord {
            display_name: "Compte1".into(),
            sub: None,
            group: "moderator".into(),
            extra_permissions: vec![],
            revoked_permissions: vec![],
            granted_at: 1_700_000_000_000,
            granted_by: "Root".into(),
        };
        {
            let mut store = AdminStore::open(&groups_path, &admins_path);
            store.admins.push(record.clone());
            store.save_admins();
        }
        let reopened = AdminStore::open(&groups_path, &admins_path);
        assert_eq!(reopened.admins, vec![record]);
    }

    #[test]
    fn admins_json_without_sub_field_still_loads_with_sub_none() {
        // Rétrocompatibilité (Task D3) : un `server_admins.json` écrit avant l'ajout du champ
        // `sub` à `AdminRecord` ne doit jamais empêcher le serveur de démarrer.
        let dir = tempdir().unwrap();
        let groups_path = dir.path().join("groups.json");
        let admins_path = dir.path().join("admins.json");
        std::fs::write(
            &admins_path,
            r#"[{"display_name":"Compte1","group":"moderator","extra_permissions":[],"revoked_permissions":[],"granted_at":1700000000000,"granted_by":"Root"}]"#,
        )
        .unwrap();
        let store = AdminStore::open(&groups_path, &admins_path);
        assert_eq!(store.admins.len(), 1);
        assert_eq!(store.admins[0].display_name, "Compte1");
        assert_eq!(store.admins[0].sub, None);
    }
}
