//! Protocole réseau partagé (FlatBuffers). Le code est généré par flatc dans OUT_DIR.

#[allow(clippy::all, unused_imports)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/protocol_generated.rs"));
}
pub use generated::cyberpunk_rp::protocol::*;

#[allow(clippy::all, unused_imports)]
mod generated_internal {
    include!(concat!(env!("OUT_DIR"), "/internal_generated.rs"));
}
pub mod internal {
    pub use super::generated_internal::cyberpunk_rp::internal::*;
}

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
        let bytes = b.finished_data().to_vec();

        // Read it back
        let env = flatbuffers::root::<ClientEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), ClientMsg::PositionUpdate);
        let pu = env.msg_as_position_update().unwrap();
        assert_eq!(pu.yaw(), 0.5);
        let p = pu.position().unwrap();
        assert_eq!((p.x(), p.y(), p.z()), (1.0, 2.0, 3.0));
    }

    #[test]
    fn admin_command_round_trip() {
        let mut b = FlatBufferBuilder::new();
        let text = b.create_string("/promote Compte1 moderator");
        let cmd = AdminCommand::create(&mut b, &AdminCommandArgs { text: Some(text) });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::AdminCommand,
                msg: Some(cmd.as_union_value()),
            },
        );
        b.finish(env, None);
        let bytes = b.finished_data().to_vec();

        let env = flatbuffers::root::<ClientEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), ClientMsg::AdminCommand);
        let cmd = env.msg_as_admin_command().unwrap();
        assert_eq!(cmd.text().unwrap(), "/promote Compte1 moderator");
    }

    #[test]
    fn command_result_round_trip() {
        let mut b = FlatBufferBuilder::new();
        let message = b.create_string("Compte1 promu");
        let cr = CommandResult::create(
            &mut b,
            &CommandResultArgs {
                success: true,
                message: Some(message),
            },
        );
        let env = ServerEnvelope::create(
            &mut b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::CommandResult,
                msg: Some(cr.as_union_value()),
            },
        );
        b.finish(env, None);
        let bytes = b.finished_data().to_vec();

        let env = flatbuffers::root::<ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), ServerMsg::CommandResult);
        let cr = env.msg_as_command_result().unwrap();
        assert!(cr.success());
        assert_eq!(cr.message().unwrap(), "Compte1 promu");
    }

    #[test]
    fn permission_sync_round_trip() {
        let mut b = FlatBufferBuilder::new();
        let node_strs = vec![
            b.create_string("admin.fly"),
            b.create_string("admin.noclip"),
        ];
        let nodes = b.create_vector(&node_strs);
        let sync = PermissionSync::create(&mut b, &PermissionSyncArgs { nodes: Some(nodes) });
        let env = ServerEnvelope::create(
            &mut b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::PermissionSync,
                msg: Some(sync.as_union_value()),
            },
        );
        b.finish(env, None);
        let bytes = b.finished_data().to_vec();

        let env = flatbuffers::root::<ServerEnvelope>(&bytes).unwrap();
        let sync = env.msg_as_permission_sync().unwrap();
        let nodes: Vec<&str> = sync.nodes().unwrap().iter().collect();
        assert_eq!(nodes, vec!["admin.fly", "admin.noclip"]);
    }

    #[test]
    fn internal_client_event_round_trip() {
        use crate::internal::*;
        use flatbuffers::FlatBufferBuilder;
        let mut b = FlatBufferBuilder::new();
        let payload = b.create_vector(&[1u8, 2, 3]);
        let ce = ClientEvent::create(
            &mut b,
            &ClientEventArgs {
                kind: EventKind::Message,
                client_id: 42,
                payload: Some(payload),
            },
        );
        let env = InternalEnvelope::create(
            &mut b,
            &InternalEnvelopeArgs {
                msg_type: InternalMsg::ClientEvent,
                msg: Some(ce.as_union_value()),
            },
        );
        b.finish(env, None);
        let bytes = b.finished_data().to_vec();

        let env = flatbuffers::root::<InternalEnvelope>(&bytes).unwrap();
        let ce = env.msg_as_client_event().unwrap();
        assert_eq!(ce.kind(), EventKind::Message);
        assert_eq!(ce.client_id(), 42);
        assert_eq!(ce.payload().unwrap().bytes(), &[1, 2, 3]);
    }
}
