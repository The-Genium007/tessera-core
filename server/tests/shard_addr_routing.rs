//! Régression de l'incident du 2026-07-20 : « on est connectés à 2, au même endroit, et on ne se
//! voit pas ».
//!
//! La chaîne qui rend un joueur visible par un autre traversait un placeholder jamais remplacé :
//! `load_authority_topology_from_artifact` assignait `format!("group-{i}")` comme ADRESSE de shard.
//! `TcpStream::connect("group-0")` — une chaîne sans port — échoue en `InvalidInput` à tous les
//! coups. Le Gateway n'ouvrait donc jamais un seul lien shard, `latest` restait vide, aucun
//! `Snapshot` n'était renvoyé, et l'erreur était avalée par un `if let Ok(true)` muet. Tout le
//! reste marchait (connexion, Join, IDs distincts), d'où un monde parfaitement vide et silencieux.
//!
//! Ce test parcourt la chaîne ENTIÈRE avec un vrai shard sur TCP, sans Docker ni GNS :
//!   manifeste → `load_authority_topology` → `locate()` → `addr_for()` → `write_to_shard()`
//!   → shard réel → snapshot renvoyé
//! et vérifie que deux joueurs placés au MÊME endroit apparaissent bien dans le snapshot l'un de
//! l'autre. Il verrouille aussi la séparation id logique / adresse réseau : `locate()` doit rendre
//! un id publiable (« group-0 »), jamais un `host:port` — cet id part au client dans
//! `ShardAssignment` et dans `shard_map.json`.

use protocol::internal::EventKind;
use server::gateway::{read_from_shards, write_to_shard, ShardLink};
use server::handoff::ShardTopology;
use server::internal_net::encode_client_event;
use std::collections::HashMap;
use std::time::Duration;

/// Encode un `ClientEnvelope{PositionUpdate}` — payload opaque que le Gateway relaie tel quel.
fn client_position(x: f32, y: f32) -> Vec<u8> {
    use flatbuffers::FlatBufferBuilder;
    use protocol::*;
    let mut b = FlatBufferBuilder::new();
    let pos = Vec3::new(x, y, 0.0);
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
async fn two_players_at_the_same_spot_see_each_other_through_the_real_shard_addr() {
    let shard_addr = "127.0.0.1:27131";
    tokio::spawn(async move {
        server::shard_main(shard_addr, 1000.0, "127.0.0.1:0", None, None, None)
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 1) Topologie chargée comme au boot du Gateway, depuis le vrai manifeste d'exemple et le vrai
    //    authority.json. On ramène juste la topologie à un seul groupe, servi par le shard qu'on
    //    vient de lancer — le reste (identité, rayons, AoI) sort du fichier tel quel.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut manifest = server::manifest::load(&manifest_dir.join("server.example.toml"))
        .expect("server.example.toml doit rester chargeable");
    manifest.runtime.topology.server_count = 1;
    manifest.runtime.topology.shard_addrs = vec![shard_addr.to_string()];

    let zones = server::manifest::load_authority_topology(&manifest.runtime.topology, manifest_dir)
        .expect("la topologie doit se charger pour server_count=1");
    let topology = ShardTopology { shards: zones };

    // 2) Placement : `locate()` rend un id LOGIQUE, publiable tel quel au client.
    let (x, y) = (-1431.17, 1302.27); // un point réel de Night City (spawn de server.example.toml)
    let placement = topology.locate(x, y, manifest.runtime.radius.base);
    assert!(
        !placement.authoritative.contains(':'),
        "locate() doit rendre un id logique, pas une adresse réseau : {}",
        placement.authoritative
    );

    // 3) Résolution id → adresse joignable. C'est l'étape qui n'existait pas : avant, l'« adresse »
    //    ÉTAIT l'id, et elle n'était donc joignable par personne.
    let resolved = topology
        .addr_for(&placement.authoritative)
        .expect("le shard autoritaire doit avoir une adresse dans le manifeste");
    assert_eq!(resolved, shard_addr);

    // 4) Écriture réelle vers le shard : deux joueurs connectés, tous deux AU MÊME ENDROIT.
    let mut shards: HashMap<String, ShardLink> = HashMap::new();
    let frames = vec![
        encode_client_event(EventKind::Connected, 1, &[]),
        encode_client_event(EventKind::Connected, 2, &[]),
        encode_client_event(EventKind::Message, 1, &client_position(x, y)),
        encode_client_event(EventKind::Message, 2, &client_position(x, y)),
    ];
    let created = write_to_shard(&mut shards, &placement.authoritative, resolved, &frames)
        .await
        .expect("l'écriture vers le shard doit réussir — c'est précisément ce qui échouait");
    assert!(created, "le premier appel doit ouvrir la connexion");
    assert!(
        shards.contains_key(&placement.authoritative),
        "la table des liens doit être indexée par id logique, pas par adresse : sinon l'élagage \
         de `latest` (qui compare aux `loaded_shards` du ShardLoader, des ids) jetterait tous les \
         snapshots et le monde redeviendrait vide"
    );

    // 5) Lecture par la VRAIE fonction du Gateway, pour couvrir aussi l'indexation de `latest` :
    //    `latest[client][clé_shard]`. La clé doit être l'id logique — c'est elle que l'élagage
    //    compare aux `loaded_shards` du `ShardLoader`.
    let mut latest: HashMap<u64, HashMap<String, Vec<u8>>> = HashMap::new();
    let mut snapshot_ticks: HashMap<u64, HashMap<String, u64>> = HashMap::new();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timeout : aucun snapshot mutuel reçu (latest = {latest:?})"
        );
        read_from_shards(&mut shards, &mut latest, 0, &mut snapshot_ticks).await;
        if others_seen_by(&latest, 1).contains(&2) && others_seen_by(&latest, 2).contains(&1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for viewer in [1u64, 2u64] {
        let keys: Vec<&String> = latest[&viewer].keys().collect();
        assert_eq!(
            keys,
            vec![&placement.authoritative],
            "`latest` doit être indexé par id logique de shard, pas par adresse"
        );
    }
}

/// Ids des autres joueurs présents dans le dernier snapshot reçu pour `viewer`.
fn others_seen_by(
    latest: &HashMap<u64, HashMap<String, Vec<u8>>>,
    viewer: u64,
) -> std::collections::HashSet<u64> {
    let mut ids = std::collections::HashSet::new();
    let Some(per_shard) = latest.get(&viewer) else {
        return ids;
    };
    for payload in per_shard.values() {
        let Ok(senv) = flatbuffers::root::<protocol::ServerEnvelope>(payload) else {
            continue;
        };
        let Some(snap) = senv.msg_as_snapshot() else {
            continue;
        };
        let Some(players) = snap.players() else {
            continue;
        };
        ids.extend(players.iter().map(|p| p.id()));
    }
    ids
}
