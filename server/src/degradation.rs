//! Mécanisme de plafond d'entités + dégradation par distance/hystérésis — CONÇU, pas réglé.
//! Les valeurs par défaut ci-dessous (k=60, seuils de p99) sont un point de départ documenté,
//! PAS une calibration issue de mesures réelles (le harnais de charge qui produirait ces mesures
//! est explicitement différé, décision utilisateur : les vrais joueurs informent le réglage
//! avant les bots synthétiques). Ne jamais présenter ces constantes comme si elles avaient été
//! validées empiriquement.

/// Plafond par défaut des k plus proches voisins retenus dans un snapshot sous charge — au-delà
/// de ce nombre, les voisins les plus lointains (dans l'AoI mais hors du plafond) sont exclus du
/// snapshot pour ce tick. Valeur proposée par la spec, NON calibrée sur mesure réelle.
pub const DEFAULT_NEIGHBOR_CAP: usize = 60;

/// Seuils de p99 de durée de tick (microsecondes) définissant les paliers de dégradation, avec
/// hystérésis : on ne redescend d'un palier qu'après être repassé sous le seuil INFÉRIEUR moins
/// une marge, pour éviter l'oscillation à la frontière.
#[derive(Debug, Clone, Copy)]
pub struct DegradationPolicy {
    pub p99_enter_degraded_micros: u64,
    pub p99_exit_degraded_micros: u64,
    pub neighbor_cap: usize,
}

impl Default for DegradationPolicy {
    fn default() -> Self {
        Self {
            // 40ms = 80% du budget de tick à 50Hz (20ms) ou du budget de 50ms mentionné dans la
            // roadmap — valeur reprise de la spec, non recalibrée ici.
            p99_enter_degraded_micros: 40_000,
            // Hystérésis : redescendre seulement sous 30ms (marge de 10ms) pour ne pas osciller
            // à la frontière si le p99 flotte autour de 40ms.
            p99_exit_degraded_micros: 30_000,
            neighbor_cap: DEFAULT_NEIGHBOR_CAP,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradationTier {
    Normal,
    Degraded,
}

impl DegradationPolicy {
    /// Détermine le palier à appliquer MAINTENANT, étant donné le p99 courant et le palier
    /// précédent (pour appliquer l'hystérésis — sans état précédent, utilise le seuil d'entrée).
    pub fn tier_for_p99(&self, p99_micros: u64, previous_tier: DegradationTier) -> DegradationTier {
        match previous_tier {
            DegradationTier::Normal => {
                if p99_micros >= self.p99_enter_degraded_micros {
                    DegradationTier::Degraded
                } else {
                    DegradationTier::Normal
                }
            }
            DegradationTier::Degraded => {
                if p99_micros <= self.p99_exit_degraded_micros {
                    DegradationTier::Normal
                } else {
                    DegradationTier::Degraded
                }
            }
        }
    }

    /// Applique le plafond de voisins à une liste déjà triée par distance croissante (le
    /// caller — futur consommateur de `World::snapshot_for` — est responsable du tri ; cette
    /// fonction ne trie pas elle-même pour rester indépendante de la structure `Pose`).
    pub fn cap_neighbors<T>(&self, sorted_by_distance: Vec<T>, tier: DegradationTier) -> Vec<T> {
        match tier {
            DegradationTier::Normal => sorted_by_distance,
            DegradationTier::Degraded => {
                sorted_by_distance.into_iter().take(self.neighbor_cap).collect()
            }
        }
    }
}

/// Fréquence d'envoi dégressive avec la distance — renvoie `true` si CE tick doit inclure
/// l'entité à `distance_from_viewer`, étant donné `tick_index` (compteur global de ticks) et un
/// palier de dégradation. En mode Normal, toujours vrai (pas de dégression). En mode Degraded,
/// les entités les plus lointaines (au-delà de la moitié du rayon) ne sont mises à jour qu'un
/// tick sur deux — un mécanisme simple, pas un design final.
pub fn should_include_this_tick(
    distance_from_viewer: f32,
    aoi_radius: f32,
    tick_index: u64,
    tier: DegradationTier,
) -> bool {
    match tier {
        DegradationTier::Normal => true,
        DegradationTier::Degraded => {
            if distance_from_viewer <= aoi_radius / 2.0 {
                true
            } else {
                tick_index % 2 == 0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_for_p99_enters_degraded_when_crossing_the_enter_threshold() {
        let policy = DegradationPolicy::default();
        assert_eq!(
            policy.tier_for_p99(40_000, DegradationTier::Normal),
            DegradationTier::Degraded
        );
    }

    #[test]
    fn tier_for_p99_stays_normal_below_the_enter_threshold() {
        let policy = DegradationPolicy::default();
        assert_eq!(
            policy.tier_for_p99(39_999, DegradationTier::Normal),
            DegradationTier::Normal
        );
    }

    #[test]
    fn tier_for_p99_hysteresis_stays_degraded_between_the_two_thresholds() {
        let policy = DegradationPolicy::default();
        // Redescendu à 35ms — entre les deux seuils (30ms exit, 40ms enter) — doit RESTER
        // Degraded, c'est exactement le point de l'hystérésis (évite l'oscillation).
        assert_eq!(
            policy.tier_for_p99(35_000, DegradationTier::Degraded),
            DegradationTier::Degraded
        );
    }

    #[test]
    fn tier_for_p99_exits_degraded_once_below_the_exit_threshold() {
        let policy = DegradationPolicy::default();
        assert_eq!(
            policy.tier_for_p99(30_000, DegradationTier::Degraded),
            DegradationTier::Normal
        );
    }

    #[test]
    fn cap_neighbors_is_a_no_op_in_normal_tier() {
        let policy = DegradationPolicy::default();
        let items: Vec<u32> = (0..100).collect();
        let result = policy.cap_neighbors(items.clone(), DegradationTier::Normal);
        assert_eq!(result, items);
    }

    #[test]
    fn cap_neighbors_truncates_to_the_configured_cap_in_degraded_tier() {
        let policy = DegradationPolicy::default();
        let items: Vec<u32> = (0..100).collect();
        let result = policy.cap_neighbors(items, DegradationTier::Degraded);
        assert_eq!(result.len(), 60);
        assert_eq!(result[0], 0, "garde les plus proches (déjà triés par distance croissante par le caller)");
    }

    #[test]
    fn cap_neighbors_is_a_no_op_when_fewer_items_than_the_cap() {
        let policy = DegradationPolicy::default();
        let items: Vec<u32> = (0..10).collect();
        let result = policy.cap_neighbors(items.clone(), DegradationTier::Degraded);
        assert_eq!(result, items);
    }

    #[test]
    fn should_include_this_tick_always_true_in_normal_tier() {
        assert!(should_include_this_tick(1000.0, 25.0, 7, DegradationTier::Normal));
    }

    #[test]
    fn should_include_this_tick_always_true_for_close_entities_even_when_degraded() {
        assert!(should_include_this_tick(5.0, 25.0, 7, DegradationTier::Degraded));
    }

    #[test]
    fn should_include_this_tick_skips_far_entities_on_odd_ticks_when_degraded() {
        assert!(!should_include_this_tick(20.0, 25.0, 7, DegradationTier::Degraded));
        assert!(should_include_this_tick(20.0, 25.0, 8, DegradationTier::Degraded));
    }
}
