use serde::{Deserialize, Serialize};

pub const DENSE_AREA_THRESHOLD: f64 = 2_000_000.0;
pub const EXCLUDED_CODES: [&str; 6] = ["AF2", "AW", "RA", "FWBS", "VT", "QC"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Regime {
    Dense,
    Periphery,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bbox {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Params {
    pub v_max: f64,
    pub rtt_handoff: f64,
    pub aoi_dense: f64,
    pub aoi_open: f64,
    pub w1: f64,
    pub w2: f64,
    pub seed: u64,
    pub grid_step: f64,
    pub dense_seed_count: usize,
    pub lloyd_iters: usize,
    pub world_bbox: Bbox,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            v_max: 55.0,
            rtt_handoff: 0.25,
            aoi_dense: 120.0,
            aoi_open: 600.0,
            w1: 1.0,
            w2: 0.5,
            seed: 0x5E55E5A,
            grid_step: 200.0,
            dense_seed_count: 4,
            lloyd_iters: 50,
            world_bbox: Bbox {
                min_x: -8400.0,
                max_x: 7800.0,
                min_y: -9100.0,
                max_y: 7050.0,
            },
        }
    }
}

impl Params {
    pub fn aoi(&self, r: Regime) -> f64 {
        match r {
            Regime::Dense => self.aoi_dense,
            Regime::Periphery => self.aoi_open,
        }
    }
    pub fn b(&self, r: Regime) -> f64 {
        self.aoi(r) + self.v_max * self.rtt_handoff
    }
    pub fn r_min(&self, r: Regime) -> f64 {
        5.0 * self.b(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r_min_dense_is_669() {
        let p = Params::default();
        assert!(
            (p.r_min(Regime::Dense) - 669.0).abs() < 1.0,
            "{}",
            p.r_min(Regime::Dense)
        );
    }

    #[test]
    fn r_min_periphery_is_3069() {
        let p = Params::default();
        assert!((p.r_min(Regime::Periphery) - 3069.0).abs() < 1.0);
    }

    #[test]
    fn kinematic_margin_is_about_14m() {
        let p = Params::default();
        assert!((p.v_max * p.rtt_handoff - 13.75).abs() < 0.01);
    }
}
