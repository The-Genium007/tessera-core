//! Test d'intégration (feature `gns`) : preuve BOUT-EN-BOUT, sur un vrai transport réseau, que le
//! pont Shard→Gateway générique (spec véhicules autonomes §5, Tasks 4-5) fonctionne réellement —
//! un véhicule autonome simulé côté Shard, dont le chemin planifié franchit une frontière de shard,
//! est vu par un VRAI client GNS dans le vecteur `vehicles` de ses snapshots, avant ET après le
//! franchissement, sans coupure prolongée.
//!
//! ## Le gap couvert
//!
//! Jusqu'ici, le pont véhicule→handoff n'était couvert que par :
//! - des tests purs/logiques (`shard_boundary_bridge.rs`, `server_loop.rs`, `snapshot_merge.rs`,
//!   `gateway.rs` — `should_report_position`, `take_pending_entity_reports`, la fusion des
//!   véhicules, le décodage d'`EntityPositionReport` — tous EN MÉMOIRE, zéro octet sur le réseau) ;
//! - `handoff_two_real_clients.rs`, qui exerce le vrai réseau mais avec deux JOUEURS, pas un
//!   véhicule simulé.
//!
//! Jamais une entité PUREMENT SIMULÉE côté Shard (qui n'a aucune connexion réseau propre, aucun
//! Join, aucun `PositionUpdate` de client réel derrière elle) n'avait déclenché un handoff observé
//! de bout en bout par un vrai client GNS. C'est précisément la nouveauté du pont §5 : une entité
//! sans connexion réseau réelle traverse une frontière et reste visible.
//!
//! ## Pourquoi ce test est déterministe (documentation vivante, pas un garde-fou silencieux)
//!
//! - Les véhicules ne sont PAS filtrés par l'AoI dans le snapshot d'un Shard (`encode_snapshot_for`,
//!   server_loop.rs : `vehicle_states` itère TOUT le registre, contrairement aux joueurs). Le Shard
//!   A porte le véhicule dès son spawn et jamais ne le retire (pas de despawn en v1) — l'observateur
//!   chargé sur A le voit donc à CHAQUE snapshot de A, quelle que soit la distance. C'est ce qui rend
//!   la visibilité continue robuste : on n'observe pas une fenêtre étroite d'AoI mais la présence
//!   pérenne du véhicule sur le Shard qui le simule.
//! - L'observateur est placé à x=950, soit 50 unités de la frontière x=1000 — au-delà du tampon
//!   (`BUFFER_RADIUS = 25`). Il n'est donc chargé QUE sur le Shard A et n'y déclenche lui-même aucun
//!   handoff : sa vue du véhicule ne dépend que du Shard A, stable pendant toute la traversée.
//! - Le graphe de nav place un waypoint PILE sur la frontière (x=1000). `should_report_position`
//!   déclenche un rapport quand le véhicule est à `vitesse × lookahead = 8 × 2 = 16` unités de son
//!   prochain waypoint : le véhicule, parti à x=985 (15 unités du waypoint x=1000), émet donc son
//!   rapport prédictif AVANT d'avoir franchi x=1000 (spec §5 : "sait à l'avance où/quand"). Le
//!   Gateway charge alors le Shard B en préchargement — mais la visibilité de l'observateur, elle,
//!   ne dépend que du Shard A (cf. ci-dessus), donc le test reste vrai que le préchargement B
//!   arrive tôt ou tard.
//! - `merge_snapshots` (Task 7bis, commit 381ae19) préserve désormais le vecteur `vehicles` — sans
//!   cette correction, l'observateur ne verrait AUCUN véhicule (le Gateway applique toujours la
//!   fusion, même sur un seul shard). Ce test est le canari réseau réel de cette propriété.

#![cfg(feature = "gns")]

use flatbuffers::FlatBufferBuilder;
use gns::{
    sys::ESteamNetworkingConnectionState as State, GnsGlobal, GnsSocket, IsClient, SendFlags,
};
use protocol::*;
use server::handoff::{CellZone, RadiusPolicy, ShardTopology, ShardZone};
use server::nav_graph::Vec3 as NavVec3;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// Frontière à x=1000, A = x<1000, B = x>=1000 — même construction que `two_shards()` dans
/// `handoff_two_real_clients.rs` (bornes "infinies" de l'ancien `Aabb` approximées par un grand
/// rectangle fini pour le point-in-polygon Voronoï).
const FAR: f64 = 1_000_000.0;

