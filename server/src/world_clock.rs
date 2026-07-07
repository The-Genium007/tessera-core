//! Horloge monde partagée (heure de jeu) — décision déjà actée (roadmap production) : le temps
//! est une ressource **globale**, pas par-shard, pour que tous les joueurs voient la même heure
//! quel que soit le shard où ils se trouvent. Le Gateway porte cette horloge et la diffuse à
//! tous les clients connectés via `ServerMsg::WorldState` (voir `gateway.rs`).
//!
//! La météo n'est PAS gérée ici : c'est une chaîne opaque configurée par l'opérateur (comme
//! l'apparence des personnages, cf. `character-creation-design.md`) — le serveur la relaie telle
//! quelle, sans en connaître le sens. Les noms de record TweakDB réels (ex. "Weather.Sunny01")
//! restent à confirmer en jeu avant que ce soit visible ; une chaîne invalide ne fait
//! qu'échouer proprement côté client (`SetWeather` renvoie `false`, API documentée), aucun
//! risque de plantage.

use std::time::Duration;

/// Heure de jeu, en secondes depuis minuit ([0, 86400)). Stockée en `f64` (pas entier) : les
/// deltas réels par tick (50 ms) sont sub-seconde, un stockage entier perdrait la fraction à
/// chaque appel et l'horloge n'avancerait jamais.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldClock {
    total_seconds: f64,
}

impl WorldClock {
    pub fn new(start_hour: u8, start_minute: u8) -> Self {
        let total_seconds =
            (start_hour as f64 % 24.0) * 3600.0 + (start_minute as f64 % 60.0) * 60.0;
        Self { total_seconds }
    }

    /// Avance l'horloge de `elapsed` (temps réel), à raison de `scale` minutes de jeu par
    /// seconde réelle (scale=1.0 → cycle de 24h en 24 minutes réelles).
    pub fn advance(&mut self, elapsed: Duration, scale: f64) {
        self.total_seconds = (self.total_seconds + elapsed.as_secs_f64() * scale * 60.0) % 86400.0;
    }

    pub fn hour(&self) -> u8 {
        (self.total_seconds / 3600.0).floor() as u8
    }

    pub fn minute(&self) -> u8 {
        ((self.total_seconds / 60.0) % 60.0).floor() as u8
    }

    pub fn second(&self) -> u8 {
        (self.total_seconds % 60.0).floor() as u8
    }

    /// Secondes écoulées depuis minuit — utilisé pour comparer à un rapport client (playtest,
    /// cf. `ClientTimeReport` dans `gateway.rs`) sans avoir à recomposer hour/minute/second à la
    /// main des deux côtés.
    pub fn total_seconds_since_midnight(&self) -> u32 {
        self.total_seconds.floor() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_given_hour_and_minute() {
        let c = WorldClock::new(9, 30);
        assert_eq!(c.hour(), 9);
        assert_eq!(c.minute(), 30);
        assert_eq!(c.second(), 0);
    }

    #[test]
    fn advance_moves_the_clock_forward() {
        let mut c = WorldClock::new(0, 0);
        c.advance(Duration::from_secs(60), 1.0); // 60 minutes de jeu à scale=1.0
        assert_eq!(c.hour(), 1);
        assert_eq!(c.minute(), 0);
    }

    #[test]
    fn advance_wraps_past_midnight() {
        let mut c = WorldClock::new(23, 50);
        c.advance(Duration::from_secs(15), 1.0); // scale=1.0 → 15s réelles = +15 minutes de jeu
        assert_eq!(c.hour(), 0);
        assert_eq!(c.minute(), 5);
    }

    #[test]
    fn small_repeated_advances_accumulate_without_losing_the_fraction() {
        // 100 avances de 10ms à scale=1.0 = 1s de temps réel = 1 minute de jeu — si l'horloge
        // était stockée en entier, chaque appel perdrait sa fraction et rien n'avancerait.
        let mut c = WorldClock::new(0, 0);
        for _ in 0..100 {
            c.advance(Duration::from_millis(10), 1.0);
        }
        assert_eq!(c.hour(), 0);
        assert_eq!(c.minute(), 1);
    }

    #[test]
    fn scale_zero_freezes_the_clock() {
        let mut c = WorldClock::new(12, 0);
        c.advance(Duration::from_secs(3600), 0.0);
        assert_eq!(c.hour(), 12);
        assert_eq!(c.minute(), 0);
    }

    #[test]
    fn second_advances_with_sub_minute_precision() {
        let mut c = WorldClock::new(0, 0);
        // scale=1.0/60.0 → 1 minute de jeu par 60s réelles = temps réel (pas d'accélération) :
        // 30s réelles avancent l'horloge de 30 secondes de jeu, pas de minute entière.
        c.advance(Duration::from_secs(30), 1.0 / 60.0);
        assert_eq!(c.minute(), 0);
        assert_eq!(c.second(), 30);
    }

    #[test]
    fn total_seconds_since_midnight_matches_hour_minute_second() {
        let c = WorldClock::new(1, 1); // 1h01 = 3660s
        assert_eq!(c.total_seconds_since_midnight(), 3660);
    }
}
