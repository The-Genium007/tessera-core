//! Résolution de permissions à nœuds hiérarchiques (façon LuckPerms Minecraft) : un nœud comme
//! `admin.fly`, un wildcard comme `admin.*` couvre tout son sous-arbre, `*` couvre tout. Pur —
//! aucune IO ; le chargement/la sauvegarde vivent dans `admin_store.rs`.

// Convention `character.slots.<N>` (2026-07-20, sous-projet flux d'arrivée) : PREMIER nœud à
// valeur du projet. Tous les autres nœuds (admin.*, server.queue_bypass) sont présence/absence
// pure — un ensemble de chaînes résolu, jamais de valeur associée. Plutôt que d'étendre
// Group/AdminRecord avec un nouveau champ typé (changement de modèle de données plus intrusif),
// la valeur est encodée dans le SUFFIXE du nœud lui-même, lu par max_character_slots(). Un
// groupe "staff" attribuerait par exemple le nœud "character.slots.5" comme n'importe quel autre
// nœud de permission (via /groupgrant staff character.slots.5).

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

/// Résout le cap de personnages depuis l'ensemble de nœuds déjà résolu (`resolve_permissions`).
/// Convention `character.slots.<N>` : le SUFFIXE numérique porte la valeur (premier nœud à
/// valeur du projet — tous les autres nœuds sont présence/absence pure). Renvoie le MAXIMUM
/// trouvé parmi tous les nœuds `character.slots.*` correspondants (un compte peut cumuler
/// plusieurs sources), ou 1 par défaut (comportement actuel implicite : un joueur normal a un
/// seul personnage).
pub fn max_character_slots(resolved: &[String]) -> u32 {
    resolved
        .iter()
        .filter_map(|node| node.strip_prefix("character.slots."))
        .filter_map(|suffix| suffix.parse::<u32>().ok())
        .max()
        .unwrap_or(1)
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

    #[test]
    fn defaults_to_one_slot_when_no_node_is_present() {
        assert_eq!(max_character_slots(&[]), 1);
    }

    #[test]
    fn reads_the_slot_count_from_a_matching_node() {
        assert_eq!(
            max_character_slots(&["character.slots.5".to_string()]),
            5
        );
    }

    #[test]
    fn ignores_unrelated_nodes() {
        assert_eq!(
            max_character_slots(&["admin.fly".to_string(), "character.slots.3".to_string()]),
            3
        );
    }

    #[test]
    fn takes_the_maximum_across_multiple_matching_nodes() {
        assert_eq!(
            max_character_slots(&["character.slots.3".to_string(), "character.slots.5".to_string()]),
            5,
            "un compte cumulant plusieurs sources garde le plafond le plus généreux"
        );
    }

    #[test]
    fn ignores_a_malformed_suffix_that_does_not_parse_as_a_number() {
        assert_eq!(
            max_character_slots(&["character.slots.beaucoup".to_string()]),
            1,
            "un suffixe non numérique est ignoré, pas une erreur silencieuse dangereuse"
        );
    }
}
