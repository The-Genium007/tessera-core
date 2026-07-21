//! Modèle serveur d'ascenseur (spec `2026-07-21-ascenseurs-modele-serveur-design.md`) : état
//! autoritaire par cabine + file d'attente SCAN directionnelle. PUR — aucune I/O, aucun réseau,
//! aucune horloge murale : tout se pilote par des numéros de tick passés par l'appelant, ce qui
//! rend le module entièrement testable sans lancer le jeu.
//!
//! ADR 0012 : le moteur vanilla déplace la cabine, le serveur DÉCIDE. Aucune position de cabine ne
//! circule jamais sur le réseau — le contrat est « même trajet, même instant de départ, même durée
//! => mêmes positions par construction ».

use std::collections::BTreeSet;

/// Sens de marche courant. PERSISTE entre deux trajets : c'est exactement ce qui rend la file
/// directionnelle (SCAN/LOOK) au lieu d'un FIFO, qui produirait des allers-retours absurdes pour le
/// même coût d'implémentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Défaut au boot. Arbitraire, mais FIXE — un défaut « vers l'appel le plus proche » rendrait
    /// les tests non reproductibles, et le déterminisme est le contrat de ce module.
    #[default]
    Up,
    Down,
}

/// État de mouvement de la cabine. Encodage fil (spec §6) : 0=Stopped 1=MovingUp 2=MovingDown
/// 3=Paused. `Paused` n'est produit par aucune transition de ce plan — il existe parce que le
/// vanilla l'expose (`gamePlatformMovementState`) et qu'on veut pouvoir le représenter sans changer
/// le schéma plus tard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MovementState {
    #[default]
    Stopped,
    MovingUp,
    MovingDown,
    Paused,
}

/// Viabilité PHYSIQUE d'un étage (donnée du jeu). Ne porte PAS d'autorisation par progression solo :
/// l'accès est par-monde (décision (b) de la spec) et le rejeu client passe par `ForceGoToFloor`,
/// qui contourne `m_floorsAuthorization` de toute façon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorSpec {
    pub index: i32,
    pub hidden: bool,
    pub inactive: bool,
}

/// État autoritaire d'une cabine. `arrival_tick` est INTERNE au serveur et ne part jamais sur le
/// fil : le client dérive l'arrivée de `depart_tick + start_delay_ms + travel_time_ms`, donc
/// l'envoyer créerait une seconde source de vérité.
#[derive(Debug, Clone, PartialEq)]
pub struct ElevatorState {
    pub elevator_id: u64,
    pub active_floor: i32,
    pub target_floor: Option<i32>,
    pub movement_state: MovementState,
    pub direction: Direction,
    /// Les BOUTONS ALLUMÉS. Répliqués tels quels : sans eux, le joueur B ne voit pas l'appel que le
    /// joueur A vient de passer. `BTreeSet` = ordonné (déterminisme du SCAN) ET idempotent (le spam
    /// de bouton ne fait rien).
    pub requested_floors: BTreeSet<i32>,
    pub floors: Vec<FloorSpec>,
    pub start_delay_ms: u32,
    pub travel_time_ms: u32,
    pub depart_tick: Option<u64>,
    pub arrival_tick: Option<u64>,
}

impl ElevatorState {
    pub fn new(
        elevator_id: u64,
        active_floor: i32,
        floors: Vec<FloorSpec>,
        start_delay_ms: u32,
        travel_time_ms: u32,
    ) -> Self {
        Self {
            elevator_id,
            active_floor,
            target_floor: None,
            movement_state: MovementState::Stopped,
            direction: Direction::default(),
            requested_floors: BTreeSet::new(),
            floors,
            start_delay_ms,
            travel_time_ms,
            depart_tick: None,
            arrival_tick: None,
        }
    }

    /// Cœur SCAN/LOOK : dessert dans le sens de la marche jusqu'à épuisement des appels dans ce
    /// sens, puis inverse. Retourne l'étage à desservir ET le sens à adopter.
    ///
    /// PURE — ne modifie rien. L'inversion de sens est ainsi une valeur de retour explicite plutôt
    /// qu'un effet de bord, ce qui la rend testable isolément.
    pub fn next_target(&self) -> Option<(i32, Direction)> {
        if self.requested_floors.is_empty() {
            return None;
        }
        // Desserte sur place : le joueur a appuyé sur l'étage où il se trouve déjà.
        if self.requested_floors.contains(&self.active_floor) {
            return Some((self.active_floor, self.direction));
        }
        let above = self
            .requested_floors
            .range((self.active_floor + 1)..)
            .next()
            .copied();
        let below = self
            .requested_floors
            .range(..self.active_floor)
            .next_back()
            .copied();
        match self.direction {
            Direction::Up => above
                .map(|f| (f, Direction::Up))
                .or_else(|| below.map(|f| (f, Direction::Down))),
            Direction::Down => below
                .map(|f| (f, Direction::Down))
                .or_else(|| above.map(|f| (f, Direction::Up))),
        }
    }

