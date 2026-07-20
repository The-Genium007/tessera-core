//! État autoritaire minimal : joueurs connectés et leurs positions.

use crate::transport::ClientId;
use std::collections::BTreeMap;

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

#[derive(Default)]
pub struct World {
    players: BTreeMap<ClientId, Pose>,
    tick: u64,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_player(&mut self, id: ClientId) {
        self.players.entry(id).or_default();
    }

    pub fn remove_player(&mut self, id: ClientId) {
        self.players.remove(&id);
    }

    pub fn set_pose(&mut self, id: ClientId, pose: Pose) {
        if let Some(p) = self.players.get_mut(&id) {
            *p = pose;
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

    /// Snapshot vu par `viewer` : les autres joueurs à `radius` ou moins (distance 2D, Z ignoré
    /// — cohérent avec `Aabb` qui ignore aussi Z pour la géométrie de sharding).
    pub fn snapshot_for(&self, viewer: ClientId, radius: f32) -> Vec<(ClientId, Pose)> {
        let Some(&viewer_pose) = self.players.get(&viewer) else {
            return Vec::new();
        };
        self.players
            .iter()
            .filter(|(id, _)| **id != viewer)
            .filter(|(_, pose)| {
                let dx = pose.x - viewer_pose.x;
                let dy = pose.y - viewer_pose.y;
                (dx * dx + dy * dy).sqrt() <= radius
            })
            .map(|(id, pose)| (*id, *pose))
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
        w.set_pose(1, Pose { x: 5.0, y: 0.0, z: 0.0, yaw: 1.0, ..Default::default() });
        w.set_locomotion(1, 2, 10, 0);
        let snap = w.snapshot_for(2, 1000.0);
        assert_eq!(snap.len(), 1);
        let (_, pose) = snap[0];
        assert_eq!(pose.x, 5.0, "la position ne doit pas être affectée par set_locomotion");
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
}
