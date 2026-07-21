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
