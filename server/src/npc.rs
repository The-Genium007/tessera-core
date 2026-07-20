//! État comportemental des PNJ (FSM, canonique côté serveur) et enregistrement PNJ. Vit à CÔTÉ du
//! canal cosmétique `Pose` (`world.rs`), jamais fusionné dedans — `Pose` reste le triplet
//! locomotion/move_dir/flags/sustained partagé joueurs+PNJ ; ce module porte le POURQUOI
//! (comportement), `Pose` porte le RENDU (anim).

use crate::transport::ClientId;

/// État comportemental FSM (spec fondation PNJ §3, modèle serveur §2). `AlerteMenace`/`Fuite`/
/// `Hostile` portent une cible (id d'entité, joueur ou PNJ, dans le même espace `ClientId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityBehavior {
    #[default]
    Calme,
    Flane,
    Alerte { menace: ClientId },
    Fuite { menace: ClientId },
    Hostile { cible: ClientId },
    ATerre,
}

/// Enregistrement complet d'un PNJ, distinct de `Pose` (le canal cosmétique). `id` est dans la
/// plage réservée (`is_npc_id`, Task 6) — jamais un id de connexion réelle. Ne porte PAS la brique
/// active en propre : `apply_brique_tick` (Task 4) reçoit le `NpcArchetypeConfig` complet à chaque
/// appel (résolu depuis `archetype` par l'appelant) plutôt que dupliquer cette info ici — une seule
/// source de vérité pour "quelles briques sont actives pour ce PNJ" (le catalogue), pas deux.
#[derive(Debug, Clone)]
pub struct NpcRecord {
    pub id: ClientId,
    pub archetype: u32,
    pub owner: ClientId, // 0 = personne (hiberné) — cf. spec ownership §4
    pub behavior: EntityBehavior,
}

impl NpcRecord {
    pub fn new(id: ClientId, archetype: u32) -> Self {
        Self {
            id,
            archetype,
            owner: 0,
            behavior: EntityBehavior::default(),
        }
    }

    /// Transition FSM déclenchée par une `EntityInteraction` rapportée par un joueur (spec §2 :
    /// « le serveur met à jour l'état canonique »). `kind` : 0=Menace/attaque 1=Parle 2=Interagit
    /// (cf. schéma `EntityInteraction`, Task 5). Seul `kind=0` déclenche une transition de peur —
    /// les autres kinds sont gérés par les briques sociales (Task 4), pas la FSM elle-même.
    pub fn apply_interaction(&mut self, from: ClientId, kind: u8) {
        if kind == 0 {
            self.behavior = EntityBehavior::Fuite { menace: from };
        }
    }

    /// Un pas du moteur de briques (spec modèle serveur §5 : « lit ces configs et pilote le FSM +
    /// les intentions »). Nav-indépendant uniquement — aucune brique ici ne modifie la position ;
    /// `marcher-route`/`patrouiller`/`aller-au-point`/`errer` sont différées au plan de navigation
    /// et n'existent pas dans ce catalogue (Task 2 les rejetterait comme brique inconnue si un
    /// opérateur les déclarait par erreur avant que le plan suivant ne les implémente — la
    /// résolution ci-dessous ignore silencieusement une brique qu'elle ne reconnaît pas, un
    /// comportement délibérément permissif pour ne pas faire planter le serveur sur une config
    /// TOML en avance sur le code : documenté ici, pas un oubli).
    ///
    /// Ne fait RIEN si `behavior` est `Fuite`/`Hostile`/`ATerre` — ces états sont pilotés par la
    /// FSM (interactions), pas par la brique passive de l'archétype (cohérent avec spec §2 : « le
    /// FSM est la cause, la brique/pathfinding est l'effet » — une brique sociale ne doit jamais
    /// écraser un état de peur/agression en cours).
    pub fn apply_brique_tick(&mut self, archetype: &crate::npc_catalog::NpcArchetypeConfig) {
        if !matches!(self.behavior, EntityBehavior::Calme | EntityBehavior::Flane) {
            return;
        }
        if archetype
            .briques
            .iter()
            .any(|b| b == "flaner-sur-place")
        {
            self.behavior = EntityBehavior::Flane;
        }
        // "rester-statique" : ne change jamais `behavior` (reste Calme) — c'est la définition même
        // de la brique (spec §4 : « le danseur hip-hop = brique… zéro navigation »).
        // "vendre"/"donner-quête"/"réagir-dialogue" : hooks sociaux, pas de transition de
        // mouvement — traités par un futur plan d'interaction (fondation d'interaction, séquencement
        // Phase 3.1), hors périmètre ici.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_behavior_is_calme() {
        assert_eq!(EntityBehavior::default(), EntityBehavior::Calme);
    }