    /// Un étage est appelable s'il existe ET n'est ni caché ni inactif. On ne modélise QUE la
    /// viabilité physique — l'autorisation par progression solo est délibérément ignorée
    /// (accès par-monde, décision (b) de la spec).
    pub fn is_floor_callable(&self, floor: i32) -> bool {
        self.floors
            .iter()
            .any(|f| f.index == floor && !f.hidden && !f.inactive)
    }

    /// Enregistre un appel. Un étage inexistant / caché / inactif est REJETÉ SILENCIEUSEMENT : le
    /// bouton ne s'allume pas, et aucune erreur technique n'est exposée au joueur (règle RP :
    /// jamais « erreur 409 »). Idempotent, et ACCEPTÉ MÊME EN MOUVEMENT — c'est la différence
    /// essentielle avec le vanilla, qui refuse toute action pendant un trajet.
    ///
    /// Retourne `true` si l'appel a été retenu (utile à l'appelant pour savoir s'il faut diffuser).
    pub fn request_floor(&mut self, floor: i32) -> bool {
        if !self.is_floor_callable(floor) {
            return false;
        }
        self.requested_floors.insert(floor)
    }

    /// Démarre un trajet si la cabine est au repos et qu'un appel attend. `tick_ms` est la durée
    /// d'un tick serveur, passée par l'appelant : ce module ne lit aucune horloge, c'est ce qui le
    /// garde pur et déterministe.
    pub fn start_trip_if_idle(&mut self, now_tick: u64, tick_ms: u32) {
        if self.movement_state != MovementState::Stopped || self.target_floor.is_some() {
            return;
        }
        let Some((target, direction)) = self.next_target() else {
            return;
        };
        self.direction = direction;
        if target == self.active_floor {
            // Desserte sur place : aucun mouvement, l'appel est simplement consommé (l'ouverture
            // des portes est un effet CLIENT, pas un état serveur).
            self.requested_floors.remove(&target);
            return;
        }
        self.target_floor = Some(target);
        self.movement_state = if target > self.active_floor {
            MovementState::MovingUp
        } else {
            MovementState::MovingDown
        };
        self.depart_tick = Some(now_tick);
        let total_ms = self.start_delay_ms as u64 + self.travel_time_ms as u64;
        let tick_ms = tick_ms.max(1) as u64; // garde-fou : un tick_ms nul diviserait par zéro
        let ticks = total_ms.div_ceil(tick_ms);
        self.arrival_tick = Some(now_tick + ticks);
    }

    /// Fait avancer la cabine d'un tick : applique l'arrivée si son tick est atteint, puis enchaîne
    /// automatiquement sur l'appel suivant. Retourne `true` si l'état a changé — l'appelant s'en
    /// sert pour ne diffuser que sur transition (spec §5.3).
    ///
    /// L'arrivée est calculée ICI, par le serveur, jamais rapportée par un client : l'état reste
    /// donc juste même si plus personne ne regarde la cabine.
    pub fn advance(&mut self, now_tick: u64, tick_ms: u32) -> bool {
        let before = self.snapshot_for_change_detection();
        if let (Some(target), Some(arrival)) = (self.target_floor, self.arrival_tick) {
            if now_tick >= arrival {
                self.active_floor = target;
                self.requested_floors.remove(&target);
                self.target_floor = None;
                self.movement_state = MovementState::Stopped;
                self.depart_tick = None;
                self.arrival_tick = None;
            }
        }
        self.start_trip_if_idle(now_tick, tick_ms);
        self.snapshot_for_change_detection() != before
    }

