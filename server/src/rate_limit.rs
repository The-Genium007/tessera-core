//! Limite le débit de messages par client, au niveau du Gateway : sans ça, un client peut
//! inonder le Gateway (chaque `PositionUpdate` déclenche `locate()` + des écritures shards —
//! amplification gratuite vers l'interne, audit prod 2026-07-03 §5.4). Pur et testable sans
//! horloge réelle (fonctionne sur des `Instant` construits en test via `Instant::now() + Duration`).

use std::time::{Duration, Instant};

/// ~2× la cadence de tick (20 Hz) : laisse une bonne marge au trafic légitime (position +
/// occasionnels Join/autres) sans permettre l'amplification vers l'interne.
pub const DEFAULT_LIMIT_PER_WINDOW: u32 = 40;

/// Nombre de fenêtres (secondes) consécutives en dépassement avant de considérer le flood
/// "soutenu" et de kicker — un pic isolé d'1-2s ne doit pas suffire.
pub const DEFAULT_KICK_AFTER_WINDOWS: u32 = 3;

/// État de fenêtre glissante pour un client : compteur de la fenêtre en cours + nombre de
/// fenêtres consécutives ayant dépassé la limite (mesure le caractère "soutenu" du flood).
pub struct RateLimitState {
    window_start: Instant,
    count_in_window: u32,
    consecutive_over_limit_windows: u32,
}

impl RateLimitState {
    pub fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            count_in_window: 0,
            consecutive_over_limit_windows: 0,
        }
    }
}

/// Décision à appliquer au message qui vient d'être compté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    /// Sous la limite : traiter normalement.
    Accept,
    /// Au-dessus de la limite pour cette fenêtre, mais pas encore "soutenu" : ignorer ce message.
    Drop,
    /// Dépassement soutenu sur plusieurs fenêtres consécutives : déconnecter le client.
    Kick,
}

/// Compte un message reçu à `now` et renvoie la décision. `limit_per_window` messages max par
/// fenêtre d'1 seconde ; au-delà de `kick_after_windows` fenêtres consécutives en dépassement,
/// `Kick`. Une fenêtre qui repasse sous la limite remet le compteur de "soutenu" à zéro — un pic
/// isolé ne doit pas condamner un client à retardement.
pub fn check_rate_limit(
    state: &mut RateLimitState,
    now: Instant,
    limit_per_window: u32,
    kick_after_windows: u32,
) -> RateDecision {
    if now.duration_since(state.window_start) >= Duration::from_secs(1) {
        let previous_window_was_over_limit = state.count_in_window > limit_per_window;
        state.window_start = now;
        state.count_in_window = 0;
        state.consecutive_over_limit_windows = if previous_window_was_over_limit {
            state.consecutive_over_limit_windows + 1
        } else {
            0
        };
    }

    state.count_in_window += 1;

    if state.count_in_window > limit_per_window {
        // Cette fenêtre est (encore) en dépassement. Si elle porte le compte à
        // `kick_after_windows` consécutives, c'est un flood soutenu → kick immédiatement,
        // sans attendre le rollover de la fenêtre suivante pour le constater a posteriori.
        if state.consecutive_over_limit_windows + 1 >= kick_after_windows {
            return RateDecision::Kick;
        }
        return RateDecision::Drop;
    }
    RateDecision::Accept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_under_the_limit_are_accepted() {
        let t0 = Instant::now();
        let mut state = RateLimitState::new(t0);
        for _ in 0..40 {
            assert_eq!(
                check_rate_limit(&mut state, t0, 40, 3),
                RateDecision::Accept
            );
        }
    }

    #[test]
    fn messages_beyond_the_limit_in_the_same_window_are_dropped() {
        let t0 = Instant::now();
        let mut state = RateLimitState::new(t0);
        for _ in 0..40 {
            check_rate_limit(&mut state, t0, 40, 3);
        }
        assert_eq!(check_rate_limit(&mut state, t0, 40, 3), RateDecision::Drop);
    }

    #[test]
    fn a_single_over_limit_window_does_not_kick() {
        let t0 = Instant::now();
        let mut state = RateLimitState::new(t0);
        for _ in 0..50 {
            check_rate_limit(&mut state, t0, 40, 3);
        }
        // Nouvelle fenêtre, sous la limite : pas de kick après une seule fenêtre en dépassement.
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(
            check_rate_limit(&mut state, t1, 40, 3),
            RateDecision::Accept
        );
    }

    #[test]
    fn sustained_flooding_across_consecutive_windows_kicks() {
        let t0 = Instant::now();
        let mut state = RateLimitState::new(t0);
        let mut last_decision = RateDecision::Accept;
        // 3 fenêtres consécutives, chacune bien au-dessus de la limite.
        for window in 0..3 {
            let t = t0 + Duration::from_secs(window);
            for _ in 0..50 {
                last_decision = check_rate_limit(&mut state, t, 40, 3);
            }
        }
        assert_eq!(last_decision, RateDecision::Kick);
    }

    #[test]
    fn a_clean_window_resets_the_sustained_counter() {
        let t0 = Instant::now();
        let mut state = RateLimitState::new(t0);
        // 2 fenêtres en dépassement...
        for window in 0..2 {
            let t = t0 + Duration::from_secs(window);
            for _ in 0..50 {
                check_rate_limit(&mut state, t, 40, 3);
            }
        }
        // ... puis une fenêtre propre : le compteur de "soutenu" doit repartir à zéro.
        let t_clean = t0 + Duration::from_secs(2);
        check_rate_limit(&mut state, t_clean, 40, 3);
        // Une nouvelle salve en dépassement ne doit PAS suffire à kicker immédiatement après.
        let t_next = t0 + Duration::from_secs(3);
        let mut last_decision = RateDecision::Accept;
        for _ in 0..50 {
            last_decision = check_rate_limit(&mut state, t_next, 40, 3);
        }
        assert_eq!(last_decision, RateDecision::Drop);
    }
}
