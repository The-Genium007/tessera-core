//! Test d'intégration (feature `gns`) : deux VRAIS clients GNS, connectés à un vrai Gateway et
//! deux vrais Shards (process séparés côté logique, dans des tâches tokio ici), franchissant LA
//! MÊME frontière de shard en même temps.
//!
//! ## Le gap couvert (audit playtest 1, Groupe F)
//!
//! Le handoff cross-shard n'était couvert jusqu'ici que par :
//! - des tests purs/logiques (`handoff.rs`'s test module — `ShardTopology::locate`/`ShardLoader`
//!   en mémoire, zéro octet sur le réseau) ;
//! - un test TCP interne à connexion unique (`tests/shard_tcp.rs`) qui simule un seul "client" en
//!   écrivant des frames `ClientEvent` à la main sur un socket TCP brut — un seul flux, jamais deux
//!   connexions concurrentes.
//!
//! Jamais deux connexions RÉSEAU réelles (GNS, la même pile qu'un vrai client de jeu) traversant
//! la même frontière au même moment n'avaient été exercées de bout en bout à travers le Gateway.
//! Ce test connecte 2 vrais clients GNS à un vrai `gateway_main` relié à 2 vrais `shard_main`, les
//! place symétriquement dans la zone tampon de part et d'autre de x=1000, puis leur fait échanger
//! leurs positions (chacun traverse la frontière dans le sens opposé de l'autre, au même tick) —
//! le pire cas pour un bug de concurrence de handoff (les deux Shards reçoivent des mises à jour
//! de position pour les deux joueurs quasi simultanément).
//!
//! ## Pourquoi ce test passe déjà (documentation vivante, pas un garde-fou silencieux)
//!
//! - `Aabb::contains` (handoff.rs) est demi-ouvert `[min, max)` : un point pile sur la frontière
//!   (x=1000.0 exactement) appartient à EXACTEMENT une zone (B) — jamais aux deux à la fois. Pas
//!   d'ambiguïté de double-appartenance autoritaire au moment précis du franchissement.
//! - `ShardTopology::locate`'s tie-break déterministe (adresse minimale via `.min()`) donne un
//!   résultat stable et reproductible même si un cas limite venait à être contenu par plusieurs
//!   zones — pas de flip aléatoire d'un tick à l'autre.
//! - Avec le rayon de tampon utilisé ici (25 m, identique aux tests purs `two_shards()` de
//!   `handoff.rs`), les deux clients sont chargés sur LES DEUX shards dès qu'ils entrent dans le
//!   tampon (990 et 1010 sont tous deux à 10 m de la frontière, <= 25). Le franchissement ne
//!   change donc PAS l'ensemble `desired` de shards chargés pour chacun (`{A, B}` des deux côtés
//!   de x=1000) — seule l'étiquette "autoritaire" change. Aucune reconnexion Shard, aucun re-seed
//!   (`reseed_frames_for_reconnected_shard`) n'est déclenché par ce scénario symétrique : la
//!   fenêtre où un bug de handoff aurait le plus de chances d'apparaître (perte d'état au moment
//!   d'une reconnexion) n'est simplement pas ouverte ici. Ce test prouve que cette propriété tient
//!   aussi avec un vrai transport réseau (latence, ordonnancement non déterministe des paquets),
//!   pas seulement en mémoire — pas qu'elle couvre tous les scénarios de handoff possibles.

#![cfg(feature = "gns")]

