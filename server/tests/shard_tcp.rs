//! Pilote le binaire Shard via une connexion TCP brute, comme le ferait le Gateway.
use protocol::internal::{EventKind, InternalEnvelope};
use server::framing::FrameReader;
use server::internal_net::encode_client_event;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// Encode un ClientEnvelope{PositionUpdate} client (payload opaque relayé).
fn client_position(x: f32) -> Vec<u8> {
    use flatbuffers::FlatBufferBuilder;
    use protocol::*;
    let mut b = FlatBufferBuilder::new();
    let pos = Vec3::new(x, 0.0, 0.0);
    let pu = PositionUpdate::create(
        &mut b,
        &PositionUpdateArgs {
            position: Some(&pos),
            yaw: 0.0,
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

#[tokio::test]
async fn shard_relays_snapshots_over_tcp() {
    // Lance le shard sur un port de test dans une tâche.
    let addr = "127.0.0.1:27130";
    tokio::spawn(async move {
        server::shard_main(addr, 1000.0, "127.0.0.1:0", None, None, None, None)
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut sock = TcpStream::connect(addr).await.unwrap();

    // Gateway → Shard : client 1 et 2 connectés, client 1 bouge en x=5.
    sock.write_all(&encode_client_event(EventKind::Connected, 1, &[]))
        .await
        .unwrap();
    sock.write_all(&encode_client_event(EventKind::Connected, 2, &[]))
        .await
        .unwrap();
    sock.write_all(&encode_client_event(
        EventKind::Message,
        1,
        &client_position(5.0),
    ))
    .await
    .unwrap();

    // Lit les frames sortants jusqu'à voir un ServerSend pour le client 2 contenant le joueur 1 en x=5.
    let mut reader = FrameReader::new();
    let mut buf = [0u8; 4096];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout : pas de snapshot attendu"
        );
        let n = tokio::time::timeout(Duration::from_millis(500), sock.read(&mut buf))
            .await
            .expect("read timeout")
            .unwrap();
        reader.push(&buf[..n]);
        while let Some(body) = reader.next_frame() {
            let env = flatbuffers::root::<InternalEnvelope>(&body).unwrap();
            let ss = env.msg_as_server_send().unwrap();
            if ss.client_id() != 2 {
                continue;
            }
            let payload = ss.payload().unwrap().bytes();
            let senv = flatbuffers::root::<protocol::ServerEnvelope>(payload).unwrap();
            let snap = senv.msg_as_snapshot().unwrap();
            if let Some(players) = snap.players() {
                if players.len() == 1
                    && players.get(0).id() == 1
                    && players.get(0).position().unwrap().x() == 5.0
                {
                    return; // succès
                }
            }
        }
    }
}
