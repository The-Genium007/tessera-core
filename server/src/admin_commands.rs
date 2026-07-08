//! Parsing et exécution des commandes de gestion des permissions (`/promote`, `/grant`...).
//! Pur : ne touche ni le réseau ni le disque — le caller (Gateway) persiste et journalise.

use crate::permissions::{AdminRecord, Group};

#[derive(Debug, PartialEq)]
pub enum ParsedCommand {
    Promote { account: String, group: String },
    Demote { account: String },
    Grant { account: String, node: String },
    Revoke { account: String, node: String },
    ListAdmins,
    ListGroups,
    GroupInfo { group: String },
    CreateGroup { name: String },
    GroupGrant { group: String, node: String },
    GroupRevoke { group: String, node: String },
    DeleteGroup { name: String },
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    UnknownCommand,
    MissingArgs,
}

/// Parse un texte tapé façon `/commande arg1 arg2`. Le `/` initial est optionnel.
pub fn parse(text: &str) -> Result<ParsedCommand, ParseError> {
    let text = text.strip_prefix('/').unwrap_or(text);
    let mut parts = text.split_whitespace();
    let cmd = parts.next().ok_or(ParseError::UnknownCommand)?;
    let rest: Vec<&str> = parts.collect();
    match cmd {
        "promote" => match rest.as_slice() {
            [account, group] => Ok(ParsedCommand::Promote {
                account: account.to_string(),
                group: group.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "demote" => match rest.as_slice() {
            [account] => Ok(ParsedCommand::Demote { account: account.to_string() }),
            _ => Err(ParseError::MissingArgs),
        },
        "grant" => match rest.as_slice() {
            [account, node] => Ok(ParsedCommand::Grant {
                account: account.to_string(),
                node: node.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "revoke" => match rest.as_slice() {
            [account, node] => Ok(ParsedCommand::Revoke {
                account: account.to_string(),
                node: node.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "admins" => Ok(ParsedCommand::ListAdmins),
        "groups" => Ok(ParsedCommand::ListGroups),
        "groupinfo" => match rest.as_slice() {
            [group] => Ok(ParsedCommand::GroupInfo { group: group.to_string() }),
            _ => Err(ParseError::MissingArgs),
        },
        "creategroup" => match rest.as_slice() {
            [name] => Ok(ParsedCommand::CreateGroup { name: name.to_string() }),
            _ => Err(ParseError::MissingArgs),
        },
        "groupgrant" => match rest.as_slice() {
            [group, node] => Ok(ParsedCommand::GroupGrant {
                group: group.to_string(),
                node: node.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "grouprevoke" => match rest.as_slice() {
            [group, node] => Ok(ParsedCommand::GroupRevoke {
                group: group.to_string(),
                node: node.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "deletegroup" => match rest.as_slice() {
            [name] => Ok(ParsedCommand::DeleteGroup { name: name.to_string() }),
            _ => Err(ParseError::MissingArgs),
        },
        _ => Err(ParseError::UnknownCommand),
    }
}

/// Résultat d'exécution : message à renvoyer à l'émetteur + le compte dont les permissions ont
/// changé (pour que le caller pousse un `PermissionSync` frais s'il est connecté), si applicable.
#[derive(Debug, PartialEq)]
pub struct ExecOutcome {
    pub success: bool,
    pub message: String,
    pub affected_account: Option<String>,
}

fn ok(message: impl Into<String>) -> ExecOutcome {
    ExecOutcome { success: true, message: message.into(), affected_account: None }
}
fn ok_for(account: &str, message: impl Into<String>) -> ExecOutcome {
    ExecOutcome { success: true, message: message.into(), affected_account: Some(account.to_string()) }
}
fn fail(message: impl Into<String>) -> ExecOutcome {
    ExecOutcome { success: false, message: message.into(), affected_account: None }
}

/// Exécute une commande déjà parsée. `is_root` doit avoir été résolu par le caller (le compte
/// émetteur est-il dans `TESSERA_ROOT_ADMINS` ?) — toutes ces commandes sont réservées aux
/// admins racine en phase 1, aucune délégation (spec admin-mode-permissions, Partie 3).
pub fn execute(
    cmd: ParsedCommand,
    is_root: bool,
    groups: &mut Vec<Group>,
    admins: &mut Vec<AdminRecord>,
    now_ms: u64,
    actor: &str,
) -> ExecOutcome {
    if !is_root {
        return fail("réservé aux admins racine");
    }
    match cmd {
        ParsedCommand::Promote { account, group } => {
            if !groups.iter().any(|g| g.name == group) {
                return fail(format!("groupe inconnu : {group}"));
            }
            if let Some(existing) = admins.iter_mut().find(|a| a.display_name == account) {
                existing.group = group;
                existing.granted_at = now_ms;
                existing.granted_by = actor.to_string();
            } else {
                admins.push(AdminRecord {
                    display_name: account.clone(),
                    group,
                    extra_permissions: Vec::new(),
                    revoked_permissions: Vec::new(),
                    granted_at: now_ms,
                    granted_by: actor.to_string(),
                });
            }
            ok_for(&account, format!("{account} promu"))
        }
        ParsedCommand::Demote { account } => {
            let before = admins.len();
            admins.retain(|a| a.display_name != account);
            if admins.len() == before {
                return fail(format!("{account} n'est pas admin"));
            }
            ok_for(&account, format!("{account} rétrogradé"))
        }
        ParsedCommand::Grant { account, node } => {
            let Some(a) = admins.iter_mut().find(|a| a.display_name == account) else {
                return fail(format!("{account} n'est pas admin"));
            };
            if !a.extra_permissions.contains(&node) {
                a.extra_permissions.push(node.clone());
            }
            a.revoked_permissions.retain(|n| n != &node);
            ok_for(&account, format!("{node} accordé à {account}"))
        }
        ParsedCommand::Revoke { account, node } => {
            let Some(a) = admins.iter_mut().find(|a| a.display_name == account) else {
                return fail(format!("{account} n'est pas admin"));
            };
            a.extra_permissions.retain(|n| n != &node);
            if !a.revoked_permissions.contains(&node) {
                a.revoked_permissions.push(node.clone());
            }
            ok_for(&account, format!("{node} retiré à {account}"))
        }
        ParsedCommand::ListAdmins => {
            let list = admins
                .iter()
                .map(|a| format!("{} ({})", a.display_name, a.group))
                .collect::<Vec<_>>()
                .join(", ");
            ok(if list.is_empty() { "aucun admin".to_string() } else { list })
        }
        ParsedCommand::ListGroups => {
            let list = groups.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ");
            ok(if list.is_empty() { "aucun groupe".to_string() } else { list })
        }
        ParsedCommand::GroupInfo { group } => {
            let Some(g) = groups.iter().find(|g| g.name == group) else {
                return fail(format!("groupe inconnu : {group}"));
            };
            let members: Vec<&str> = admins
                .iter()
                .filter(|a| a.group == group)
                .map(|a| a.display_name.as_str())
                .collect();
            ok(format!(
                "{}: [{}] — membres: [{}]",
                g.name,
                g.permissions.join(", "),
                members.join(", ")
            ))
        }
        ParsedCommand::CreateGroup { name } => {
            if groups.iter().any(|g| g.name == name) {
                return fail(format!("groupe déjà existant : {name}"));
            }
            groups.push(Group { name: name.clone(), permissions: Vec::new() });
            ok(format!("groupe {name} créé"))
        }
        ParsedCommand::GroupGrant { group, node } => {
            let Some(g) = groups.iter_mut().find(|g| g.name == group) else {
                return fail(format!("groupe inconnu : {group}"));
            };
            if !g.permissions.contains(&node) {
                g.permissions.push(node.clone());
            }
            ok(format!("{node} accordé au groupe {group}"))
        }
        ParsedCommand::GroupRevoke { group, node } => {
            let Some(g) = groups.iter_mut().find(|g| g.name == group) else {
                return fail(format!("groupe inconnu : {group}"));
            };
            g.permissions.retain(|n| n != &node);
            ok(format!("{node} retiré du groupe {group}"))
        }
        ParsedCommand::DeleteGroup { name } => {
            if admins.iter().any(|a| a.group == name) {
                return fail(format!("groupe {name} encore assigné à des comptes"));
            }
            let before = groups.len();
            groups.retain(|g| g.name != name);
            if groups.len() == before {
                return fail(format!("groupe inconnu : {name}"));
            }
            ok(format!("groupe {name} supprimé"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_promote_with_leading_slash() {
        assert_eq!(
            parse("/promote Compte1 moderator"),
            Ok(ParsedCommand::Promote { account: "Compte1".into(), group: "moderator".into() })
        );
    }

    #[test]
    fn parses_without_leading_slash_too() {
        assert_eq!(
            parse("admins"),
            Ok(ParsedCommand::ListAdmins)
        );
    }

    #[test]
    fn promote_missing_args_is_an_error() {
        assert_eq!(parse("/promote Compte1"), Err(ParseError::MissingArgs));
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert_eq!(parse("/nope"), Err(ParseError::UnknownCommand));
    }

    #[test]
    fn parses_all_zero_or_one_arg_commands() {
        assert_eq!(parse("/demote Compte1"), Ok(ParsedCommand::Demote { account: "Compte1".into() }));
        assert_eq!(parse("/groups"), Ok(ParsedCommand::ListGroups));
        assert_eq!(parse("/groupinfo moderator"), Ok(ParsedCommand::GroupInfo { group: "moderator".into() }));
        assert_eq!(parse("/creategroup vip"), Ok(ParsedCommand::CreateGroup { name: "vip".into() }));
        assert_eq!(parse("/deletegroup vip"), Ok(ParsedCommand::DeleteGroup { name: "vip".into() }));
    }

    #[test]
    fn parses_all_two_arg_commands() {
        assert_eq!(
            parse("/grant Compte1 admin.fly"),
            Ok(ParsedCommand::Grant { account: "Compte1".into(), node: "admin.fly".into() })
        );
        assert_eq!(
            parse("/revoke Compte1 admin.fly"),
            Ok(ParsedCommand::Revoke { account: "Compte1".into(), node: "admin.fly".into() })
        );
        assert_eq!(
            parse("/groupgrant moderator admin.fly"),
            Ok(ParsedCommand::GroupGrant { group: "moderator".into(), node: "admin.fly".into() })
        );
        assert_eq!(
            parse("/grouprevoke moderator admin.fly"),
            Ok(ParsedCommand::GroupRevoke { group: "moderator".into(), node: "admin.fly".into() })
        );
    }

    fn moderator_group() -> Group {
        Group { name: "moderator".into(), permissions: vec!["admin.noclip".into()] }
    }

    #[test]
    fn non_root_cannot_execute_any_management_command() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let outcome = execute(
            ParsedCommand::Promote { account: "Compte1".into(), group: "moderator".into() },
            false,
            &mut groups,
            &mut admins,
            0,
            "PasRoot",
        );
        assert!(!outcome.success);
        assert!(admins.is_empty());
    }

    #[test]
    fn promote_to_unknown_group_fails() {
        let mut groups = vec![];
        let mut admins = vec![];
        let outcome = execute(
            ParsedCommand::Promote { account: "Compte1".into(), group: "ghost".into() },
            true,
            &mut groups,
            &mut admins,
            0,
            "Root",
        );
        assert!(!outcome.success);
        assert!(admins.is_empty());
    }

    #[test]
    fn promote_to_known_group_succeeds_and_reports_affected_account() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let outcome = execute(
            ParsedCommand::Promote { account: "Compte1".into(), group: "moderator".into() },
            true,
            &mut groups,
            &mut admins,
            1000,
            "Root",
        );
        assert!(outcome.success);
        assert_eq!(outcome.affected_account, Some("Compte1".to_string()));
        assert_eq!(admins.len(), 1);
        assert_eq!(admins[0].group, "moderator");
        assert_eq!(admins[0].granted_by, "Root");
    }

    #[test]
    fn demote_unknown_account_fails() {
        let mut groups = vec![];
        let mut admins = vec![];
        let outcome = execute(
            ParsedCommand::Demote { account: "Compte1".into() },
            true,
            &mut groups,
            &mut admins,
            0,
            "Root",
        );
        assert!(!outcome.success);
    }

    #[test]
    fn grant_and_revoke_edit_the_account_overrides() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        execute(
            ParsedCommand::Promote { account: "Compte1".into(), group: "moderator".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        let outcome = execute(
            ParsedCommand::Grant { account: "Compte1".into(), node: "admin.fly".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        assert!(outcome.success);
        assert_eq!(admins[0].extra_permissions, vec!["admin.fly".to_string()]);

        let outcome = execute(
            ParsedCommand::Revoke { account: "Compte1".into(), node: "admin.fly".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        assert!(outcome.success);
        assert!(admins[0].extra_permissions.is_empty());
        assert_eq!(admins[0].revoked_permissions, vec!["admin.fly".to_string()]);
    }

    #[test]
    fn deletegroup_refused_while_members_remain() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        execute(
            ParsedCommand::Promote { account: "Compte1".into(), group: "moderator".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        let outcome = execute(
            ParsedCommand::DeleteGroup { name: "moderator".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        assert!(!outcome.success);
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn deletegroup_succeeds_once_no_members_remain() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let outcome = execute(
            ParsedCommand::DeleteGroup { name: "moderator".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        assert!(outcome.success);
        assert!(groups.is_empty());
    }

    #[test]
    fn creategroup_then_groupgrant_populates_its_permissions() {
        let mut groups = vec![];
        let mut admins = vec![];
        execute(ParsedCommand::CreateGroup { name: "vip".into() }, true, &mut groups, &mut admins, 0, "Root");
        assert_eq!(groups.len(), 1);
        execute(
            ParsedCommand::GroupGrant { group: "vip".into(), node: "admin.teleport".into() },
            true, &mut groups, &mut admins, 0, "Root",
        );
        assert_eq!(groups[0].permissions, vec!["admin.teleport".to_string()]);
    }
}
