//! Pont Shard→Gateway GÉNÉRIQUE pour toute entité simulée côté Shard qui doit déclencher un vrai
//! handoff cross-shard (spec véhicules autonomes §5, handoff prédictif). Jusqu'ici, `ShardLoader`
//! (handoff.rs) — qui pilote RÉELLEMENT le chargement/déchargement d'un shard — ne vit QUE côté
//! Gateway et n'est atteint que par la boucle d'événements d'un client réseau réel
//! (`TransportEvent::Connected/Disconnected/Message`). Aucune entité purement simulée (PNJ,
//! véhicule) n'a jamais pu la déclencher — la fondation PNJ documentait ceci comme un prérequis
//! différé, "à traiter avec le sous-projet qui en a besoin en premier" (véhicules, ici).
//!
//! Principe : le Shard envoie un `EntityPositionReport` (internal.fbs) à chaque tick pour toute
//! entité DONT LE CHEMIN PLANIFIÉ ENTRE DANS LE TAMPON d'un shard voisin (prédictif — pas quand
//! elle franchit réellement, spec §5 : "sait à l'avance où/quand"). Le Gateway, en le recevant,
//! traite `entity_id` EXACTEMENT comme un `ClientId` de client réel : `topology.locate(x, y,
//! rank_bonus)` puis `ShardLoader::feed(TransportEvent::Message{from: entity_id, data: <frame
//! ClientEvent synthétique>}, Some(placement))` — RÉUTILISE tout le chemin de placement/chargement
//! existant sans aucune nouvelle logique, seulement une nouvelle SOURCE d'événement. Ce module ne
//! contient QUE la détection "faut-il envoyer un rapport ce tick" — le câblage réel
//! Shard→TCP→Gateway→ShardLoader vit dans `shard.rs`/`gateway.rs` (Task 5).
//!
//! **Primitive volontairement générique** : ce module ne sait rien de "véhicule". Un futur PNJ
//! piéton cross-shard (mentionné comme travail futur par la fondation PNJ) réutilise cette même
//! primitive sans aucune modification ici.

use crate::nav_graph::Vec3;

/// Doit-on envoyer un rapport de position ce tick pour cette entité ? Vrai si sa position actuelle
/// est à portée `boundary_lookahead_radius` d'une frontière (approximée ici par : distance à `to`,
/// le point de destination du segment de chemin courant, en-dessous du seuil — v1 simplifiée ;
/// une détection géométrique réelle de frontière de shard nécessiterait la topologie complète côté
/// Shard, qui ne l'a pas aujourd'hui — cf. Hors périmètre). `speed` sert à calculer un tampon
/// proportionnel (spec §5 : `rank_bonus ≈ vitesse × N secondes`).
pub fn should_report_position(
    current: Vec3,
    next_waypoint: Vec3,
    speed_units_per_sec: f32,
    lookahead_seconds: f32,
) -> bool {
    let lookahead_distance = speed_units_per_sec * lookahead_seconds;
    current.distance(&next_waypoint) <= lookahead_distance
}

/// Bonus de rayon proportionnel à la vitesse, pour le tampon prédictif (spec §5 : "rank_bonus ≈
/// vitesse × N s, N à régler, plafonné pour ne pas double-charger tout un centre-ville"). `cap`
/// borne le bonus max — sans plafond, un véhicule très rapide gonflerait le tampon sans limite.
pub fn predictive_rank_bonus(speed_units_per_sec: f32, seconds: f32, cap: f32) -> f32 {
    (speed_units_per_sec * seconds).min(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_from_the_next_waypoint_does_not_trigger_a_report() {
        let current = Vec3::new(0.0, 0.0, 0.0);
        let next = Vec3::new(1000.0, 0.0, 0.0);
        assert!(!should_report_position(current, next, 8.0, 2.0));
    }

    #[test]
    fn within_lookahead_distance_triggers_a_report() {
        // vitesse 8 u/s * 2s lookahead = 16 unités de portée.
        let current = Vec3::new(0.0, 0.0, 0.0);
        let next = Vec3::new(10.0, 0.0, 0.0);
        assert!(should_report_position(current, next, 8.0, 2.0));
    }

    #[test]
    fn exactly_at_the_lookahead_boundary_triggers_a_report() {
        let current = Vec3::new(0.0, 0.0, 0.0);
        let next = Vec3::new(16.0, 0.0, 0.0);
        assert!(should_report_position(current, next, 8.0, 2.0));
    }

    #[test]
    fn predictive_rank_bonus_scales_with_speed() {
        assert_eq!(predictive_rank_bonus(10.0, 2.0, 1000.0), 20.0);
    }

    #[test]
    fn predictive_rank_bonus_is_capped() {
        assert_eq!(predictive_rank_bonus(1000.0, 2.0, 50.0), 50.0);
    }

    #[test]
    fn a_stationary_entity_never_triggers_a_report_regardless_of_distance() {
        let current = Vec3::new(0.0, 0.0, 0.0);
        let next = Vec3::new(5.0, 0.0, 0.0);
        assert!(!should_report_position(current, next, 0.0, 2.0));
    }
}
