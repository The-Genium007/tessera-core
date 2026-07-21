//! Parsing et exécution des commandes de gestion des permissions (`/promote`, `/grant`...).
//! Pur : ne touche ni le réseau ni le disque — le caller (Gateway) persiste et journalise.

use crate::ban_store::{BanRecord, BanScope};
use crate::permissions::{AdminRecord, Group};

#[derive(Debug, PartialEq)]
pub enum ParsedCommand {
    Promote {
        account: String,
        group: String,
    },
    Demote {
        account: String,
    },
    Grant {
        account: String,
        node: String,
    },
    Revoke {
        account: String,
        node: String,
    },
    ListAdmins,
    ListGroups,
    GroupInfo {
        group: String,
    },
    CreateGroup {
        name: String,
    },
    GroupGrant {
        group: String,
        node: String,
    },
    GroupRevoke {
        group: String,
        node: String,
    },
    DeleteGroup {
        name: String,
    },
    Ban {
        target: String,
        vector: String,
        duration_secs: Option<u64>,
        reason: String,
    },
    Unban {
        target: String,
    },
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
            [account] => Ok(ParsedCommand::Demote {
                account: account.to_string(),
            }),
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
            [group] => Ok(ParsedCommand::GroupInfo {
                group: group.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "creategroup" => match rest.as_slice() {
            [name] => Ok(ParsedCommand::CreateGroup {
                name: name.to_string(),
            }),
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
            [name] => Ok(ParsedCommand::DeleteGroup {
                name: name.to_string(),
            }),
            _ => Err(ParseError::MissingArgs),
        },
        "ban" => {
            if rest.len() < 3 {
                return Err(ParseError::MissingArgs);
            }
            let target = rest[0].to_string();
            let vector_field = rest[1];
            let (vector, duration_secs) = if let Some(secs) = vector_field.strip_prefix("temp:") {
                let secs: u64 = secs.parse().map_err(|_| ParseError::MissingArgs)?;
                ("temp".to_string(), Some(secs))
            } else if vector_field == "perm" {
                ("perm".to_string(), None)
            } else {
                return Err(ParseError::MissingArgs);
            };
            let reason = rest[2..].join(" ");
            Ok(ParsedCommand::Ban {
                target,
                vector,
                duration_secs,
                reason,
            })
        }
        "unban" => match rest.as_slice() {
            [target] => Ok(ParsedCommand::Unban {
                target: target.to_string(),
            }),
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
    ExecOutcome {
        success: true,
        message: message.into(),
        affected_account: None,
    }
}
fn ok_for(account: &str, message: impl Into<String>) -> ExecOutcome {
    ExecOutcome {
        success: true,
        message: message.into(),
        affected_account: Some(account.to_string()),
    }
}
fn fail(message: impl Into<String>) -> ExecOutcome {
    ExecOutcome {
        success: false,
        message: message.into(),
        affected_account: None,
    }
}

/// Exécute une commande déjà parsée. `is_root` doit avoir été résolu par le caller (le compte
/// émetteur est-il dans `TESSERA_ROOT_ADMINS` ?) — toutes ces commandes sont réservées aux
/// admins racine en phase 1, aucune délégation (spec admin-mode-permissions, Partie 3).
pub fn execute(
    cmd: ParsedCommand,
    is_root: bool,
    groups: &mut Vec<Group>,
    admins: &mut Vec<AdminRecord>,
    bans: &mut Vec<BanRecord>,
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
                    // `/promote` est une commande tapée par un humain (l'admin émetteur saisit un
                    // display_name lisible au clavier, jamais un `sub` OIDC opaque) — le `sub` de
                    // ce compte, s'il en obtient un, est découvert et rattaché à son prochain Join
                    // sur un serveur public (voir `gateway.rs`, résolution admin au Join, Task D3),
                    // pas ici. Rien à migrer dans ce fichier : voir note de scope Task D3 dans le
                    // rapport de tâche.
                    sub: None,
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
            ok(if list.is_empty() {
                "aucun admin".to_string()
            } else {
                list
            })
        }
        ParsedCommand::ListGroups => {
            let list = groups
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            ok(if list.is_empty() {
                "aucun groupe".to_string()
            } else {
                list
            })
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
            groups.push(Group {
                name: name.clone(),
                permissions: Vec::new(),
            });
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
        ParsedCommand::Ban {
            target,
            vector,
            duration_secs,
            reason,
        } => {
            let (subject, ip, hwid_hash) = if let Some(v) = target.strip_prefix("account:") {
                (Some(v.to_string()), None, None)
            } else if let Some(v) = target.strip_prefix("ip:") {
                (None, Some(v.to_string()), None)
            } else if let Some(v) = target.strip_prefix("hwid:") {
                (None, None, Some(v.to_string()))
            } else {
                return fail(format!(
                    "cible invalide : {target} (attendu account:/ip:/hwid:)"
                ));
            };
            let scope = if vector == "perm" {
                BanScope::Perm
            } else {
                BanScope::Temp
            };
            let expires_at = duration_secs.map(|secs| (now_ms / 1000) as i64 + secs as i64);
            bans.push(BanRecord {
                subject,
                ip,
                hwid_hash,
                scope,
                reason: reason.clone(),
                expires_at,
                banned_by: actor.to_string(),
            });
            ok(format!("{target} banni ({vector}) : {reason}"))
        }
        ParsedCommand::Unban { target } => {
            let before = bans.len();
            if let Some(v) = target.strip_prefix("account:") {
                bans.retain(|b| b.subject.as_deref() != Some(v));
            } else if let Some(v) = target.strip_prefix("ip:") {
                bans.retain(|b| b.ip.as_deref() != Some(v));
            } else if let Some(v) = target.strip_prefix("hwid:") {
                bans.retain(|b| b.hwid_hash.as_deref() != Some(v));
            } else {
                return fail(format!("cible invalide : {target}"));
            }
            if bans.len() == before {
                return fail(format!("{target} n'est pas banni"));
            }
            ok(format!("{target} débanni"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ban_store::{BanRecord, BanScope};

    #[test]
    fn parses_promote_with_leading_slash() {
        assert_eq!(
            parse("/promote Compte1 moderator"),
            Ok(ParsedCommand::Promote {
                account: "Compte1".into(),
                group: "moderator".into()
            })
        );
    }

    #[test]
    fn parses_without_leading_slash_too() {
        assert_eq!(parse("admins"), Ok(ParsedCommand::ListAdmins));
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
        assert_eq!(
            parse("/demote Compte1"),
            Ok(ParsedCommand::Demote {
                account: "Compte1".into()
            })
        );
        assert_eq!(parse("/groups"), Ok(ParsedCommand::ListGroups));
        assert_eq!(
            parse("/groupinfo moderator"),
            Ok(ParsedCommand::GroupInfo {
                group: "moderator".into()
            })
        );
        assert_eq!(
            parse("/creategroup vip"),
            Ok(ParsedCommand::CreateGroup { name: "vip".into() })
        );
        assert_eq!(
            parse("/deletegroup vip"),
            Ok(ParsedCommand::DeleteGroup { name: "vip".into() })
        );
    }

    #[test]
    fn parses_all_two_arg_commands() {
        assert_eq!(
            parse("/grant Compte1 admin.fly"),
            Ok(ParsedCommand::Grant {
                account: "Compte1".into(),
                node: "admin.fly".into()
            })
        );
        assert_eq!(
            parse("/revoke Compte1 admin.fly"),
            Ok(ParsedCommand::Revoke {
                account: "Compte1".into(),
                node: "admin.fly".into()
            })
        );
        assert_eq!(
            parse("/groupgrant moderator admin.fly"),
            Ok(ParsedCommand::GroupGrant {
                group: "moderator".into(),
                node: "admin.fly".into()
            })
        );
        assert_eq!(
            parse("/grouprevoke moderator admin.fly"),
            Ok(ParsedCommand::GroupRevoke {
                group: "moderator".into(),
                node: "admin.fly".into()
            })
        );
    }

    fn moderator_group() -> Group {
        Group {
            name: "moderator".into(),
            permissions: vec!["admin.noclip".into()],
        }
    }

    #[test]
    fn non_root_cannot_execute_any_management_command() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let mut bans = vec![];
        let outcome = execute(
            ParsedCommand::Promote {
                account: "Compte1".into(),
                group: "moderator".into(),
            },
            false,
            &mut groups,
            &mut admins,
            &mut bans,
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
        let mut bans = vec![];
        let outcome = execute(
            ParsedCommand::Promote {
                account: "Compte1".into(),
                group: "ghost".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
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
        let mut bans = vec![];
        let outcome = execute(
            ParsedCommand::Promote {
                account: "Compte1".into(),
                group: "moderator".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
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
        let mut bans = vec![];
        let outcome = execute(
            ParsedCommand::Demote {
                account: "Compte1".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert!(!outcome.success);
    }

    #[test]
    fn grant_and_revoke_edit_the_account_overrides() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let mut bans = vec![];
        execute(
            ParsedCommand::Promote {
                account: "Compte1".into(),
                group: "moderator".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        let outcome = execute(
            ParsedCommand::Grant {
                account: "Compte1".into(),
                node: "admin.fly".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert!(outcome.success);
        assert_eq!(admins[0].extra_permissions, vec!["admin.fly".to_string()]);

        let outcome = execute(
            ParsedCommand::Revoke {
                account: "Compte1".into(),
                node: "admin.fly".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert!(outcome.success);
        assert!(admins[0].extra_permissions.is_empty());
        assert_eq!(admins[0].revoked_permissions, vec!["admin.fly".to_string()]);
    }

    #[test]
    fn deletegroup_refused_while_members_remain() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let mut bans = vec![];
        execute(
            ParsedCommand::Promote {
                account: "Compte1".into(),
                group: "moderator".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        let outcome = execute(
            ParsedCommand::DeleteGroup {
                name: "moderator".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert!(!outcome.success);
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn deletegroup_succeeds_once_no_members_remain() {
        let mut groups = vec![moderator_group()];
        let mut admins = vec![];
        let mut bans = vec![];
        let outcome = execute(
            ParsedCommand::DeleteGroup {
                name: "moderator".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert!(outcome.success);
        assert!(groups.is_empty());
    }

    #[test]
    fn creategroup_then_groupgrant_populates_its_permissions() {
        let mut groups = vec![];
        let mut admins = vec![];
        let mut bans = vec![];
        execute(
            ParsedCommand::CreateGroup { name: "vip".into() },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert_eq!(groups.len(), 1);
        execute(
            ParsedCommand::GroupGrant {
                group: "vip".into(),
                node: "admin.teleport".into(),
            },
            true,
            &mut groups,
            &mut admins,
            &mut bans,
            0,
            "Root",
        );
        assert_eq!(groups[0].permissions, vec!["admin.teleport".to_string()]);
    }

    #[test]
    fn parse_ban_account_temp_with_duration_and_reason() {
        let cmd = parse("/ban account:Compte1 temp:3600 flood").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::Ban {
                target: "account:Compte1".to_string(),
                vector: "temp".to_string(),
                duration_secs: Some(3600),
                reason: "flood".to_string(),
            }
        );
    }

    #[test]
    fn parse_ban_hwid_perm_has_no_duration() {
        let cmd = parse("/ban hwid:abc123 perm cheating").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::Ban {
                target: "hwid:abc123".to_string(),
                vector: "perm".to_string(),
                duration_secs: None,
                reason: "cheating".to_string(),
            }
        );
    }

    #[test]
    fn parse_ban_missing_args_is_an_error() {
        assert_eq!(parse("/ban account:Compte1"), Err(ParseError::MissingArgs));
    }

    #[test]
    fn parse_unban_takes_a_single_target() {
        let cmd = parse("/unban account:Compte1").unwrap();
        assert_eq!(
            cmd,
            ParsedCommand::Unban {
                target: "account:Compte1".to_string(),
            }
        );
    }

    #[test]
    fn execute_ban_appends_a_ban_record_for_account_vector() {
        let mut groups = Vec::new();
        let mut admins = Vec::new();
        let mut bans = Vec::new();
        let cmd = ParsedCommand::Ban {
            target: "account:Compte1".to_string(),
            vector: "perm".to_string(),
            duration_secs: None,
            reason: "cheating".to_string(),
        };
        let outcome = execute(cmd, true, &mut groups, &mut admins, &mut bans, 1000, "root");
        assert!(outcome.success);
        assert_eq!(bans.len(), 1);
        assert_eq!(bans[0].reason, "cheating");
        assert_eq!(bans[0].scope, BanScope::Perm);
    }

    #[test]
    fn execute_ban_requires_root() {
        let mut groups = Vec::new();
        let mut admins = Vec::new();
        let mut bans = Vec::new();
        let cmd = ParsedCommand::Ban {
            target: "account:Compte1".to_string(),
            vector: "perm".to_string(),
            duration_secs: None,
            reason: "x".to_string(),
        };
        let outcome = execute(
            cmd,
            false,
            &mut groups,
            &mut admins,
            &mut bans,
            1000,
            "actor",
        );
        assert!(!outcome.success);
        assert!(bans.is_empty());
    }

    #[test]
    fn execute_unban_removes_matching_ban_records_by_target() {
        let mut groups = Vec::new();
        let mut admins = Vec::new();
        let mut bans = vec![BanRecord {
            subject: Some("Compte1".to_string()),
            ip: None,
            hwid_hash: None,
            scope: BanScope::Perm,
            reason: "x".to_string(),
            expires_at: None,
            banned_by: "root".to_string(),
        }];
        let cmd = ParsedCommand::Unban {
            target: "account:Compte1".to_string(),
        };
        let outcome = execute(cmd, true, &mut groups, &mut admins, &mut bans, 1000, "root");
        assert!(outcome.success);
        assert!(bans.is_empty());
    }

    #[test]
    fn execute_unban_fails_when_no_matching_ban() {
        let mut groups = Vec::new();
        let mut admins = Vec::new();
        let mut bans = Vec::new();
        let cmd = ParsedCommand::Unban {
            target: "account:Inconnu".to_string(),
        };
        let outcome = execute(cmd, true, &mut groups, &mut admins, &mut bans, 1000, "root");
        assert!(!outcome.success);
    }
}
