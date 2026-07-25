//! Registre de destructibles Classe A (état autoritaire serveur, spec 2026-07-23 §2 + plan
//! 2026-07-25 §2.1). Sibling de `ElevatorRegistry`/`NpcRegistry` — même patron : un `HashMap`
//! indexé par une clé stable (`persistent_id`), peuplé au boot depuis un catalogue, muté par les
//! événements de gameplay.

use crate::destructible_catalog::{DestructibleCatalog, DestructibleKind, DestructibleRecord};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct DestructibleRegistry {
    pub records: HashMap<u64, DestructibleRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageOutcome {
    /// Dégâts appliqués, le device tient encore (Device seulement — un Explosive n'a pas d'état
    /// intermédiaire, spec §2 : "casse au premier hit rangé/explosion, sans PV" pour les feux ;
    /// un Device générique suit sa durability).
    NoChange,
    /// Ce coup a fait passer le destructible à `destroyed = true` (ou `exploded = true` pour un
    /// Explosive) — la transition vient de se produire, à répliquer sur le fil.
    Destroyed,
    /// Déjà détruit avant ce coup — aucune transition, aucun message à envoyer (idempotence).
    AlreadyDestroyed,
    /// `persistent_id` absent du registre — le serveur ne fabrique jamais d'état sur la foi d'un
    /// id inconnu (même garde que partout ailleurs dans ce projet).
    Unknown,
}

impl DestructibleRegistry {
    pub fn from_catalog(catalog: DestructibleCatalog) -> Self {
        let records = catalog
            .into_states()
            .into_iter()
            .map(|r| (r.persistent_id, r))
            .collect();
        Self { records }
    }

    /// Applique `amount` de dégâts à `persistent_id`. Un `Explosive` explose et casse au premier
    /// hit quel que soit `amount` (spec §2 : rayon/chaîne, pas de PV). Un `Device` décrémente sa
    /// `durability` et devient `destroyed` en atteignant 0. Propage aux esclaves déclarés
    /// (`master == Some(persistent_id)`) en les détruisant directement, sans consommer leurs
    /// propres PV — un master détruit détruit tout son groupe (spec §2 : "le serveur propage
    /// lui-même la liste d'esclaves").
    pub fn apply_damage(&mut self, persistent_id: u64, amount: u32) -> DamageOutcome {
        let Some(record) = self.records.get_mut(&persistent_id) else {
            return DamageOutcome::Unknown;
        };
        if record.destroyed {
            return DamageOutcome::AlreadyDestroyed;
        }
        match record.kind {
            DestructibleKind::Explosive => {
                record.destroyed = true;
                record.exploded = true;
            }
            DestructibleKind::Device => {
                record.durability = record.durability.saturating_sub(amount);
                if record.durability == 0 {
                    record.destroyed = true;
                }
            }
        }
        if !record.destroyed {
            return DamageOutcome::NoChange;
        }
        let slave_ids: Vec<u64> = self
            .records
            .iter()
            .filter(|(_, r)| r.master == Some(persistent_id))
            .map(|(id, _)| *id)
            .collect();
        for id in slave_ids {
            if let Some(slave) = self.records.get_mut(&id) {
                slave.destroyed = true;
            }
        }
        DamageOutcome::Destroyed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::destructible_catalog::parse_and_validate;

    fn registry_from(toml: &str) -> DestructibleRegistry {
        DestructibleRegistry::from_catalog(parse_and_validate(toml).unwrap())
    }

    #[test]
    fn damaging_an_unknown_id_returns_unknown() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "x"
               kind = "device""#,
        );
        assert_eq!(reg.apply_damage(999, 50), DamageOutcome::Unknown);
    }

    #[test]
    fn a_device_survives_damage_below_its_durability() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "x"
               kind = "device"
               durability = 100"#,
        );
        assert_eq!(reg.apply_damage(1, 40), DamageOutcome::NoChange);
        assert_eq!(reg.records[&1].durability, 60);
        assert!(!reg.records[&1].destroyed);
    }

    #[test]
    fn a_device_is_destroyed_when_durability_reaches_zero() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "x"
               kind = "device"
               durability = 50"#,
        );
        assert_eq!(reg.apply_damage(1, 50), DamageOutcome::Destroyed);
        assert!(reg.records[&1].destroyed);
    }

    #[test]
    fn overkill_damage_does_not_underflow_durability() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "x"
               kind = "device"
               durability = 10"#,
        );
        assert_eq!(reg.apply_damage(1, 9999), DamageOutcome::Destroyed);
        assert_eq!(reg.records[&1].durability, 0);
    }

    #[test]
    fn an_explosive_is_destroyed_and_exploded_by_any_hit() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "baril"
               kind = "explosive""#,
        );
        assert_eq!(reg.apply_damage(1, 1), DamageOutcome::Destroyed);
        assert!(reg.records[&1].destroyed);
        assert!(reg.records[&1].exploded);
    }

    #[test]
    fn damaging_an_already_destroyed_record_is_idempotent() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "baril"
               kind = "explosive""#,
        );
        assert_eq!(reg.apply_damage(1, 1), DamageOutcome::Destroyed);
        assert_eq!(reg.apply_damage(1, 1), DamageOutcome::AlreadyDestroyed);
    }

    #[test]
    fn destroying_a_master_propagates_to_its_slaves() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "master"
               kind = "device"
               durability = 10
               [[destructible]]
               persistent_id = "2"
               name = "slave"
               kind = "device"
               durability = 999
               master = "1""#,
        );
        assert_eq!(reg.apply_damage(1, 10), DamageOutcome::Destroyed);
        assert!(reg.records[&1].destroyed);
        assert!(
            reg.records[&2].destroyed,
            "un esclave doit être détruit avec son master, sans consommer ses propres PV"
        );
    }

    #[test]
    fn destroying_a_slave_directly_does_not_affect_its_master() {
        let mut reg = registry_from(
            r#"format_version = 1
               [[destructible]]
               persistent_id = "1"
               name = "master"
               kind = "device"
               durability = 999
               [[destructible]]
               persistent_id = "2"
               name = "slave"
               kind = "device"
               durability = 10
               master = "1""#,
        );
        assert_eq!(reg.apply_damage(2, 10), DamageOutcome::Destroyed);
        assert!(reg.records[&2].destroyed);
        assert!(
            !reg.records[&1].destroyed,
            "détruire un esclave ne détruit pas son master"
        );
    }
}
