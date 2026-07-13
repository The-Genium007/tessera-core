//! Preuve de bout en bout (Task G4, §5.6) : le chemin `bin/gateway.rs` — c'est-à-dire
//! `manifest::load_authority_topology` (I/O + résolution `assignment_patterns[server_count]`,
//! Task G3) suivi de `ShardTopology::locate` (routage, Task G2) — route correctement une
//! position connue vers le bon groupe de shards, en utilisant le VRAI `authority.json` v3
//! (10 cellules, `tools/district-topology/`), pas une fixture synthétique.
//!
//! `gateway_main`/`shard_main` réels ne sont pas démarrés ici : `gateway_main` est gns-gated
//! (nécessite GameNetworkingSockets, cf. ADR 0003) et n'a de toute façon aucune logique de
//! routage propre — il délègue entièrement à `ShardTopology::locate`, exactement comme ce test
//! l'exerce. Ce test prouve donc le câblage réel (artefact → zones → routage), pas une nouvelle
//! logique de simulation réseau.

use server::handoff::ShardTopology;
use server::manifest::{load_authority_topology, TopologyConfig};
use std::collections::HashMap;

/// Répertoire du manifeste utilisé par les tests : celui où vit le vrai `authority.json` v3.
fn district_topology_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/district-topology")
}

#[test]
fn gateway_routes_to_correct_shard_group_using_real_v3_artifact() {
    let config = TopologyConfig {
        authority_artifact: "authority.json".into(),
        server_count: 4,
        radius_overrides: HashMap::new(),
    };

    // Étape 1 (Task G3, déjà testée isolément dans manifest.rs) : charger les zones depuis le
    // vrai artefact. On la rejoue ici pour construire le harnais de routage de bout en bout.
    let zones = load_authority_topology(&config, &district_topology_dir())
        .expect("le vrai authority.json v3 doit se charger pour server_count=4");
    let topology = ShardTopology { shards: zones };

    // `assignment_patterns["4"]` du vrai authority.json v3 (vérifié via
    // `python3 -c "json.load(...)"` avant d'écrire ce test) :
    //   [[0,1,2,3,4,5,6], [7], [8], [9]]
    // groupe 0 = 7 cellules fusionnées (dont la cellule 0, "ARR-CH-JT-VDR") ; groupes 1-3 =
    // cellules seules (7 "JP", 8 "ORB", 9 "YUC"). Les seeds Voronoï de ces cellules (issus de
    // l'artefact) sont par construction à l'intérieur de leur propre cellule.

    // Groupe fusionné (7 cellules) : seed de la cellule 0 "ARR-CH-JT-VDR".
    let placement = topology.locate(-524.740_86, -133.716_03, 0.0);
    assert_eq!(
        placement.authoritative, "group-0",
        "le seed de la cellule 0 (groupe fusionné 7 cellules) doit router vers group-0"
    );

    // Groupe mono-cellule : seed de la cellule 8 "ORB".
    let placement = topology.locate(4743.625, -1091.775, 0.0);
    assert_eq!(
        placement.authoritative, "group-2",
        "le seed de la cellule 8 (ORB) doit router vers group-2"
    );

    // Groupe mono-cellule : seed de la cellule 9 "YUC".
    let placement = topology.locate(-4014.148_3, -6574.603_6, 0.0);
    assert_eq!(
        placement.authoritative, "group-3",
        "le seed de la cellule 9 (YUC) doit router vers group-3"
    );
}
