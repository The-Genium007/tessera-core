//! Squelette du serveur autoritaire. Constantes de simulation (voir spec §6).

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
