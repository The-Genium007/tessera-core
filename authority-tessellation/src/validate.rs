use crate::artifact::Artifact;
use std::collections::BTreeMap;

pub fn validate(art: &Artifact) -> Vec<String> {
    let mut v = Vec::new();

    // (1) raster complet & owners valides
    if art.raster.cells.len() != art.raster.nx * art.raster.ny {
        v.push(format!(
            "raster: taille {} != nx*ny {}",
            art.raster.cells.len(),
            art.raster.nx * art.raster.ny
        ));
    }
    if art.raster.cells.iter().any(|&o| o >= art.cells.len()) {
        v.push("raster: owner hors bornes (trou de partition)".to_string());
    }

    // (2) autorité non-ambiguë
    let mut owner_of: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &art.cells {
        for code in &c.district_codes {
            if let Some(prev) = owner_of.insert(code.as_str(), c.id) {
                v.push(format!(
                    "autorité ambiguë: quartier {code} dans cellules {prev} et {}",
                    c.id
                ));
            }
        }
    }

    // (3) r_inscribed >= r_min
    for c in &art.cells {
        if c.r_inscribed < c.r_min {
            v.push(format!(
                "cellule {} ({}): r_inscribed {:.0} < r_min {:.0}",
                c.id, c.code, c.r_inscribed, c.r_min
            ));
        }
    }

    // (4) contour présent
    for c in &art.cells {
        if c.boundary_rings.is_empty() {
            v.push(format!(
                "cellule {} ({}): aucun anneau de contour",
                c.id, c.code
            ));
        }
    }

    v
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
    fn valid_artifact_has_no_violations_except_possibly_rmin() {
        let art = generate(SAMPLE, None, &Params::default()).unwrap();
        let v = validate(&art);
        // La partition et la non-ambiguïté doivent toujours passer.
        assert!(!v.iter().any(|s| s.contains("raster")), "{v:?}");
        assert!(!v.iter().any(|s| s.contains("ambigu")), "{v:?}");
    }

    #[test]
    fn detects_ambiguous_authority() {
        let mut art = generate(SAMPLE, None, &Params::default()).unwrap();
        // Injecter un doublon de code dans deux cellules.
        let dup = art.cells[0].district_codes[0].clone();
        art.cells[1].district_codes.push(dup);
        let v = validate(&art);
        assert!(v.iter().any(|s| s.contains("ambigu")), "{v:?}");
    }
}
