//! Résolution de permissions à nœuds hiérarchiques (façon LuckPerms Minecraft) : un nœud comme
//! `admin.fly`, un wildcard comme `admin.*` couvre tout son sous-arbre, `*` couvre tout. Pur —
//! aucune IO ; le chargement/la sauvegarde vivent dans `admin_store.rs`.

use crate::handoff::Rank;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminRecord {
    pub display_name: String,
    /// `sub` OIDC vérifié du compte, quand connu (Task D3 — migration de l'indexation admin vers
    /// `sub` pour fermer définitivement le bug playtest 1 : deux comptes distincts avec le même
    /// `display_name` ne doivent jamais partager d'autorité admin). `None` pour les admins
    /// attribués sur un serveur privé (`identity.public = false`), où seul le `display_name` a un
    /// sens — et pour tout enregistrement créé avant cette migration. `#[serde(default)]` : un
    /// `server_admins.json` existant sans ce champ continue de charger (désérialisation
    /// tolérante, même discipline que `AdminStore::open`).
    #[serde(default)]
    pub sub: Option<String>,
    pub group: String,
    pub extra_permissions: Vec<String>,
    pub revoked_permissions: Vec<String>,
    pub granted_at: u64,
    pub granted_by: String,
}

/// Vrai si `node` est couvert par `granted` — `granted` peut être `*` (couvre tout) ou un
/// wildcard suffixé `.* ` sur un préfixe complet (`admin.*` couvre `admin.fly`, `admin.x.y`,
/// et `admin` lui-même). Pas de wildcard au milieu d'un nœud.
pub fn node_matches(granted: &str, node: &str) -> bool {
    if granted == "*" || granted == node {
        return true;
    }
    if let Some(prefix) = granted.strip_suffix(".*") {
        return node == prefix || node.starts_with(&format!("{prefix}."));
    }
    false
}

/// Vrai si `node` est couvert par au moins un nœud de `granted_set`.
pub fn has_permission(granted_set: &[String], node: &str) -> bool {
    granted_set.iter().any(|g| node_matches(g, node))
}

/// Résout l'ensemble effectif de nœuds pour un compte. Racine → `["*"]` toujours, peu importe
/// le contenu du store. Sinon : nœuds du groupe (vide si le groupe a été supprimé entre-temps —
/// ne panique jamais) ∪ `extra_permissions`, moins tout nœud couvert par `revoked_permissions`.
pub fn resolve_permissions(
    is_root: bool,
    record: Option<&AdminRecord>,
    groups: &[Group],
) -> Vec<String> {
    if is_root {
        return vec!["*".to_string()];
    }
    let Some(record) = record else {
        return Vec::new();
    };
    let group_nodes: Vec<String> = groups
        .iter()
        .find(|g| g.name == record.group)
        .map(|g| g.permissions.clone())
        .unwrap_or_default();
    group_nodes
        .into_iter()
        .chain(record.extra_permissions.iter().cloned())
        .filter(|n| !has_permission(&record.revoked_permissions, n))
        .collect()
}

/// Dérive le `Rank` existant (zone tampon + bypass anti-triche, `handoff.rs`) depuis l'ensemble
/// de permissions résolu — comportement inchangé pour les admins actuels (wildcard complet →
/// GameMaster), `Moderator` devient enfin assignable (nœud `admin.*` partiel).
pub fn derive_rank(resolved: &[String]) -> Rank {
    let full_admin = resolved.iter().any(|n| n == "*" || n == "admin.*");
    if full_admin {
        Rank::GameMaster
    } else if resolved
        .iter()
        .any(|n| n.starts_with("admin.") || n == "admin")
    {
        Rank::Moderator
    } else {
        Rank::Player
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_node_matches_itself_only() {
        assert!(node_matches("admin.fly", "admin.fly"));
        assert!(!node_matches("admin.fly", "admin.noclip"));
    }

    #[test]
    fn wildcard_covers_its_subtree_and_itself() {
        assert!(node_matches("admin.*", "admin.fly"));
        assert!(node_matches("admin.*", "admin.world_edit"));
        assert!(node_matches("admin.*", "admin"));
        assert!(!node_matches("admin.*", "server.queue_bypass"));
    }

    #[test]
    fn bare_star_covers_everything() {
        assert!(node_matches("*", "admin.fly"));
        assert!(node_matches("*", "anything.at.all"));
    }

    #[test]
    fn root_always_resolves_to_wildcard_regardless_of_store() {
        let groups = vec![];
        assert_eq!(
            resolve_permissions(true, None, &groups),
            vec!["*".to_string()]
        );
    }

    #[test]
    fn account_with_no_record_resolves_empty() {
        let groups = vec![Group {
            name: "admin".into(),
            permissions: vec!["admin.*".into()],
        }];
        assert_eq!(
            resolve_permissions(false, None, &groups),
            Vec::<String>::new()
        );
    }

    #[test]
    fn account_resolves_to_union_of_group_and_extra_minus_revoked() {
        let groups = vec![Group {
            name: "moderator".into(),
            permissions: vec!["admin.noclip".into(), "admin.invisible".into()],
        }];
        let record = AdminRecord {
            display_name: "Compte1".into(),
            sub: None,
            group: "moderator".into(),
            extra_permissions: vec!["admin.fly".into()],
            revoked_permissions: vec!["admin.invisible".into()],
            granted_at: 0,
            granted_by: "Root".into(),
        };
        let mut resolved = resolve_permissions(false, Some(&record), &groups);
        resolved.sort();
        assert_eq!(
            resolved,
            vec!["admin.fly".to_string(), "admin.noclip".to_string()]
        );
    }

    #[test]
    fn deleted_group_resolves_empty_group_part_without_panicking() {
        let groups: Vec<Group> = vec![]; // le groupe "moderator" n'existe plus
        let record = AdminRecord {
            display_name: "Compte1".into(),
            sub: None,
            group: "moderator".into(),
            extra_permissions: vec!["admin.fly".into()],
            revoked_permissions: vec![],
            granted_at: 0,
            granted_by: "Root".into(),
        };
        assert_eq!(
            resolve_permissions(false, Some(&record), &groups),
            vec!["admin.fly".to_string()]
        );
    }

    #[test]
    fn full_wildcard_derives_gamemaster_rank() {
        assert_eq!(derive_rank(&["*".to_string()]), Rank::GameMaster);
        assert_eq!(derive_rank(&["admin.*".to_string()]), Rank::GameMaster);
    }

    #[test]
    fn partial_admin_node_derives_moderator_rank() {
        assert_eq!(derive_rank(&["admin.noclip".to_string()]), Rank::Moderator);
    }

    #[test]
    fn no_admin_node_derives_player_rank() {
        assert_eq!(derive_rank(&[]), Rank::Player);
        assert_eq!(
            derive_rank(&["server.queue_bypass".to_string()]),
            Rank::Player
        );
    }
}
