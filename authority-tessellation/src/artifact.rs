use crate::adjacency::adjacency;
use crate::cluster::{cluster_dense, ProtoCell};
use crate::districts::{event_weight, load, District};
use crate::geometry::{inscribed_radius, Point};
use crate::params::{Params, Regime};
use crate::patterns::assignment_patterns;
use crate::periphery::proto_cells_periphery;
use crate::raster::{rasterize, trace_boundary, Grid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    pub id: usize,
    pub code: String,
    pub regime: Regime,
    pub authority_type: String,
    pub district_codes: Vec<String>,
    pub seed: Option<Point>,
    pub b: f64,
    pub r_min: f64,
    pub r_inscribed: f64,
    pub area: f64,
    pub estimated_load: f64,
    pub boundary_rings: Vec<Vec<Point>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterIndex {
    pub origin: Point,
    pub step: f64,
    pub nx: usize,
    pub ny: usize,
    pub cells: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub schema_version: u32,
    pub params: Params,
    pub cells: Vec<Cell>,
    pub adjacency: Vec<(usize, usize)>,
    pub portal_points: Vec<Point>,
    pub assignment_patterns: BTreeMap<usize, Vec<Vec<usize>>>,
    pub raster: RasterIndex,
}

fn cell_code(codes: &[String]) -> String {
    codes.join("-")
}

pub fn generate(
    districts_json: &str,
    manual_json: Option<&str>,
    p: &Params,
) -> anyhow::Result<Artifact> {
    let districts = load(districts_json, manual_json)?;
    let dense: Vec<District> = districts
        .iter()
        .filter(|d| d.regime == Regime::Dense)
        .cloned()
        .collect();
    let periph: Vec<District> = districts
        .iter()
        .filter(|d| d.regime == Regime::Periphery)
        .cloned()
        .collect();

    let mut protos: Vec<ProtoCell> = cluster_dense(&dense, p);
    protos.extend(proto_cells_periphery(&periph));
    // Ordre stable global.
    for c in &mut protos {
        c.district_codes.sort();
    }
    protos.sort_by(|a, b| a.district_codes[0].cmp(&b.district_codes[0]));

    // Sans proto-cellule, la rastérisation attribuerait le vide à une cellule inexistante
    // (index hors bornes plus bas) : on échoue proprement plutôt que de paniquer.
    if protos.is_empty() {
        anyhow::bail!("aucun quartier exploitable après filtrage — rien à tesseller");
    }

    let grid: Grid = rasterize(&protos, &districts, p);
    let adj = adjacency(&grid, protos.len());

    // event_weight par code (pour estimated_load)
    let w_by_code: BTreeMap<&str, f64> = districts
        .iter()
        .map(|d| (d.code.as_str(), event_weight(d)))
        .collect();
    let total_w: f64 = w_by_code.values().sum();

    // Charge estimée par proto-cellule, indexée exactement comme `protos` (donc comme
    // `adj` et `grid.owner`) — calculée AVANT les patterns : la fusion pondérée en dépend.
    let loads: Vec<f64> = protos
        .iter()
        .map(|proto| {
            proto
                .district_codes
                .iter()
                .filter_map(|c| w_by_code.get(c.as_str()))
                .sum::<f64>()
                / if total_w > 0.0 { total_w } else { 1.0 }
        })
        .collect();

    let patterns = assignment_patterns(protos.len(), &adj, &loads);

    let cell_area = grid.step * grid.step;
    let mut counts = vec![0usize; protos.len()];
    for &o in &grid.owner {
        counts[o] += 1;
    }

    let mut cells = Vec::new();
    for (i, proto) in protos.iter().enumerate() {
        let rings = trace_boundary(&grid, i);
        // r_inscribed sur l'anneau le plus grand (territoire final, vide inclus)
        let r_inscribed = rings
            .iter()
            .map(|r| inscribed_radius(r, 30))
            .fold(0.0, f64::max);
        cells.push(Cell {
            id: i,
            code: cell_code(&proto.district_codes),
            regime: proto.regime,
            authority_type: "cell".to_string(),
            district_codes: proto.district_codes.clone(),
            seed: proto.seed,
            b: p.b(proto.regime),
            r_min: p.r_min(proto.regime),
            r_inscribed,
            area: counts[i] as f64 * cell_area,
            estimated_load: loads[i],
            boundary_rings: rings,
        });
    }

    let raster = RasterIndex {
        origin: grid.origin,
        step: grid.step,
        nx: grid.nx,
        ny: grid.ny,
        cells: grid.owner,
    };

    Ok(Artifact {
        schema_version: 1,
        params: p.clone(),
        cells,
        adjacency: adj,
        portal_points: Vec::new(),
        assignment_patterns: patterns,
        raster,
    })
}

pub fn to_json(art: &Artifact) -> String {
    serde_json::to_string_pretty(art).expect("artefact sérialisable")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"entries":[
      {"id":"a","tweakdb_path":"Districts.Kabuki","code":"KAB","has_transform":true,
       "polygon":[[-1500,-1500],[-500,-1500],[-500,-500],[-1500,-500]]},
      {"id":"b","tweakdb_path":"Districts.LittleChina","code":"LC","has_transform":true,
       "polygon":[[-500,-1500],[500,-1500],[500,-500],[-500,-500]]},
      {"id":"c","tweakdb_path":"Districts.BiotechnicaFlats","code":"BF","has_transform":true,
       "polygon":[[2000,2000],[6000,2000],[6000,6000],[2000,6000]]}
    ],"warnings":[]}"#;

    #[test]
    fn generate_produces_cells_and_full_partition() {
        let p = Params::default();
        let art = generate(SAMPLE, None, &p).unwrap();
        assert!(!art.cells.is_empty());
        // raster couvre 100 % : chaque owner < nombre de cellules
        assert!(art.raster.cells.iter().all(|&o| o < art.cells.len()));
        assert_eq!(art.raster.cells.len(), art.raster.nx * art.raster.ny);
    }

    #[test]
    fn generation_is_deterministic() {
        let p = Params::default();
        let a = to_json(&generate(SAMPLE, None, &p).unwrap());
        let b = to_json(&generate(SAMPLE, None, &p).unwrap());
        assert_eq!(a, b);
    }

    #[test]
    fn each_district_in_exactly_one_cell() {
        let art = generate(SAMPLE, None, &Params::default()).unwrap();
        let mut seen: Vec<String> = art
            .cells
            .iter()
            .flat_map(|c| c.district_codes.clone())
            .collect();
        let n = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            n,
            "un quartier apparaît dans plusieurs cellules"
        );
    }

    #[test]
    fn empty_input_returns_err_not_panic() {
        let err = generate(r#"{"entries":[],"warnings":[]}"#, None, &Params::default());
        assert!(
            err.is_err(),
            "générer sans quartier doit échouer proprement, pas paniquer"
        );
    }
    #[test]
    fn hottest_cell_stays_alone_at_first_merge() {
        // Trois quartiers en périphérie (aire > DENSE_AREA_THRESHOLD = 3.2 km², donc une
        // proto-cellule chacun, triés par code : AAA=0, MMM=1, ZZZ=2). event_weight = 1/aire :
        // AAA (2×2 km, 4 km²) est la cellule CHAUDE ; MMM et ZZZ (4×4 km, 16 km²) sont froides.
        // Disposés en trois bandes verticales dans le raster → adjacence en chaîne AAA—MMM—ZZZ.
        // À N = cells-1 (une seule fusion), la paire fusionnée doit être la plus froide
        // (MMM, ZZZ) — et surtout NE PAS contenir AAA.
        let sample = r#"{"entries":[
          {"id":"a","tweakdb_path":"Districts.HotSpot","code":"AAA","has_transform":true,
           "polygon":[[-7000,-1000],[-5000,-1000],[-5000,1000],[-7000,1000]]},
          {"id":"b","tweakdb_path":"Districts.ColdMid","code":"MMM","has_transform":true,
           "polygon":[[-3000,-2000],[1000,-2000],[1000,2000],[-3000,2000]]},
          {"id":"c","tweakdb_path":"Districts.ColdEast","code":"ZZZ","has_transform":true,
           "polygon":[[3000,-2000],[7000,-2000],[7000,2000],[3000,2000]]}
        ],"warnings":[]}"#;
        let art = generate(sample, None, &Params::default()).unwrap();
        assert_eq!(art.cells.len(), 3);
        let hottest = art
            .cells
            .iter()
            .max_by(|a, b| a.estimated_load.partial_cmp(&b.estimated_load).unwrap())
            .unwrap();
        assert_eq!(
            hottest.code, "AAA",
            "la plus petite aire doit être la plus chaude"
        );
        let n = art.cells.len() - 1;
        let pattern = &art.assignment_patterns[&n];
        let hot_group = pattern.iter().find(|g| g.contains(&hottest.id)).unwrap();
        assert_eq!(
            hot_group.len(),
            1,
            "la cellule la plus chaude ne doit pas participer à la première fusion: {pattern:?}"
        );
    }
}
