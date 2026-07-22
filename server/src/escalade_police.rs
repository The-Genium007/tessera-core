//! Politique d'escalade policière (spec PNJ hostiles §3) : "la police = un archétype hostile + une
//! politique d'escalade". Ce module est PUR — aucune I/O, aucun accès à `World`/`Server`. Le heat
//! est un scalaire SERVEUR-AUTORITAIRE mono-district (même simplification que `PopulationDirector`,
//! le vrai multi-district topologique est différé, cf. Global Constraints du plan fondation PNJ) —
//! il n'est JAMAIS transporté sur le protocole (spec §3 : "aucun champ protocole, seuls ses EFFETS
//! voyagent").

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeatTracker {
    pub heat: u32,
}

impl HeatTracker {
    /// Augmente le heat suite à un rapport de menace (spec §3 : "alimenté par les canaux déjà
    /// arbitrés — EntityInteraction{Menace}, transitions FSM des témoins"). Saturé (pas de
    /// débordement) plutôt que d'introduire un plafond arbitraire non spécifié.
    pub fn report_incident(&mut self, amount: u32) {
        self.heat = self.heat.saturating_add(amount);
    }

    /// Décroissance temporelle (spec §3 : "decay temporel") — appelée une fois par tick avec le
    /// montant de decay de la politique active.
    pub fn decay(&mut self, amount: u32) {
        self.heat = self.heat.saturating_sub(amount);
    }
}

/// Un seuil de la politique d'escalade : à partir de `heat_min`, `effectif` policiers sont
/// souhaités. Triée par `heat_min` croissant par la validation (Task 4, chargement TOML), pas ici
/// (ce type reste un simple couple de données, la garantie d'ordre est de la responsabilité du
/// parsing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscalationThreshold {
    pub heat_min: u32,
    pub effectif: u32,
}

#[derive(Debug, Clone)]
pub struct EscalationPolicy {
    pub seuils: Vec<EscalationThreshold>,
    pub decay_par_tick: u32,
}

/// Résout l'effectif souhaité pour un niveau de heat donné (spec §3 : "seuils de heat,
/// effectifs/niveau"). Le seuil applicable est le plus HAUT `heat_min` <= `heat` — si `heat` est
/// sous le premier seuil, effectif = 0 (aucune police nécessaire).
pub fn heat_to_effectif(heat: u32, policy: &EscalationPolicy) -> u32 {
    policy
        .seuils
        .iter()
        .filter(|s| s.heat_min <= heat)
        .map(|s| s.effectif)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> EscalationPolicy {
        EscalationPolicy {
            seuils: vec![
                EscalationThreshold {
                    heat_min: 0,
                    effectif: 0,
                },
                EscalationThreshold {
                    heat_min: 50,
                    effectif: 1,
                },
                EscalationThreshold {
                    heat_min: 100,
                    effectif: 3,
                },
            ],
            decay_par_tick: 1,
        }
    }

    #[test]
    fn heat_below_first_threshold_wants_no_police() {
        assert_eq!(heat_to_effectif(10, &policy()), 0);
    }

    #[test]
    fn heat_at_a_threshold_wants_that_threshold_effectif() {
        assert_eq!(heat_to_effectif(50, &policy()), 1);
        assert_eq!(heat_to_effectif(100, &policy()), 3);
    }

    #[test]
    fn heat_between_thresholds_wants_the_lower_ones_effectif() {
        assert_eq!(heat_to_effectif(75, &policy()), 1);
    }

    #[test]
    fn report_incident_increases_heat_saturating() {
        let mut h = HeatTracker::default();
        h.report_incident(30);
        assert_eq!(h.heat, 30);
        h.report_incident(u32::MAX);
        assert_eq!(
            h.heat,
            u32::MAX,
            "saturating_add ne doit jamais paniquer/déborder"
        );
    }

    #[test]
    fn decay_decreases_heat_saturating_at_zero() {
        let mut h = HeatTracker { heat: 5 };
        h.decay(10);
        assert_eq!(h.heat, 0, "saturating_sub ne doit jamais passer sous zéro");
    }
}