/// Rayon de tampon — identique aux tests purs `two_shards()` de `handoff.rs` et à
/// `handoff_two_real_clients.rs`. L'observateur, à 50 unités de la frontière, est HORS de ce tampon
/// (chargé sur A seul), ce qui isole sa vue du véhicule au seul Shard A.
const BUFFER_RADIUS: f32 = 25.0;

fn two_shards(addr_a: &str, addr_b: &str) -> ShardTopology {
    ShardTopology {
        shards: vec![
            ShardZone {
                id: addr_a.to_string(),
                addr: addr_a.to_string(),
                cells: vec![(
                    CellZone {
                        boundary_rings: vec![vec![
                            [-FAR, -FAR],
                            [1000.0, -FAR],
                            [1000.0, FAR],
                            [-FAR, FAR],
                            [-FAR, -FAR],
                        ]],
                    },
                    BUFFER_RADIUS,
                )],
            },
            ShardZone {
                id: addr_b.to_string(),
                addr: addr_b.to_string(),
                cells: vec![(
                    CellZone {
                        boundary_rings: vec![vec![
                            [1000.0, -FAR],
                            [FAR, -FAR],
                            [FAR, FAR],
                            [1000.0, FAR],
                            [1000.0, -FAR],
                        ]],
                    },
                    BUFFER_RADIUS,
                )],
            },
        ],
    }
}

