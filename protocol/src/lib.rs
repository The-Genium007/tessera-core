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

    /// Aller-retour d'un `PositionUpdate` sur le fil.
    ///
    /// ⚠️ Ce test est resté PÉRIMÉ du 2026-07-23 (gel de protocole palier 2 : `Vec3`→`QVec3`,
    /// yaw `float`→`ushort`, ajout de `frame`/`slot` pour le modèle de repère ADR 0013) au
    /// 2026-07-30 — sept jours pendant lesquels `cargo test -p protocol` NE COMPILAIT PAS, donc
    /// le workflow `server-image` échouait à chaque push sur `main` sans que personne ne le relie
    /// au gel. La cause est bête : on teste d'habitude `-p server`, et le crate `protocol` n'a
    /// que ces quelques tests, qu'on ne pense pas à lancer.
    ///
    /// `frame`/`slot` sont posés EXPLICITEMENT à 0 (= repère monde) plutôt que laissés au défaut :
    /// c'est la règle du schéma pour tout champ dont le défaut est significatif — un champ omis
    /// compile sans broncher et ment sur le fil.
    #[test]
    fn position_update_round_trip() {
        let mut b = FlatBufferBuilder::new();
        // Positions quantifiées : mètres × 131072 (2^17), cf. `QVec3` dans le schéma.
        let pos = QVec3::new(1 << 17, 2 << 17, 3 << 17);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw: 12345,
                locomotion: 0,
                move_dir: 0,
                flags: 0,
                frame: 0,
                slot: 0,
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
        assert_eq!(pu.yaw(), 12345);
        assert_eq!(pu.frame(), 0, "repère monde posé explicitement");
        assert_eq!(pu.slot(), 0);
        let p = pu.position().unwrap();
        assert_eq!((p.x(), p.y(), p.z()), (1 << 17, 2 << 17, 3 << 17));
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
