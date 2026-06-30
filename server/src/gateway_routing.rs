//! Logique de routage du Gateway (M3) : extraire la position du protocole client + assigner les
//! clients aux shards selon leur 1re position. Pur, testable sans GNS/TCP.

use protocol::{ClientEnvelope, ClientMsg};

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
    fn extract_position_reads_position_update_and_ignores_join() {
        assert_eq!(
            extract_position(&client_position(2387.0, -1295.0, 63.0)),
            Some((2387.0, -1295.0, 63.0))
        );
        assert_eq!(extract_position(&client_join()), None);
        assert_eq!(extract_position(&[0, 1, 2]), None); // garbage → None
    }
}
