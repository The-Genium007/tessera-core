//! Client « fantôme » de démo : se connecte au serveur TesseraSynth live, envoie `Join`, puis
//! diffuse une `PositionUpdate` qui OSCILLE autour d'un point — pour qu'un VRAI joueur en jeu
//! voie un avatar apparaître ET bouger à côté de lui (tranche verticale 0-D : « voir l'autre »).
//!
//! Build : `--features gns`. Usage :
//!   cargo run -p server --features gns --bin ghost -- 51.38.189.234:27020 2387.47 -1295.70 63.33
//! (x y z = centre de l'oscillation ; défaut = position de test du joueur).

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}

#[cfg(feature = "gns")]
fn main() {
    use flatbuffers::FlatBufferBuilder;
    use gns::{
        sys::ESteamNetworkingConnectionState as State, GnsGlobal, GnsSocket, IsClient, SendFlags,
    };
    use protocol::*;
    use std::net::Ipv4Addr;
    use std::str::FromStr;
    use std::time::{Duration, Instant};

    let mut args = std::env::args().skip(1);
    let addr = args.next().expect("usage: ghost <ip:port> [x y z]");
    let cx: f32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(2387.47);
    let cy: f32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(-1295.70);
    let cz: f32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(63.33);

    let (ip_s, port_s) = addr.rsplit_once(':').expect("format attendu ip:port");
    let ip = Ipv4Addr::from_str(ip_s).expect("IPv4 invalide");
    let port: u16 = port_s.parse().expect("port invalide");

    println!("→ Fantôme : connexion à {addr}, oscillation autour de ({cx}, {cy}, {cz}) ...");
    let g = GnsGlobal::get().expect("GnsGlobal::get");
    let client: GnsSocket<IsClient> = GnsSocket::new(g).connect(ip.into(), port).expect("connect");

    let mut connected = false;
    let mut join_sent = false;
    let start = Instant::now();
    let run_for = Duration::from_secs(120);
    let mut last_pos = Instant::now();

    while start.elapsed() < run_for {
        g.poll_callbacks();

        for ev in client.receive_events() {
            match ev.info().state() {
                State::k_ESteamNetworkingConnectionState_Connected => {
                    if !connected {
                        connected = true;
                        println!("✅ Fantôme CONNECTÉ");
                    }
                }
                State::k_ESteamNetworkingConnectionState_ClosedByPeer
                | State::k_ESteamNetworkingConnectionState_ProblemDetectedLocally => {
                    println!("❌ Connexion fermée par le serveur");
                    return;
                }
                _ => {}
            }
        }

        if connected && !join_sent {
            let mut b = FlatBufferBuilder::new();
            let name = b.create_string("Fantome-Tessera");
            let hwid_hash = b.create_string("");
            let join = Join::create(
                &mut b,
                &JoinArgs {
                    display_name: Some(name),
                    token: None,
                    protocol_version: server::gateway_routing::CURRENT_PROTOCOL_VERSION,
                    hwid_hash: Some(hwid_hash),
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
            let msg = g.utils().allocate_message(
                client.connection(),
                SendFlags::RELIABLE,
                b.finished_data().to_vec(),
            );
            client.send_messages(vec![msg]);
            join_sent = true;
            println!("→ Join envoyé. Diffusion des positions (oscillation) ...");
        }

        // ~20 Hz : envoie une position qui oscille de ±3 m en X, yaw qui tourne.
        if connected && last_pos.elapsed() >= Duration::from_millis(50) {
            last_pos = Instant::now();
            let t = start.elapsed().as_secs_f32();
            let x = cx + 3.0 * (t * 0.8).sin();
            let yaw = (t * 60.0) % 360.0;

            let mut b = FlatBufferBuilder::new();
            let pos = Vec3::new(x, cy, cz);
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
            let msg = g.utils().allocate_message(
                client.connection(),
                SendFlags::UNRELIABLE,
                b.finished_data().to_vec(),
            );
            client.send_messages(vec![msg]);
        }

        // Draine les snapshots (pour ne pas saturer) et montre combien d'autres joueurs on voit.
        for m in client.receive_messages::<64>().expect("receive") {
            if let Ok(env) = flatbuffers::root::<ServerEnvelope>(m.payload()) {
                if let Some(snap) = env.msg_as_snapshot() {
                    let others = snap.players().map(|p| p.len()).unwrap_or(0);
                    if others > 0 {
                        println!("  (le fantôme voit {others} autre(s) joueur(s) — donc TOI)");
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    println!("⏹  Fantôme déconnecté (fin des 120 s).");
}
