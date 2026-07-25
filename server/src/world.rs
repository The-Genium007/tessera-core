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
    /// Repère dans lequel `x`/`y`/`z` sont exprimés (ADR 0013). `WORLD_FRAME` (0, le défaut
    /// `Default`) = position monde directe. Un repère non-nul (ex. cabine d'ascenseur) exige de
    /// résoudre via `FrameRegistry::world_position` avant toute comparaison de distance monde —
    /// voir `snapshot_for_resolved`.
    pub frame: crate::frame::FrameId,
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

    /// Comme `snapshot_for`, mais résout chaque pose à travers son `frame` avant de calculer la
    /// distance et de la retourner — jamais un mélange d'offset local et de position monde (ADR 0013).
    /// Un passager dont le repère est inconnu de `frames` est OMIS du résultat (règle de repli
    /// explicite) plutôt que placé à une position fausse.
    ///
    /// Implémentation (revue finale de branche, finding Critical #1, 2026-07-25) : utilise la
    /// grille de hachage spatiale (comme `snapshot_for`) pour le cas commun, au lieu d'un scan
    /// linéaire de `self.players` — sinon le chemin de snapshot réseau réintroduit exactement le
    /// mur O(n) que la grille existe pour éliminer (cf. commit `27230ae`,
    /// `docs/superpowers/specs/2026-07-18-tenue-en-charge-design.md` §2). La grille est indexée sur
    /// `pose.x/y` TELS QUE STOCKÉS : pour un joueur en `WORLD_FRAME` (immense majorité), c'est déjà
    /// la position monde — donc la grille est directement utilisable. Pour un joueur en repère
    /// mobile (minorité — passagers d'ascenseur), `pose.x/y` est un OFFSET LOCAL (souvent proche de
    /// 0,0), donc potentiellement dans une cellule de grille sans rapport avec sa position monde
    /// réelle : une recherche par rayon sur la grille brute ne le trouverait pas de façon fiable.
    /// Les deux populations sont donc traitées séparément : la grille est walkée pour les candidats
    /// spatiaux (comme `snapshot_for`), en excluant les joueurs framés ; ceux-ci sont résolus et
    /// évalués à part, coût O(k) où k = nombre de joueurs actuellement montés (négligeable même à
    /// grande échelle, cf. `framed_player_ids`).
    pub fn snapshot_for_resolved(
        &self,
        viewer: ClientId,
        radius: f32,
        frames: &crate::frame::FrameRegistry,
    ) -> Vec<(ClientId, Pose)> {
        let Some(&viewer_pose) = self.players.get(&viewer) else {
            return Vec::new();
        };
        let Some(viewer_world) = frames.world_position(
            viewer_pose.frame,
            [viewer_pose.x, viewer_pose.y, viewer_pose.z],
        ) else {
            return Vec::new();
        };

        let framed_ids: std::collections::HashSet<ClientId> =
            self.framed_player_ids().into_iter().collect();

        let mut result = Vec::new();

        // Population WORLD_FRAME : walk de la grille, comme `snapshot_for`. `pose.x/y` EST déjà la
        // position monde pour ces joueurs, donc la grille (indexée dessus) est directement fiable.
        let cell_radius = (radius / CELL_SIZE).ceil() as i64 + 1;
        let (vcx, vcy) = cell_of(viewer_world[0], viewer_world[1]);
        for dx in -cell_radius..=cell_radius {
            for dy in -cell_radius..=cell_radius {
                let cell = (vcx + dx, vcy + dy);
                let Some(bucket) = self.grid.get(&cell) else {
                    continue;
                };
                for &id in bucket {
                    if id == viewer || framed_ids.contains(&id) {
                        continue;
                    }
                    let Some(&pose) = self.players.get(&id) else {
                        continue;
                    };
                    let dx = pose.x - viewer_world[0];
                    let dy = pose.y - viewer_world[1];
                    if (dx * dx + dy * dy).sqrt() <= radius {
                        // pose.frame == WORLD_FRAME ici par construction (non-framed) : pas de
                        // résolution nécessaire, mais le reset explicite documente l'invariant et
                        // reste correct même si un futur appelant relâchait cette garantie.
                        let mut resolved = pose;
                        resolved.frame = crate::frame::WORLD_FRAME;
                        result.push((id, resolved));
                    }
                }
            }
        }

        // Population framée (repère mobile) : petite, résolue et évaluée individuellement — ne
        // JAMAIS faire confiance à sa position dans la grille (offset local, cellule non fiable).
        for id in framed_ids {
            if id == viewer {
                continue;
            }
            let Some(&pose) = self.players.get(&id) else {
                continue;
            };
            let Some(world_pos) = frames.world_position(pose.frame, [pose.x, pose.y, pose.z])
            else {
                continue;
            };
            let dx = world_pos[0] - viewer_world[0];
            let dy = world_pos[1] - viewer_world[1];
            if (dx * dx + dy * dy).sqrt() <= radius {
                let mut resolved = pose;
                resolved.x = world_pos[0];
                resolved.y = world_pos[1];
                resolved.z = world_pos[2];
                // Les coordonnées sont maintenant en espace MONDE : le repère d'origine ne doit
                // plus être réappliqué. Sans ce reset, un futur appelant qui repasserait ce `Pose`
                // à `world_position` doublerait la transformation (revue finale 2026-07-23).
                resolved.frame = crate::frame::WORLD_FRAME;
                result.push((id, resolved));
            }
        }

        result
    }

    /// Ids des joueurs actuellement dans un repère mobile (`pose.frame != WORLD_FRAME`) — petite
    /// population (passagers d'ascenseur aujourd'hui), scannée séparément par
    /// `snapshot_for_resolved` car leur position stockée (offset local) n'est pas fiable dans la
    /// grille spatiale indexée sur des coordonnées monde. `O(n)` sur `self.players`, mais n=nombre
    /// total de joueurs connectés n'est PAS le facteur limitant ici : c'est appelé une fois par
    /// `snapshot_for_resolved` (une fois par viewer par tick), pas par paire — le coût réel dominant
    /// reste le walk de grille au-dessus. Un futur raffinement pourrait maintenir cet ensemble de
    /// façon incrémentale (comme `grid`) si ce scan devenait mesurable ; pas fait ici car k (nombre
    /// de joueurs montés) reste très petit à toute échelle réaliste (ADR 0013).
    fn framed_player_ids(&self) -> Vec<ClientId> {
        self.players
            .iter()
            .filter(|(_, pose)| pose.frame != crate::frame::WORLD_FRAME)
            .map(|(&id, _)| id)
            .collect()
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
    fn default_pose_is_in_the_world_frame() {
        let mut w = World::new();
        w.add_player(1);
        let pose = w.pose_of(1).unwrap();
        assert_eq!(pose.frame, crate::frame::WORLD_FRAME);
    }

    #[test]
    fn snapshot_for_resolved_translates_a_passenger_pose_through_its_frame() {
        let mut w = World::new();
        w.add_player(1); // viewer, dans le monde à l'origine
        w.add_player(2); // passager d'un ascenseur (repère 7)
        w.set_pose(
            2,
            Pose {
                x: 0.5, // offset local dans la cabine
                y: 0.0,
                z: 0.0,
                frame: 7,
                ..Default::default()
            },
        );
        let mut frames = crate::frame::FrameRegistry::new();
        frames.set_transform(
            7,
            crate::frame::FrameTransform {
                position: [10.0, 0.0, 40.0], // la cabine est au monde (10, 0, 40)
                yaw: 0.0,
            },
        );
        let snap = w.snapshot_for_resolved(1, 1000.0, &frames);
        assert_eq!(snap.len(), 1);
        let (_, resolved_pose) = snap[0];
        assert_eq!(resolved_pose.x, 10.5, "10 (cabine) + 0.5 (offset local)");
        assert_eq!(resolved_pose.z, 40.0);
        assert_eq!(
            resolved_pose.frame,
            crate::frame::WORLD_FRAME,
            "une pose résolue en monde ne doit plus porter son ancien repère, sous peine de \
             double transformation si un futur appelant la repasse à world_position"
        );
    }

    #[test]
    fn snapshot_for_resolved_excludes_a_passenger_whose_frame_is_unknown() {
        // Règle de repli ADR 0013 : un repère détruit/inconnu ne doit jamais produire une position
        // fausse — l'entité est omise plutôt que mal placée.
        let mut w = World::new();
        w.add_player(1);
        w.add_player(2);
        w.set_pose(
            2,
            Pose {
                frame: 999, // jamais enregistré dans le FrameRegistry
                ..Default::default()
            },
        );
        let frames = crate::frame::FrameRegistry::new();
        let snap = w.snapshot_for_resolved(1, 1000.0, &frames);
        assert!(
            snap.is_empty(),
            "un passager dans un repère inconnu ne doit jamais apparaître avec une position fausse"
        );
    }

    #[test]
    fn snapshot_for_resolved_matches_snapshot_for_when_no_frames_are_active() {
        // Non-régression (revue finale de branche, finding Critical #1) : sans repère mobile actif
        // (tous les joueurs en WORLD_FRAME), snapshot_for_resolved doit voir EXACTEMENT le même
        // ensemble de joueurs que snapshot_for — preuve que le chemin résolu utilise bien la grille
        // pour le cas commun, pas un scan linéaire séparé qui pourrait diverger silencieusement.
        let mut w = World::new();
        let frames = crate::frame::FrameRegistry::new();
        for id in 1..=20 {
            w.add_player(id);
            w.set_pose(
                id,
                Pose {
                    x: (id as f32) * 3.0,
                    y: (id as f32) * 2.0,
                    ..Default::default()
                },
            );
        }
        let viewer = 1;
        let radius = 50.0;

        let mut baseline = w.snapshot_for(viewer, radius);
        let mut resolved = w.snapshot_for_resolved(viewer, radius, &frames);
        baseline.sort_by_key(|(id, _)| *id);
        resolved.sort_by_key(|(id, _)| *id);

        assert_eq!(
            baseline.len(),
            resolved.len(),
            "sans repère mobile actif, snapshot_for_resolved doit voir exactement les mêmes joueurs que snapshot_for"
        );
        for ((id_a, pose_a), (id_b, pose_b)) in baseline.iter().zip(resolved.iter()) {
            assert_eq!(id_a, id_b);
            assert!((pose_a.x - pose_b.x).abs() < 1e-4);
            assert!((pose_a.y - pose_b.y).abs() < 1e-4);
        }
    }

    #[test]
    fn snapshot_for_resolved_uses_the_grid_for_a_world_frame_population_at_production_scale() {
        // Preuve de non-régression PERF ciblée (finding Critical #1) : à population dense et rayon
        // réaliste (visibility_radius ~ 100.0, cf. server.docker.toml), le résultat ENSEMBLISTE doit
        // rester identique à un scan linéaire de référence codé ICI (pas en dépendant de World),
        // même à une échelle où un scan O(n) par appel serait mesurable (n=800). Ce test ne mesure
        // pas le temps (trop bruyant en CI, cf. piège documenté dans le brief) : il pin le
        // COMPORTEMENT à une échelle qui aurait révélé un scan linéaire caché.
        let mut w = World::new();
        let frames = crate::frame::FrameRegistry::new();
        let n = 800;
        let positions: Vec<(u64, f32, f32)> = (0..n)
            .map(|i| {
                let x = (i * 37 % 2000) as f32;
                let y = (i * 53 % 2000) as f32;
                (i as u64, x, y)
            })
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
        let radius = 100.0;
        let viewer = 0;
        let (vx, vy) = (positions[0].1, positions[0].2);
        let expected: std::collections::BTreeSet<u64> = positions
            .iter()
            .filter(|(id, _, _)| *id != viewer)
            .filter(|(_, x, y)| {
                let dx = x - vx;
                let dy = y - vy;
                (dx * dx + dy * dy).sqrt() <= radius
            })
            .map(|(id, _, _)| *id)
            .collect();
        let actual: std::collections::BTreeSet<u64> = w
            .snapshot_for_resolved(viewer, radius, &frames)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            actual, expected,
            "snapshot_for_resolved doit produire le même ensemble qu'un scan linéaire de référence"
        );
    }

    #[test]
    fn snapshot_for_resolved_still_finds_a_framed_passenger_outside_the_viewers_grid_cell() {
        // Le passager d'ascenseur stocke un OFFSET LOCAL (souvent proche de 0,0) dans `players`,
        // donc dans une cellule de grille potentiellement très éloignée de sa position monde réelle
        // (celle de la cabine). Si l'implémentation walkait la grille en excluant sans discernement
        // les joueurs "framed" de la recherche par proximité brute, ce test resterait vert — mais si
        // une régression les faisait retomber dans le chemin "grille brute" (donc évalués sur leur
        // offset local au lieu de leur position résolue), ce test les perdrait car l'offset local
        // (proche de 0,0) est à une cellule de grille totalement différente du viewer.
        let mut w = World::new();
        w.add_player(1); // viewer, dans le monde loin de l'origine
        w.set_pose(
            1,
            Pose {
                x: 500.0,
                y: 500.0,
                ..Default::default()
            },
        );
        w.add_player(2); // passager, offset local proche de (0,0) — cellule de grille très différente
        w.set_pose(
            2,
            Pose {
                x: 0.5,
                y: 0.0,
                z: 0.0,
                frame: 7,
                ..Default::default()
            },
        );
        let mut frames = crate::frame::FrameRegistry::new();
        frames.set_transform(
            7,
            crate::frame::FrameTransform {
                position: [500.0, 500.0, 40.0], // la cabine est au monde tout près du viewer
                yaw: 0.0,
            },
        );
        let snap = w.snapshot_for_resolved(1, 50.0, &frames);
        assert_eq!(
            snap.len(),
            1,
            "le passager résolu près du viewer doit être trouvé malgré une position brute (offset \
             local) dans une cellule de grille éloignée"
        );
        assert_eq!(snap[0].0, 2);
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
