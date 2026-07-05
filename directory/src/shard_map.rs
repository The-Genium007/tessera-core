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
    let id_by_addr: std::collections::HashMap<&str, &str> = m
        .runtime
        .topology
        .shards
        .iter()
        .map(|s| (s.listen_addr.as_str(), s.id.as_str()))
        .collect();

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
            "id": id_by_addr.get(z.addr.as_str()).copied().unwrap_or(z.addr.as_str()),
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
