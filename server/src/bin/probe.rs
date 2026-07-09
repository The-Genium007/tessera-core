//! Client de test GNS : se connecte à un serveur TesseraSynth en ligne, envoie un `Join`,
//! et attend un `Snapshot`. Sert à PROUVER de bout en bout qu'un vrai serveur déployé accepte
//! une connexion et parle le protocole. Build : `--features gns`.
//!
//!   cargo run -p server --features gns --bin probe -- 51.38.189.234:27020

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

    let addr = std::env::args().nth(1).expect("usage: probe <ip:port>");
    let (ip_s, port_s) = addr.rsplit_once(':').expect("format attendu ip:port");
    let ip = Ipv4Addr::from_str(ip_s).expect("IPv4 invalide");
    let port: u16 = port_s.parse().expect("port invalide");

    println!("→ Connexion à {addr} ...");
    let g = GnsGlobal::get().expect("GnsGlobal::get");
    let client: GnsSocket<IsClient> = GnsSocket::new(g).connect(ip.into(), port).expect("connect");

    let mut connected = false;
    let mut join_sent = false;
    let mut got_snapshot = false;
    let deadline = Instant::now() + Duration::from_secs(8);

    while Instant::now() < deadline && !got_snapshot {
        g.poll_callbacks();

        for ev in client.receive_events() {
            match ev.info().state() {
                State::k_ESteamNetworkingConnectionState_Connected => {
                    if !connected {
                        connected = true;
                        println!("✅ CONNECTÉ au serveur en ligne");
                    }
                }
                State::k_ESteamNetworkingConnectionState_ClosedByPeer
                | State::k_ESteamNetworkingConnectionState_ProblemDetectedLocally => {
                    println!("❌ Connexion fermée/refusée par le serveur");
                }
                _ => {}
            }
        }

        if connected && !join_sent {
            let mut b = FlatBufferBuilder::new();
            let name = b.create_string("probe-mac");
            let join = Join::create(
                &mut b,
                &JoinArgs {
                    display_name: Some(name),
                    token: None,
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
            println!("→ Join envoyé, attente d'un Snapshot ...");
        }

        for m in client.receive_messages::<64>().expect("receive") {
            if let Ok(env) = flatbuffers::root::<ServerEnvelope>(m.payload()) {
                if let Some(snap) = env.msg_as_snapshot() {
                    got_snapshot = true;
                    let others = snap.players().map(|p| p.len()).unwrap_or(0);
                    println!(
                        "✅ SNAPSHOT reçu — tick {}, {} autre(s) joueur(s)",
                        snap.tick(),
                        others
                    );
                }
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    println!("\n========================================");
    if connected && got_snapshot {
        println!(
            "🎉 PREUVE ABSOLUE : un vrai client a établi une connexion ET échangé le protocole"
        );
        println!("   avec le serveur déployé sur {addr}, à travers internet.");
        std::process::exit(0);
    } else if connected {
        println!("✅ Connexion établie (pas de snapshot dans le temps imparti).");
        std::process::exit(0);
    } else {
        println!(
            "❌ Aucune connexion établie (timeout). Le port répond mais pas le protocole GNS ?"
        );
        std::process::exit(1);
    }
}
