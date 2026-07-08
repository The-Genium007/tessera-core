use crate::cluster::ProtoCell;
use crate::districts::District;
use crate::params::Regime;

/// Une proto-cellule par grand quartier de la périphérie (`seed = None`).
/// Le vide inter-quartiers est attaché plus tard par la rastérisation.
pub fn proto_cells_periphery(periph: &[District]) -> Vec<ProtoCell> {
    let mut cells: Vec<ProtoCell> = periph
        .iter()
        .map(|d| ProtoCell {
            district_codes: vec![d.code.clone()],
            seed: None,
            regime: Regime::Periphery,
        })
        .collect();
    cells.sort_by(|a, b| a.district_codes[0].cmp(&b.district_codes[0]));
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::districts::District;
    use crate::params::Regime;

    fn d(code: &str) -> District {
        District {
            code: code.into(),
            name: code.into(),
            polygon: vec![[0.0, 0.0], [3000.0, 0.0], [3000.0, 3000.0], [0.0, 3000.0]],
            synthetic: false,
            area: 9_000_000.0,
            centroid: [1500.0, 1500.0],
            regime: Regime::Periphery,
        }
    }

    #[test]
    fn one_cell_per_big_district() {
        let cells = proto_cells_periphery(&[d("BF"), d("JP")]);
        assert_eq!(cells.len(), 2);
        assert!(cells
            .iter()
            .all(|c| c.seed.is_none() && c.regime == Regime::Periphery));
    }
}
