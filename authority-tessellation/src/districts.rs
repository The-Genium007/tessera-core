use crate::geometry::{area, bbox, centroid, Point};
use crate::params::{Regime, DENSE_AREA_THRESHOLD, EXCLUDED_CODES};
use serde::Deserialize;

/// Étendue minimale (mètres) pour qu'un `Districts.*` compte comme quartier géographique.
/// En dessous, c'est un marqueur d'intérieur/POI (étage de tour, boutique, club) — exclu de la
/// topologie d'autorité, sinon il pollue le clustering et déforme les cellules. Les zones
/// manuelles (`synthetic`, ex. la station orbitale) échappent au filtre.
pub const MIN_DISTRICT_SPAN: f64 = 50.0;

#[derive(Debug, Clone)]
pub struct District {
    pub code: String,
    pub name: String,
    pub polygon: Vec<Point>,
    pub synthetic: bool,
    pub area: f64,
    pub centroid: Point,
    pub regime: Regime,
}

#[derive(Deserialize)]
struct RawEntry {
    tweakdb_path: String,
    code: String,
    polygon: Vec<Point>,
    #[serde(default)]
    synthetic: bool,
}

#[derive(Deserialize)]
struct RawFile {
    entries: Vec<RawEntry>,
}

fn build(raw: RawEntry) -> Option<District> {
    if EXCLUDED_CODES.contains(&raw.code.as_str()) || raw.polygon.len() < 3 {
        return None;
    }
    let (min_x, max_x, min_y, max_y) = bbox(&raw.polygon);
    let span = (max_x - min_x).max(max_y - min_y);
    if !raw.synthetic && span < MIN_DISTRICT_SPAN {
        return None;
    }
    let a = area(&raw.polygon);
    let regime = if a < DENSE_AREA_THRESHOLD {
        Regime::Dense
    } else {
        Regime::Periphery
    };
    let name = raw
        .tweakdb_path
        .rsplit('.')
        .next()
        .unwrap_or(&raw.code)
        .to_string();
    Some(District {
        code: raw.code,
        name,
        centroid: centroid(&raw.polygon),
        polygon: raw.polygon,
        synthetic: raw.synthetic,
        area: a,
        regime,
    })
}

pub fn load(districts_json: &str, manual_json: Option<&str>) -> anyhow::Result<Vec<District>> {
    let file: RawFile = serde_json::from_str(districts_json)?;
    let mut out: Vec<District> = file.entries.into_iter().filter_map(build).collect();
    if let Some(m) = manual_json {
        let manual: Vec<RawEntry> = serde_json::from_str(m)?;
        out.extend(manual.into_iter().filter_map(build));
    }
    // Ordre stable (déterminisme) : par code.
    out.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(out)
}

pub fn event_weight(d: &District) -> f64 {
    1.0 / d.area
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Regime;

    const SAMPLE: &str = r#"{"entries":[
      {"id":"a","tweakdb_path":"Districts.Kabuki","code":"KAB","has_transform":true,
       "polygon":[[0,0],[500,0],[500,500],[0,500]]},
      {"id":"b","tweakdb_path":"Districts.BiotechnicaFlats","code":"BF","has_transform":true,
       "polygon":[[0,0],[3000,0],[3000,3000],[0,3000]]},
      {"id":"c","tweakdb_path":"Districts.VR_Tutorial","code":"VT","has_transform":true,
       "polygon":[[0,0],[400,0],[400,400],[0,400]]}
    ],"warnings":[]}"#;

    #[test]
    fn excludes_non_geographic_codes() {
        let ds = load(SAMPLE, None).unwrap();
        assert!(ds.iter().all(|d| d.code != "VT"));
    }

    #[test]
    fn classifies_regime_by_area() {
        let ds = load(SAMPLE, None).unwrap();
        let kab = ds.iter().find(|d| d.code == "KAB").unwrap();
        let bf = ds.iter().find(|d| d.code == "BF").unwrap();
        assert_eq!(kab.regime, Regime::Dense);
        assert_eq!(bf.regime, Regime::Periphery);
    }

    #[test]
    fn name_is_last_tweakdb_segment() {
        let ds = load(SAMPLE, None).unwrap();
        assert_eq!(ds.iter().find(|d| d.code == "KAB").unwrap().name, "Kabuki");
    }

    #[test]
    fn manual_zones_are_appended() {
        let manual = r#"[{"id":"m","tweakdb_path":"Manual.OrbitalStation","code":"ORB",
          "has_transform":null,"synthetic":true,"note":"x",
          "polygon":[[9000,0],[9400,0],[9400,400],[9000,400]]}]"#;
        let ds = load(SAMPLE, Some(manual)).unwrap();
        let orb = ds.iter().find(|d| d.code == "ORB").unwrap();
        assert!(orb.synthetic);
    }

    #[test]
    fn excludes_small_poi_by_span() {
        // Marqueur d'intérieur (bbox 30x30 < 50) — exclu ; le vrai quartier reste.
        let json = r#"{"entries":[
          {"id":"p","tweakdb_path":"Districts.MistysShop","code":"MS","has_transform":true,
           "polygon":[[0,0],[30,0],[30,30],[0,30]]},
          {"id":"k","tweakdb_path":"Districts.Kabuki","code":"KAB","has_transform":true,
           "polygon":[[0,0],[500,0],[500,500],[0,500]]}
        ],"warnings":[]}"#;
        let ds = load(json, None).unwrap();
        assert!(
            ds.iter().all(|d| d.code != "MS"),
            "le POI MS aurait dû être exclu"
        );
        assert!(
            ds.iter().any(|d| d.code == "KAB"),
            "le vrai quartier KAB doit rester"
        );
    }

    #[test]
    fn synthetic_small_zone_is_kept_despite_span() {
        // Une zone manuelle petite ne doit PAS être filtrée par la taille.
        let manual = r#"[{"id":"s","tweakdb_path":"Manual.Tiny","code":"TNY",
          "synthetic":true,"polygon":[[0,0],[20,0],[20,20],[0,20]]}]"#;
        let ds = load(r#"{"entries":[],"warnings":[]}"#, Some(manual)).unwrap();
        assert!(
            ds.iter().any(|d| d.code == "TNY"),
            "zone synthétique petite conservée"
        );
    }
}
