pub type Point = [f64; 2];

pub fn area(poly: &[Point]) -> f64 {
    let n = poly.len();
    let mut s = 0.0;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        s += a[0] * b[1] - b[0] * a[1];
    }
    s.abs() / 2.0
}

pub fn centroid(poly: &[Point]) -> Point {
    let n = poly.len() as f64;
    let (mut cx, mut cy) = (0.0, 0.0);
    for p in poly {
        cx += p[0];
        cy += p[1];
    }
    [cx / n, cy / n]
}

pub fn bbox(poly: &[Point]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in poly {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    (min_x, max_x, min_y, max_y)
}

pub fn point_in_polygon(p: Point, poly: &[Point]) -> bool {
    let (x, y) = (p[0], p[1]);
    let n = poly.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (poly[i][0], poly[i][1]);
        let (xj, yj) = (poly[j][0], poly[j][1]);
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn dist_point_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if dx == 0.0 && dy == 0.0 {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - qx).powi(2) + (p[1] - qy).powi(2)).sqrt()
}

pub fn dist_point_polygon(p: Point, poly: &[Point]) -> f64 {
    let n = poly.len();
    let mut best = f64::INFINITY;
    for i in 0..n {
        let d = dist_point_segment(p, poly[i], poly[(i + 1) % n]);
        best = best.min(d);
    }
    best
}

/// Plus grand cercle inscrit (pôle d'inaccessibilité) par échantillonnage de grille.
pub fn inscribed_radius(poly: &[Point], samples: usize) -> f64 {
    let (min_x, max_x, min_y, max_y) = bbox(poly);
    let mut best = 0.0;
    for i in 0..=samples {
        let x = min_x + (max_x - min_x) * i as f64 / samples as f64;
        for j in 0..=samples {
            let y = min_y + (max_y - min_y) * j as f64 / samples as f64;
            if point_in_polygon([x, y], poly) {
                let d = dist_point_polygon([x, y], poly);
                if d > best {
                    best = d;
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Point> {
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]
    }

    #[test]
    fn area_of_unit_square_is_100() {
        assert!((area(&square()) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn centroid_of_square_is_center() {
        let c = centroid(&square());
        assert!((c[0] - 5.0).abs() < 1e-9 && (c[1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn bbox_of_square() {
        assert_eq!(bbox(&square()), (0.0, 10.0, 0.0, 10.0));
    }

    #[test]
    fn point_inside_and_outside() {
        assert!(point_in_polygon([5.0, 5.0], &square()));
        assert!(!point_in_polygon([15.0, 5.0], &square()));
    }

    #[test]
    fn dist_to_polygon_edge() {
        assert!((dist_point_polygon([5.0, 15.0], &square()) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn inscribed_radius_of_square_is_half_side() {
        let r = inscribed_radius(&square(), 40);
        assert!((r - 5.0).abs() < 0.2, "r={r}");
    }
}
