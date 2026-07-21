//! État autoritaire minimal : joueurs connectés et leurs positions. L'AoI (`snapshot_for`) est
//! servie par une grille de hachage spatiale (cellules = taille de l'AoI typique), index
//! secondaire dérivé de `players` — la source de vérité canonique reste `players`, jamais la
//! grille (qui peut toujours être reconstruite depuis `players` si besoin).

use crate::transport::ClientId;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub locomotion: u8,
    pub move_dir: u8,
    pub flags: u8,
    pub sustained: u32,
}

/// Coordonnée de cellule de grille (division entière de x/y par la taille de cellule).
type CellCoord = (i64, i64);

/// Taille de cellule par défaut — cohérente avec un AoI typique (25-75 unités selon `RadiusPolicy`
/// du Gateway, cf. handoff.rs). Une cellule plus grande que le plus petit rayon utilisé garantit
/// qu'une recherche n'a jamais besoin de scanner plus d'un anneau de cellules voisines.
const CELL_SIZE: f32 = 32.0;

/// Plage d'ids réservée aux PNJ, disjointe de tout id de connexion réelle. Les connexions réelles
/// utilisent des ids attribués par le Gateway (compteur croissant depuis une connexion réseau
/// réelle, jamais aussi élevé que ceci en pratique — mais la garde `is_npc_id` reste la source de
/// vérité, jamais une hypothèse sur "les vrais ids restent petits"). Choix : réutiliser
/// `players: BTreeMap<ClientId, Pose>` tel quel pour les PNJ (même snapshot_for, même grille) plutôt
/// que dupliquer une collection parallèle — cf. Global Constraints de ce plan.
pub const NPC_ID_RANGE_START: ClientId = 1 << 48;

pub fn is_npc_id(id: ClientId) -> bool {
    id >= NPC_ID_RANGE_START
}

/// Plage réservée aux véhicules, DISTINCTE de la plage PNJ piétons (`NPC_ID_RANGE_START`, compte
/// vers le haut) et de la plage PNJ nominatifs (compte vers le bas depuis `u64::MAX`, fondation
/// d'interaction) — un bit de poids fort supplémentaire garantit 2^48 ids d'écart avec la plage
/// piétons, marge gigantesque à toute échelle réaliste (même raisonnement que la séparation
/// piétons/nominatifs déjà en place).
///
/// # Note de relation avec `is_npc_id`
/// `VEHICLE_ID_RANGE_START > NPC_ID_RANGE_START`, donc un id véhicule satisfait aussi `is_npc_id`.
/// Les appelants existants de `is_npc_id` (notamment `tick_npcs`'s calcul de `player_count` en
/// `server_loop.rs`) doivent être vérifiés à Task 3 pour s'assurer qu'un futur véhicule ne serait
/// pas accidentellement compté comme "PNJ" dans une logique qui ne l'attend pas — à ce stade,
/// `is_npc_id` doit soit rester "piétons strictement" ou devenir `is_simulated_entity_id` englobant
/// (piétons + véhicules).
pub const VEHICLE_ID_RANGE_START: ClientId = 1 << 49;

pub fn is_vehicle_id(id: ClientId) -> bool {
    id >= VEHICLE_ID_RANGE_START
}

fn cell_of(x: f32, y: f32) -> CellCoord {
    (
        (x / CELL_SIZE).floor() as i64,
        (y / CELL_SIZE).floor() as i64,
    )
}

