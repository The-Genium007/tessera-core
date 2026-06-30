//! Logique de handoff (M4) : topologie des shards (zones AABB), calcul du placement d'un joueur
//! (shard autoritaire + shards en zone tampon), rayon par rang, et machine de chargement.
//! Pur et testable sans GNS/TCP.

/// Boîte alignée sur les axes (zone d'un shard sur le plan X/Y ; Z ignoré pour le sharding).
#[derive(Debug, Clone, Copy)]
pub struct Aabb {
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
}

impl Aabb {
    /// Appartenance demi-ouverte `[min, max)` → un point sur une frontière partagée n'appartient
    /// qu'à une seule zone (déterminisme ; pas de double-appartenance).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.min_x <= x && x < self.max_x && self.min_y <= y && y < self.max_y
    }

    /// Distance euclidienne du point à la boîte (0 si dedans). Pour un voisin partageant un bord,
    /// c'est la distance perpendiculaire au bord ; pour un voisin diagonal, la distance au coin.
    pub fn dist(&self, x: f32, y: f32) -> f32 {
        let dx = if x < self.min_x {
            self.min_x - x
        } else if x > self.max_x {
            x - self.max_x
        } else {
            0.0
        };
        let dy = if y < self.min_y {
            self.min_y - y
        } else if y > self.max_y {
            y - self.max_y
        } else {
            0.0
        };
        (dx * dx + dy * dy).sqrt()
    }
}

/// Un shard et la zone du monde dont il est responsable. `addr` sert aussi d'identifiant.
#[derive(Debug, Clone)]
pub struct ShardZone {
    pub addr: String,
    pub zone: Aabb,
}

/// L'ensemble des shards et leurs zones (une tuile du monde par shard).
#[derive(Debug, Clone)]
pub struct ShardTopology {
    pub shards: Vec<ShardZone>,
}

/// Résultat du placement d'un joueur : un autoritaire + 0..n shards en zone tampon.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub authoritative: String,
    pub overlaps: Vec<String>,
}

impl ShardTopology {
    /// Place un joueur en `(x,y)` : le shard autoritaire (celui dont la zone contient le point ;
    /// tie-break = adresse minimale) et les shards en zone tampon (tout autre shard dont la zone
    /// est à <= `radius` du point — c.-à-d. dont la frontière tombe dans le rayon).
    pub fn locate(&self, x: f32, y: f32, radius: f32) -> Placement {
        // Autoritaire : parmi les zones contenant le point, l'adresse minimale ; si aucune ne
        // contient (point hors couverture), le shard le plus proche (tie-break adresse minimale).
        let authoritative = self
            .shards
            .iter()
            .filter(|s| s.zone.contains(x, y))
            .map(|s| s.addr.clone())
            .min()
            .unwrap_or_else(|| {
                self.shards
                    .iter()
                    .min_by(|a, b| {
                        a.zone
                            .dist(x, y)
                            .total_cmp(&b.zone.dist(x, y))
                            .then(a.addr.cmp(&b.addr))
                    })
                    .map(|s| s.addr.clone())
                    .unwrap_or_default()
            });

        let mut overlaps: Vec<String> = self
            .shards
            .iter()
            .filter(|s| s.addr != authoritative && s.zone.dist(x, y) <= radius)
            .map(|s| s.addr.clone())
            .collect();
        overlaps.sort();
        Placement {
            authoritative,
            overlaps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2 shards : A = x<1000, B = x>=1000 (Y plein). Frontière à x=1000.
    fn two_shards() -> ShardTopology {
        ShardTopology {
            shards: vec![
                ShardZone {
                    addr: "A".into(),
                    zone: Aabb {
                        min_x: f32::NEG_INFINITY,
                        max_x: 1000.0,
                        min_y: f32::NEG_INFINITY,
                        max_y: f32::INFINITY,
                    },
                },
                ShardZone {
                    addr: "B".into(),
                    zone: Aabb {
                        min_x: 1000.0,
                        max_x: f32::INFINITY,
                        min_y: f32::NEG_INFINITY,
                        max_y: f32::INFINITY,
                    },
                },
            ],
        }
    }

    // 4 quadrants autour de (0,0) : coin où 4 shards se touchent.
    fn quad_shards() -> ShardTopology {
        let big = f32::INFINITY;
        ShardTopology {
            shards: vec![
                ShardZone {
                    addr: "SW".into(),
                    zone: Aabb {
                        min_x: -big,
                        max_x: 0.0,
                        min_y: -big,
                        max_y: 0.0,
                    },
                },
                ShardZone {
                    addr: "SE".into(),
                    zone: Aabb {
                        min_x: 0.0,
                        max_x: big,
                        min_y: -big,
                        max_y: 0.0,
                    },
                },
                ShardZone {
                    addr: "NW".into(),
                    zone: Aabb {
                        min_x: -big,
                        max_x: 0.0,
                        min_y: 0.0,
                        max_y: big,
                    },
                },
                ShardZone {
                    addr: "NE".into(),
                    zone: Aabb {
                        min_x: 0.0,
                        max_x: big,
                        min_y: 0.0,
                        max_y: big,
                    },
                },
            ],
        }
    }

    #[test]
    fn far_from_boundary_loads_only_authoritative() {
        let p = two_shards().locate(500.0, 0.0, 25.0);
        assert_eq!(p.authoritative, "A");
        assert!(p.overlaps.is_empty());
    }

    #[test]
    fn inside_buffer_dual_loads_neighbor() {
        // x=990 : autoritaire A, à 10 m de la frontière (<=25) → overlap B.
        let p = two_shards().locate(990.0, 0.0, 25.0);
        assert_eq!(p.authoritative, "A");
        assert_eq!(p.overlaps, vec!["B".to_string()]);
        // x=1000 (sur la frontière) → appartient à B (demi-ouvert), overlap A.
        let p2 = two_shards().locate(1000.0, 0.0, 25.0);
        assert_eq!(p2.authoritative, "B");
        assert_eq!(p2.overlaps, vec!["A".to_string()]);
    }

    #[test]
    fn junction_near_corner_loads_three_neighbors_but_edge_loads_one() {
        // Près du coin (-2,-2), rayon 5 : autoritaire SW, voisins SE (bord x=0, d=2),
        // NW (bord y=0, d=2), NE (coin, d=2.83) → les 3 dans le rayon.
        let corner = quad_shards().locate(-2.0, -2.0, 5.0);
        assert_eq!(corner.authoritative, "SW");
        assert_eq!(
            corner.overlaps,
            vec!["NE".to_string(), "NW".to_string(), "SE".to_string()]
        ); // triés

        // Loin du coin mais près d'un seul bord (-2,-50), rayon 5 : seul SE (x=0, d=2).
        // NW (y=0, d=50) et NE (coin, d>50) hors rayon → on NE charge PAS tous les voisins.
        let edge = quad_shards().locate(-2.0, -50.0, 5.0);
        assert_eq!(edge.authoritative, "SW");
        assert_eq!(edge.overlaps, vec!["SE".to_string()]);
    }
}
