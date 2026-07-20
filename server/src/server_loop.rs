//! Boucle serveur : draine les events transport, met à jour le World, diffuse les snapshots.
//! Générique sur `Transport` → testable avec `InMemoryTransport`, branché sur GNS en prod.

use crate::transport::{ClientId, Transport, TransportEvent};
use crate::world::{Pose, World};
use flatbuffers::FlatBufferBuilder;
use protocol::*;

pub struct Server {
    world: World,
    aoi_radius: f32,
}

impl Server {
    pub fn new(aoi_radius: f32) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
        }
    }

    /// Nombre de joueurs actuellement dans le monde de ce Shard — pour l'endpoint métriques.
    pub fn player_count(&self) -> usize {
        self.world.player_ids().len()
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
        let mut b = FlatBufferBuilder::new();
        for id in self.world.player_ids() {
            b.reset();
            let bytes = self.encode_snapshot_for(id, &mut b);
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
                                ..self.world.pose_of(from).unwrap_or_default()
                            },
                        );
                    }
                    self.world
                        .set_locomotion(from, pu.locomotion(), pu.move_dir(), pu.flags());
                }
            }
            _ => {}
        }
    }

    fn encode_snapshot_for(&self, viewer: ClientId, b: &mut FlatBufferBuilder) -> Vec<u8> {
        let states: Vec<_> = self
            .world
            .snapshot_for(viewer, self.aoi_radius)
            .into_iter()
            .map(|(id, pose)| {
                let pos = Vec3::new(pose.x, pose.y, pose.z);
                PlayerState::create(
                    b,
                    &PlayerStateArgs {
                        id,
                        position: Some(&pos),
                        yaw: pose.yaw,
                        locomotion: pose.locomotion,
                        move_dir: pose.move_dir,
                        flags: pose.flags,
                        sustained: pose.sustained,
                    },
                )
            })
            .collect();
        let players = b.create_vector(&states);
        let snap = Snapshot::create(
            b,
            &SnapshotArgs {
                tick: self.world.tick(),
                players: Some(players),
            },
        );
        let env = ServerEnvelope::create(
            b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
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
                locomotion: 0,
                move_dir: 0,
                flags: 0,
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
        let mut server = Server::new(1000.0);
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

    fn encode_position_with_locomotion(
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        locomotion: u8,
        move_dir: u8,
    ) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
                locomotion,
                move_dir,
                flags: 0,
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
    fn position_update_carries_locomotion_into_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 3, 42),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.locomotion(), 3);
        assert_eq!(p.move_dir(), 42);
    }

    #[test]
    fn repeated_position_updates_do_not_reset_locomotion_to_idle() {
        // Piège identifié en Task 2 : set_pose remplace toute la Pose. Un deuxième PositionUpdate
        // (même sans nouveau champ de locomotion explicite envoyé par le client, qui renvoie
        // toujours son état courant à chaque update selon la spec §8.1) ne doit jamais faire
        // disparaître la valeur précédente si le client continue de la reporter correctement —
        // ce test vérifie surtout que l'ordre d'application (set_pose puis set_locomotion, ou une
        // fusion) ne perd pas le champ dans le MÊME message.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(6.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.position().unwrap().x(), 6.0);
        assert_eq!(p.locomotion(), 2);
    }

    #[test]
    fn position_update_never_touches_sustained() {
        // Le canal cosmétique continu (locomotion) et la pose tenue (sustained, pilotée par
        // EmoteReport UNIQUEMENT) doivent rester complètement indépendants — un PositionUpdate ne
        // doit jamais remettre sustained à 0 s'il était déjà posé.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // (Task 4 posera sustained via EmoteReport ; ici on vérifie juste qu'un PositionUpdate seul
        // sur un joueur au sustained par défaut à 0 le laisse à 0 - non-régression basique, le test
        // complet d'indépendance vraie est en Task 4 une fois EmoteReport câblé.)
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 1, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    #[test]
    fn players_far_apart_do_not_see_each_other() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();

        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(500.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(
            players.len(),
            0,
            "client 1 est à 500 unités, hors du rayon de 50 — ne doit pas apparaître"
        );
    }
}
