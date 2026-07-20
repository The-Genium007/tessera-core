//! Registre des PNJ NOMINATIFS runtime (fondation d'interaction §6) : assigne un id `ClientId` à
//! chaque entrée du manifeste au boot, et tient le mapping id-manifeste (stable, éternel) ↔ id
//! runtime (éphémère, réattribué à chaque redémarrage). Partage la MÊME plage réservée que la
//! foule anonyme (`world::NPC_ID_RANGE_START`) mais depuis l'EXTRÉMITÉ HAUTE de la plage (compte à
//! rebours depuis `u64::MAX`) pour ne jamais collisionner avec le compteur croissant de
//! `NpcRegistry` (fondation PNJ, qui compte depuis `NPC_ID_RANGE_START` vers le haut) — les deux
//! registres partagent l'espace `is_npc_id`, mais chacun avance dans une direction opposée.

use crate::named_npc_catalog::NamedNpcCatalog;
use crate::transport::ClientId;

#[derive(Debug)]
pub struct NamedNpcRegistry {
    manifest_to_runtime: std::collections::HashMap<String, ClientId>,
    runtime_to_manifest: std::collections::HashMap<ClientId, String>,
}

impl NamedNpcRegistry {
    /// Assigne un id runtime à chaque entrée du catalogue, en comptant à rebours depuis
    /// `u64::MAX` — garantit qu'aucun PNJ nominatif ne collisionne avec un PNJ de foule (qui
    /// compte depuis `NPC_ID_RANGE_START` vers le haut) tant que le nombre total de PNJ des deux
    /// registres reste très inférieur à l'écart entre les deux extrémités (2^64 - 2^48, marge
    /// gigantesque pour toute échelle réaliste).
    pub fn from_catalog(catalog: &NamedNpcCatalog) -> Self {
        let mut manifest_to_runtime = std::collections::HashMap::new();
        let mut runtime_to_manifest = std::collections::HashMap::new();
        let mut next_id = u64::MAX;
        let mut ids: Vec<&str> = catalog.ids();
        ids.sort(); // ordre déterministe (HashMap::keys() n'a pas d'ordre stable)
        for manifest_id in ids {
            let runtime_id = next_id;
            next_id -= 1;
            manifest_to_runtime.insert(manifest_id.to_string(), runtime_id);
            runtime_to_manifest.insert(runtime_id, manifest_id.to_string());
        }
        Self {
            manifest_to_runtime,
            runtime_to_manifest,
        }
    }

    pub fn runtime_id_of(&self, manifest_id: &str) -> Option<ClientId> {
        self.manifest_to_runtime.get(manifest_id).copied()
    }

    pub fn manifest_id_of(&self, runtime_id: ClientId) -> Option<&str> {
        self.runtime_to_manifest.get(&runtime_id).map(String::as_str)
    }

    pub fn runtime_ids(&self) -> Vec<ClientId> {
        self.runtime_to_manifest.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::named_npc_catalog::parse_and_validate;

    fn two_entry_catalog() -> NamedNpcCatalog {
        parse_and_validate(
            r#"
            format_version = 1
            [[pnj]]
            id = "ripperdoc-watson-01"
            archetype = "a"
            position = [0.0, 0.0, 0.0]
            briques = ["rester-statique"]
            [[pnj]]
            id = "fixer-corpo-plaza-01"
            archetype = "b"
            position = [0.0, 0.0, 0.0]
            briques = ["rester-statique"]
            "#,
        )
        .unwrap()
    }

    #[test]
    fn every_manifest_entry_gets_a_distinct_runtime_id() {
        let reg = NamedNpcRegistry::from_catalog(&two_entry_catalog());
        let a = reg.runtime_id_of("ripperdoc-watson-01").unwrap();
        let b = reg.runtime_id_of("fixer-corpo-plaza-01").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn runtime_ids_are_reachable_backward_from_the_manifest_id() {
        let reg = NamedNpcRegistry::from_catalog(&two_entry_catalog());
        let runtime_id = reg.runtime_id_of("ripperdoc-watson-01").unwrap();
        assert_eq!(reg.manifest_id_of(runtime_id), Some("ripperdoc-watson-01"));
    }

    #[test]
    fn unknown_manifest_id_returns_none() {
        let reg = NamedNpcRegistry::from_catalog(&two_entry_catalog());
        assert!(reg.runtime_id_of("does-not-exist").is_none());
    }

    #[test]
    fn assigned_runtime_ids_never_collide_with_the_crowd_range_start() {
        // La plage foule compte depuis NPC_ID_RANGE_START (bas) vers le haut ; ce registre compte
        // depuis u64::MAX (haut) vers le bas — à toute échelle réaliste, les deux ne se
        // rencontrent jamais. Ce test verrouille juste que les ids assignés restent bien
        // au-dessus de NPC_ID_RANGE_START (dans la plage PNJ), jamais dans la plage joueurs réels.
        let reg = NamedNpcRegistry::from_catalog(&two_entry_catalog());
        for id in reg.runtime_ids() {
            assert!(crate::world::is_npc_id(id));
        }
    }

    #[test]
    fn an_empty_catalog_produces_an_empty_registry() {
        let empty = parse_and_validate("format_version = 1\npnj = []\n").unwrap();
        let reg = NamedNpcRegistry::from_catalog(&empty);
        assert!(reg.runtime_ids().is_empty());
    }
}
