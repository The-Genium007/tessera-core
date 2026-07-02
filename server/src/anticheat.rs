//! Validation de cohérence des positions reçues des clients : rejette les vitesses/téléports
//! physiquement impossibles avant qu'ils n'atteignent un Shard. Portée volontairement minimale
//! (détecter l'impossible, pas un système anti-triche complet — voir spec B, hors périmètre).

use std::time::Duration;

/// Vitesse maximale plausible, en mètres/seconde (sprint + véhicule + marge). Valeur de départ,
/// ajustable une fois du vrai playtest disponible — pas une valeur définitive de gameplay.
pub const MAX_PLAYER_SPEED_MPS: f32 = 60.0;

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
}
