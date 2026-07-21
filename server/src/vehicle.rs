//! Véhicule autonome (spec véhicules autonomes §1 : « capsule sur rails de trafic, pas de
//! physique v1 »). Sibling de `NpcRecord` (npc.rs) — PAS une extension d'`EntityBehavior`, dont le
//! garde d'exhaustivité (`behavior_to_u8`) est délibérément piéton-only. Un véhicule porte sa
//! propre vitesse (`EntityState.speed`, spec §3) et son propre lien passager (invariant convoi,
//! spec §4) — jamais dans `Pose`, qui reste le canal cosmétique partagé par TOUT acteur
//! (joueur/PNJ/véhicule), agnostique à ce qui le pilote.

use crate::transport::ClientId;

/// État de mouvement d'un véhicule autonome. Volontairement plus simple que `EntityBehavior`
/// (piéton) — pas de FSM comportemental complexe pour le noyau v1 (spec §8 : annulation/éjection
/// différées, nécessitent des états FSM véhicule qui n'existent pas encore).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VehicleMovementState {
    #[default]
    EnRoute,
    /// Arrêté (borne d'attente hélage, spec §8 — différé au-delà du noyau, mais l'état existe pour
    /// ne pas re-gonfler l'enum plus tard).
    Arrete,
}

/// Enregistrement complet d'un véhicule. `id` dans la plage réservée véhicules (`is_vehicle_id`,
/// disjointe des plages PNJ piétons/nominatifs et des connexions réelles). `passenger` : au plus UN
/// passager en v1 (spec §8 : « logique v1 possiblement mono, owner = 1er monté » — le protocole
/// autorise plusieurs sièges, cf. Task 6, mais ce noyau ne gère qu'un occupant).
#[derive(Debug, Clone)]
pub struct VehicleRecord {
    pub id: ClientId,
    pub archetype: u32,
    pub speed_units_per_sec: f32,
    pub movement: VehicleMovementState,
    /// `None` = personne à bord. `Some(client_id)` = le passager dont la position suit le véhicule
    /// (spec §4, invariant convoi — Task 5 câble le mécanisme réel).
    pub passenger: Option<ClientId>,
}

impl VehicleRecord {
    pub fn new(id: ClientId, archetype: u32, speed_units_per_sec: f32) -> Self {
        Self {
            id,
            archetype,
            speed_units_per_sec,
            movement: VehicleMovementState::default(),
            passenger: None,
        }
    }

    /// Un joueur monte — refuse si déjà occupé (spec §8 : mono-passager v1).
    pub fn mount(&mut self, client_id: ClientId) -> Result<(), VehicleMountError> {
        if self.passenger.is_some() {
            return Err(VehicleMountError::AlreadyOccupied);
        }
        self.passenger = Some(client_id);
        Ok(())
    }

    /// Le passager descend. No-op silencieux si ce n'était pas lui (spec anti-triche implicite :
    /// un joueur ne peut jamais faire descendre le passager d'un autre).
    pub fn unmount(&mut self, client_id: ClientId) {
        if self.passenger == Some(client_id) {
            self.passenger = None;
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum VehicleMountError {
    AlreadyOccupied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_vehicle_starts_en_route_and_empty() {
        let v = VehicleRecord::new(1, 1, 8.0);
        assert_eq!(v.movement, VehicleMovementState::EnRoute);
        assert_eq!(v.passenger, None);
    }

    #[test]
    fn mounting_an_empty_vehicle_succeeds() {
        let mut v = VehicleRecord::new(1, 1, 8.0);
        assert!(v.mount(42).is_ok());
        assert_eq!(v.passenger, Some(42));
    }

    #[test]
    fn mounting_an_occupied_vehicle_fails() {
        let mut v = VehicleRecord::new(1, 1, 8.0);
        v.mount(42).unwrap();
        assert_eq!(v.mount(99), Err(VehicleMountError::AlreadyOccupied));
        assert_eq!(v.passenger, Some(42), "le premier passager reste en place");
    }

    #[test]
    fn unmounting_the_actual_passenger_clears_the_seat() {
        let mut v = VehicleRecord::new(1, 1, 8.0);
        v.mount(42).unwrap();
        v.unmount(42);
        assert_eq!(v.passenger, None);
    }

    #[test]
    fn unmounting_a_different_client_is_a_silent_no_op() {
        let mut v = VehicleRecord::new(1, 1, 8.0);
        v.mount(42).unwrap();
        v.unmount(99); // pas le vrai passager
        assert_eq!(
            v.passenger,
            Some(42),
            "un imposteur ne peut pas éjecter le vrai passager"
        );
    }
}
