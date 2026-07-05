//! Export JSON « carte des shards » (spec playtest-shards §#2) : frontières (splits) + zones
//! (AABB aplatis) + rayons de zone tampon, depuis le manifeste TOML. Consommé par le mod
//! client (HUD/balises/mappins). Ne divulgue JAMAIS les adresses internes des shards.

use serde_json::json;
use server::handoff::ShardZone;
use server::manifest::{Axis, Manifest};

/// Borne d'AABB → JSON : nombre fini, ou null pour ±infini (zone ouverte du BSP racine).
fn finite(v: f32) -> serde_json::Value {
    if v.is_finite() {
        json!(v)
    } else {
        serde_json::Value::Null
    }
}

pub fn shard_map_json(m: &Manifest, zones: &[ShardZone]) -> serde_json::Value {
    // flatten_topology identifie les zones par listen_addr ; on re-mappe vers l'id logique
    // pour ne jamais exposer d'adresse interne au client.
    let mut id_by_addr: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for s in &m.runtime.topology.shards {
        if let Some(existing_id) = id_by_addr.insert(s.listen_addr.as_str(), s.id.as_str()) {
            // Deux shards partagent le même listen_addr : manifest.rs ne valide que
            // l'unicité des id, pas des adresses. Si ça arrive, mieux vaut planter
            // bruyamment que de mal étiqueter silencieusement une zone.
            panic!(
                "bug: deux shards partagent le même listen_addr {:?} ({} et {})",
                s.listen_addr, existing_id, s.id
            );
        }
    }

    json!({
        "formatVersion": 1,
        "radius": {
            "base": m.runtime.radius.base,
            "moderator": m.runtime.radius.moderator,
            "gameMaster": m.runtime.radius.game_master,
        },
        "splits": m.runtime.topology.splits.iter().map(|s| json!({
            "axis": match s.axis { Axis::X => "x", Axis::Y => "y" },
            "at": s.at,
        })).collect::<Vec<_>>(),
        "shards": zones.iter().map(|z| json!({
            "id": id_by_addr.get(z.addr.as_str()).copied().unwrap_or_else(|| {
                // zones et manifest désynchronisés : plutôt que de fuiter l'adresse
                // interne (l'invariant même que ce module doit garantir), on plante.
                panic!(
                    "bug: ShardZone.addr {:?} absent de topology.shards — zones et manifest désynchronisés",
                    z.addr
                )
            }),
            "minX": finite(z.zone.min_x),
            "maxX": finite(z.zone.max_x),
            "minY": finite(z.zone.min_y),
            "maxY": finite(z.zone.max_y),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use server::handoff::Aabb;
    use server::manifest::{
        AoiConfig, GatewayConfig, Identity, RadiusConfig, Runtime, ShardConfig, TopologyConfig,
    };

    /// Manifeste minimal (2 shards, pas de splits) pour les tests de garde ci-dessous.
    fn minimal_manifest(shards: Vec<ShardConfig>) -> Manifest {
        Manifest {
            format_version: 1,
            identity: Identity {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: "".to_string(),
                region: "eu".to_string(),
                language: "fr".to_string(),
                max_players: 10,
                tags: vec![],
                discord_url: "".to_string(),
                website_url: "".to_string(),
                required_modset: "".to_string(),
                voice_required: false,
            },
            runtime: Runtime {
                whitelist: false,
                store_path: "store".to_string(),
                gateway: GatewayConfig {
                    listen_addr: "127.0.0.1:1".to_string(),
                    advertise_addr: "127.0.0.1:1".to_string(),
                },
                topology: TopologyConfig {
                    active_preset: "default".to_string(),
                    splits: vec![],
                    shards,
                },
                radius: RadiusConfig {
                    base: 1.0,
                    moderator: 2.0,
                    game_master: 3.0,
                },
                aoi: AoiConfig {
                    visibility_radius: 1.0,
                },
            },
        }
    }

    fn shard(id: &str, listen_addr: &str) -> ShardConfig {
        ShardConfig {
            id: id.to_string(),
            listen_addr: listen_addr.to_string(),
            default_entry: false,
            spawn_points: vec![],
        }
    }

    #[test]
    #[should_panic(expected = "zones et manifest désynchronisés")]
    fn shard_map_panics_instead_of_leaking_addr_when_zones_desync_from_manifest() {
        let manifest = minimal_manifest(vec![shard("shard-a", "127.0.0.1:27031")]);
        // zones référence une adresse absente du manifest (topologie désynchronisée).
        let zones = vec![ShardZone {
            addr: "127.0.0.1:27099".to_string(),
            zone: Aabb {
                min_x: 0.0,
                max_x: 1.0,
                min_y: 0.0,
                max_y: 1.0,
            },
        }];

        shard_map_json(&manifest, &zones);
    }

    #[test]
    #[should_panic(expected = "partagent le même listen_addr")]
    fn shard_map_panics_on_duplicate_listen_addr_across_shards() {
        let manifest = minimal_manifest(vec![
            shard("shard-a", "127.0.0.1:27031"),
            shard("shard-b", "127.0.0.1:27031"),
        ]);

        shard_map_json(&manifest, &[]);
    }

    #[test]
    fn shard_map_exports_ids_not_addresses_and_finite_bounds_or_null() {
        let manifest =
            server::manifest::load(std::path::Path::new("../server/server.example.toml"))
                .expect("server.example.toml doit être chargeable");
        let zones = server::manifest::flatten_topology(&manifest.runtime.topology)
            .expect("topologie valide");

        let v = shard_map_json(&manifest, &zones);

        assert_eq!(v["formatVersion"], 1);
        assert!(v["radius"]["base"].as_f64().unwrap() > 0.0);
        let shards = v["shards"].as_array().unwrap();
        assert_eq!(shards.len(), zones.len());
        for s in shards {
            let id = s["id"].as_str().unwrap();
            // Un id logique, pas une adresse listen (host:port).
            assert!(!id.contains(':'), "id '{id}' ressemble à une adresse");
            // Chaque borne est soit un nombre fini, soit null (infini).
            for k in ["minX", "maxX", "minY", "maxY"] {
                assert!(s[k].is_null() || s[k].as_f64().unwrap().is_finite());
            }
        }
        // Autant de splits que le manifeste en déclare.
        assert_eq!(
            v["splits"].as_array().unwrap().len(),
            manifest.runtime.topology.splits.len()
        );
    }
}
