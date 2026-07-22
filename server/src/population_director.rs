//! Director de population (spec fondation PNJ §3, modèle serveur §3) : maintient une densité
//! cible de PNJ par district, configurée par l'opérateur (jamais calquée sur la densité native du
//! jeu). Spawn près des joueurs présents, hiberne/despawn ailleurs. Pur — ne touche ni `World` ni
//! le réseau ; produit une liste d'actions que l'appelant applique (Task 6).

use crate::npc_catalog::NpcCatalog;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum DirectorAction {
    /// Faire naître un nouveau PNJ de cet archétype dans ce district.
    Spawn { district: String, archetype_id: u32 },
    /// Réduire la population de ce district (le nombre exact de despawns reste au choix de
    /// l'appelant — cette action indique juste "il y en a trop ici").
    Despawn { district: String, excess: u32 },
}

/// `Clone` : `shard_main` (Task 7) reconstruit un `Server::new_with_npcs` frais à chaque
/// connexion Gateway acceptée (même patron que `Server::new`/`new_with_metrics`) — il lui faut
/// donc pouvoir cloner le director construit une seule fois au boot plutôt que de le reconstruire
/// depuis le manifeste à chaque reconnexion.
#[derive(Clone)]
pub struct PopulationDirector {
    /// Densité cible par code de district (config manifeste, cf. Task 7 — `[runtime.population]`).
    target_density: HashMap<String, u32>,
    /// Liste de REJET de l'opérateur : tout archétype portant l'un de ces tags n'est jamais
    /// spawné (spec ambiance §4, modèle « permissif par défaut avec exclusions »). Vide par
    /// défaut => tout le catalogue est spawnable, comportement d'avant la curation préservé.
    excluded_tags: HashSet<String>,
}

impl PopulationDirector {
    pub fn new(target_density: HashMap<String, u32>) -> Self {
        Self {
            target_density,
            excluded_tags: HashSet::new(),
        }
    }

    /// Ajoute la liste de rejet de l'opérateur (`[runtime.population] exclure_tags`). Builder
    /// plutôt qu'un paramètre de `new` : les ~18 appelants existants de `new` restent intacts, et
    /// un déploiement sans curation ne change pas de comportement.
    pub fn with_excluded_tags(mut self, excluded_tags: HashSet<String>) -> Self {
        self.excluded_tags = excluded_tags;
        self
    }

