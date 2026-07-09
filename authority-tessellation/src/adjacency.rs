use crate::raster::Grid;
use std::collections::BTreeSet;

pub fn adjacency(grid: &Grid, _cell_count: usize) -> Vec<(usize, usize)> {
    let mut set: BTreeSet<(usize, usize)> = BTreeSet::new();
    let at = |i: usize, j: usize| grid.owner[i * grid.ny + j];
    for i in 0..grid.nx {
        for j in 0..grid.ny {
            let o = at(i, j);
            if i + 1 < grid.nx {
                let r = at(i + 1, j);
                if r != o {
                    set.insert((o.min(r), o.max(r)));
                }
            }
            if j + 1 < grid.ny {
                let u = at(i, j + 1);
                if u != o {
                    set.insert((o.min(u), o.max(u)));
                }
            }
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Grid;

    #[test]
    fn adjacent_cells_detected() {
        // Grille 2x1 : colonne 0 = cellule 0, colonne 1 = cellule 1.
        let g = Grid {
            origin: [0.0, 0.0],
            step: 1.0,
            nx: 2,
            ny: 1,
            owner: vec![0, 1],
        };
        assert_eq!(adjacency(&g, 2), vec![(0, 1)]);
    }

    #[test]
    fn no_self_adjacency() {
        let g = Grid {
            origin: [0.0, 0.0],
            step: 1.0,
            nx: 2,
            ny: 1,
            owner: vec![0, 0],
        };
        assert!(adjacency(&g, 1).is_empty());
    }
}
