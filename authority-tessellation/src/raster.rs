use crate::cluster::ProtoCell;
use crate::districts::District;
use crate::geometry::{dist_point_polygon, point_in_polygon, Point};
use crate::params::Params;
use std::collections::{BTreeMap, HashMap};

pub struct Grid {
    pub origin: Point,
    pub step: f64,
    pub nx: usize,
    pub ny: usize,
    pub owner: Vec<usize>,
}

pub fn rasterize(cells: &[ProtoCell], districts: &[District], p: &Params) -> Grid {
    let bb = p.world_bbox;
    let step = p.grid_step;
    let nx = ((bb.max_x - bb.min_x) / step).ceil() as usize;
    let ny = ((bb.max_y - bb.min_y) / step).ceil() as usize;
    let origin = [bb.min_x, bb.min_y];

    // code -> index de proto-cellule
    let mut code_to_cell: HashMap<&str, usize> = HashMap::new();
    for (ci, c) in cells.iter().enumerate() {
        for code in &c.district_codes {
            code_to_cell.insert(code.as_str(), ci);
        }
    }
    // district par code (pour PIP et distance)
    let by_code: HashMap<&str, &District> =
        districts.iter().map(|d| (d.code.as_str(), d)).collect();

    let mut owner = vec![0usize; nx * ny];
    for i in 0..nx {
        let x = origin[0] + (i as f64 + 0.5) * step;
        for j in 0..ny {
            let y = origin[1] + (j as f64 + 0.5) * step;
            let pt = [x, y];
            // 1) dans un quartier ? -> sa cellule (le plus petit quartier gagne en cas de recouvrement)
            let mut owner_cell: Option<usize> = None;
            let mut best_area = f64::INFINITY;
            for d in districts {
                if point_in_polygon(pt, &d.polygon) && d.area < best_area {
                    if let Some(&ci) = code_to_cell.get(d.code.as_str()) {
                        best_area = d.area;
                        owner_cell = Some(ci);
                    }
                }
            }
            // 2) sinon -> cellule dont un quartier membre est le plus proche
            let ci = owner_cell.unwrap_or_else(|| {
                let mut best = 0usize;
                let mut best_d = f64::INFINITY;
                for (idx, c) in cells.iter().enumerate() {
                    for code in &c.district_codes {
                        if let Some(d) = by_code.get(code.as_str()) {
                            let dd = dist_point_polygon(pt, &d.polygon);
                            if dd < best_d {
                                best_d = dd;
                                best = idx;
                            }
                        }
                    }
                }
                best
            });
            owner[i * ny + j] = ci;
        }
    }
    Grid {
        origin,
        step,
        nx,
        ny,
        owner,
    }
}

