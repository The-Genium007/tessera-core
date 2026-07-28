//! Squelette du serveur autoritaire. Constantes de simulation (voir spec §6).
//!
//! Réalign de base 0.1.6 (2026-07-19) : bump volontaire (aucun changement fonctionnel) pour
//! forcer un rebuild serveur frais à la base 0.1.6, condition du promote playtest du désossage
//! (dev était resté en base 0.1.5 sous playtest 0.1.6 ; cf. release/promote-desossage-playtest-0.1.6).

pub mod admin_commands;
pub mod admin_store;
pub mod anticheat;
pub mod appearance_relay;
pub mod attestation_display;
pub mod ban_store;
pub mod character_migration;
pub mod character_store;
pub mod crowd_producer;
pub mod degradation;
pub mod elevator;
pub mod elevator_catalog;
pub mod escalade_police;
pub mod frame;
pub mod framing;
pub mod game_hash;
pub mod gateway;
pub mod gateway_routing;
pub mod handoff;
pub mod hostile_telemetry;
pub mod hot_state_cache;
pub mod interaction_session;
pub mod internal_attestation_http;
pub mod internal_net;
pub mod jwks;
pub mod maintenance;
pub mod manifest;
pub mod metrics;
pub mod named_npc_catalog;
pub mod named_npc_registry;
pub mod nav;
pub mod nav_graph;
pub mod nav_state;
pub mod npc;
pub mod npc_catalog;
pub mod npc_tags;
pub mod permissions;
pub mod persistence;
pub mod player_store_impl;
pub mod population_director;
pub mod postgres_store;
pub mod puppet_catalog;
pub mod quant;
pub mod queue;
pub mod rate_limit;
pub mod server_loop;
pub mod session_log;
pub mod session_log_html;
pub mod shard;
pub mod shard_boundary_bridge;
pub mod shutdown;
pub mod snapshot_merge;
pub mod stub;
pub mod transport;
pub mod vehicle;
pub mod world;
pub mod world_clock;
pub mod write_behind;
pub mod write_behind_journal;

#[cfg(feature = "gns")]
pub mod gns_transport;

pub use shard::{shard_main, VehicleShardSeed};

/// Fréquence de tick par défaut de la simulation, en Hz (spec §6 : 20–60 ticks/s).
pub fn default_tick_rate_hz() -> u32 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tick_rate_is_20hz() {
        assert_eq!(default_tick_rate_hz(), 20);
    }
}
