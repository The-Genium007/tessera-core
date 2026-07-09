use crate::artifact::Artifact;
use crate::params::Regime;

pub fn report(art: &Artifact) -> String {
    let mut out = String::new();
    out.push_str(&format!("Cellules : {}\n", art.cells.len()));

    let overhead = |regime: Regime| -> Option<f64> {
        let cells: Vec<&_> = art.cells.iter().filter(|c| c.regime == regime).collect();
        if cells.is_empty() {
            return None;
        }
        let area: f64 = cells.iter().map(|c| c.area).sum();
        if area == 0.0 {
            return None;
        }
        // 2b/r pondéré par aire
        let w: f64 = cells
            .iter()
            .map(|c| {
                if c.r_inscribed > 0.0 {
                    (2.0 * c.b / c.r_inscribed) * c.area
                } else {
                    0.0
                }
            })
            .sum();
        Some(w / area)
    };

    for (label, regime) in [("dense", Regime::Dense), ("périphérie", Regime::Periphery)] {
        if let Some(o) = overhead(regime) {
            out.push_str(&format!(
                "Aire dupliquée (overhead ghosts) {label} : {:.0}%\n",
                o * 100.0
            ));
        }
    }
    // overhead global pondéré aire
    let total_area: f64 = art.cells.iter().map(|c| c.area).sum();
    if total_area > 0.0 {
        let g: f64 = art
            .cells
            .iter()
            .map(|c| {
                if c.r_inscribed > 0.0 {
                    (2.0 * c.b / c.r_inscribed) * c.area
                } else {
                    0.0
                }
            })
            .sum::<f64>()
            / total_area;
        out.push_str(&format!("Aire dupliquée globale : {:.0}%\n", g * 100.0));
    }

    // variance de charge
    let loads: Vec<f64> = art.cells.iter().map(|c| c.estimated_load).collect();
    let mean = loads.iter().sum::<f64>() / loads.len().max(1) as f64;
    let var = loads.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / loads.len().max(1) as f64;
    out.push_str(&format!(
        "Variance de charge : {var:.4} (moyenne {mean:.3})\n"
    ));

    // top-5 frontières les plus chargées
    let mut edges: Vec<(f64, usize, usize)> = art
        .adjacency
        .iter()
        .map(|&(a, b)| {
            (
                art.cells[a].estimated_load + art.cells[b].estimated_load,
                a,
                b,
            )
        })
        .collect();
    edges.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap());
    out.push_str("Top frontières (charge cumulée) :\n");
    for (load, a, b) in edges.iter().take(5) {
        out.push_str(&format!(
            "  {} — {} : {:.3}\n",
            art.cells[*a].code, art.cells[*b].code, load
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::generate;
    use crate::params::Params;

    const SAMPLE: &str = r#"{"entries":[
      {"id":"a","tweakdb_path":"Districts.Kabuki","code":"KAB","has_transform":true,
       "polygon":[[-1500,-1500],[-500,-1500],[-500,-500],[-1500,-500]]},
      {"id":"c","tweakdb_path":"Districts.BiotechnicaFlats","code":"BF","has_transform":true,
       "polygon":[[2000,2000],[6000,2000],[6000,6000],[2000,6000]]}
    ],"warnings":[]}"#;

    #[test]
    fn report_mentions_cell_count_and_overhead() {
        let art = generate(SAMPLE, None, &Params::default()).unwrap();
        let r = report(&art);
        // Le rapport capitalise le début de chaque ligne (« Cellules : » etc.) ;
        // comparaison insensible à la casse pour ne pas coupler le test au style d'affichage.
        let lower = r.to_lowercase();
        assert!(lower.contains("cellules"));
        assert!(lower.contains("overhead") || lower.contains("dupliquée"));
    }
}