/// Suit le bord de la cellule `cell_idx` et rend des anneaux fermés (coords monde).
/// Arêtes orientées avec la cellule possédée à gauche (boucles anti-horaires) ; tout est
/// déterministe (BTreeMap + choix du plus petit sommet/arête).
pub fn trace_boundary(grid: &Grid, cell_idx: usize) -> Vec<Vec<Point>> {
    let corner = |i: usize, j: usize| -> Point {
        [
            ((grid.origin[0] + i as f64 * grid.step) * 1000.0).round() / 1000.0,
            ((grid.origin[1] + j as f64 * grid.step) * 1000.0).round() / 1000.0,
        ]
    };
    let owns = |i: i64, j: i64| -> bool {
        i >= 0
            && j >= 0
            && (i as usize) < grid.nx
            && (j as usize) < grid.ny
            && grid.owner[i as usize * grid.ny + j as usize] == cell_idx
    };
    type Key = (i64, i64);
    let key = |p: Point| -> Key {
        (
            (p[0] * 1000.0).round() as i64,
            (p[1] * 1000.0).round() as i64,
        )
    };
    let unkey = |k: Key| -> Point { [k.0 as f64 / 1000.0, k.1 as f64 / 1000.0] };

    // Arêtes de bord orientées (cellule possédée à gauche de a->b). Un sommet peut avoir
    // plusieurs sorties (points de contact degré-4) : on stocke un Vec et on le consomme.
    let mut out: BTreeMap<Key, Vec<Key>> = BTreeMap::new();
    let add = |a: Point, b: Point, out: &mut BTreeMap<Key, Vec<Key>>| {
        out.entry(key(a)).or_default().push(key(b));
    };
    for i in 0..grid.nx {
        for j in 0..grid.ny {
            if grid.owner[i * grid.ny + j] != cell_idx {
                continue;
            }
            let (ii, jj) = (i as i64, j as i64);
            if !owns(ii, jj - 1) {
                add(corner(i, j), corner(i + 1, j), &mut out);
            }
            if !owns(ii + 1, jj) {
                add(corner(i + 1, j), corner(i + 1, j + 1), &mut out);
            }
            if !owns(ii, jj + 1) {
                add(corner(i + 1, j + 1), corner(i, j + 1), &mut out);
            }
            if !owns(ii - 1, jj) {
                add(corner(i, j + 1), corner(i, j), &mut out);
            }
        }
    }
    for v in out.values_mut() {
        v.sort();
    }

    let mut rings: Vec<Vec<Point>> = Vec::new();
    while let Some(start) = out.iter().find(|(_, v)| !v.is_empty()).map(|(k, _)| *k) {
        let mut ring_keys: Vec<Key> = vec![start];
        let mut cur = start;
        loop {
            let nexts = out.get_mut(&cur).expect("sommet de bord présent");
            if nexts.is_empty() {
                break;
            }
            let nxt = nexts.remove(0);
            ring_keys.push(nxt);
            cur = nxt;
            if nxt == start {
                break;
            }
        }
        if ring_keys.len() >= 4 && ring_keys.first() == ring_keys.last() {
            rings.push(ring_keys.into_iter().map(unkey).collect());
        }
    }
    rings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::ProtoCell;
    use crate::districts::District;
    use crate::params::{Bbox, Params, Regime};

    fn dcell(code: &str, x0: f64, x1: f64) -> (District, ProtoCell) {
        let poly = vec![[x0, -1000.0], [x1, -1000.0], [x1, 1000.0], [x0, 1000.0]];
        let d = District {
            code: code.into(),
            name: code.into(),
            polygon: poly,
            synthetic: false,
            area: (x1 - x0) * 2000.0,
            centroid: [(x0 + x1) / 2.0, 0.0],
            regime: Regime::Dense,
        };
        let c = ProtoCell {
            district_codes: vec![code.into()],
            seed: Some([(x0 + x1) / 2.0, 0.0]),
            regime: Regime::Dense,
        };
        (d, c)
    }

    fn tiny_params() -> Params {
        Params {
            grid_step: 500.0,
            world_bbox: Bbox {
                min_x: -2000.0,
                max_x: 2000.0,
                min_y: -2000.0,
                max_y: 2000.0,
            },
            ..Params::default()
        }
    }

    #[test]
    fn every_grid_cell_is_owned() {
        let (d1, c1) = dcell("A", -1000.0, 0.0);
        let (d2, c2) = dcell("B", 0.0, 1000.0);
        let g = rasterize(&[c1, c2], &[d1, d2], &tiny_params());
        assert!(g.owner.iter().all(|&o| o < 2));
        assert_eq!(g.owner.len(), g.nx * g.ny);
    }

    #[test]
    fn void_attaches_to_nearest_cell() {
        // Point loin à droite (x=1800) -> plus proche de B.
        let (d1, c1) = dcell("A", -1000.0, -500.0);
        let (d2, c2) = dcell("B", 500.0, 1000.0);
        let p = tiny_params();
        let g = rasterize(&[c1, c2], &[d1, d2], &p);
        let i = ((1800.0 - g.origin[0]) / g.step) as usize;
        let j = ((0.0 - g.origin[1]) / g.step) as usize;
        assert_eq!(g.owner[i * g.ny + j], 1);
    }

    #[test]
    fn boundary_is_closed_ring() {
        let (d1, c1) = dcell("A", -1000.0, 0.0);
        let (d2, c2) = dcell("B", 0.0, 1000.0);
        let g = rasterize(&[c1, c2], &[d1, d2], &tiny_params());
        let rings = trace_boundary(&g, 0);
        assert!(!rings.is_empty());
        for r in &rings {
            assert_eq!(r.first(), r.last());
        }
    }

    #[test]
    fn boundary_is_deterministic() {
        let (d1, c1) = dcell("A", -1000.0, 0.0);
        let (d2, c2) = dcell("B", 0.0, 1000.0);
        let g = rasterize(&[c1, c2], &[d1, d2], &tiny_params());
        let rings1 = trace_boundary(&g, 0);
        let rings2 = trace_boundary(&g, 0);
        assert_eq!(rings1, rings2);
    }
}
