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

    /// Inverse exact de `world_position` : convertit une position MONDE en offset local dans le
    /// repère `frame`. `Some(world)` tel quel si `frame == WORLD_FRAME`, `None` si le repère est
    /// inconnu — même règle de repli, pour la même raison (mieux vaut un appelant qui sait qu'il ne
    /// peut pas convertir qu'un offset silencieusement faux).
    ///
    /// C'est la moitié manquante du modèle de repère. Sans elle, **monter** dans une cabine posait
    /// `Pose.frame` en laissant `x/y/z` en coordonnées monde : l'offset local valait alors la
    /// position monde du joueur, soit des centaines d'unités hors de la cabine. Et **descendre**
    /// souffrait du défaut symétrique (un offset local relu comme une position monde, donc un
    /// joueur téléporté près de l'origine). Les deux bascules doivent convertir, pas seulement
    /// changer d'étiquette.
    pub fn local_offset(&self, frame: FrameId, world_position: [f32; 3]) -> Option<[f32; 3]> {
        if frame == WORLD_FRAME {
            return Some(world_position);
        }
        let transform = self.transform_of(frame)?;
        let dx = world_position[0] - transform.position[0];
        let dy = world_position[1] - transform.position[1];
        // Rotation inverse : -yaw. `sin(-a) = -sin(a)`, `cos(-a) = cos(a)` — on réutilise donc le
        // même couple sin/cos que l'aller, avec le signe du sinus inversé, plutôt que de rappeler
        // `sin_cos` sur `-yaw` (bit-à-bit identique, et l'intention se lit).
        let (sin, cos) = transform.yaw.sin_cos();
        Some([
            dx * cos + dy * sin,
            -dx * sin + dy * cos,
            world_position[2] - transform.position[2],
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

    #[test]
    fn local_offset_of_the_world_frame_is_the_world_position_itself() {
        let reg = FrameRegistry::new();
        assert_eq!(
            reg.local_offset(WORLD_FRAME, [12.0, -3.0, 4.5]),
            Some([12.0, -3.0, 4.5])
        );
    }

    #[test]
    fn local_offset_of_an_unknown_frame_is_none() {
        let reg = FrameRegistry::new();
        assert_eq!(reg.local_offset(42, [1.0, 2.0, 3.0]), None);
    }

    #[test]
    fn local_offset_subtracts_the_frame_position_when_there_is_no_rotation() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            7,
            FrameTransform {
                position: [100.0, 200.0, 30.0],
                yaw: 0.0,
            },
        );
        let offset = reg.local_offset(7, [101.0, 198.0, 31.5]).unwrap();
        assert_eq!(offset, [1.0, -2.0, 1.5]);
    }

    /// La propriete qui compte vraiment : les deux sens sont exactement inverses. Un joueur qui
    /// monte puis descend sans bouger doit retrouver sa position monde d'origine — sinon chaque
    /// montee/descente le deplacerait un peu, et l'anti-triche verrait une teleportation.
    #[test]
    fn local_offset_and_world_position_are_exact_inverses_even_with_rotation() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            9,
            FrameTransform {
                position: [-988.7, 2834.1, 30.05],
                yaw: 0.9,
            },
        );
        let world = [-985.2, 2836.4, 31.7];
        let offset = reg.local_offset(9, world).unwrap();
        let back = reg.world_position(9, offset).unwrap();
        for i in 0..3 {
            assert!(
                (back[i] - world[i]).abs() < 1e-3,
                "axe {i} : aller-retour {} -> {} (offset {})",
                world[i],
                back[i],
                offset[i]
            );
        }
    }

    /// Et l'aller-retour dans l'autre ordre : un offset local pose dans la cabine doit survivre a
    /// une resolution monde puis a une reconversion.
    #[test]
    fn world_position_then_local_offset_returns_the_original_offset() {
        let mut reg = FrameRegistry::new();
        reg.set_transform(
            9,
            FrameTransform {
                position: [10.0, -20.0, 3.0],
                yaw: -2.1,
            },
        );
        let offset = [0.4, -1.2, 0.0];
        let world = reg.world_position(9, offset).unwrap();
        let back = reg.local_offset(9, world).unwrap();
        for i in 0..3 {
            assert!((back[i] - offset[i]).abs() < 1e-4, "axe {i}");
        }
    }
}
