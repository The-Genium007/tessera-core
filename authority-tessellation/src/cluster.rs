use crate::districts::{event_weight, District};
use crate::geometry::Point;
use crate::params::{Params, Regime};
use crate::prng::SplitMix64;

#[derive(Debug, Clone)]
pub struct ProtoCell {
    pub district_codes: Vec<String>,
    pub seed: Option<Point>,
    pub regime: Regime,
}

fn dist2(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Germes initiaux : k-means++ pondéré déterministe (PRNG splitmix64).
fn init_seeds(dense: &[District], k: usize, rng: &mut SplitMix64) -> Vec<Point> {
    let mut seeds = Vec::new();
    // Premier germe : tirage pondéré par event_weight.
    let weights: Vec<f64> = dense.iter().map(event_weight).collect();
    let total: f64 = weights.iter().sum();
    let pick = |cum_target: f64, w: &[f64]| -> usize {
        let mut acc = 0.0;
        for (i, wi) in w.iter().enumerate() {
            acc += wi;
            if acc >= cum_target {
                return i;
            }
        }
        w.len() - 1
    };
    let first = pick(rng.next_f64() * total, &weights);
    seeds.push(dense[first].centroid);
    while seeds.len() < k {
        // D²-pondéré (k-means++), pondéré aussi par event_weight.
        let d2: Vec<f64> = dense
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let nearest = seeds
                    .iter()
                    .map(|s| dist2(d.centroid, *s))
                    .fold(f64::INFINITY, f64::min);
                nearest * weights[i]
            })
            .collect();
        let sum: f64 = d2.iter().sum();
        if sum == 0.0 {
            break;
        }
        let idx = pick(rng.next_f64() * sum, &d2);
        seeds.push(dense[idx].centroid);
    }
    seeds
}

fn assign(dense: &[District], seeds: &[Point]) -> Vec<usize> {
    dense
        .iter()
        .map(|d| {
            let mut best = 0;
            let mut best_d = f64::INFINITY;
            for (i, s) in seeds.iter().enumerate() {
                let dd = dist2(d.centroid, *s);
                if dd < best_d {
                    best_d = dd;
                    best = i;
                }
            }
            best
        })
        .collect()
}

fn move_seeds(dense: &[District], assign: &[usize], k: usize) -> Vec<Point> {
    let mut acc = vec![[0.0f64; 2]; k];
    let mut wsum = vec![0.0f64; k];
    for (d, &a) in dense.iter().zip(assign) {
        let w = event_weight(d);
        acc[a][0] += d.centroid[0] * w;
        acc[a][1] += d.centroid[1] * w;
        wsum[a] += w;
    }
    acc.iter()
        .zip(wsum)
        .map(|(a, w)| {
            if w > 0.0 {
                [a[0] / w, a[1] / w]
            } else {
                [0.0, 0.0]
            }
        })
        .collect()
}

pub fn cluster_dense(dense: &[District], p: &Params) -> Vec<ProtoCell> {
    if dense.is_empty() {
        return Vec::new();
    }
    let k = p.dense_seed_count.min(dense.len()).max(1);
    let mut rng = SplitMix64(p.seed);
    let mut seeds = init_seeds(dense, k, &mut rng);
    let mut a = assign(dense, &seeds);
    for _ in 0..p.lloyd_iters {
        seeds = move_seeds(dense, &a, seeds.len());
        a = assign(dense, &seeds);
    }
    // Regrouper par germe, ignorer les clusters vides.
    let mut cells: Vec<ProtoCell> = Vec::new();
    for (i, s) in seeds.iter().enumerate() {
        let codes: Vec<String> = dense
            .iter()
            .zip(&a)
            .filter(|(_, &aa)| aa == i)
            .map(|(d, _)| d.code.clone())
            .collect();
        if !codes.is_empty() {
            cells.push(ProtoCell {
                district_codes: codes,
                seed: Some(*s),
                regime: Regime::Dense,
            });
        }
    }
    // Ordre stable : par premier code trié.
    for c in &mut cells {
        c.district_codes.sort();
    }
    cells.sort_by(|x, y| x.district_codes[0].cmp(&y.district_codes[0]));
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::districts::District;
    use crate::geometry::Point;
    use crate::params::{Params, Regime};

    fn d(code: &str, cx: f64, cy: f64) -> District {
        let poly: Vec<Point> = vec![
            [cx - 200.0, cy - 200.0],
            [cx + 200.0, cy - 200.0],
            [cx + 200.0, cy + 200.0],
            [cx - 200.0, cy + 200.0],
        ];
        District {
            code: code.into(),
            name: code.into(),
            polygon: poly.clone(),
            synthetic: false,
            area: 160000.0,
            centroid: [cx, cy],
            regime: Regime::Dense,
        }
    }

    #[test]
    fn every_district_assigned_exactly_once() {
        let dense = vec![
            d("A", 0.0, 0.0),
            d("B", 100.0, 0.0),
            d("C", 5000.0, 0.0),
            d("D", 5100.0, 0.0),
        ];
        let p = Params {
            dense_seed_count: 2,
            ..Params::default()
        };
        let cells = cluster_dense(&dense, &p);
        let mut all: Vec<String> = cells
            .iter()
            .flat_map(|c| c.district_codes.clone())
            .collect();
        all.sort();
        assert_eq!(all, vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn deterministic_across_runs() {
        let dense = vec![
            d("A", 0.0, 0.0),
            d("B", 100.0, 0.0),
            d("C", 5000.0, 0.0),
            d("D", 5100.0, 0.0),
        ];
        let p = Params {
            dense_seed_count: 2,
            ..Params::default()
        };
        let a = cluster_dense(&dense, &p);
        let b = cluster_dense(&dense, &p);
        let fa: Vec<Vec<String>> = a.iter().map(|c| c.district_codes.clone()).collect();
        let fb: Vec<Vec<String>> = b.iter().map(|c| c.district_codes.clone()).collect();
        assert_eq!(fa, fb);
    }

    #[test]
    fn two_far_clusters_separate() {
        // A,B proches à l'origine ; C,D proches à x=5000 -> 2 clusters {A,B} et {C,D}.
        let dense = vec![
            d("A", 0.0, 0.0),
            d("B", 100.0, 0.0),
            d("C", 5000.0, 0.0),
            d("D", 5100.0, 0.0),
        ];
        let p = Params {
            dense_seed_count: 2,
            ..Params::default()
        };
        let cells = cluster_dense(&dense, &p);
        assert_eq!(cells.len(), 2);
        for c in &cells {
            let mut codes = c.district_codes.clone();
            codes.sort();
            assert!(
                codes == vec!["A", "B"] || codes == vec!["C", "D"],
                "{codes:?}"
            );
        }
    }
}
