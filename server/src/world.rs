//! État autoritaire minimal : joueurs connectés et leurs positions.

use crate::transport::ClientId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
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
            },
        );
        w.set_pose(
            3,
            Pose {
                x: 500.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
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
}
