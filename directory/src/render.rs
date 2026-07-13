//! Rendu 2D **abstrait** (polygones + labels) d'une topologie Voronoï — pas de rendu 3D, pas
//! d'asset du jeu, juste de la géométrie pour repérer une erreur d'imbrication avant tout test
//! in-game. Les zones ne sont plus des rectangles (BSP/AABB) mais des cellules d'autorité
//! polygonales (`CellZone::boundary_rings`), aplaties depuis `authority.json`.

use server::handoff::ShardZone;
use server::manifest::TopologyConfig;

/// Bornes du viewBox, calculées depuis les points de toutes les cellules de toutes les zones
/// (et non plus depuis `topo.splits`, qui n'existe plus dans le modèle Voronoï). Même marge de
/// 500.0 qu'avant ; même repli `(-2000.0, 2000.0)` quand `zones` est vide.
fn finite_bounds(zones: &[ShardZone]) -> (f32, f32, f32, f32) {
    let margin = 500.0;
    let mut xs: Vec<f32> = Vec::new();
    let mut ys: Vec<f32> = Vec::new();
    for z in zones {
        for (cell, _radius) in &z.cells {
            for ring in &cell.boundary_rings {
                for p in ring {
                    xs.push(p[0] as f32);
                    ys.push(p[1] as f32);
                }
            }
        }
    }
    let range = |vals: &[f32]| -> (f32, f32) {
        if vals.is_empty() {
            return (-2000.0, 2000.0);
        }
        let min = vals.iter().cloned().fold(f32::INFINITY, f32::min) - margin;
        let max = vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + margin;
        (min, max)
    };
    let (min_x, max_x) = range(&xs);
    let (min_y, max_y) = range(&ys);
    (min_x, max_x, min_y, max_y)
}

/// Centroïde approximatif d'une zone (moyenne de tous les points de tous ses anneaux), pour
/// positionner le label — il n'y a plus de rectangle dont prendre le centre.
fn approximate_centroid(z: &ShardZone) -> (f32, f32) {
    let mut sum_x = 0.0f32;
    let mut sum_y = 0.0f32;
    let mut count = 0u32;
    for (cell, _radius) in &z.cells {
        for ring in &cell.boundary_rings {
            for p in ring {
                sum_x += p[0] as f32;
                sum_y += p[1] as f32;
                count += 1;
            }
        }
    }
    if count == 0 {
        (0.0, 0.0)
    } else {
        (sum_x / count as f32, sum_y / count as f32)
    }
}

// `topo` n'est plus utilisé pour calculer les bornes (celles-ci viennent désormais de `zones`),
// mais le paramètre reste dans la signature pour ne pas toucher le call site de `main.rs`
// (hors périmètre de cette tâche).
pub fn render_svg(_topo: &TopologyConfig, zones: &[ShardZone]) -> String {
    let (min_x, max_x, min_y, max_y) = finite_bounds(zones);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{min_x} {min_y} {} {}\">\n",
        max_x - min_x,
        max_y - min_y
    );
    for z in zones {
        for (cell, _radius) in &z.cells {
            for ring in &cell.boundary_rings {
                let points = ring
                    .iter()
                    .map(|p| format!("{},{}", p[0], p[1]))
                    .collect::<Vec<_>>()
                    .join(" ");
                svg.push_str(&format!(
                    "  <polygon points=\"{points}\" fill=\"none\" stroke=\"black\"/>\n"
                ));
            }
        }
        let (cx, cy) = approximate_centroid(z);
        svg.push_str(&format!(
            "  <text x=\"{cx}\" y=\"{cy}\">{}</text>\n",
            z.addr
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use server::handoff::CellZone;
    use std::collections::HashMap;

    fn square(min_x: f64, min_y: f64, side: f64) -> Vec<[f64; 2]> {
        vec![
            [min_x, min_y],
            [min_x + side, min_y],
            [min_x + side, min_y + side],
            [min_x, min_y + side],
            [min_x, min_y],
        ]
    }

    #[test]
    fn renders_svg_with_shard_labels() {
        let topo = TopologyConfig {
            authority_artifact: "authority.json".into(),
            server_count: 2,
            radius_overrides: HashMap::new(),
        };
        let zones = vec![
            ShardZone {
                addr: "group-0".into(),
                cells: vec![(
                    CellZone {
                        boundary_rings: vec![square(0.0, 0.0, 10.0)],
                    },
                    25.0,
                )],
            },
            ShardZone {
                addr: "group-1".into(),
                cells: vec![(
                    CellZone {
                        boundary_rings: vec![square(1000.0, 0.0, 10.0)],
                    },
                    25.0,
                )],
            },
        ];
        let svg = render_svg(&topo, &zones);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("group-0"));
        assert!(svg.contains("group-1"));
        assert!(svg.ends_with("</svg>\n"));
        assert!(svg.contains("<polygon"));
    }
}