use flatbuffers::FlatBufferBuilder;
use gns::{
    sys::ESteamNetworkingConnectionState as State, GnsGlobal, GnsSocket, IsClient, SendFlags,
};
use protocol::*;
use server::handoff::{CellZone, RadiusPolicy, ShardTopology, ShardZone};
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// Même topologie que `two_shards()` dans `handoff.rs` (tests purs) : frontière à x=1000,
/// A = x<1000, B = x>=1000. Adresses paramétrées ici (au lieu des littéraux "A"/"B") car ce sont
/// de vraies adresses TCP internes Gateway→Shard.
///
/// `Aabb`/rayon global ont été remplacés par la tessellation Voronoï (Groupe G) : chaque
/// `ShardZone` porte désormais des `CellZone` polygonales + un rayon de tampon par cellule. Les
/// bornes "infinies" de l'ancien `Aabb` sont approximées par un grand rectangle fini (le
/// point-in-polygon n'a pas de notion d'infini) — largement suffisant pour ce test, qui ne place
/// des clients qu'à proximité immédiate de x=1000 (990/1010).
const FAR: f64 = 1_000_000.0;

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

/// Rayon de tampon — identique à celui des tests purs `two_shards()` de `handoff.rs`
/// (`inside_buffer_dual_loads_neighbor`, `far_from_boundary_loads_only_authoritative`), pour
/// réutiliser le raisonnement déjà validé sur le chargement double en zone tampon.
const BUFFER_RADIUS: f32 = 25.0;

