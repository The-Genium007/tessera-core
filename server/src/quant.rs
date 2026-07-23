//! Quantization du fil (gel palier 2, 2026-07-23) — conversions entre le cœur serveur (f32) et
//! la représentation gelée du protocole (`QVec3` fixed-point + yaw `u16`).
//!
//! Décision d'implémentation (gel §5 laissait le choix) : le CŒUR reste en f32 (`Pose`, anticheat,
//! nav, interpolation — inchangés), la conversion se fait AU BORD (encode/décode des messages,
//! `gateway_routing.rs` / `server_loop.rs` / `snapshot_merge.rs` / `gateway.rs`). Un seul module
//! porte les constantes et les fonctions — jamais de `* 131072.0` inline ailleurs.
//!
//! Position : fixed-point WORLD-ABSOLU, mètres = bits / 131072 (2^17) — exactement la
//! représentation native du jeu (spike navmesh) et des données nav extraites. Grille absolue :
//! une position s'encode bit-à-bit identiquement quel que soit le shard (zéro décohérence au
//! handoff). Précision ~0,0076 mm. Plage : ±2^31/2^17 = ±16384 m — couvre Night City entière
//! (le monde tient dans ±4 km autour de l'origine).
//!
//! Yaw : u16, 0..65535 = 0..360° (~0,0055°/cran). Le yaw du jeu est en degrés, potentiellement
//! négatif ou ≥360 — normalisé dans [0, 360) avant quantization.

/// Facteur fixed-point position : 2^17 bits par mètre (représentation native du jeu).
pub const POS_SCALE: f32 = 131072.0;

/// Facteur yaw : crans par degré (65536 crans pour 360°).
const YAW_SCALE: f32 = 65536.0 / 360.0;

/// Quantifie une coordonnée monde (mètres, f32) en fixed-point i32.
#[inline]
pub fn q_pos(meters: f32) -> i32 {
    (meters * POS_SCALE).round() as i32
}

/// Déquantifie une coordonnée fixed-point i32 en mètres f32.
#[inline]
pub fn dq_pos(bits: i32) -> f32 {
    bits as f32 / POS_SCALE
}

/// Quantifie un yaw en degrés (quelconque, y compris négatif) en u16 (0..65535 = 0..360°).
#[inline]
pub fn q_yaw(degrees: f32) -> u16 {
    let normalized = degrees.rem_euclid(360.0);
    // rem_euclid garantit [0, 360) ; *YAW_SCALE donne [0, 65536) ; round peut atteindre 65536
    // exactement (359.9973°+) → modulo pour rester en u16 (65536 ≡ 0°, même angle).
    ((normalized * YAW_SCALE).round() as u32 % 65536) as u16
}

/// Déquantifie un yaw u16 en degrés f32 dans [0, 360).
#[inline]
pub fn dq_yaw(q: u16) -> f32 {
    q as f32 / YAW_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_meters_round_trip_exactly() {
        // Les coordonnées entières (et tout multiple de 2^-17) sont représentables exactement —
        // le round-trip est sans perte, pas « à epsilon près ».
        for m in [-4096.0f32, -1295.0, 0.0, 63.0, 2387.0, 4096.0] {
            assert_eq!(dq_pos(q_pos(m)), m);
        }
    }

    #[test]
    fn precision_is_within_one_quantum() {
        let m = 1234.567_89_f32;
        let err = (dq_pos(q_pos(m)) - m).abs();
        assert!(err <= 0.5 / POS_SCALE, "erreur {err} > demi-quantum");
    }

    #[test]
    fn world_extent_fits_in_i32() {
        // Night City tient dans ±16384 m — la borne i32 du fixed-point 2^17.
        assert_eq!(q_pos(16383.0), 16383 * 131072);
        assert_eq!(q_pos(-16383.0), -16383 * 131072);
    }

    #[test]
    fn yaw_quarter_turns_are_exact() {
        // 65536/4 = 16384 crans par quart de tour : 0/90/180/270° sont représentables exactement.
        assert_eq!(q_yaw(0.0), 0);
        assert_eq!(q_yaw(90.0), 16384);
        assert_eq!(q_yaw(180.0), 32768);
        assert_eq!(q_yaw(270.0), 49152);
        assert_eq!(dq_yaw(16384), 90.0);
    }

    #[test]
    fn yaw_is_normalized_into_0_360() {
        // Le jeu rend des yaw négatifs (-180..180) : même angle → mêmes crans.
        assert_eq!(q_yaw(-90.0), q_yaw(270.0));
        assert_eq!(q_yaw(450.0), q_yaw(90.0));
        // 360° ≡ 0° (le round vers 65536 ne doit pas déborder du u16).
        assert_eq!(q_yaw(360.0), 0);
        assert_eq!(q_yaw(359.999), 0); // arrondi au cran le plus proche = 65536 ≡ 0
    }

    #[test]
    fn yaw_precision_is_within_one_quantum() {
        let deg = 123.456_f32;
        let err = (dq_yaw(q_yaw(deg)) - deg).abs();
        assert!(err <= 0.5 / YAW_SCALE, "erreur {err} > demi-cran");
    }
}
