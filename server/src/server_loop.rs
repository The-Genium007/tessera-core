//! Boucle serveur : draine les events transport, met à jour le World, diffuse les snapshots.
//! Générique sur `Transport` → testable avec `InMemoryTransport`, branché sur GNS en prod.

use crate::transport::{ClientId, Transport, TransportEvent};
use crate::world::{Pose, World};
use flatbuffers::FlatBufferBuilder;
use protocol::*;

pub struct Server {
    world: World,
}

impl Server {
    pub fn new() -> Self {
        Self {
            world: World::new(),
        }
    }

    /// Un tick : applique les events entrants, avance le monde, envoie un snapshot à chaque client.
    pub fn tick<T: Transport>(&mut self, transport: &mut T) {
        for ev in transport.poll() {
            match ev {
                TransportEvent::Connected(id) => self.world.add_player(id),
                TransportEvent::Disconnected(id) => self.world.remove_player(id),
                TransportEvent::Message { from, data } => self.apply_client_message(from, &data),
            }
        }
        self.world.advance_tick();
        for id in self.world.player_ids() {
            let bytes = self.encode_snapshot_for(id);
            transport.send(id, &bytes);
        }
    }

    fn apply_client_message(&mut self, from: ClientId, data: &[u8]) {
        let Ok(env) = flatbuffers::root::<ClientEnvelope>(data) else {
            return;
        };
        match env.msg_type() {
            ClientMsg::Join => { /* TODO(Phase-1): stocker le display_name */ }
            ClientMsg::PositionUpdate => {
                if let Some(pu) = env.msg_as_position_update() {
                    if let Some(p) = pu.position() {
                        self.world.set_pose(
                            from,
                            Pose {
                                x: p.x(),
                                y: p.y(),
                                z: p.z(),
                                yaw: pu.yaw(),
                            },
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn encode_snapshot_for(&self, viewer: ClientId) -> Vec<u8> {
        // TODO(perf, Phase 2): réutiliser un FlatBufferBuilder par tick (reset()) au lieu d'en allouer un par joueur.
        let mut b = FlatBufferBuilder::new();
        let states: Vec<_> = self
            .world
            .snapshot_for(viewer)
            .into_iter()
            .map(|(id, pose)| {
                let pos = Vec3::new(pose.x, pose.y, pose.z);
                PlayerState::create(
                    &mut b,
                    &PlayerStateArgs {
                        id,
                        position: Some(&pos),
                        yaw: pose.yaw,
                    },
                )
            })
            .collect();
        let players = b.create_vector(&states);
        let snap = Snapshot::create(
            &mut b,
            &SnapshotArgs {
                tick: self.world.tick(),
                players: Some(players),
            },
        );
        let env = ServerEnvelope::create(
            &mut b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;

    fn encode_position(x: f32, y: f32, z: f32, yaw: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn two_clients_see_each_other_move() {
        let mut server = Server::new();
        let mut t = InMemoryTransport::new();

        // Deux clients se connectent.
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // Client 1 bouge en (5,0,0).
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(5.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        // Le client 2 doit recevoir un snapshot contenant le joueur 1 en x=5.
        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1, "un snapshot envoyé au client 2");
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(players.len(), 1);
        let p = players.get(0);
        assert_eq!(p.id(), 1);
        assert_eq!(p.position().unwrap().x(), 5.0);

        let sent_to_1 = t.take_sent(1);
        assert_eq!(sent_to_1.len(), 1, "un snapshot envoyé au client 1");
        let env1 = flatbuffers::root::<ServerEnvelope>(&sent_to_1[0]).unwrap();
        let snap1 = env1.msg_as_snapshot().unwrap();
        let players1 = snap1.players().unwrap();
        assert_eq!(players1.len(), 1);
        assert_eq!(players1.get(0).id(), 2); // client 1 voit client 2
    }
}
