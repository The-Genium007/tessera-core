use crate::artifact::Artifact;

fn load_color(load: f64) -> String {
    // charge 0 -> bleu froid, charge élevée -> rouge chaud (interpolation simple)
    let t = load.clamp(0.0, 1.0);
    let r = (60.0 + t * 180.0) as u8;
    let g = (90.0 + (1.0 - t) * 90.0) as u8;
    let b = (200.0 - t * 150.0) as u8;
    format!("rgb({r},{g},{b})")
}

pub fn render_svg(art: &Artifact) -> String {
    let bb = art.params.world_bbox;
    let w = bb.max_x - bb.min_x;
    let h = bb.max_y - bb.min_y;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {:.0} {:.0}\" font-family=\"monospace\">\n",
        w, h
    );
    // monde -> écran : x'=x-min_x ; y'=max_y - y (Y inversé)
    let sx = |x: f64| x - bb.min_x;
    let sy = |y: f64| bb.max_y - y;
    for c in &art.cells {
        let fill = load_color(c.estimated_load);
        for ring in &c.boundary_rings {
            let pts: Vec<String> = ring
                .iter()
                .map(|p| format!("{:.1},{:.1}", sx(p[0]), sy(p[1])))
                .collect();
            s.push_str(&format!(
                "<polygon points=\"{}\" fill=\"{}\" fill-opacity=\"0.55\" stroke=\"#111\" stroke-width=\"12\"><title>{} (charge {:.2}, r_in {:.0})</title></polygon>\n",
                pts.join(" "),
                fill,
                c.code,
                c.estimated_load,
                c.r_inscribed
            ));
        }
    }
    s.push_str("</svg>\n");
    s
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
    fn svg_is_wellformed_and_has_polygons() {
        let art = generate(SAMPLE, None, &Params::default()).unwrap();
        let s = render_svg(&art);
        assert!(s.starts_with("<svg"));
        assert!(s.contains("</svg>"));
        assert!(s.contains("<polygon"));
    }
}