    /// Compare la population actuelle par district à la cible et produit les actions nécessaires.
    /// Ne spawn QUE dans un district où au moins un joueur est présent (spec §3 : « près des
    /// joueurs présents ») — un district vide de joueurs ne produit jamais de `Spawn`, même sous
    /// sa cible, cohérent avec le principe LOD (§5/§7 : pas de simulation sans observateur).
    pub fn reconcile(
        &self,
        catalog: &NpcCatalog,
        players_by_district: &HashMap<String, u32>,
        existing_npc_count_by_district: &HashMap<String, u32>,
    ) -> Vec<DirectorAction> {
        let mut actions = Vec::new();
        for (district, &target) in &self.target_density {
            let has_players = players_by_district.get(district).copied().unwrap_or(0) > 0;
            let current = existing_npc_count_by_district
                .get(district)
                .copied()
                .unwrap_or(0);
            if !has_players {
                if current > 0 {
                    actions.push(DirectorAction::Despawn {
                        district: district.clone(),
                        excess: current,
                    });
                }
                continue;
            }
            if current < target {
                // On tire parmi les archétypes ÉLIGIBLES (la liste de rejet appliquée), et non
                // parmi tout le catalogue. Le choix reste déterministe — le plus petit id éligible
                // — la répartition PONDÉRÉE par archétype restant un raffinement futur (spec §5).
                // Si l'opérateur a tout exclu, aucun archétype n'est éligible : on ne spawn rien,
                // silencieusement (une config qui vide son propre pool est un choix de l'opérateur,
                // pas une erreur du director — pur, sans I/O, il ne peut pas logguer).
                if let Some(&archetype_id) = catalog
                    .eligible_archetype_ids(&self.excluded_tags)
                    .iter()
                    .min()
                {
                    for _ in current..target {
                        actions.push(DirectorAction::Spawn {
                            district: district.clone(),
                            archetype_id,
                        });
                    }
                }
            } else if current > target {
                actions.push(DirectorAction::Despawn {
                    district: district.clone(),
                    excess: current - target,
                });
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_catalog::parse_and_validate;

    fn test_catalog() -> NpcCatalog {
        parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur-de-rue"
            briques = ["flaner-sur-place"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn a_district_below_target_with_players_present_spawns_the_deficit() {
        let director = PopulationDirector::new(HashMap::from([("centre".to_string(), 5)]));
        let players = HashMap::from([("centre".to_string(), 1)]);
        let existing = HashMap::from([("centre".to_string(), 2)]);
        let actions = director.reconcile(&test_catalog(), &players, &existing);
        let spawns = actions
            .iter()
            .filter(|a| matches!(a, DirectorAction::Spawn { .. }))
            .count();
        assert_eq!(spawns, 3, "5 cible - 2 existants = 3 spawns");
    }

    #[test]
    fn a_district_with_no_players_never_spawns_even_below_target() {
        let director = PopulationDirector::new(HashMap::from([("desert".to_string(), 10)]));
        let players = HashMap::from([("desert".to_string(), 0)]);
        let existing = HashMap::from([("desert".to_string(), 0)]);
        let actions = director.reconcile(&test_catalog(), &players, &existing);
        assert!(
            actions.is_empty(),
            "aucun joueur présent -> aucun spawn, même à 0/10"
        );
    }

    #[test]
    fn a_district_with_no_players_but_existing_npcs_despawns_them() {
        let director = PopulationDirector::new(HashMap::from([("desert".to_string(), 10)]));
        let players = HashMap::from([("desert".to_string(), 0)]);
        let existing = HashMap::from([("desert".to_string(), 4)]);
        let actions = director.reconcile(&test_catalog(), &players, &existing);
        assert_eq!(
            actions,
            vec![DirectorAction::Despawn {
                district: "desert".to_string(),
                excess: 4
            }]
        );
    }

    #[test]
    fn a_district_above_target_despawns_the_excess() {
        let director = PopulationDirector::new(HashMap::from([("centre".to_string(), 5)]));
        let players = HashMap::from([("centre".to_string(), 3)]);
        let existing = HashMap::from([("centre".to_string(), 8)]);
        let actions = director.reconcile(&test_catalog(), &players, &existing);
        assert_eq!(
            actions,
            vec![DirectorAction::Despawn {
                district: "centre".to_string(),
                excess: 3
            }]
        );
    }

    #[test]
    fn a_district_exactly_at_target_produces_no_action() {
        let director = PopulationDirector::new(HashMap::from([("centre".to_string(), 5)]));
        let players = HashMap::from([("centre".to_string(), 1)]);
        let existing = HashMap::from([("centre".to_string(), 5)]);
        let actions = director.reconcile(&test_catalog(), &players, &existing);
        assert!(actions.is_empty());
    }

    #[test]
    fn a_district_not_in_target_density_is_ignored() {
        let director = PopulationDirector::new(HashMap::new());
        let players = HashMap::from([("hors-config".to_string(), 5)]);
        let existing = HashMap::new();
        let actions = director.reconcile(&test_catalog(), &players, &existing);
        assert!(actions.is_empty());
    }

    fn tagged_catalog() -> NpcCatalog {
        // id 1 = ambiant neutre (spawnable), id 2 = enfant (rejetable). Ids choisis pour que
        // l'exclusion CHANGE le résultat : sans exclusion le min est 1, mais on vérifie qu'exclure
        // le 1 force le repli sur le 2 — sinon le test passerait par coïncidence du min.
        parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "corpo"
            briques = ["flaner-sur-place"]
            tags = ["corpo"]
            [[archetype]]
            id = 2
            name = "enfant"
            briques = ["flaner-sur-place"]
            tags = ["child"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn an_excluded_tag_removes_that_archetype_from_the_spawn_pool() {
        // On exclut "corpo" (id 1, le plus petit) : les spawns doivent basculer sur l'enfant (id 2),
        // preuve que la sélection tire dans le pool ÉLIGIBLE et pas dans le catalogue entier.
        let director = PopulationDirector::new(HashMap::from([("centre".to_string(), 3)]))
            .with_excluded_tags(HashSet::from(["corpo".to_string()]));
        let players = HashMap::from([("centre".to_string(), 1)]);
        let existing = HashMap::new();
        let actions = director.reconcile(&tagged_catalog(), &players, &existing);
        assert_eq!(actions.len(), 3);
        assert!(
            actions.iter().all(|a| matches!(
                a,
                DirectorAction::Spawn {
                    archetype_id: 2,
                    ..
                }
            )),
            "l'archétype corpo étant exclu, tous les spawns doivent être l'enfant (id 2)"
        );
    }

    #[test]
    fn excluding_every_available_tag_produces_no_spawn() {
        // Piège de config : l'opérateur exclut tout son propre pool. Le district reste sous sa
        // cible mais rien n'est spawné — pas d'action, pas de panique, pas de despawn parasite.
        let director = PopulationDirector::new(HashMap::from([("centre".to_string(), 5)]))
            .with_excluded_tags(HashSet::from(["corpo".to_string(), "child".to_string()]));
        let players = HashMap::from([("centre".to_string(), 1)]);
        let existing = HashMap::new();
        let actions = director.reconcile(&tagged_catalog(), &players, &existing);
        assert!(
            actions.is_empty(),
            "pool entièrement exclu => aucun spawn, et aucune autre action"
        );
    }

    #[test]
    fn no_exclusion_preserves_the_pre_curation_behaviour() {
        // Sans liste de rejet, le director doit se comporter exactement comme avant : min id.
        let director = PopulationDirector::new(HashMap::from([("centre".to_string(), 2)]));
        let players = HashMap::from([("centre".to_string(), 1)]);
        let existing = HashMap::new();
        let actions = director.reconcile(&tagged_catalog(), &players, &existing);
        assert!(
            actions.iter().all(|a| matches!(
                a,
                DirectorAction::Spawn {
                    archetype_id: 1,
                    ..
                }
            )),
            "aucune exclusion => plus petit id (1), comportement d'avant la curation"
        );
    }
}
