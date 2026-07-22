//! État comportemental des PNJ (FSM, canonique côté serveur) et enregistrement PNJ. Vit à CÔTÉ du
//! canal cosmétique `Pose` (`world.rs`), jamais fusionné dedans — `Pose` reste le triplet
//! locomotion/move_dir/flags/sustained partagé joueurs+PNJ ; ce module porte le POURQUOI
//! (comportement), `Pose` porte le RENDU (anim).

use crate::transport::ClientId;

/// État comportemental FSM (spec fondation PNJ §3, modèle serveur §2). `Alerte`/`Fuite`/
/// `Hostile` portent une cible (id d'entité, joueur ou PNJ, dans le même espace `ClientId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityBehavior {
    #[default]
    Calme,
    Flane,
    Alerte {
        menace: ClientId,
    },
    Fuite {
        menace: ClientId,
    },
    Hostile {
        cible: ClientId,
    },
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
    /// PV actuels du PNJ — `None` = increvable (archétype sans `NpcCombatConfig`, cf.
    /// `NpcArchetypeConfig::combat`). `Some(0)` n'existe jamais comme état stable : dès que les PV
    /// atteignent 0, `apply_damage` bascule immédiatement `behavior` à `ATerre` dans le MÊME appel
    /// (spec §1 : "bascule le FSM en à-terre/mort à zéro").
    pub hp: Option<u32>,
    /// Horodatage (ms depuis un epoch arbitraire cohérent avec l'appelant, cf. `now_ms` du
    /// paramètre d'`apply_damage`) du dernier rapport de dégâts ACCEPTÉ par (attaquant, cette
    /// cible) — anti-spam de cadence (spec §2). Absence de clé = jamais encore de dégâts reçus de
    /// cet attaquant.
    pub last_damage_at_ms: std::collections::HashMap<ClientId, u64>,
}

impl NpcRecord {
    pub fn new(id: ClientId, archetype: u32) -> Self {
        Self {
            id,
            archetype,
            owner: 0,
            behavior: EntityBehavior::default(),
            hp: None,
            last_damage_at_ms: std::collections::HashMap::new(),
        }
    }