#[derive(Default)]
pub struct World {
    players: BTreeMap<ClientId, Pose>,
    /// Index secondaire : cellule -> ensemble des ids de joueurs dans cette cellule. Maintenu en
    /// synchronisation à `add_player`/`remove_player`/`set_pose` (les 3 seuls points qui changent
    /// l'appartenance d'un joueur à une cellule). Ne JAMAIS lire ceci comme source de vérité — un
    /// bug de synchronisation ici ne doit affecter que la PERFORMANCE, jamais la correction
    /// (Step 5 : un test dédié vérifie explicitement cette invariante en comparant à un scan
    /// linéaire de secours).
    grid: HashMap<CellCoord, Vec<ClientId>>,
    tick: u64,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, id: ClientId) {
        if self.players.contains_key(&id) {
            return;
        }
        self.players.insert(id, Pose::default());
        let cell = cell_of(0.0, 0.0);
        self.grid.entry(cell).or_default().push(id);
    }

    pub fn remove_player(&mut self, id: ClientId) {
        if let Some(pose) = self.players.remove(&id) {
            let cell = cell_of(pose.x, pose.y);
            if let Some(bucket) = self.grid.get_mut(&cell) {
                bucket.retain(|&i| i != id);
                if bucket.is_empty() {
                    self.grid.remove(&cell);
                }
            }
        }
    }

    pub fn set_pose(&mut self, id: ClientId, pose: Pose) {
        if let Some(p) = self.players.get_mut(&id) {
            let old_cell = cell_of(p.x, p.y);
            let new_cell = cell_of(pose.x, pose.y);
            *p = pose;
            if old_cell != new_cell {
                if let Some(bucket) = self.grid.get_mut(&old_cell) {
                    bucket.retain(|&i| i != id);
                    if bucket.is_empty() {
                        self.grid.remove(&old_cell);
                    }
                }
                self.grid.entry(new_cell).or_default().push(id);
            }
        }
    }

    /// Met à jour l'état de locomotion cosmétique sans toucher position/yaw — reste no-op si le
    /// joueur n'est pas (encore/plus) connu du World (race déconnexion, cf. set_pose/snapshot_for).
    pub fn set_locomotion(&mut self, id: ClientId, locomotion: u8, move_dir: u8, flags: u8) {
        if let Some(p) = self.players.get_mut(&id) {
            p.locomotion = locomotion;
            p.move_dir = move_dir;
            p.flags = flags;
        }
    }

    /// Pose tenue (assis, adossé...) : id d'émote natif, 0 = aucune. Piloté par EmoteReport
    /// (start=true pose l'id, start=false repasse à 0) — jamais par PositionUpdate.
    pub fn set_sustained(&mut self, id: ClientId, emote: u32) {
        if let Some(p) = self.players.get_mut(&id) {
            p.sustained = emote;
        }
    }

    /// Lecture seule de la pose courante d'un joueur — sert à préserver locomotion/sustained lors
    /// d'un remplacement partiel de position (cf. server_loop::apply_client_message, Task 3).
    pub fn pose_of(&self, id: ClientId) -> Option<Pose> {
        self.players.get(&id).copied()
    }

    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Snapshot vu par `viewer` : les autres joueurs à `radius` ou moins (distance 2D, Z ignoré).
    /// Implémentation : scanne uniquement les cellules de la grille qui PEUVENT contenir un
    /// joueur dans le rayon (l'anneau de cellules couvrant un carré de côté `2*radius` centré sur
    /// le viewer), au lieu de scanner tous les joueurs du monde — le nombre de cellules scannées
    /// est indépendant de n, seul le nombre de joueurs RÉELLEMENT dans ces cellules est parcouru.
    pub fn snapshot_for(&self, viewer: ClientId, radius: f32) -> Vec<(ClientId, Pose)> {
        let Some(&viewer_pose) = self.players.get(&viewer) else {
            return Vec::new();
        };
        let cell_radius = (radius / CELL_SIZE).ceil() as i64 + 1;
        let (vcx, vcy) = cell_of(viewer_pose.x, viewer_pose.y);
        let mut result = Vec::new();
        for dx in -cell_radius..=cell_radius {
            for dy in -cell_radius..=cell_radius {
                let cell = (vcx + dx, vcy + dy);
                let Some(bucket) = self.grid.get(&cell) else {
                    continue;
                };
                for &id in bucket {
                    if id == viewer {
                        continue;
                    }
                    let Some(pose) = self.players.get(&id) else {
                        continue;
                    };
                    let dx = pose.x - viewer_pose.x;
                    let dy = pose.y - viewer_pose.y;
                    if (dx * dx + dy * dy).sqrt() <= radius {
                        result.push((id, *pose));
                    }
                }
            }
        }
        result
    }

    pub fn player_ids(&self) -> Vec<ClientId> {
        self.players.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_npc_id_distinguishes_the_reserved_range_from_real_connection_ids() {
        assert!(!is_npc_id(1));
        assert!(!is_npc_id(1_000_000));
        assert!(is_npc_id(NPC_ID_RANGE_START));
        assert!(is_npc_id(NPC_ID_RANGE_START + 1));
    }

    #[test]
    fn a_npc_id_inserted_via_add_player_appears_in_snapshot_for_like_any_other_entity() {
        // World ne distingue pas les PNJ des joueurs au niveau du stockage (Global Constraints) — un
        // id de la plage réservée, inséré via add_player, doit apparaître dans snapshot_for exactement
        // comme un joueur. C'est le test qui verrouille cette décision architecturale.
        let mut w = World::new();
        let npc_id = NPC_ID_RANGE_START + 1;
        w.add_player(npc_id);
        w.add_player(1); // viewer réel
        w.set_pose(
            npc_id,
            Pose {
                x: 1.0,
                y: 1.0,
                z: 0.0,
                ..Default::default()
            },
        );
        let seen = w.snapshot_for(1, 50.0);
        assert!(seen.iter().any(|(id, _)| *id == npc_id));
    }

    #[test]
    fn snapshot_excludes_the_viewer_and_includes_others() {
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        w.set_pose(
            1,
            Pose {
                x: 5.0,
                y: 0.0,
                z: 0.0,
                yaw: 1.0,
                ..Default::default()
            },
        );

        let snap = w.snapshot_for(2, 1000.0);
        assert_eq!(snap.len(), 1, "le viewer ne se voit pas lui-même");
        assert_eq!(snap[0].0, 1);
        assert_eq!(snap[0].1.x, 5.0);
    }

    #[test]
    fn removed_player_disappears_from_snapshots() {
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        w.remove_player(1);
        assert!(w.snapshot_for(2, 1000.0).is_empty());
    }

    #[test]
    fn excludes_players_beyond_the_radius() {
        let mut w = World::new();
        w.add_player(1); // viewer, stays at origin (default pose)
        w.add_player(2); // near
        w.add_player(3); // far
        w.set_pose(
            2,
            Pose {
                x: 10.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                ..Default::default()
            },
        );
        w.set_pose(
            3,
            Pose {
                x: 500.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                ..Default::default()
            },
        );

        let snap = w.snapshot_for(1, 50.0);
        assert_eq!(snap.len(), 1, "seul le joueur proche doit apparaître");
        assert_eq!(snap[0].0, 2);
    }

    #[test]
    fn viewer_missing_from_world_returns_empty_snapshot() {
        let mut w = World::new();
        w.add_player(2);
        // client 1 n'a jamais été ajouté (ex: race avec une déconnexion) — pas de panic attendu.
        assert!(w.snapshot_for(1, 1000.0).is_empty());
    }

    #[test]
    fn set_locomotion_updates_pose_fields_without_touching_position() {
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        w.set_pose(
            1,
            Pose {
                x: 5.0,
                y: 0.0,
                z: 0.0,
                yaw: 1.0,
                ..Default::default()
            },
        );
        w.set_locomotion(1, 2, 10, 0);
        let snap = w.snapshot_for(2, 1000.0);
        assert_eq!(snap.len(), 1);
        let (_, pose) = snap[0];
        assert_eq!(
            pose.x, 5.0,
            "la position ne doit pas être affectée par set_locomotion"
        );
        assert_eq!(pose.locomotion, 2);
        assert_eq!(pose.move_dir, 10);
    }

    #[test]
    fn set_locomotion_on_unknown_player_does_not_panic() {
        let mut w = World::new();
        w.set_locomotion(999, 1, 0, 0); // joueur jamais ajouté (race déconnexion) — pas de panic.
    }

    #[test]
    fn set_sustained_updates_pose_field() {
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        w.set_sustained(1, 42);
        let snap = w.snapshot_for(2, 1000.0);
        assert_eq!(snap[0].1.sustained, 42);
    }

    #[test]
    fn set_sustained_zero_clears_the_pose() {
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        w.set_sustained(1, 42);
        w.set_sustained(1, 0);
        let snap = w.snapshot_for(2, 1000.0);
        assert_eq!(snap[0].1.sustained, 0);
    }

    #[test]
    fn default_pose_has_idle_locomotion_and_no_sustained_emote() {
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        let snap = w.snapshot_for(2, 1000.0);
        assert_eq!(snap[0].1.locomotion, 0);
        assert_eq!(snap[0].1.sustained, 0);
    }

    #[test]
    fn pose_of_returns_current_pose_for_known_player() {
        let mut w = World::new();
        w.add_player(1);
        w.set_sustained(1, 7);
        let p = w.pose_of(1).expect("le joueur 1 est connu");
        assert_eq!(p.sustained, 7);
    }

    #[test]
    fn pose_of_returns_none_for_unknown_player() {
        let w = World::new();
        assert_eq!(w.pose_of(999), None);
    }

    #[test]
    fn players_at_a_grid_cell_boundary_still_see_each_other_within_radius() {
        // Piège classique d'une grille spatiale mal implémentée : deux joueurs à une distance RÉELLE
        // inférieure au rayon, mais placés dans des cellules de grille ADJACENTES (pas la même
        // cellule) — un index naïf qui ne scanne que la cellule du viewer les raterait. La grille
        // DOIT scanner aussi les cellules voisines dans le rayon de recherche.
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        // CELL_SIZE = 32.0 → frontières de cellule à x = 0, 32, 64, ... Les deux joueurs sont
        // placés de part et d'autre de la frontière x=32 (cellules ADJACENTES réelles, pas la
        // même cellule), à une distance réelle de 2.0 — bien sous radius=25.0.
        w.set_pose(
            1,
            Pose {
                x: 31.0,
                y: 0.0,
                ..Default::default()
            },
        );
        w.set_pose(
            2,
            Pose {
                x: 33.0,
                y: 0.0,
                ..Default::default()
            },
        ); // distance réelle = 2.0
        let snap = w.snapshot_for(1, 25.0);
        assert_eq!(
            snap.len(),
            1,
            "joueur 2 doit être visible malgré une frontière de cellule potentielle entre les deux"
        );
        assert_eq!(snap[0].0, 2);
    }

    #[test]
    fn many_players_scattered_across_multiple_cells_produce_the_same_set_as_linear_scan() {
        // Test de non-régression ensembliste : construit un scénario à 20 joueurs répartis sur une
        // grille large, calcule le snapshot attendu par un scan linéaire de référence codé ICI (pas
        // en dépendant de World, pour ne pas biaiser la comparaison), et vérifie que World::snapshot_for
        // produit exactement le même ENSEMBLE d'ids (l'ordre peut différer, cf. Step 5 sur le tri).
        let mut w = World::new();
        let positions: Vec<(u64, f32, f32)> = (1..=20)
            .map(|i| (i as u64, (i as f32) * 7.0, (i as f32 % 3 as f32) * 11.0))
            .collect();
        for (id, x, y) in &positions {
            w.add_player(*id);
            w.set_pose(
                *id,
                Pose {
                    x: *x,
                    y: *y,
                    ..Default::default()
                },
            );
        }
        let radius = 20.0;
        for (viewer_id, vx, vy) in &positions {
            let expected: std::collections::BTreeSet<u64> = positions
                .iter()
                .filter(|(id, _, _)| id != viewer_id)
                .filter(|(_, x, y)| {
                    let dx = x - vx;
                    let dy = y - vy;
                    (dx * dx + dy * dy).sqrt() <= radius
                })
                .map(|(id, _, _)| *id)
                .collect();
            let actual: std::collections::BTreeSet<u64> = w
                .snapshot_for(*viewer_id, radius)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            assert_eq!(
                actual, expected,
                "viewer {viewer_id} : l'ensemble de voisins doit être identique au scan linéaire de référence"
            );
        }
    }

    #[test]
    #[ignore = "chronométrage indicatif, pas un test de correction — exécuter à la demande avec --ignored"]
    fn snapshot_for_scales_sublinearly_with_scattered_population() {
        // Preuve indicative (pas un test de correction strict, fragile en CI par nature d'un
        // chronométrage) : à population dense mais RÉPARTIE spatialement (pas concentrée en un seul
        // point), le coût de snapshot_for doit rester largement inférieur au coût O(n) d'un scan
        // linéaire complet, démontré empiriquement plutôt que par une assertion Big-O formelle.
        use std::time::Instant;
        let mut w = World::new();
        let n = 5_000;
        for i in 0..n {
            w.add_player(i as u64);
            // Répartition large (grille 1000x1000 unités) pour garantir peu de voisins par cellule.
            let x = (i * 37 % 1000) as f32;
            let y = (i * 53 % 1000) as f32;
            w.set_pose(
                i as u64,
                Pose {
                    x,
                    y,
                    ..Default::default()
                },
            );
        }
        let start = Instant::now();
        let snap = w.snapshot_for(0, 32.0); // rayon ~= une cellule
        let elapsed = start.elapsed();
        println!(
            "snapshot_for sur {n} joueurs dispersés : {elapsed:?}, {} voisins trouvés",
            snap.len()
        );
        assert!(
            elapsed.as_millis() < 5,
            "un snapshot sur une population dispersée doit rester sous quelques ms, pas dépendre de n"
        );
    }
}
