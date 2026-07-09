//! Squelette du serveur autoritaire. Constantes de simulation (voir spec §6).

pub mod admin_commands;
pub mod admin_store;
pub mod anticheat;
pub mod framing;
pub mod gateway;
pub mod gateway_routing;
pub mod handoff;
pub mod hot_state_cache;
pub mod internal_net;
pub mod manifest;
pub mod metrics;
pub mod permissions;
pub mod persistence;
pub mod rate_limit;
pub mod server_loop;
pub mod session_log;
pub mod shard;
pub mod snapshot_merge;
pub mod transport;
pub mod world;
pub mod world_clock;

#[cfg(feature = "gns")]
pub mod gns_transport;

pub use shard::shard_main;

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