fn encode_join(name: &str) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let n = b.create_string(name);
    let hwid_hash = b.create_string("");
    let join = Join::create(
        &mut b,
        &JoinArgs {
            display_name: Some(n),
            token: None,
            protocol_version: server::gateway_routing::CURRENT_PROTOCOL_VERSION,
            hwid_hash: Some(hwid_hash),
            space_id: 0,
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

fn encode_position(x: f32, y: f32, z: f32) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let pos = QVec3::new(
        server::quant::q_pos(x),
        server::quant::q_pos(y),
        server::quant::q_pos(z),
    );
    let pu = PositionUpdate::create(
        &mut b,
        &PositionUpdateArgs {
            position: Some(&pos),
            yaw: 0,
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
    b.finished_data().to_vec()
}

/// Décode un `ServerEnvelope`/`Snapshot` et renvoie l'UNIQUE véhicule vu (id, x, y, z), ou `None` si
/// le snapshot n'en contient aucun. Ce test ne fait naître qu'un seul véhicule, donc plus d'une
/// entrée signalerait un doublon de fusion (le bug que `deduplicates_vehicle_present_on_both_shards`
/// couvre en mémoire — ici on l'exclut aussi sur le vrai réseau). Séparé de la lecture des joueurs :
/// on assert sur `snap.vehicles()`, pas `snap.players()`.
fn vehicle_from_snapshot(payload: &[u8]) -> Option<(u64, f32, f32, f32)> {
    let env = flatbuffers::root::<ServerEnvelope>(payload).ok()?;
    let snap = env.msg_as_snapshot()?;
    let vehicles = snap.vehicles()?;
    assert!(
        vehicles.len() <= 1,
        "un seul véhicule fait naître dans ce test — {} vus dans un même snapshot signalerait un \
         doublon de fusion cross-shard (ids: {:?})",
        vehicles.len(),
        (0..vehicles.len())
            .map(|i| vehicles.get(i).id())
            .collect::<Vec<_>>()
    );
    if vehicles.is_empty() {
        return None;
    }
    let v = vehicles.get(0);
    let pos = v.position()?;
    Some((
        v.id(),
        server::quant::dq_pos(pos.x()),
        server::quant::dq_pos(pos.y()),
        server::quant::dq_pos(pos.z()),
    ))
}

/// État de l'observateur, mis à jour à chaque pompage de la boucle de poll.
#[derive(Default)]
struct VehicleWatch {
    connected: bool,
    /// Dernier véhicule vu (id, x, y, z).
    latest_vehicle: Option<(u64, f32, f32, f32)>,
    /// Horodatage de la dernière fois où le véhicule a été vu — pour détecter une disparition
    /// prolongée (pas juste un trou d'un tick dû à la livraison UNRELIABLE des snapshots).
    last_seen_at: Option<Instant>,
    /// A-t-on vu le véhicule à un x < 1000 (avant franchissement) au moins une fois ?
    seen_before_boundary: bool,
    /// A-t-on vu le véhicule à un x >= 1000 (après franchissement) au moins une fois ?
    seen_after_boundary: bool,
}

/// Tolérance de disparition — identique à `handoff_two_real_clients.rs` : un trou plus court est
/// imputable à la livraison UNRELIABLE des snapshots Gateway→client, pas à un bug de handoff.
const DISAPPEARANCE_GRACE: Duration = Duration::from_millis(1500);

/// Frontière observée : le waypoint intermédiaire du véhicule est PILE dessus.
const BOUNDARY_X: f32 = 1000.0;

/// Pompe les événements/messages GNS de l'observateur et met à jour son `VehicleWatch`. Panique
/// immédiatement si le véhicule disparaît plus longtemps que `DISAPPEARANCE_GRACE` après avoir déjà
/// été vu — c'est le cœur de l'assertion "pas de coupure au franchissement".
fn pump_observer(client: &GnsSocket<IsClient>, watch: &mut VehicleWatch) {
    for ev in client.receive_events() {
        match ev.info().state() {
            State::k_ESteamNetworkingConnectionState_Connected => watch.connected = true,
            State::k_ESteamNetworkingConnectionState_ClosedByPeer
            | State::k_ESteamNetworkingConnectionState_ProblemDetectedLocally => {
                panic!("observateur : connexion fermée/refusée de façon inattendue par le Gateway");
            }
            _ => {}
        }
    }
    for m in client.receive_messages::<64>().expect("receive_messages") {
        if let Some(vehicle) = vehicle_from_snapshot(m.payload()) {
            let (_, x, _, _) = vehicle;
            if x < BOUNDARY_X {
                watch.seen_before_boundary = true;
            } else {
                watch.seen_after_boundary = true;
            }
            watch.latest_vehicle = Some(vehicle);
            watch.last_seen_at = Some(Instant::now());
        } else if watch.latest_vehicle.is_some() {
            // Snapshot reçu mais sans véhicule : disparition trop longue ?
            if let Some(last) = watch.last_seen_at {
                assert!(
                    last.elapsed() <= DISAPPEARANCE_GRACE,
                    "l'observateur a cessé de voir le véhicule pendant {:?} (> tolérance de \
                     {DISAPPEARANCE_GRACE:?}) — coupure de visibilité au franchissement",
                    last.elapsed()
                );
            }
        }
    }
}

// `GnsTransport` (utilisé en interne par `gateway_main`) contient des pointeurs bruts et n'est donc
// pas `Send` : il faut passer par un `LocalSet`/`spawn_local` pour faire tourner un `gateway_main`
// réel concurremment au corps du test, dans le même thread (même contrainte que
// `handoff_two_real_clients.rs`).
#[tokio::test]
async fn a_vehicle_crossing_a_shard_boundary_stays_visible_to_a_real_gns_client() {
    let local = tokio::task::LocalSet::new();
    local.run_until(run_test()).await;
}

async fn run_test() {
    // Sans subscriber, tout tracing::warn!/info! du Gateway/Shard est silencieusement avalé —
    // lancer avec `RUST_LOG=info ... -- --nocapture` pour voir les lignes de Handoff/pont en cas
    // d'échec.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Ports DÉDIÉS à ce test (distincts de handoff_two_real_clients.rs : 27150/27151/27160, de
    // shard_tcp.rs : 27130, shard_addr_routing.rs : 27131, shard.rs : 27131/27132) pour éviter toute
    // collision si ces binaires de test tournent en parallèle dans le même run cargo.
    let shard_a_addr = "127.0.0.1:27170";
    let shard_b_addr = "127.0.0.1:27171";
    let gateway_addr = "127.0.0.1:27180";

    let tmp = tempfile::tempdir().expect("tempdir");
    // Safety: mutation de variables d'environnement de PROCESS avant tout accès concurrent à ces
    // clés — ce fichier est le seul test de ce binaire d'intégration (`vehicle_predictive_handoff`,
    // un process cargo-test séparé), donc aucune course avec un autre test lisant ces mêmes clés.
    // On isole le journal de session + les endpoints métriques/journal du Gateway dans le tempdir et
    // on laisse l'OS choisir les ports (":0") — même isolation que handoff_two_real_clients.rs.
    unsafe {
        std::env::set_var("TESSERA_SESSION_LOG_PATH", tmp.path().join("session.jsonl"));
        std::env::set_var("TESSERA_GATEWAY_METRICS_ADDR", "127.0.0.1:0");
        std::env::set_var("TESSERA_GATEWAY_SESSIONLOG_ADDR", "127.0.0.1:0");
    }

    // ── Shard A : porte le véhicule autonome. Graphe à 3 nœuds dont le waypoint intermédiaire est
    //    PILE sur la frontière x=1000. Le véhicule part à x=985 (15 unités du waypoint x=1000, dans
    //    la fenêtre de rapport prédictif de 16 unités) et roule jusqu'à x=1015 — il franchit donc
    //    réellement x=1000 en ~1.9 s (15 unités / 8 u/s) et atteint x=1015 en ~3.75 s.
    let seed = server::VehicleShardSeed {
        nodes: vec![
            NavVec3::new(985.0, 0.0, 0.0),
            NavVec3::new(1000.0, 0.0, 0.0),
            NavVec3::new(1015.0, 0.0, 0.0),
        ],
        edges: vec![(0, 1), (1, 2)],
        vehicles: vec![(
            1, // archétype quelconque (v1 : pas encore différencié, cf. spawn_vehicle)
            NavVec3::new(985.0, 0.0, 0.0),
            NavVec3::new(1015.0, 0.0, 0.0),
        )],
    };

    tokio::spawn(async move {
        server::shard_main(
            shard_a_addr,
            1000.0,
            "127.0.0.1:0",
            None,
            None,
            None,
            Some(seed),
        )
        .await
        .expect("shard A ne devrait pas échouer");
    });
    // Shard B : voisin cible du handoff prédictif, aucun véhicule propre.
    tokio::spawn(async move {
        server::shard_main(shard_b_addr, 1000.0, "127.0.0.1:0", None, None, None, None)
            .await
            .expect("shard B ne devrait pas échouer");
    });

    let topology = two_shards(shard_a_addr, shard_b_addr);
    let radius = RadiusPolicy {
        base: BUFFER_RADIUS,
        moderator: BUFFER_RADIUS,
        game_master: BUFFER_RADIUS,
    };
    let store = server::player_store_impl::PlayerStoreImpl::File(
        server::persistence::FileStore::open(tmp.path().join("players.json")),
    );
    // Redis local requis (même exigence que hot_state_cache.rs / handoff_two_real_clients.rs).
    let hot_state = server::hot_state_cache::HotStateCache::connect("redis://127.0.0.1:6379")
        .await
        .expect("Redis local (127.0.0.1:6379) requis pour ce test — voir hot_state_cache.rs");
    let admin_store = server::admin_store::AdminStore::open(
        tmp.path().join("permission_groups.json"),
        tmp.path().join("server_admins.json"),
    );
    let gateway_addr_owned = gateway_addr.to_string();
    let jwks_cache = std::sync::Arc::new(server::jwks::JwksCache::new());
    tokio::task::spawn_local(async move {
        server::gateway::gateway_main(
            &gateway_addr_owned,
            topology,
            radius,
            store,
            admin_store,
            [950.0, 0.0, 0.0], // spawn par défaut de l'observateur : sur A, hors tampon
            16,
            jwks_cache,
            false,
            false,
            std::collections::HashSet::new(),
            hot_state,
            None, // serveur privé : pas de BanStore Postgres
            None, // serveur privé : pas de store personnage (flux d'arrivée inerte)
        )
        .await
        .expect("gateway ne devrait pas échouer");
    });

    // Laisse les 2 Shards et le Gateway se binder avant de connecter le client.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (ip_s, port_s) = gateway_addr.rsplit_once(':').unwrap();
    let ip = Ipv4Addr::from_str(ip_s).unwrap();
    let port: u16 = port_s.parse().unwrap();

    let g = GnsGlobal::get().expect("GnsGlobal::get");
    let observer: GnsSocket<IsClient> = GnsSocket::new(g)
        .connect(ip.into(), port)
        .expect("connect observer");

    let mut w = VehicleWatch::default();

    let send = |client: &GnsSocket<IsClient>, payload: Vec<u8>, flags: SendFlags| {
        let msg = g
            .utils()
            .allocate_message(client.connection(), flags, payload);
        client.send_messages(std::iter::once(msg));
    };

    // ── 1. Attendre la connexion de l'observateur ───────────────────────────────────────────────
    let deadline = Instant::now() + Duration::from_secs(5);
    while !w.connected && Instant::now() < deadline {
        g.poll_callbacks();
        pump_observer(&observer, &mut w);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(w.connected, "l'observateur devrait se connecter au Gateway");
    println!("[test] observateur connecté");

    // ── 2. Join + position d'observation (x=950, sur A, hors tampon → ne déclenche aucun handoff)
    send(&observer, encode_join("observer"), SendFlags::RELIABLE);
    send(
        &observer,
        encode_position(950.0, 0.0, 0.0),
        SendFlags::RELIABLE,
    );
    println!("[test] Join + position d'observation (x=950) envoyés");

    // ── 3. Attendre de voir le véhicule AVANT le franchissement (x < 1000) ─────────────────────
    //    Le véhicule part à x=985 et avance : on doit le voir strictement sous la frontière au moins
    //    une fois avant qu'il ne la franchisse (preuve du préchargement prédictif observable).
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        g.poll_callbacks();
        pump_observer(&observer, &mut w);
        if w.seen_before_boundary {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timeout : l'observateur n'a jamais vu le véhicule avant le franchissement — \
             dernier véhicule vu : {:?}",
            w.latest_vehicle
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let vehicle_id = w.latest_vehicle.unwrap().0;
    assert!(
        vehicle_id >= server::world::VEHICLE_ID_RANGE_START,
        "l'id vu doit être dans la plage véhicule (>= {}), vu : {vehicle_id}",
        server::world::VEHICLE_ID_RANGE_START
    );
    println!(
        "[test] véhicule vu AVANT le franchissement (id={vehicle_id}, x={:.2})",
        w.latest_vehicle.unwrap().1
    );

    // ── 4. Continuer d'observer pendant que le véhicule franchit x=1000 puis roule au-delà ──────
    //    `pump_observer` panique en continu si le véhicule disparaît plus de DISAPPEARANCE_GRACE :
    //    la traversée elle-même est donc vérifiée à chaque tour de boucle, pas seulement à la fin.
    //    On attend d'avoir vu le véhicule à x >= 1000 (franchissement réel effectué et observé),
    //    tout en exigeant que sa progression soit monotone croissante (il avance vraiment, il n'est
    //    pas figé).
    let post_deadline = Instant::now() + Duration::from_secs(10);
    let mut max_x_seen = w.latest_vehicle.unwrap().1;
    loop {
        g.poll_callbacks();
        pump_observer(&observer, &mut w);

        if let Some((_, x, _, _)) = w.latest_vehicle {
            if x > max_x_seen {
                max_x_seen = x;
            }
        }
        if w.seen_after_boundary {
            break;
        }
        assert!(
            Instant::now() < post_deadline,
            "timeout : le véhicule n'a jamais été vu APRÈS le franchissement (x >= 1000) — \
             x max observé : {max_x_seen:.2}, dernier véhicule vu : {:?}",
            w.latest_vehicle
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Invariants finaux : le véhicule a bien été vu des deux côtés de la frontière, sous le même id,
    // et a progressé (x final > x de départ). La continuité (pas de coupure > grâce) a déjà été
    // vérifiée en continu par `pump_observer` pendant toute la traversée ci-dessus.
    assert!(
        w.seen_before_boundary && w.seen_after_boundary,
        "le véhicule doit avoir été vu AVANT (x<1000) ET APRÈS (x>=1000) le franchissement"
    );
    assert_eq!(
        w.latest_vehicle.unwrap().0,
        vehicle_id,
        "l'id du véhicule doit rester stable de part et d'autre du franchissement"
    );
    assert!(
        max_x_seen >= BOUNDARY_X,
        "le véhicule doit avoir réellement franchi x=1000 (x max observé : {max_x_seen:.2})"
    );

    println!(
        "[test] OK — véhicule autonome franchissant une frontière de shard : vu de bout en bout par \
         un vrai client GNS, avant (x<1000) et après (x>=1000) le franchissement, sans coupure de \
         visibilité (x max observé : {max_x_seen:.2})"
    );
}