fn encode_join(name: &str) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let n = b.create_string(name);
    let join = Join::create(
        &mut b,
        &JoinArgs {
            display_name: Some(n),
            token: None,
            protocol_version: server::gateway_routing::CURRENT_PROTOCOL_VERSION,
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
    let pos = Vec3::new(x, y, z);
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

/// Décode un `ServerEnvelope`/`Snapshot` et renvoie l'unique "autre" joueur vu (id, x, y, z), ou
/// `None` si le snapshot ne contient personne. Avec exactement 2 vrais clients connectés dans ce
/// test, un client ne devrait JAMAIS voir plus d'un "autre" joueur (`World::snapshot_for` exclut
/// le viewer lui-même — cf. `world.rs::snapshot_excludes_the_viewer_and_includes_others`) : plus
/// d'une entrée ici serait précisément le doublon de joueur que ce test existe pour détecter.
fn other_player_from_snapshot(payload: &[u8]) -> Option<(u64, f32, f32, f32)> {
    let env = flatbuffers::root::<ServerEnvelope>(payload).ok()?;
    let snap = env.msg_as_snapshot()?;
    let players = snap.players()?;
    assert!(
        players.len() <= 1,
        "BUG DE CONCURRENCE : {} joueur(s) vus dans un même snapshot au lieu d'au plus 1 \
         (doublon de joueur) — ids: {:?}",
        players.len(),
        (0..players.len())
            .map(|i| players.get(i).id())
            .collect::<Vec<_>>()
    );
    if players.is_empty() {
        return None;
    }
    let p = players.get(0);
    let pos = p.position()?;
    Some((p.id(), pos.x(), pos.y(), pos.z()))
}

/// État d'un client de test, mis à jour à chaque pompage de la boucle de poll.
#[derive(Default)]
struct ClientWatch {
    connected: bool,
    /// Dernier "autre joueur" vu dans un snapshot (id, x, y, z).
    latest_other: Option<(u64, f32, f32, f32)>,
    /// Horodatage de la dernière fois où un "autre joueur" a été vu (pour détecter une
    /// disparition prolongée — pas juste un trou d'un tick dû à la livraison UNRELIABLE des
    /// snapshots Gateway→client).
    last_seen_other_at: Option<Instant>,
}

const DISAPPEARANCE_GRACE: Duration = Duration::from_millis(1500);

/// Pompe les événements/messages GNS d'un client et met à jour son `ClientWatch`. Panique
/// immédiatement si l'"autre joueur" disparaît plus longtemps que `DISAPPEARANCE_GRACE` après
/// avoir déjà été vu — couvre le "joueur disparu" listé par le point d'attention de cette tâche.
fn pump_client(client: &GnsSocket<IsClient>, watch: &mut ClientWatch, label: &str) {
    for ev in client.receive_events() {
        match ev.info().state() {
            State::k_ESteamNetworkingConnectionState_Connected => watch.connected = true,
            State::k_ESteamNetworkingConnectionState_ClosedByPeer
            | State::k_ESteamNetworkingConnectionState_ProblemDetectedLocally => {
                panic!("{label} : connexion fermée/refusée de façon inattendue par le Gateway");
            }
            _ => {}
        }
    }
    for m in client.receive_messages::<64>().expect("receive_messages") {
        if let Some(other) = other_player_from_snapshot(m.payload()) {
            watch.latest_other = Some(other);
            watch.last_seen_other_at = Some(Instant::now());
        } else if watch.latest_other.is_some() {
            // Snapshot reçu mais vide : l'autre joueur a-t-il disparu trop longtemps ?
            if let Some(last) = watch.last_seen_other_at {
                assert!(
                    last.elapsed() <= DISAPPEARANCE_GRACE,
                    "BUG DE CONCURRENCE : {label} a cessé de voir l'autre joueur pendant {:?} \
                     (> tolérance de {DISAPPEARANCE_GRACE:?}) — joueur disparu",
                    last.elapsed()
                );
            }
        }
    }
}

// `GnsTransport` (utilisé en interne par `gateway_main`) contient des pointeurs bruts
// (`*mut SteamNetworkingMessage_t`) et n'est donc pas `Send` — `tokio::spawn` (qui exige
// `Future + Send`, quel que soit le flavor du runtime) ne peut pas l'accueillir. Il faut passer
// par un `LocalSet`/`spawn_local` pour faire tourner un `gateway_main` réel concurremment au
// reste du corps du test, dans le même thread.
#[tokio::test]
async fn two_real_clients_crossing_same_boundary_simultaneously_both_see_correct_state() {
    let local = tokio::task::LocalSet::new();
    local.run_until(run_test()).await;
}

async fn run_test() {
    // Sans subscriber, tout tracing::warn!/info! du Gateway/Shard est silencieusement avalé —
    // utile pour voir les rejets anti-triche/rate-limit ou les lignes de Handoff en cas d'échec
    // (lancer avec `RUST_LOG=info ... -- --nocapture` pour les voir).
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    // Ports dédiés à ce test (distincts de gns_loopback/shard_tcp/shard.rs) pour éviter toute
    // collision avec d'autres tests exécutés en parallèle dans le même run.
    let shard_a_addr = "127.0.0.1:27150";
    let shard_b_addr = "127.0.0.1:27151";
    let gateway_addr = "127.0.0.1:27160";

    let tmp = tempfile::tempdir().expect("tempdir");
    // Le Gateway écrit un journal de session + sert des endpoints métriques/journal sur des
    // adresses par défaut fixes (9100/9102) et un chemin relatif ("session.jsonl") — jamais
    // exercé par un test d'intégration jusqu'ici (seul `bin/gateway.rs` l'utilise en prod). On
    // isole tout dans le tempdir de ce test et on laisse l'OS choisir les ports (":0") pour ne
    // rien coller au disque du repo ni entrer en collision avec un vrai serveur qui tournerait
    // sur la machine.
    // Safety: mutation de variables d'environnement de PROCESS avant tout accès concurrent à ces
    // clés — ce fichier est le seul test de ce binaire d'intégration (`handoff_two_real_clients`,
    // un process cargo-test séparé), donc aucune course avec un autre test lisant ces mêmes clés.
    unsafe {
        std::env::set_var("TESSERA_SESSION_LOG_PATH", tmp.path().join("session.jsonl"));
        std::env::set_var("TESSERA_GATEWAY_METRICS_ADDR", "127.0.0.1:0");
        std::env::set_var("TESSERA_GATEWAY_SESSIONLOG_ADDR", "127.0.0.1:0");
    }

    tokio::spawn(async move {
        server::shard_main(shard_a_addr, 1000.0, "127.0.0.1:0")
            .await
            .expect("shard A ne devrait pas échouer");
    });
    tokio::spawn(async move {
        server::shard_main(shard_b_addr, 1000.0, "127.0.0.1:0")
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
    // Redis local requis (même exigence que hot_state_cache.rs) — cohérent avec le câblage
    // HotStateCache désormais obligatoire dans gateway_main.
    let hot_state = server::hot_state_cache::HotStateCache::connect("redis://127.0.0.1:6379")
        .await
        .expect("Redis local (127.0.0.1:6379) requis pour ce test — voir hot_state_cache.rs");
    let admin_store = server::admin_store::AdminStore::open(
        tmp.path().join("permission_groups.json"),
        tmp.path().join("server_admins.json"),
    );
    let gateway_addr_owned = gateway_addr.to_string();
    // Serveur privé (identity_public=false) pour ce test : le Join utilise display_name seul,
    // comme avant Task C2/C3 — aucun token/JWKS réel nécessaire pour exercer le handoff.
    let jwks_cache = std::sync::Arc::new(server::jwks::JwksCache::new());
    tokio::task::spawn_local(async move {
        server::gateway::gateway_main(
            &gateway_addr_owned,
            topology,
            radius,
            store,
            admin_store,
            [990.0, 0.0, 0.0],
            16,
            jwks_cache,
            false,
            false,
            std::collections::HashSet::new(),
            hot_state,
        )
        .await
        .expect("gateway ne devrait pas échouer");
    });

    // Laisse les 2 Shards et le Gateway se binder avant de connecter les clients.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let (ip_s, port_s) = gateway_addr.rsplit_once(':').unwrap();
    let ip = Ipv4Addr::from_str(ip_s).unwrap();
    let port: u16 = port_s.parse().unwrap();

    let g = GnsGlobal::get().expect("GnsGlobal::get");
    let client1: GnsSocket<IsClient> = GnsSocket::new(g)
        .connect(ip.into(), port)
        .expect("connect client1");
    let client2: GnsSocket<IsClient> = GnsSocket::new(g)
        .connect(ip.into(), port)
        .expect("connect client2");

    let mut w1 = ClientWatch::default();
    let mut w2 = ClientWatch::default();

    // ── 1. Attendre la connexion des deux clients ────────────────────────────────────────────
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!w1.connected || !w2.connected) && Instant::now() < deadline {
        g.poll_callbacks();
        pump_client(&client1, &mut w1, "client1");
        pump_client(&client2, &mut w2, "client2");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(w1.connected, "client1 devrait se connecter au Gateway");
    assert!(w2.connected, "client2 devrait se connecter au Gateway");
    println!("[test] les deux clients sont connectés");

    // ── 2. Join, puis position initiale dans le tampon, de part et d'autre de x=1000 ─────────
    // client1 : x=990 → autoritaire A, overlap B. client2 : x=1010 → autoritaire B, overlap A.
    // Les deux sont donc chargés sur LES DEUX shards dès le départ (cf. doc en tête de fichier).
    let send = |client: &GnsSocket<IsClient>, payload: Vec<u8>, flags: SendFlags| {
        let msg = g
            .utils()
            .allocate_message(client.connection(), flags, payload);
        client.send_messages(std::iter::once(msg));
    };
    send(&client1, encode_join("client1"), SendFlags::RELIABLE);
    send(&client2, encode_join("client2"), SendFlags::RELIABLE);
    send(
        &client1,
        encode_position(990.0, 0.0, 0.0),
        SendFlags::RELIABLE,
    );
    send(
        &client2,
        encode_position(1010.0, 0.0, 0.0),
        SendFlags::RELIABLE,
    );
    println!("[test] Join + position initiale (tampon) envoyés pour les deux clients");

    // ── 3. Attendre l'état stable initial : chacun voit l'autre à sa position de départ ──────
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        g.poll_callbacks();
        pump_client(&client1, &mut w1, "client1");
        pump_client(&client2, &mut w2, "client2");

        let baseline_ok = matches!(w1.latest_other, Some((_, x, y, z)) if (x, y, z) == (1010.0, 0.0, 0.0))
            && matches!(w2.latest_other, Some((_, x, y, z)) if (x, y, z) == (990.0, 0.0, 0.0));
        if baseline_ok {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timeout : état stable initial jamais atteint — client1 voit {:?}, client2 voit {:?}",
            w1.latest_other,
            w2.latest_other
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let other_id_seen_by_1 = w1.latest_other.unwrap().0;
    let other_id_seen_by_2 = w2.latest_other.unwrap().0;
    assert_ne!(
        other_id_seen_by_1, other_id_seen_by_2,
        "client1 et client2 ne devraient jamais se voir l'un l'autre sous le même id"
    );
    println!(
        "[test] état stable initial atteint (client1 voit id={other_id_seen_by_1}, \
         client2 voit id={other_id_seen_by_2})"
    );

    // L'anti-triche (`is_plausible_move`, `anticheat.rs`) borne la vitesse à
    // `MAX_PLAYER_SPEED_MPS` (60 m/s) depuis la DERNIÈRE position ACCEPTÉE de CE client — pas
    // depuis le début du test. L'état stable ci-dessus est atteint très vite (dizaines de ms),
    // donc sans cette pause le franchissement de 20 m serait rejeté comme un téléport implausible
    // (20 m / ~0.1 s = 200 m/s). On attend explicitement au-delà de 20 m / 60 m/s (~0.34 s) avant
    // d'envoyer le franchissement, avec une marge confortable.
    let wait_deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < wait_deadline {
        g.poll_callbacks();
        pump_client(&client1, &mut w1, "client1");
        pump_client(&client2, &mut w2, "client2");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // ── 4. Franchissement simultané : les deux clients échangent leurs positions, chacun ─────
    //    traversant x=1000 dans le sens opposé de l'autre, au même tick (aucune attente entre
    //    les deux envois — c'est le scénario que l'audit playtest 1 n'avait jamais exercé avec
    //    de vraies connexions réseau concurrentes).
    send(
        &client1,
        encode_position(1010.0, 0.0, 0.0),
        SendFlags::RELIABLE,
    );
    send(
        &client2,
        encode_position(990.0, 0.0, 0.0),
        SendFlags::RELIABLE,
    );
    println!("[test] franchissement simultané envoyé (client1 → B, client2 → A)");

    // ── 5. Observer pendant une fenêtre après le franchissement : jamais de doublon (vérifié en
    //    continu par `other_player_from_snapshot`/`pump_client`, qui paniquent immédiatement),
    //    jamais de disparition prolongée (idem), et convergence finale vers l'état échangé.
    let post_cross_deadline = Instant::now() + Duration::from_secs(6);
    let mut stable_since: Option<Instant> = None;
    const REQUIRED_STABLE: Duration = Duration::from_millis(400);
    loop {
        g.poll_callbacks();
        pump_client(&client1, &mut w1, "client1");
        pump_client(&client2, &mut w2, "client2");

        let swapped_ok = matches!(w1.latest_other, Some((id, x, y, z)) if id == other_id_seen_by_1 && (x, y, z) == (990.0, 0.0, 0.0))
            && matches!(w2.latest_other, Some((id, x, y, z)) if id == other_id_seen_by_2 && (x, y, z) == (1010.0, 0.0, 0.0));

        match (swapped_ok, stable_since) {
            (true, None) => stable_since = Some(Instant::now()),
            (true, Some(since)) if since.elapsed() >= REQUIRED_STABLE => break,
            (true, Some(_)) => {}
            (false, _) => stable_since = None,
        }

        assert!(
            Instant::now() < post_cross_deadline,
            "timeout : état échangé jamais atteint/stabilisé après le franchissement — \
             client1 voit {:?} (attendu id={other_id_seen_by_1} @ (990,0,0)), \
             client2 voit {:?} (attendu id={other_id_seen_by_2} @ (1010,0,0))",
            w1.latest_other,
            w2.latest_other
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    println!(
        "[test] OK — franchissement simultané de la même frontière : les deux clients voient \
         l'état correct, sans doublon ni disparition, des deux côtés du franchissement"
    );
}
