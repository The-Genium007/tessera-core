//! `CrowdProducer` : le producteur de foule piétonne (spec
//! `2026-07-23-spec-implementation-serveur-monde-vivant.md` §3, incrément 2 du plan de code §7).
//!
//! Un producteur transforme une INTENTION de peuplement (« il manque N piétons près de ce joueur »,
//! produite par `population_director`) en `EntityStub`s CONCRETS : il décide OÙ (placement) et pose
//! l'archétype. Il ne matérialise rien lui-même — il crée des stubs `Dormant` que `StubPool` promeut
//! ensuite par proximité (le socle de l'incrément 1). C'est le maillon manquant entre « combien » (le
//! director) et « où/quel » (ici), sans lequel un spawn atterrissait à l'origine par défaut.
//!
//! ⚠️ **Placement = placeholder assumé.** Le RE (§3) indique que la foule naît SUR DES VOIES
//! (`worldTrafficLaneCrowdCreationInfo` porté par la voie), mais le graphe de voies de la foule n'est
//! pas encore extrait (gated Phase 6 — cf. backlog in-game). En attendant, ce module place les stubs
//! par un **semis déterministe en anneau** autour du joueur (spirale à angle d'or : réparti, sans
//! agglutinement, sans RNG donc testable). C'est plausible et intégrable AUJOURD'HUI, mais ce n'est
//! PAS le placement fidèle — le remplacer par le suivi de voies quand le graphe sera extrait est un
//! creux explicitement tracé, pas un oubli. Pur (aucune I/O), à l'image de `population_director.rs`.

use crate::stub::{EntityKind, EntityStub, ProducerId};
use crate::transport::ClientId;

/// Angle d'or en radians (`π·(3−√5)`). Deux points consécutifs d'une spirale espacés de cet angle
/// ne s'alignent jamais et couvrent le disque uniformément — d'où l'absence d'agglutinement sans
/// avoir à tirer de RNG (placement DÉTERMINISTE, donc reproductible et testable).
const GOLDEN_ANGLE: f32 = 2.399_963_2;

/// Config de placement de la foule autour d'un joueur. `inner`/`outer` bornent l'anneau : on ne
/// spawne pas collé au joueur (il verrait l'apparition), ni au-delà du rayon de promotion (sinon le
/// stub resterait `Dormant` pour rien). Dérivée du `WorldConfig` du socle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrowdPlacement {
    /// Rayon interne : distance mini d'un spawn au joueur (pas d'apparition dans son champ proche).
    pub inner: f32,
    /// Rayon externe : distance maxi — pris au rayon de promotion (`WorldConfig::spawn_dist`) pour
    /// qu'un stub tout juste produit soit immédiatement éligible à la promotion.
    pub outer: f32,
}

impl CrowdPlacement {
    /// Dérive l'anneau du `WorldConfig` du socle : externe = `spawn_dist` (le stub est produit juste
    /// dans le rayon de promotion), interne = moitié (placeholder raisonnable, à ajuster Phase 6).
    pub fn from_world_config(cfg: &crate::stub::WorldConfig) -> Self {
        Self {
            inner: cfg.spawn_dist * 0.5,
            outer: cfg.spawn_dist,
        }
    }

    /// Produit `count` stubs piétons `Dormant` placés autour de `player_pos`, ids consécutifs à
    /// partir de `first_id` (dans la plage réservée — garanti par l'appelant qui possède
    /// l'allocateur `next_npc_id`). Positions par spirale à angle d'or dans l'anneau `[inner, outer]`.
    /// `z` recopie celui du joueur (le placement vertical fin — trottoir vs voie — viendra avec le
    /// graphe de voies). Déterministe : mêmes entrées ⇒ mêmes positions (aucun RNG).
    ///
    /// `yaw` : orienté vers l'extérieur (dos au joueur) — placeholder ; l'orientation réelle suivra
    /// la tangente de la voie quand elle existera.
    pub fn produce(
        &self,
        count: u32,
        archetype: u32,
        player_pos: (f32, f32, f32),
        first_id: ClientId,
    ) -> Vec<EntityStub> {
        self.positions(count, player_pos)
            .into_iter()
            .enumerate()
            .map(|(i, (x, y, z, yaw))| {
                EntityStub::new_dormant(
                    first_id + i as ClientId,
                    EntityKind::Pedestrian,
                    archetype,
                    (x, y, z),
                    yaw,
                    ProducerId::Crowd,
                )
            })
            .collect()
    }