    /// Empreinte des champs dont un changement doit déclencher une diffusion. Volontairement
    /// distincte de `PartialEq` sur la struct entière : `arrival_tick` est interne et ne justifie
    /// pas à lui seul un message réseau.
    fn snapshot_for_change_detection(&self) -> (i32, Option<i32>, MovementState, usize) {
        (
            self.active_floor,
            self.target_floor,
            self.movement_state,
            self.requested_floors.len(),
        )
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    fn three_floors() -> Vec<FloorSpec> {
        (0..=5)
            .map(|i| FloorSpec {
                index: i,
                hidden: false,
                inactive: false,
            })
            .collect()
    }

    fn at(active: i32, direction: Direction, requested: &[i32]) -> ElevatorState {
        let mut e = ElevatorState::new(1, active, three_floors(), 1000, 4000);
        e.direction = direction;
        e.requested_floors = requested.iter().copied().collect();
        e
    }

    #[test]
    fn default_direction_is_up_and_fixed() {
        let e = ElevatorState::new(1, 0, three_floors(), 1000, 4000);
        assert_eq!(e.direction, Direction::Up);
    }

    #[test]
    fn no_request_yields_no_target() {
        let e = at(2, Direction::Up, &[]);
        assert_eq!(e.next_target(), None);
    }

    #[test]
    fn going_up_serves_the_nearest_floor_above_first() {
        // Appels en 3 et 5 depuis l'étage 2 en montée : on sert 3 AVANT 5.
        let e = at(2, Direction::Up, &[3, 5]);
        assert_eq!(e.next_target(), Some((3, Direction::Up)));
    }

    #[test]
    fn going_up_with_nothing_above_reverses_to_the_highest_below() {
        // Plus rien au-dessus : on inverse et on prend le plus HAUT en dessous (le plus proche).
        let e = at(4, Direction::Up, &[1, 3]);
        assert_eq!(e.next_target(), Some((3, Direction::Down)));
    }

    #[test]
    fn going_down_serves_the_nearest_floor_below_first() {
        let e = at(4, Direction::Down, &[1, 3]);
        assert_eq!(e.next_target(), Some((3, Direction::Down)));
    }

    #[test]
    fn going_down_with_nothing_below_reverses_to_the_lowest_above() {
        let e = at(1, Direction::Down, &[3, 5]);
        assert_eq!(e.next_target(), Some((3, Direction::Up)));
    }

    #[test]
    fn a_request_for_the_current_floor_is_served_in_place_without_changing_direction() {
        let e = at(2, Direction::Down, &[2, 5]);
        assert_eq!(e.next_target(), Some((2, Direction::Down)));
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    const TICK_MS: u32 = 50;

    fn floors_0_to_5() -> Vec<FloorSpec> {
        (0..=5)
            .map(|i| FloorSpec {
                index: i,
                hidden: false,
                inactive: false,
            })
            .collect()
    }

    /// start_delay 1000 ms + travel 4000 ms = 5000 ms => 100 ticks à 50 ms.
    fn fresh() -> ElevatorState {
        ElevatorState::new(42, 0, floors_0_to_5(), 1000, 4000)
    }

    #[test]
    fn requesting_a_callable_floor_lights_the_button() {
        let mut e = fresh();
        assert!(e.request_floor(3));
        assert!(e.requested_floors.contains(&3));
    }

    #[test]
    fn requesting_an_unknown_floor_is_silently_rejected() {
        let mut e = fresh();
        assert!(!e.request_floor(99));
        assert!(e.requested_floors.is_empty(), "aucun bouton ne doit s'allumer");
    }

    #[test]
    fn requesting_a_hidden_or_inactive_floor_is_silently_rejected() {
        let mut e = ElevatorState::new(
            42,
            0,
            vec![
                FloorSpec { index: 0, hidden: false, inactive: false },
                FloorSpec { index: 1, hidden: true, inactive: false },
                FloorSpec { index: 2, hidden: false, inactive: true },
            ],
            1000,
            4000,
        );
        assert!(!e.request_floor(1));
        assert!(!e.request_floor(2));
        assert!(e.requested_floors.is_empty());
    }

    #[test]
    fn requesting_the_same_floor_twice_is_idempotent() {
        let mut e = fresh();
        e.request_floor(3);
        e.request_floor(3);
        assert_eq!(e.requested_floors.len(), 1, "le spam de bouton ne doit rien faire");
    }

    #[test]
    fn a_request_during_movement_is_accepted_and_queued() {
        // LA différence avec le vanilla, qui refuse toute action pendant un trajet.
        let mut e = fresh();
        e.request_floor(5);
        e.start_trip_if_idle(0, TICK_MS);
        assert_eq!(e.movement_state, MovementState::MovingUp);
        assert!(e.request_floor(2), "un appel pendant le mouvement doit être accepté");
        assert!(e.requested_floors.contains(&2));
    }

    #[test]
    fn starting_a_trip_sets_target_movement_and_both_ticks() {
        let mut e = fresh();
        e.request_floor(3);
        e.start_trip_if_idle(10, TICK_MS);
        assert_eq!(e.target_floor, Some(3));
        assert_eq!(e.movement_state, MovementState::MovingUp);
        assert_eq!(e.depart_tick, Some(10));
        assert_eq!(e.arrival_tick, Some(10 + 100), "1000+4000 ms à 50 ms/tick = 100 ticks");
    }

    #[test]
    fn a_downward_trip_reports_moving_down() {
        let mut e = ElevatorState::new(42, 5, floors_0_to_5(), 1000, 4000);
        e.request_floor(1);
        e.start_trip_if_idle(0, TICK_MS);
        assert_eq!(e.movement_state, MovementState::MovingDown);
        assert_eq!(e.target_floor, Some(1));
    }

    #[test]
    fn a_request_for_the_current_floor_is_served_without_any_movement() {
        let mut e = fresh(); // active_floor = 0
        e.request_floor(0);
        e.start_trip_if_idle(0, TICK_MS);
        assert_eq!(e.movement_state, MovementState::Stopped);
        assert_eq!(e.target_floor, None);
        assert!(e.requested_floors.is_empty(), "l'appel est consommé sur place");
    }

    #[test]
    fn advance_before_the_arrival_tick_changes_nothing() {
        let mut e = fresh();
        e.request_floor(3);
        e.start_trip_if_idle(0, TICK_MS);
        let before = e.clone();
        e.advance(50, TICK_MS);
        assert_eq!(e, before, "à mi-trajet, rien ne doit bouger");
    }

    #[test]
    fn advance_at_the_arrival_tick_completes_the_trip() {
        let mut e = fresh();
        e.request_floor(3);
        e.start_trip_if_idle(0, TICK_MS);
        e.advance(100, TICK_MS);
        assert_eq!(e.active_floor, 3);
        assert_eq!(e.movement_state, MovementState::Stopped);
        assert_eq!(e.target_floor, None);
        assert_eq!(e.depart_tick, None);
        assert_eq!(e.arrival_tick, None);
        assert!(!e.requested_floors.contains(&3), "le bouton s'éteint à l'arrivée");
    }

    #[test]
    fn arrival_happens_without_any_client_reporting_it() {
        // Décision embarquée n°1 : l'arrivée est calculée par le SERVEUR. On n'appelle donc AUCUNE
        // méthode de rapport — seul le temps passe.
        let mut e = fresh();
        e.request_floor(2);
        e.start_trip_if_idle(0, TICK_MS);
        for t in 1..=100 {
            e.advance(t, TICK_MS);
        }
        assert_eq!(e.active_floor, 2);
    }

    #[test]
    fn advance_chains_automatically_to_the_next_call() {
        let mut e = fresh();
        e.request_floor(2);
        e.request_floor(4);
        e.start_trip_if_idle(0, TICK_MS);
        e.advance(100, TICK_MS); // arrivée au 2, puis enchaînement vers le 4
        assert_eq!(e.active_floor, 2);
        assert_eq!(e.target_floor, Some(4), "la cabine repart immédiatement vers l'appel suivant");
        assert_eq!(e.movement_state, MovementState::MovingUp);
    }

    #[test]
    fn advance_reports_whether_the_state_changed() {
        let mut e = fresh();
        e.request_floor(3);
        e.start_trip_if_idle(0, TICK_MS);
        assert!(!e.advance(50, TICK_MS), "mi-trajet : aucun changement");
        assert!(e.advance(100, TICK_MS), "arrivée : changement");
    }

    #[test]
    fn the_same_call_sequence_produces_the_same_state_sequence() {
        // LE test qui garde le contrat : c'est de cette propriété que dépend l'accord entre tous
        // les clients (aucune position de cabine ne circulant, seul le déterminisme les aligne).
        fn run() -> Vec<(i32, Option<i32>, MovementState)> {
            let mut e = fresh();
            let mut trace = Vec::new();
            for tick in 0..400u64 {
                if tick == 5 {
                    e.request_floor(4);
                }
                if tick == 30 {
                    e.request_floor(1);
                }
                if tick == 120 {
                    e.request_floor(5);
                }
                e.advance(tick, TICK_MS);
                trace.push((e.active_floor, e.target_floor, e.movement_state));
            }
            trace
        }
        assert_eq!(run(), run());
    }
}
