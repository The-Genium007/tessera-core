//! Export JSON « carte des shards » (spec playtest-shards §#2) : anneaux de frontière des
//! cellules d'autorité Voronoï (aplatis depuis `authority.json`, généré par
//! `tools/authority-tessellation`) + rayons de zone tampon, depuis le manifeste TOML. Consommé
//! par le mod client (HUD/balises/mappins). Ne divulgue JAMAIS les adresses internes des shards.

use serde_json::json;
use server::handoff::ShardZone;
use server::manifest::Manifest;

pub fn shard_map_json(m: &Manifest, zones: &[ShardZone]) -> serde_json::Value {
    json!({
        "formatVersion": 1,
        "radius": {
            "base": m.runtime.radius.base,
            "moderator": m.runtime.radius.moderator,
            "gameMaster": m.runtime.radius.game_master,
        },
        "shards": zones.iter().map(|z| json!({
            // `z.addr` n'est plus une adresse réseau : dans le modèle Voronoï,
            // `load_authority_topology_from_artifact` lui assigne déjà un identifiant logique
            // opaque (ex. "group-0"), pas de `listen_addr`. Il n'y a donc plus besoin de table
            // de correspondance addr→id ni de garde anti-désynchronisation : `z.addr` est
            // directement exposable.
            "id": z.addr,
            // Libellé humain affiché par le HUD (« Shard: <name> ») au lieu de l'id logique brut
            // (« group-0 »), exigence playtest 2026-07-17. Repli sur l'id tant qu'aucune table de
            // noms de quartiers réels n'est câblée : garantit un affichage non vide dès maintenant
            // (enrichissement district ultérieur, hors périmètre de ce lot). Le HUD Lua lit `name`
            // en priorité, avec repli sur `id`, donc un vieux shard-map.json sans `name` fonctionne.
            "name": z.addr,
            "boundaryRings": z.cells.iter()
                .flat_map(|(cell, _radius)| cell.boundary_rings.iter())
                .collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_map_exports_ids_not_addresses_and_finite_boundary_rings() {
        let manifest =
            server::manifest::load(std::path::Path::new("../server/server.example.toml"))
                .expect("server.example.toml doit être chargeable");
        // "../server" (pas "../../tools/district-topology") : authority_artifact est un chemin
        // relatif au manifeste (voir server.example.toml), et authority.json est déjà copié à
        // côté du manifeste dans server/ — le répertoire tools/ n'existe pas dans le mirror
        // public tessera-core (non vendoré, cf. .github/workflows/core-subtree.yml).
        let zones = server::manifest::load_authority_topology(
            &manifest.runtime.topology,
            std::path::Path::new("../server"),
        )
        .expect("le vrai authority.json doit se charger pour server_count=2");

        let v = shard_map_json(&manifest, &zones);

        assert_eq!(v["formatVersion"], 1);
        assert!(v["radius"]["base"].as_f64().unwrap() > 0.0);
        let shards = v["shards"].as_array().unwrap();
        assert_eq!(shards.len(), zones.len());
        for s in shards {
            let id = s["id"].as_str().unwrap();
            // Un id logique ("group-N"), pas une adresse listen (host:port).
            assert!(!id.contains(':'), "id '{id}' ressemble à une adresse");
            let rings = s["boundaryRings"].as_array().unwrap();
            assert!(
                !rings.is_empty(),
                "chaque shard doit avoir au moins un anneau"
            );
            for ring in rings {
                let points = ring.as_array().unwrap();
                assert!(!points.is_empty(), "un anneau ne doit pas être vide");
                for p in points {
                    let coords = p.as_array().unwrap();
                    assert_eq!(coords.len(), 2);
                    assert!(coords[0].as_f64().unwrap().is_finite());
                    assert!(coords[1].as_f64().unwrap().is_finite());
                }
            }
        }
    }

    #[test]
    fn shard_map_emits_name_field_falling_back_to_id() {
        use server::handoff::CellZone;

        let manifest =
            server::manifest::load(std::path::Path::new("../server/server.example.toml"))
                .expect("server.example.toml doit être chargeable");
        let ring = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]];
        let zones = vec![ShardZone {
            addr: "group-0".to_string(),
            cells: vec![(
                CellZone {
                    boundary_rings: vec![ring],
                },
                25.0,
            )],
        }];

        let v = shard_map_json(&manifest, &zones);
        let shards = v["shards"].as_array().unwrap();
        let name = shards[0]["name"]
            .as_str()
            .expect("chaque shard doit exposer un champ name (string)");
        assert_eq!(name, "group-0", "name doit reprendre l'id en repli");
        assert!(!name.is_empty(), "name ne doit jamais être vide");
    }

    #[test]
    fn shard_map_flattens_boundary_rings_across_multiple_cells_in_one_shard() {
        use server::handoff::CellZone;

        let manifest =
            server::manifest::load(std::path::Path::new("../server/server.example.toml"))
                .expect("server.example.toml doit être chargeable");

        let ring_a = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let ring_b = vec![[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 3.0], [2.0, 2.0]];
        let zones = vec![ShardZone {
            addr: "group-0".to_string(),
            cells: vec![
                (
                    CellZone {
                        boundary_rings: vec![ring_a.clone()],
                    },
                    25.0,
                ),
                (
                    CellZone {
                        boundary_rings: vec![ring_b.clone()],
                    },
                    30.0,
                ),
            ],
        }];

        let v = shard_map_json(&manifest, &zones);
        let shards = v["shards"].as_array().unwrap();
        assert_eq!(shards.len(), 1);
        assert_eq!(shards[0]["id"], "group-0");
        let rings = shards[0]["boundaryRings"].as_array().unwrap();
        // Les deux cellules du shard ont chacune un anneau : les deux doivent apparaître,
        // aplatis dans le même tableau (le rayon tampon par cellule n'est pas exposé ici).
        assert_eq!(rings.len(), 2);
    }
}
