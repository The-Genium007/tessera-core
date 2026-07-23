//! Modèle de repère serveur (ADR 0013) : une position de joueur PORTÉ (ascenseur, plus tard
//! véhicule/moto) n'est jamais une position monde brute — c'est un `(repère, offset local)`.
//! PUR — aucune I/O, comme `elevator.rs`/`anticheat.rs` : entièrement testable sans lancer le jeu.

use std::collections::HashMap;

/// Identifiant de repère. `WORLD_FRAME` = le monde (l'offset EST alors la position monde). Un
/// repère non-nul est la clé `EntityID` u64 de l'objet porteur (cabine d'ascenseur aujourd'hui —
/// cohérent avec `elevator_id:ulong` déjà sur le fil, ADR 0012 §2.5 ; véhicule/moto plus tard).
pub type FrameId = u64;

pub const WORLD_FRAME: FrameId = 0;

/// Transformée courante d'un repère mobile dans le monde : sa position et son orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTransform {
    pub position: [f32; 3],
    pub yaw: f32,
}

/// Table des repères mobiles actuellement actifs. Ne contient JAMAIS `WORLD_FRAME` : le monde est
/// l'origine implicite, pas une entrée de la table.
#[derive(Default)]
pub struct FrameRegistry {
    transforms: HashMap<FrameId, FrameTransform>,
}

impl FrameRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pose ou remplace la transformée d'un repère mobile. Panique sur `WORLD_FRAME` : le monde
    /// n'a pas de transformée à poser, c'est l'origine implicite (Global Constraints).
    pub fn set_transform(&mut self, frame: FrameId, transform: FrameTransform) {
        assert_ne!(
            frame, WORLD_FRAME,
            "WORLD_FRAME n'a pas de transformée : c'est l'origine implicite"
        );
        self.transforms.insert(frame, transform);
    }

    /// Retire un repère de la table. No-op silencieux si absent — un repère qui disparaît
    /// (cabine détruite, cas futur) ne doit jamais paniquer un appelant qui ignore son état exact.
    pub fn remove_transform(&mut self, frame: FrameId) {
        self.transforms.remove(&frame);
    }

    pub fn transform_of(&self, frame: FrameId) -> Option<FrameTransform> {
        self.transforms.get(&frame).copied()
    }

    /// Résout un offset local dans le repère `frame` en position monde. `Some(local_offset)` tel
    /// quel si `frame == WORLD_FRAME` (l'offset EST la position monde). Sinon compose la
    /// transformée du repère (rotation par son yaw, puis translation) — `None` si le repère est
    /// inconnu (règle de repli explicite, ADR 0013 § Négatives : mieux vaut un appelant qui sait
    /// qu'il ne peut pas résoudre plutôt qu'une position silencieusement fausse).
    pub fn world_position(&self, frame: FrameId, local_offset: [f32; 3]) -> Option<[f32; 3]> {
        if frame == WORLD_FRAME {
            return Some(local_offset);
        }
        let transform = self.transform_of(frame)?;
        let (sin, cos) = transform.yaw.sin_cos();
        let rotated_x = local_offset[0] * cos - local_offset[1] * sin;
        let rotated_y = local_offset[0] * sin + local_offset[1] * cos;
        Some([
            transform.position[0] + rotated_x,
            transform.position[1] + rotated_y,
            transform.position[2] + local_offset[2],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_frame_offset_resolves_to_itself() {
        let reg = FrameRegistry::new();
        let world_pos = reg.world_position(WORLD_FRAME, [5.0, 6.0, 7.0]);
        assert_eq!(world_pos, Some([5.0, 6.0, 7.0]));
    }

    #[test]
    fn an_unknown_frame_resolves_to_none() {
        let reg = FrameRegistry::new();
        assert_eq!(reg.world_position(42, [0.0, 0.0, 0.0]), None);
    }

    #[test]
    fn a_known_frame_with_zero_yaw_translates_the_offset() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            7,
            FrameTransform {
                position: [100.0, 200.0, 30.0],
                yaw: 0.0,
            },
        );
        let world_pos = reg.world_position(7, [1.0, 2.0, 0.0]);
        assert_eq!(world_pos, Some([101.0, 202.0, 30.0]));
    }

    #[test]
    fn a_known_frame_with_a_quarter_turn_rotates_the_offset() {
        // yaw = 90° : l'axe local +x pointe vers le monde +y (convention main droite, z vertical).
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            7,
            FrameTransform {
                position: [0.0, 0.0, 0.0],
                yaw: 90.0_f32.to_radians(),
            },
        );
        let world_pos = reg.world_position(7, [1.0, 0.0, 0.0]).unwrap();
        assert!(
            (world_pos[0] - 0.0).abs() < 1e-4,
            "x attendu ~0, obtenu {}",
            world_pos[0]
        );
        assert!(
            (world_pos[1] - 1.0).abs() < 1e-4,
            "y attendu ~1, obtenu {}",
            world_pos[1]
        );
    }

    #[test]
    fn removing_a_frame_makes_it_resolve_to_none_again() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            7,
            FrameTransform {
                position: [1.0, 1.0, 1.0],
                yaw: 0.0,
            },
        );
        reg.remove_transform(7);
        assert_eq!(reg.world_position(7, [0.0, 0.0, 0.0]), None);
    }

    #[test]
    fn removing_an_unknown_frame_is_a_silent_no_op() {
        let mut reg = FrameRegistry::new();
        reg.remove_transform(999); // ne doit pas paniquer
        assert_eq!(reg.world_position(999, [0.0, 0.0, 0.0]), None);
    }

    #[test]
    fn updating_a_frame_transform_overwrites_the_previous_one() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            7,
            FrameTransform {
                position: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
        );
        reg.set_transform(
            7,
            FrameTransform {
                position: [10.0, 0.0, 0.0],
                yaw: 0.0,
            },
        );
        assert_eq!(
            reg.world_position(7, [0.0, 0.0, 0.0]),
            Some([10.0, 0.0, 0.0])
        );
    }

    #[test]
    #[should_panic]
    fn setting_a_transform_on_the_world_frame_panics() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            WORLD_FRAME,
            FrameTransform {
                position: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
        );
    }

    #[test]
    fn transform_of_returns_the_current_transform() {
        let mut reg = FrameRegistry::new();
        let t = FrameTransform {
            position: [3.0, 4.0, 5.0],
            yaw: 1.5,
        };
        reg.set_transform(7, t);
        assert_eq!(reg.transform_of(7), Some(t));
    }

    #[test]
    fn transform_of_an_unknown_frame_is_none() {
        let reg = FrameRegistry::new();
        assert_eq!(reg.transform_of(999), None);
    }
}
