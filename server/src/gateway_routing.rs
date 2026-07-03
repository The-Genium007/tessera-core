//! Logique de routage du Gateway (M3) : extraire la position du protocole client + assigner les
//! clients aux shards selon leur 1re position. Pur, testable sans GNS/TCP.

use protocol::{
    ClientEnvelope, ClientEnvelopeArgs, ClientMsg, PositionUpdate, PositionUpdateArgs, Vec3,
};

/// Décode un `ClientEnvelope` client ; si c'est un `PositionUpdate`, renvoie sa position.
pub fn extract_position(client_payload: &[u8]) -> Option<(f32, f32, f32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::PositionUpdate {
        return None;
    }
    let pu = env.msg_as_position_update()?;
    let p = pu.position()?;
    Some((p.x(), p.y(), p.z()))
}

/// Construit le payload client d'un `PositionUpdate` — utilisé pour re-semer, sur un shard qui
/// vient de perdre son état, la dernière position connue d'un client par le Gateway (le client
/// réel n'a pas renvoyé cette position, elle est reconstruite depuis `last_pos`). Yaw à 0 : une
/// orientation temporairement fausse s'auto-corrige au prochain vrai `PositionUpdate` du client.
pub fn encode_position_update(pos: [f32; 3]) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let p = Vec3::new(pos[0], pos[1], pos[2]);
    let pu = PositionUpdate::create(
        &mut b,
        &PositionUpdateArgs {
            position: Some(&p),
            yaw: 0.0,
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

/// Décode un `ClientEnvelope` client ; si c'est un `Join`, renvoie son `display_name`.
pub fn extract_join_name(client_payload: &[u8]) -> Option<String> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::Join {
        return None;
    }
    let join = env.msg_as_join()?;
    join.display_name().map(|s| s.to_string())
}

use crate::internal_net::event_to_client_event_frame;
use crate::transport::{ClientId, TransportEvent};
use std::collections::HashMap;

/// Action décidée par l'assigneur pour un événement client.
#[derive(Debug)]
pub enum AssignAction {
    /// Événement bufferisé (client pas encore assigné, pas de position).
    Buffered,
    /// Une position est arrivée pour un client non assigné → le caller doit interroger le Router
    /// avec (x,y,z) puis appeler `assign(client_id, shard)`.
    NeedRoute {
        client_id: ClientId,
        x: f32,
        y: f32,
        z: f32,
    },
    /// Frames `ClientEvent` à écrire au shard indiqué.
    Forward { shard: String, frames: Vec<Vec<u8>> },
}

/// Suit l'état de chaque client : non assigné (buffer) → assigné à un shard.
#[derive(Default)]
pub struct ShardAssigner {
    assigned: HashMap<ClientId, String>,
    buffer: HashMap<ClientId, Vec<TransportEvent>>,
}

impl ShardAssigner {
    pub fn new() -> Self {
        Self::default()
    }

    fn client_of(ev: &TransportEvent) -> ClientId {
        match ev {
            TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
            TransportEvent::Message { from, .. } => *from,
        }
    }

    pub fn feed(&mut self, ev: TransportEvent) -> AssignAction {
        let id = Self::client_of(&ev);

        // Déjà assigné → on relaie directement vers son shard.
        if let Some(shard) = self.assigned.get(&id) {
            return AssignAction::Forward {
                shard: shard.clone(),
                frames: vec![event_to_client_event_frame(&ev)],
            };
        }

        // Non assigné : si c'est une position, on bufferise ET on demande un routage.
        if let TransportEvent::Message { from, data } = &ev {
            if let Some((x, y, z)) = extract_position(data) {
                let from = *from;
                self.buffer.entry(id).or_default().push(ev);
                return AssignAction::NeedRoute {
                    client_id: from,
                    x,
                    y,
                    z,
                };
            }
        }

        // Sinon (Connected/Join/autre) : on bufferise et on attend.
        self.buffer.entry(id).or_default().push(ev);
        AssignAction::Buffered
    }

    pub fn assign(&mut self, client_id: ClientId, shard: String) -> AssignAction {
        self.assigned.insert(client_id, shard.clone());
        let pending = self.buffer.remove(&client_id).unwrap_or_default();
        let frames = pending.iter().map(event_to_client_event_frame).collect();
        AssignAction::Forward { shard, frames }
    }

    pub fn forget(&mut self, client_id: ClientId) {
        self.assigned.remove(&client_id);
        self.buffer.remove(&client_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;
    use protocol::*;

    fn client_position(x: f32, y: f32, z: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw: 0.0,
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

    fn client_join() -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let name = b.create_string("v");
        let join = Join::create(
            &mut b,
            &JoinArgs {
                display_name: Some(name),
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::Join,
                msg: Some(join.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn encode_position_update_round_trips_through_extract_position() {
        let payload = encode_position_update([2387.0, -1295.0, 63.0]);
        assert_eq!(extract_position(&payload), Some((2387.0, -1295.0, 63.0)));
    }

    #[test]
    fn extract_position_reads_position_update_and_ignores_join() {
        assert_eq!(
            extract_position(&client_position(2387.0, -1295.0, 63.0)),
            Some((2387.0, -1295.0, 63.0))
        );
        assert_eq!(extract_position(&client_join()), None);
        assert_eq!(extract_position(&[0, 1, 2]), None); // garbage → None
    }

    #[test]
    fn extract_join_name_reads_display_name() {
        assert_eq!(extract_join_name(&client_join()), Some("v".to_string()));
        assert_eq!(extract_join_name(&client_position(1.0, 2.0, 3.0)), None); // pas un Join
        assert_eq!(extract_join_name(&[9, 9, 9]), None); // garbage
    }

    use crate::framing::FrameReader;
    use crate::internal_net::decode_client_event;
    use crate::transport::TransportEvent;

    #[test]
    fn buffers_until_first_position_then_flushes_on_assign() {
        let mut a = ShardAssigner::new();
        // Connecté + Join : bufferisés, rien à router encore.
        assert!(matches!(
            a.feed(TransportEvent::Connected(1)),
            AssignAction::Buffered
        ));
        assert!(matches!(
            a.feed(TransportEvent::Message {
                from: 1,
                data: client_join()
            }),
            AssignAction::Buffered
        ));
        // 1re position : besoin de router (et l'événement est bufferisé).
        match a.feed(TransportEvent::Message {
            from: 1,
            data: client_position(500.0, 0.0, 0.0),
        }) {
            AssignAction::NeedRoute { client_id, x, .. } => {
                assert_eq!(client_id, 1);
                assert_eq!(x, 500.0);
            }
            other => panic!("attendu NeedRoute, eu {other:?}"),
        }
        // Le caller a interrogé le Router → shard "A". assign flushe les 3 événements framés.
        let act = a.assign(1, "A".to_string());
        let AssignAction::Forward { shard, frames } = act else {
            panic!("attendu Forward")
        };
        assert_eq!(shard, "A");
        assert_eq!(frames.len(), 3); // Connected + Join + Position
                                     // Le 1er frame est un ClientEvent décodable = Connected(1).
        let mut r = FrameReader::new();
        r.push(&frames[0]);
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Connected(1))
        );
    }

    #[test]
    fn assigned_client_forwards_directly_to_its_shard() {
        let mut a = ShardAssigner::new();
        a.feed(TransportEvent::Message {
            from: 2,
            data: client_position(2000.0, 0.0, 0.0),
        }); // NeedRoute
        a.assign(2, "B".to_string());
        // Désormais assigné : un nouvel événement part direct vers B.
        match a.feed(TransportEvent::Message {
            from: 2,
            data: vec![1, 2, 3],
        }) {
            AssignAction::Forward { shard, frames } => {
                assert_eq!(shard, "B");
                assert_eq!(frames.len(), 1);
            }
            other => panic!("attendu Forward, eu {other:?}"),
        }
    }
}