    #[test]
    fn alerte_and_fuite_carry_the_threat_id() {
        let b = EntityBehavior::Alerte { menace: 42 };
        assert_eq!(b, EntityBehavior::Alerte { menace: 42 });
        assert_ne!(b, EntityBehavior::Alerte { menace: 43 });
    }

    #[test]
    fn hostile_carries_the_target_id() {
        let b = EntityBehavior::Hostile { cible: 7 };
        assert_eq!(b, EntityBehavior::Hostile { cible: 7 });
    }
}

#[cfg(test)]
mod record_tests {
    use super::*;

    #[test]
    fn new_record_starts_calme_and_unowned() {
        let r = NpcRecord::new(1_000_000, 7);
        assert_eq!(r.behavior, EntityBehavior::Calme);
        assert_eq!(r.owner, 0);
        assert_eq!(r.archetype, 7);
    }

    #[test]
    fn a_threat_interaction_triggers_fuite_with_the_reporter_as_threat() {
        let mut r = NpcRecord::new(1_000_000, 7);
        r.apply_interaction(55, 0);
        assert_eq!(r.behavior, EntityBehavior::Fuite { menace: 55 });
    }

    #[test]
    fn a_non_threat_interaction_does_not_change_behavior() {
        let mut r = NpcRecord::new(1_000_000, 7);
        r.apply_interaction(55, 2); // kind=2=Interagit
        assert_eq!(r.behavior, EntityBehavior::Calme);
    }

    #[test]
    fn a_second_threat_interaction_updates_the_threat_id() {
        let mut r = NpcRecord::new(1_000_000, 7);
        r.apply_interaction(55, 0);
        r.apply_interaction(99, 0);
        assert_eq!(r.behavior, EntityBehavior::Fuite { menace: 99 });
    }
}

#[cfg(test)]
mod brique_engine_tests {
    use super::*;
    use crate::npc_catalog::parse_and_validate;

    fn archetype_with_briques(briques: &[&str]) -> crate::npc_catalog::NpcArchetypeConfig {
        let toml = format!(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"test\"\nbriques = [{}]\n",
            briques
                .iter()
                .map(|b| format!("\"{b}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        parse_and_validate(&toml).unwrap().archetype(1).unwrap().clone()
    }

    #[test]
    fn flaner_sur_place_brique_moves_calme_npc_to_flane() {
        let mut r = NpcRecord::new(1, 1);
        let archetype = archetype_with_briques(&["flaner-sur-place"]);
        r.apply_brique_tick(&archetype);
        assert_eq!(r.behavior, EntityBehavior::Flane);
    }

    #[test]
    fn rester_statique_brique_never_changes_calme() {
        let mut r = NpcRecord::new(1, 1);
        let archetype = archetype_with_briques(&["rester-statique"]);
        r.apply_brique_tick(&archetype);
        assert_eq!(r.behavior, EntityBehavior::Calme);
    }

    #[test]
    fn brique_tick_never_overrides_an_active_fuite_state() {
        let mut r = NpcRecord::new(1, 1);
        r.apply_interaction(99, 0); // déclenche Fuite
        let archetype = archetype_with_briques(&["flaner-sur-place"]);
        r.apply_brique_tick(&archetype);
        assert_eq!(
            r.behavior,
            EntityBehavior::Fuite { menace: 99 },
            "une brique passive ne doit jamais écraser un état de peur en cours"
        );
    }

    #[test]
    fn an_unrecognized_brique_name_is_silently_ignored_not_a_panic() {
        let mut r = NpcRecord::new(1, 1); // brique nav, pas encore implémentée ici
        let archetype = archetype_with_briques(&["marcher-route"]);
        r.apply_brique_tick(&archetype); // ne doit pas paniquer
        assert_eq!(r.behavior, EntityBehavior::Calme);
    }
}