    /// Le CŒUR géométrique du placement, exposé seul (position + yaw) pour les appelants qui posent
    /// une pose sans construire de `EntityStub` — ex. `server_loop::tick_npcs` qui place un
    /// `NpcRecord` existant. `produce` en est la version « qui emballe en stubs ». Retourne
    /// `(x, y, z, yaw)` : `z` recopie celui du joueur, `yaw` = angle spiralé (dos au joueur,
    /// placeholder jusqu'à la tangente de voie). Déterministe, aucun RNG.
    pub fn positions(&self, count: u32, player_pos: (f32, f32, f32)) -> Vec<(f32, f32, f32, f32)> {
        (0..count)
            .map(|i| {
                let angle = i as f32 * GOLDEN_ANGLE;
                // Rayon en √ pour une densité uniforme sur le disque (sinon tout se tasse au centre).
                // `(i+0.5)/count` ∈ (0,1) mappé sur [inner, outer].
                let t = ((i as f32 + 0.5) / count as f32).sqrt();
                let radius = self.inner + (self.outer - self.inner) * t;
                let x = player_pos.0 + radius * angle.cos();
                let y = player_pos.1 + radius * angle.sin();
                (x, y, player_pos.2, angle)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::{Lifecycle, WorldConfig};

    fn placement() -> CrowdPlacement {
        CrowdPlacement::from_world_config(&WorldConfig::crowd_defaults())
    }

    #[test]
    fn produces_exactly_the_requested_count() {
        let stubs = placement().produce(5, 216, (0.0, 0.0, 0.0), crate::world::NPC_ID_RANGE_START);
        assert_eq!(stubs.len(), 5);
    }

    #[test]
    fn produced_stubs_are_dormant_pedestrians_from_the_crowd_producer() {
        let stubs = placement().produce(3, 216, (0.0, 0.0, 0.0), crate::world::NPC_ID_RANGE_START);
        for s in &stubs {
            assert_eq!(s.lifecycle, Lifecycle::Dormant);
            assert_eq!(s.kind, EntityKind::Pedestrian);
            assert_eq!(s.producer, ProducerId::Crowd);
            assert_eq!(s.archetype, 216);
        }
    }

    #[test]
    fn ids_are_consecutive_from_first_id() {
        let base = crate::world::NPC_ID_RANGE_START + 100;
        let stubs = placement().produce(4, 216, (0.0, 0.0, 0.0), base);
        let ids: Vec<ClientId> = stubs.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![base, base + 1, base + 2, base + 3]);
    }

    #[test]
    fn every_stub_is_placed_within_the_placement_ring_around_the_player() {
        let p = placement(); // inner = 20, outer = 40 (spawn_dist 40)
        let player = (100.0, 200.0, 5.0);
        let stubs = p.produce(20, 216, player, crate::world::NPC_ID_RANGE_START);
        for s in &stubs {
            let dx = s.position.0 - player.0;
            let dy = s.position.1 - player.1;
            let d = (dx * dx + dy * dy).sqrt();
            assert!(
                d >= p.inner - 0.01 && d <= p.outer + 0.01,
                "stub à {d} m hors de l'anneau [{}, {}]",
                p.inner,
                p.outer
            );
            assert_eq!(s.position.2, player.2, "z recopie celui du joueur");
        }
    }

    #[test]
    fn placement_is_deterministic() {
        let a = placement().produce(6, 216, (10.0, 20.0, 0.0), crate::world::NPC_ID_RANGE_START);
        let b = placement().produce(6, 216, (10.0, 20.0, 0.0), crate::world::NPC_ID_RANGE_START);
        let pa: Vec<_> = a.iter().map(|s| s.position).collect();
        let pb: Vec<_> = b.iter().map(|s| s.position).collect();
        assert_eq!(pa, pb, "mêmes entrées => mêmes positions (aucun RNG)");
    }

    #[test]
    fn stubs_do_not_all_land_on_the_same_point() {
        // Anti-agglutinement : au moins 2 positions distinctes sur un petit lot (la spirale répartit).
        let stubs = placement().produce(4, 216, (0.0, 0.0, 0.0), crate::world::NPC_ID_RANGE_START);
        let first = stubs[0].position;
        assert!(
            stubs.iter().any(|s| s.position != first),
            "les stubs doivent être répartis, pas empilés au même point"
        );
    }

    #[test]
    fn zero_count_produces_no_stubs() {
        let stubs = placement().produce(0, 216, (0.0, 0.0, 0.0), crate::world::NPC_ID_RANGE_START);
        assert!(stubs.is_empty());
    }

    /// Vérifie l'intégration avec le socle : un stub tout juste produit est placé DANS le rayon de
    /// promotion, donc `update_lifecycle` le promeut immédiatement si le joueur est là — la chaîne
    /// producteur→socle boucle bien (pas de stub né mort-né hors de portée).
    #[test]
    fn a_produced_stub_is_immediately_promotable_by_the_socle() {
        let cfg = WorldConfig::crowd_defaults();
        let player = (0.0, 0.0, 0.0);
        let mut stubs = CrowdPlacement::from_world_config(&cfg).produce(
            1,
            216,
            player,
            crate::world::NPC_ID_RANGE_START,
        );
        let s = &mut stubs[0];
        let dx = s.position.0 - player.0;
        let dy = s.position.1 - player.1;
        let dist_sq = dx * dx + dy * dy;
        let changed = s.update_lifecycle(Some(dist_sq), 0, &cfg);
        assert!(changed);
        assert_eq!(s.lifecycle, Lifecycle::Materializing);
    }
}