    /// Applique un rapport de dégâts (spec §1/§2 : "n'importe quel attaquant rapporte ses hits...
    /// clamp de plausibilité... bascule le FSM à ATerre à zéro"). `archetype` fournit le clamp
    /// (`degats_max_par_rapport`) et la cadence minimale — même patron que `apply_brique_tick`
    /// (reçoit le catalogue résolu par l'appelant, ne le duplique pas). Retourne `true` SEULEMENT
    /// si CET appel vient de faire passer le PNJ à `ATerre` (transition, pas état) — permet à
    /// l'appelant de ne déclencher un effet ponctuel (ex. future notification) qu'une seule fois,
    /// pas à chaque tick où le PNJ reste à terre.
    pub fn apply_damage(
        &mut self,
        attacker: ClientId,
        archetype: &crate::npc_catalog::NpcArchetypeConfig,
        degats: u32,
        now_ms: u64,
    ) -> bool {
        let Some(combat) = &archetype.combat else {
            return false; // increvable — aucun état PV, aucune transition possible
        };
        if self.behavior == EntityBehavior::ATerre {
            return false; // déjà à terre — un rapport tardif n'a plus d'effet (pas de "sur-mort")
        }
        // Cadence anti-spam : ignore silencieusement un rapport trop rapproché du précédent pour CE
        // MÊME attaquant (spec §2 "cadence/précision inhumaine") — un rapport ignoré ne consomme pas
        // le cooldown (l'horodatage n'est mis à jour QUE sur un rapport accepté), donc un spam constant
        // ne fait jamais "glisser" la fenêtre plus loin que le premier coup accepté.
        if let Some(&last) = self.last_damage_at_ms.get(&attacker) {
            if now_ms.saturating_sub(last) < combat.cadence_min_ms as u64 {
                return false;
            }
        }
        self.last_damage_at_ms.insert(attacker, now_ms);
        let clamped = degats.min(combat.degats_max_par_rapport);
        let current = self.hp.unwrap_or(combat.hp);
        let new_hp = current.saturating_sub(clamped);
        self.hp = Some(new_hp);
        if new_hp == 0 {
            self.behavior = EntityBehavior::ATerre;
            return true;
        }
        false
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
    /// La brique `flaner-sur-place` ne s'applique qu'aux états `Calme`/`Flane` (cohérent avec spec
    /// §2 : « le FSM est la cause, la brique/pathfinding est l'effet » — une brique sociale ne doit
    /// jamais écraser un état de peur/agression en cours) : la condition est portée localement sur
    /// cette brique précise, pas par un early-return sur toute la fonction — `Fuite`/`Hostile`/
    /// `ATerre` continuent d'exécuter cette fonction (changement délibéré, plan navigation) pour
    /// permettre à un futur appelant d'y piloter leur destination via `decide_destination`.
    pub fn apply_brique_tick(&mut self, archetype: &crate::npc_catalog::NpcArchetypeConfig) {
        if matches!(self.behavior, EntityBehavior::Calme | EntityBehavior::Flane)
            && archetype.briques.iter().any(|b| b == "flaner-sur-place")
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

/// Résout la brique/l'état FSM actif d'un PNJ en une destination cible (spec §4 : « chaque brique
/// produit une intention de destination ; le pathfinding la réalise » ; spec §5 : « fuite → aller-
/// au-point dans la direction opposée à la menace... hostile → aller-au-point vers la cible »).
/// `menace_or_cible_position` : position ACTUELLE de la menace/cible (résolue par l'appelant via
/// `World::pose_of`, ce module n'accède jamais à `World` directement — cohérent avec le reste de
/// `npc.rs`, pur). `region_center`/`region_radius` : bornes de la brique `errer` (spec §4 :
/// « destinations aléatoires dans une région »). Retourne `None` si aucune brique/état ne pilote
/// de destination (ex. `rester-statique`, `ATerre` — un PNJ à terre ne se déplace jamais).
pub fn decide_destination(
    behavior: EntityBehavior,
    archetype: &crate::npc_catalog::NpcArchetypeConfig,
    current_position: (f32, f32, f32),
    menace_or_cible_position: Option<(f32, f32, f32)>,
    region_center: (f32, f32, f32),
    region_radius: f32,
    rng_unit: f32, // [0.0, 1.0), injecté par l'appelant pour rester testable déterministe
) -> Option<(f32, f32, f32)> {
    match behavior {
        EntityBehavior::ATerre => None,
        EntityBehavior::Fuite { .. } => {
            let menace = menace_or_cible_position?;
            let (cx, cy, cz) = current_position;
            let (mx, my, _) = menace;
            let (dx, dy) = (cx - mx, cy - my);
            let len = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
            // Direction opposée à la menace, distance fixe (spec : « dans la direction opposée »).
            const FLEE_DISTANCE: f32 = 30.0;
            Some((
                cx + (dx / len) * FLEE_DISTANCE,
                cy + (dy / len) * FLEE_DISTANCE,
                cz,
            ))
        }
        EntityBehavior::Hostile { .. } => menace_or_cible_position,
        // `Alerte` = vigilance accrue, pas encore de fuite/agression (spec §2 : reste interactible
        // comme Calme/Flane) — aucune spec §4/§5 ne lui attribue de destination dirigée propre ;
        // elle suit donc la même résolution que Calme/Flane (briques passives de l'archétype).
        EntityBehavior::Calme | EntityBehavior::Flane | EntityBehavior::Alerte { .. } => {
            if archetype.briques.iter().any(|b| b == "errer") {
                // Destination aléatoire dans la région (spec §4). rng_unit in [0,1) mappé sur un
                // angle+rayon dans le disque de région — déterministe pour les tests (rng injecté).
                let angle = rng_unit * std::f32::consts::TAU;
                let radius = region_radius * ((rng_unit * 7.0) % 1.0).sqrt();
                let (cx, cy, cz) = region_center;
                Some((cx + radius * angle.cos(), cy + radius * angle.sin(), cz))
            } else if archetype.briques.iter().any(|b| b == "marcher-route") {
                // v1 minimal : la "route" EST la région (liste de points différée, spec §4 note
                // "config : liste de points/région" — la variante liste-de-points explicite est un
                // raffinement futur, non implémenté ici ; documenté, pas un oubli).
                Some(region_center)
            } else if archetype.briques.iter().any(|b| b == "patrouiller") {
                Some(region_center)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod destination_tests {
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
        parse_and_validate(&toml)
            .unwrap()
            .archetype(1)
            .unwrap()
            .clone()
    }

    #[test]
    fn aterre_never_produces_a_destination() {
        let archetype = archetype_with_briques(&["errer"]);
        let dest = decide_destination(
            EntityBehavior::ATerre,
            &archetype,
            (0.0, 0.0, 0.0),
            None,
            (0.0, 0.0, 0.0),
            10.0,
            0.5,
        );
        assert_eq!(dest, None);
    }

    #[test]
    fn fuite_produces_a_destination_away_from_the_threat() {
        let archetype = archetype_with_briques(&["errer"]);
        let dest = decide_destination(
            EntityBehavior::Fuite { menace: 99 },
            &archetype,
            (0.0, 0.0, 0.0),
            Some((10.0, 0.0, 0.0)), // menace à l'est
            (0.0, 0.0, 0.0),
            10.0,
            0.5,
        )
        .unwrap();
        assert!(
            dest.0 < 0.0,
            "doit fuir vers l'ouest, loin de la menace à l'est"
        );
    }

    #[test]
    fn fuite_without_a_resolvable_threat_position_produces_no_destination() {
        let archetype = archetype_with_briques(&["errer"]);
        let dest = decide_destination(
            EntityBehavior::Fuite { menace: 99 },
            &archetype,
            (0.0, 0.0, 0.0),
            None,
            (0.0, 0.0, 0.0),
            10.0,
            0.5,
        );
        assert_eq!(dest, None);
    }

    #[test]
    fn hostile_produces_the_target_position_directly() {
        let archetype = archetype_with_briques(&["errer"]);
        let dest = decide_destination(
            EntityBehavior::Hostile { cible: 5 },
            &archetype,
            (0.0, 0.0, 0.0),
            Some((7.0, 8.0, 9.0)),
            (0.0, 0.0, 0.0),
            10.0,
            0.5,
        );
        assert_eq!(dest, Some((7.0, 8.0, 9.0)));
    }

    #[test]
    fn calme_with_errer_brique_produces_a_destination_within_the_region_radius() {
        let archetype = archetype_with_briques(&["errer"]);
        let dest = decide_destination(
            EntityBehavior::Calme,
            &archetype,
            (0.0, 0.0, 0.0),
            None,
            (100.0, 100.0, 0.0),
            5.0,
            0.3,
        )
        .unwrap();
        let dx = dest.0 - 100.0;
        let dy = dest.1 - 100.0;
        assert!((dx * dx + dy * dy).sqrt() <= 5.0 + 0.01);
    }

    #[test]
    fn calme_with_rester_statique_only_produces_no_destination() {
        let archetype = archetype_with_briques(&["rester-statique"]);
        let dest = decide_destination(
            EntityBehavior::Calme,
            &archetype,
            (0.0, 0.0, 0.0),
            None,
            (0.0, 0.0, 0.0),
            10.0,
            0.5,
        );
        assert_eq!(dest, None);
    }
}

/// Une cible en `Fuite`/`Hostile`/`ATerre` refuse toute interaction fonctionnelle (spec §2 : « un
/// vendeur `fuite`/`à terre` refuse ») — matrice interaction×FSM minimale (spec §10, « à écrire
/// dans le moteur de briques »). `Calme`/`Flane`/`Alerte` restent interactibles : `Alerte` n'est
/// qu'une vigilance accrue, pas un refus de contact (un PNJ qui a repéré une menace au loin peut
/// encore répondre à un client qui l'aborde calmement).
///
/// Note : contrairement à `behavior_to_u8` (match exhaustif, sans bras `_`, qui casse
/// volontairement à la compilation si `EntityBehavior` gagne un variant), cette fonction utilise
/// `matches!` sur une liste de refus — un futur variant non listé ici serait AUTORISÉ par défaut,
/// pas refusé. Si un futur plan ajoute un état FSM qui doit refuser l'interaction, l'ajouter
/// explicitement à ce `matches!`, ne pas compter sur une erreur de compilation pour le rappeler.
pub fn interaction_allowed(behavior: EntityBehavior) -> bool {
    !matches!(
        behavior,
        EntityBehavior::Fuite { .. } | EntityBehavior::Hostile { .. } | EntityBehavior::ATerre
    )
}

/// Résultat d'une transaction d'interaction (spec §4 : « transaction atomique »). `ok=false` porte
/// une raison IN-FICTION (spec §2 : jamais « erreur 409 ») — le CONTENU de cette raison est
/// hors périmètre ici (aucune string réelle n'est décidée par ce squelette), seul le TRANSPORT
/// (succès/échec + payload opaque) est fondé.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionOutcome {
    pub ok: bool,
    pub payload: Vec<u8>,
}

/// Squelette de transaction atomique (spec §4, §9 : contenu différé, plomberie seule). Suit
/// EXACTEMENT le patron `admin_commands::execute` (server/src/admin_commands.rs:194-202) : PURE,
/// synchrone, aucune I/O, aucun `.await` — la persistance et le log surviennent APRÈS, dans
/// l'appelant (Task 5), jamais ici. `apply` reçoit une closure représentant LE contenu réel de la
/// transaction (débit/stock/inventaire — inconnu de ce module, injecté par l'appelant) ; ce
/// squelette ne fait que garantir l'ordre (vérifier `interaction_allowed` D'ABORD, exécuter
/// ENSUITE, jamais l'inverse) — la seule garantie d'atomicité que ce plan peut honnêtement offrir
/// sans connaître le contenu réel (spec §9 : l'économie est un gros sous-système séparé, différé).
pub fn execute_transaction(
    target_behavior: EntityBehavior,
    apply: impl FnOnce() -> TransactionOutcome,
) -> TransactionOutcome {
    if !interaction_allowed(target_behavior) {
        return TransactionOutcome {
            ok: false,
            payload: Vec::new(),
        };
    }
    apply()
}

#[cfg(test)]
mod interaction_fsm_tests {
    use super::*;

    #[test]
    fn calme_flane_and_alerte_allow_interaction() {
        assert!(interaction_allowed(EntityBehavior::Calme));
        assert!(interaction_allowed(EntityBehavior::Flane));
        assert!(interaction_allowed(EntityBehavior::Alerte { menace: 1 }));
    }

    #[test]
    fn fuite_hostile_and_aterre_refuse_interaction() {
        assert!(!interaction_allowed(EntityBehavior::Fuite { menace: 1 }));
        assert!(!interaction_allowed(EntityBehavior::Hostile { cible: 1 }));
        assert!(!interaction_allowed(EntityBehavior::ATerre));
    }

    #[test]
    fn execute_transaction_calls_apply_when_the_target_is_interactible() {
        let outcome = execute_transaction(EntityBehavior::Calme, || TransactionOutcome {
            ok: true,
            payload: vec![1, 2, 3],
        });
        assert_eq!(
            outcome,
            TransactionOutcome {
                ok: true,
                payload: vec![1, 2, 3]
            }
        );
    }

    #[test]
    fn execute_transaction_never_calls_apply_when_the_target_refuses() {
        // La closure ne doit JAMAIS s'exécuter si la cible refuse — vérifié en la faisant paniquer
        // si elle est appelée : un test qui passerait silencieusement sans cette assertion ne
        // prouverait rien sur l'ORDRE des opérations.
        let outcome = execute_transaction(EntityBehavior::ATerre, || {
            panic!("apply ne doit jamais être appelée si interaction_allowed est faux")
        });
        assert_eq!(outcome.ok, false);
        assert!(outcome.payload.is_empty());
    }
}

/// Encodage `EntityBehavior` -> `NpcState.behavior:ubyte` (protocol.fbs, doc comment sur `NpcState` :
/// 0=Calme 1=Flane 2=Alerte 3=Fuite 4=Hostile 5=ATerre). `EntityBehavior` porte des données sur
/// certains variants (`ClientId` pour Alerte/Fuite/Hostile) et n'a pas de `#[repr(u8)]` — un `as u8`
/// nu ne compile pas dessus, d'où cette fonction dédiée. Match exhaustif SANS bras `_` : si un futur
/// plan ajoute un variant à `EntityBehavior`, cette fonction cesse de compiler tant qu'elle n'est pas
/// mise à jour — propriété délibérée, ne pas ajouter de bras générique pour "corriger" une erreur de
/// compilation ici.
pub fn behavior_to_u8(b: EntityBehavior) -> u8 {
    match b {
        EntityBehavior::Calme => 0,
        EntityBehavior::Flane => 1,
        EntityBehavior::Alerte { .. } => 2,
        EntityBehavior::Fuite { .. } => 3,
        EntityBehavior::Hostile { .. } => 4,
        EntityBehavior::ATerre => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_to_u8_matches_the_documented_protocol_encoding() {
        assert_eq!(behavior_to_u8(EntityBehavior::Calme), 0);
        assert_eq!(behavior_to_u8(EntityBehavior::Flane), 1);
        assert_eq!(behavior_to_u8(EntityBehavior::Alerte { menace: 1 }), 2);
        assert_eq!(behavior_to_u8(EntityBehavior::Fuite { menace: 1 }), 3);
        assert_eq!(behavior_to_u8(EntityBehavior::Hostile { cible: 1 }), 4);
        assert_eq!(behavior_to_u8(EntityBehavior::ATerre), 5);
    }

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
        parse_and_validate(&toml)
            .unwrap()
            .archetype(1)
            .unwrap()
            .clone()
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

    #[test]
    fn apply_brique_tick_no_longer_early_returns_on_fuite_hostile_aterre() {
        // Changement de comportement délibéré (plan navigation) : avant, apply_brique_tick ne faisait
        // RIEN pour ces 3 états. Après, la fonction continue de s'exécuter (même si aucune brique
        // nav-indépendante ne s'applique à ces états) — ce test verrouille juste l'absence de panique/
        // early-return silencieux, la logique de destination réelle vit dans decide_destination
        // (testé séparément ci-dessus), appelée par Server::tick_npcs (Task 5), pas ici.
        let mut r = NpcRecord::new(1, 1);
        r.apply_interaction(99, 0); // -> Fuite { menace: 99 }
        let archetype = crate::npc_catalog::parse_and_validate(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"flaner-sur-place\"]\n",
        )
        .unwrap()
        .archetype(1)
        .unwrap()
        .clone();
        r.apply_brique_tick(&archetype);
        // flaner-sur-place ne s'applique qu'à Calme/Flane -> le behavior Fuite reste inchangé.
        assert_eq!(r.behavior, EntityBehavior::Fuite { menace: 99 });
    }
}

#[cfg(test)]
mod combat_tests {
    use super::*;
    use crate::npc_catalog::parse_and_validate;

    fn combat_archetype(
        hp: u32,
        degats_max: u32,
        cadence_ms: u32,
    ) -> crate::npc_catalog::NpcArchetypeConfig {
        let toml = format!(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"attaquer-cible\"]\n\
             [archetype.combat]\nhp = {hp}\ndegats_max_par_rapport = {degats_max}\ncadence_min_ms = {cadence_ms}\n"
        );
        parse_and_validate(&toml)
            .unwrap()
            .archetype(1)
            .unwrap()
            .clone()
    }

    #[test]
    fn an_increvable_npc_never_transitions_to_aterre() {
        let mut r = NpcRecord::new(1, 1);
        let toml =
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"errer\"]\n";
        let archetype = parse_and_validate(toml)
            .unwrap()
            .archetype(1)
            .unwrap()
            .clone();
        let transitioned = r.apply_damage(99, &archetype, 1000, 0);
        assert!(!transitioned);
        assert_eq!(r.behavior, EntityBehavior::Calme);
        assert!(r.hp.is_none());
    }

    #[test]
    fn damage_is_clamped_per_report_and_hp_decreases() {
        let mut r = NpcRecord::new(1, 1);
        let archetype = combat_archetype(100, 40, 0);
        r.apply_damage(99, &archetype, 1000, 0); // demande 1000, clampé à 40
        assert_eq!(r.hp, Some(60));
    }

    #[test]
    fn hp_reaching_zero_transitions_to_aterre_and_reports_true_once() {
        let mut r = NpcRecord::new(1, 1);
        let archetype = combat_archetype(40, 40, 0);
        let transitioned = r.apply_damage(99, &archetype, 40, 0);
        assert!(transitioned);
        assert_eq!(r.behavior, EntityBehavior::ATerre);

        // Un second rapport, déjà à terre : aucune transition supplémentaire signalée.
        let transitioned_again = r.apply_damage(99, &archetype, 40, 1000);
        assert!(!transitioned_again);
    }

    #[test]
    fn a_report_faster_than_cadence_min_is_silently_ignored() {
        let mut r = NpcRecord::new(1, 1);
        let archetype = combat_archetype(100, 40, 500);
        r.apply_damage(99, &archetype, 40, 0);
        assert_eq!(r.hp, Some(60));
        r.apply_damage(99, &archetype, 40, 100); // 100ms < 500ms de cadence -> ignoré
        assert_eq!(
            r.hp,
            Some(60),
            "le second rapport, trop rapproché, ne doit rien changer"
        );
        r.apply_damage(99, &archetype, 40, 600); // 600ms > 500ms depuis le PREMIER rapport accepté
        assert_eq!(r.hp, Some(20));
    }

    #[test]
    fn different_attackers_have_independent_cadence_windows() {
        let mut r = NpcRecord::new(1, 1);
        let archetype = combat_archetype(100, 40, 500);
        r.apply_damage(1, &archetype, 40, 0);
        r.apply_damage(2, &archetype, 40, 10); // attaquant DIFFÉRENT, quasi simultané -> accepté
        assert_eq!(r.hp, Some(20));
    }
}
