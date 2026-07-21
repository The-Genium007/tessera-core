//! État de navigation d'un PNJ : le chemin courant (liste de `Waypoint`) et la progression le
//! long de ce chemin. Pur — aucune I/O, aucun accès à `World`. Sibling de `NpcRecord` (FSM), pas
//! fusionné dedans (spec navigation §1 : « le serveur planifie, le client réalise l'anim » — ceci
//! est la partie « le serveur avance la position », dead-reckoning, spec §6).

use crate::nav::Waypoint;
use crate::nav_graph::Vec3;

#[derive(Debug, Clone, Default)]
pub struct NavState {
    path: Vec<Waypoint>,
    /// Index du PROCHAIN waypoint visé (pas encore atteint). `path.len()` = chemin terminé.
    next_index: usize,
}

impl NavState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remplace le chemin courant par un nouveau, repart du premier waypoint. Appelé quand une
    /// brique de déplacement (Task 4) décide d'une nouvelle destination.
    pub fn set_path(&mut self, path: Vec<Waypoint>) {
        self.path = path;
        self.next_index = 0;
    }

    pub fn has_path(&self) -> bool {
        !self.path.is_empty() && self.next_index < self.path.len()
    }

    pub fn has_arrived(&self) -> bool {
        !self.path.is_empty() && self.next_index >= self.path.len()
    }

    /// Avance de `distance` unités le long du chemin depuis `current_position`, en peut-être
    /// franchissant plusieurs waypoints proches en un seul appel (vitesse élevée + waypoints
    /// rapprochés — spec ne l'exclut pas, un tick à 20 Hz avec un PNJ rapide peut couvrir plusieurs
    /// segments courts). Retourne la nouvelle position. Ne fait rien (retourne
    /// `current_position` inchangée) si le chemin est terminé ou vide.
    pub fn advance(&mut self, current_position: Vec3, distance: f32) -> Vec3 {
        let mut position = current_position;
        let mut remaining = distance;
        while remaining > 0.0 && self.next_index < self.path.len() {
            let target = self.path[self.next_index].position;
            let to_target = target_distance(position, target);
            if to_target <= remaining {
                position = target;
                remaining -= to_target;
                self.next_index += 1;
            } else {
                position = lerp_towards(position, target, remaining / to_target.max(f32::EPSILON));
                remaining = 0.0;
            }
        }
        position
    }
}

fn target_distance(a: Vec3, b: Vec3) -> f32 {
    a.distance(&b)
}

fn lerp_towards(from: Vec3, to: Vec3, t: f32) -> Vec3 {
    Vec3::new(
        from.x + (to.x - from.x) * t,
        from.y + (to.y - from.y) * t,
        from.z + (to.z - from.z) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(x: f32, y: f32, z: f32) -> Waypoint {
        Waypoint {
            position: Vec3::new(x, y, z),
        }
    }

    #[test]
    fn a_fresh_nav_state_has_no_path() {
        assert!(!NavState::new().has_path());
        assert!(!NavState::new().has_arrived());
    }

    #[test]
    fn advancing_less_than_the_first_segment_moves_partway_there() {
        let mut nav = NavState::new();
        nav.set_path(vec![wp(10.0, 0.0, 0.0)]);
        let pos = nav.advance(Vec3::new(0.0, 0.0, 0.0), 4.0);
        assert_eq!(pos, Vec3::new(4.0, 0.0, 0.0));
        assert!(!nav.has_arrived(), "4 sur 10 -> pas encore arrivé");
    }

    #[test]
    fn advancing_exactly_the_segment_length_arrives_at_the_waypoint() {
        let mut nav = NavState::new();
        nav.set_path(vec![wp(10.0, 0.0, 0.0)]);
        let pos = nav.advance(Vec3::new(0.0, 0.0, 0.0), 10.0);
        assert_eq!(pos, Vec3::new(10.0, 0.0, 0.0));
        assert!(nav.has_arrived());
    }

    #[test]
    fn advancing_past_one_waypoint_continues_into_the_next_segment_same_call() {
        let mut nav = NavState::new();
        nav.set_path(vec![wp(5.0, 0.0, 0.0), wp(5.0, 5.0, 0.0)]);
        // 8 unités : 5 pour atteindre le 1er waypoint, 3 restantes vers le 2e.
        let pos = nav.advance(Vec3::new(0.0, 0.0, 0.0), 8.0);
        assert_eq!(pos, Vec3::new(5.0, 3.0, 0.0));
        assert!(!nav.has_arrived());
    }

    #[test]
    fn advancing_beyond_the_full_path_stops_at_the_last_waypoint() {
        let mut nav = NavState::new();
        nav.set_path(vec![wp(5.0, 0.0, 0.0)]);
        let pos = nav.advance(Vec3::new(0.0, 0.0, 0.0), 1000.0);
        assert_eq!(pos, Vec3::new(5.0, 0.0, 0.0));
        assert!(nav.has_arrived());
    }

    #[test]
    fn advancing_an_empty_path_never_moves() {
        let mut nav = NavState::new();
        let pos = nav.advance(Vec3::new(1.0, 2.0, 3.0), 100.0);
        assert_eq!(pos, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn set_path_resets_progression_even_if_a_previous_path_had_arrived() {
        let mut nav = NavState::new();
        nav.set_path(vec![wp(1.0, 0.0, 0.0)]);
        nav.advance(Vec3::new(0.0, 0.0, 0.0), 100.0);
        assert!(nav.has_arrived());
        nav.set_path(vec![wp(2.0, 0.0, 0.0)]);
        assert!(!nav.has_arrived());
        assert!(nav.has_path());
    }
}
