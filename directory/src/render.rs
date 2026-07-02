//! Rendu 2D **abstrait** (rectangles + labels) d'une topologie aplatie — pas de rendu 3D, pas
//! d'asset du jeu, juste de la géométrie pour repérer une erreur d'imbrication avant tout test
//! in-game.

use server::handoff::ShardZone;
use server::manifest::{Axis, TopologyConfig};

fn finite_bounds(topo: &TopologyConfig) -> (f32, f32, f32, f32) {
    let margin = 500.0;
    let mut xs: Vec<f32> = Vec::new();
    let mut ys: Vec<f32> = Vec::new();
    for s in &topo.splits {
        match s.axis {
            Axis::X => xs.push(s.at),
            Axis::Y => ys.push(s.at),
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

pub fn render_svg(topo: &TopologyConfig, zones: &[ShardZone]) -> String {
    let (min_x, max_x, min_y, max_y) = finite_bounds(topo);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{min_x} {min_y} {} {}\">\n",
        max_x - min_x,
        max_y - min_y
    );
    for z in zones {
        let x = z.zone.min_x.max(min_x);
        let y = z.zone.min_y.max(min_y);
        let w = z.zone.max_x.min(max_x) - x;
        let h = z.zone.max_y.min(max_y) - y;
        svg.push_str(&format!(
            "  <rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"none\" stroke=\"black\"/>\n"
        ));
        svg.push_str(&format!(
            "  <text x=\"{}\" y=\"{}\">{}</text>\n",
            x + w / 2.0,
            y + h / 2.0,
            z.addr
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use server::handoff::Aabb;

    #[test]
    fn renders_svg_with_shard_labels() {
        let topo = TopologyConfig {
            active_preset: "2-shards".into(),
            splits: vec![],
            shards: vec![],
        };
        let zones = vec![
            ShardZone {
                addr: "127.0.0.1:27030".into(),
                zone: Aabb {
                    min_x: f32::NEG_INFINITY,
                    max_x: 1000.0,
                    min_y: f32::NEG_INFINITY,
                    max_y: f32::INFINITY,
                },
            },
            ShardZone {
                addr: "127.0.0.1:27031".into(),
                zone: Aabb {
                    min_x: 1000.0,
                    max_x: f32::INFINITY,
                    min_y: f32::NEG_INFINITY,
                    max_y: f32::INFINITY,
                },
            },
        ];
        let svg = render_svg(&topo, &zones);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("127.0.0.1:27030"));
        assert!(svg.contains("127.0.0.1:27031"));
        assert!(svg.ends_with("</svg>\n"));
    }
}
