//! Protocole réseau partagé (FlatBuffers). Le code est généré par flatc dans OUT_DIR.
#![allow(clippy::all, unused_imports)]

include!(concat!(env!("OUT_DIR"), "/protocol_generated.rs"));

pub use cyberpunk_rp::protocol::*;

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;

    #[test]
    fn position_update_round_trip() {
        // Build a ClientEnvelope { PositionUpdate { (1,2,3), yaw 0.5 } }
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw: 0.5,
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
        let bytes = b.finished_data().to_vec();

        // Read it back
        let env = flatbuffers::root::<ClientEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), ClientMsg::PositionUpdate);
        let pu = env.msg_as_position_update().unwrap();
        assert_eq!(pu.yaw(), 0.5);
        let p = pu.position().unwrap();
        assert_eq!((p.x(), p.y(), p.z()), (1.0, 2.0, 3.0));
    }
}
