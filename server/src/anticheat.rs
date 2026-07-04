//! Validation de cohérence des positions reçues des clients : rejette les vitesses/téléports
//! physiquement impossibles avant qu'ils n'atteignent un Shard. Portée volontairement minimale
//! (détecter l'impossible, pas un système anti-triche complet — voir spec B, hors périmètre).

use std::time::Duration;

/// Vitesse maximale plausible, en mètres/seconde (sprint + véhicule + marge). Valeur de départ,
/// ajustable une fois du vrai playtest disponible — pas une valeur définitive de gameplay.
pub const MAX_PLAYER_SPEED_MPS: f32 = 60.0;

/// Fenêtre maximale prise en compte pour le calcul de vitesse (voir `cap_elapsed`).
pub const MAX_ELAPSED_WINDOW: Duration = Duration::from_secs(2);

/// Plafonne `elapsed` à `max_window` avant de le passer à `is_plausible_move`. Sans ce plafond,
/// un client silencieux longtemps (perte réseau ou triche) accumule un `elapsed` énorme qui
/// "légalise" un grand saut : 3600 m en 60 s passe pile sous le seuil de 60 m/s. En plafonnant
/// la fenêtre, la même distance mesurée sur au plus `max_window` dépasse largement le seuil.
pub fn cap_elapsed(elapsed: Duration, max_window: Duration) -> Duration {
    elapsed.min(max_window)
}

/// Vrai si le déplacement de `prev` à `next` en `elapsed` est plausible à `max_speed_mps` près.
/// `elapsed == Duration::ZERO` est toujours plausible : pas assez d'information pour juger
/// (couvre la 1re position reçue après un `Join`, qui n'a pas de référence temporelle).
pub fn is_plausible_move(
    prev: [f32; 3],
    next: [f32; 3],
    elapsed: Duration,
    max_speed_mps: f32,
) -> bool {
    if elapsed.is_zero() {
        return true;
    }
    let dx = next[0] - prev[0];
    let dy = next[1] - prev[1];
    let dz = next[2] - prev[2];
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    let speed = dist / elapsed.as_secs_f32();
    speed <= max_speed_mps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plausible_walk_is_accepted() {
        // 1 m en 50 ms = 20 m/s, sous le seuil.
        assert!(is_plausible_move(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            Duration::from_millis(50),
            MAX_PLAYER_SPEED_MPS
        ));
    }

    #[test]
    fn an_impossible_teleport_is_rejected() {
        // 10 000 m en 50 ms — bien au-delà de tout mode de déplacement plausible.
        assert!(!is_plausible_move(
            [0.0, 0.0, 0.0],
            [10_000.0, 0.0, 0.0],
            Duration::from_millis(50),
            MAX_PLAYER_SPEED_MPS
        ));
    }

    #[test]
    fn staying_still_is_always_accepted() {
        assert!(is_plausible_move(
            [5.0, 5.0, 5.0],
            [5.0, 5.0, 5.0],
            Duration::from_secs(10),
            MAX_PLAYER_SPEED_MPS
        ));
    }

    #[test]
    fn zero_elapsed_time_is_always_accepted() {
        // Pas assez d'information pour juger (et évite une division par zéro) — couvre
        // notamment la toute 1re position reçue après un Join, sans référence temporelle.
        assert!(is_plausible_move(
            [0.0, 0.0, 0.0],
            [10_000.0, 0.0, 0.0],
            Duration::ZERO,
            MAX_PLAYER_SPEED_MPS
        ));
    }

    #[test]
    fn exactly_at_the_speed_limit_is_accepted() {
        // 60 m en 1 s = exactement MAX_PLAYER_SPEED_MPS — limite inclusive.
        assert!(is_plausible_move(
            [0.0, 0.0, 0.0],
            [MAX_PLAYER_SPEED_MPS, 0.0, 0.0],
            Duration::from_secs(1),
            MAX_PLAYER_SPEED_MPS
        ));
    }

    #[test]
    fn cap_elapsed_leaves_a_short_duration_untouched() {
        assert_eq!(
            cap_elapsed(Duration::from_millis(50), MAX_ELAPSED_WINDOW),
            Duration::from_millis(50)
        );
    }

    #[test]
    fn cap_elapsed_clamps_a_long_silence_to_the_window() {
        assert_eq!(
            cap_elapsed(Duration::from_secs(60), MAX_ELAPSED_WINDOW),
            MAX_ELAPSED_WINDOW
        );
    }

    #[test]
    fn silence_then_teleport_is_rejected_once_elapsed_is_capped() {
        // Bug audit prod 2026-07-03 §5.4 : `last_pos_at` n'avance que sur position acceptée —
        // un client silencieux 60s peut ensuite sauter 3600 m et se faire accepter, puisque
        // 3600 m / 60 s = exactement MAX_PLAYER_SPEED_MPS (60 m/s), la limite inclusive.
        let uncapped_elapsed = Duration::from_secs(60);
        assert!(
            is_plausible_move(
                [0.0, 0.0, 0.0],
                [3600.0, 0.0, 0.0],
                uncapped_elapsed,
                MAX_PLAYER_SPEED_MPS
            ),
            "précondition : sans plafond, ce saut est accepté (c'est le bug)"
        );

        // Avec la fenêtre plafonnée (cf. gateway.rs, appelé sur CHAQUE PositionUpdate) : le même
        // saut, mesuré sur au plus MAX_ELAPSED_WINDOW, donne une vitesse largement au-dessus du
        // seuil → rejeté.
        let capped_elapsed = cap_elapsed(uncapped_elapsed, MAX_ELAPSED_WINDOW);
        assert!(!is_plausible_move(
            [0.0, 0.0, 0.0],
            [3600.0, 0.0, 0.0],
            capped_elapsed,
            MAX_PLAYER_SPEED_MPS
        ));
    }
}
