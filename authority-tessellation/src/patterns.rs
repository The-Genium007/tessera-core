use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Vrai si le sous-ensemble `group` est connexe dans le graphe `adj`.
pub fn is_connected(group: &[usize], adj: &[(usize, usize)]) -> bool {
    if group.len() <= 1 {
        return true;
    }
    let set: BTreeSet<usize> = group.iter().copied().collect();
    let mut nbrs: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(a, b) in adj {
        if set.contains(&a) && set.contains(&b) {
            nbrs.entry(a).or_default().push(b);
            nbrs.entry(b).or_default().push(a);
        }
    }
    let start = group[0];
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut q = VecDeque::from([start]);
    seen.insert(start);
    while let Some(u) = q.pop_front() {
        if let Some(ns) = nbrs.get(&u) {
            for &v in ns {
                if seen.insert(v) {
                    q.push_back(v);
                }
            }
        }
    }
    seen.len() == group.len()
}

/// Partition contiguë en N groupes : on part de K groupes singletons puis on fusionne
/// itérativement la paire de groupes adjacents dont la somme des charges membres est
/// minimale (les cellules froides — périphérie/océan — fusionnent d'abord, les cellules
/// chaudes restent séparées le plus longtemps). Égalité départagée par les indices de
/// groupes les plus bas (a puis b) — déterministe.
pub fn assignment_patterns(
    cell_count: usize,
    adjacency: &[(usize, usize)],
    loads: &[f64],
) -> BTreeMap<usize, Vec<Vec<usize>>> {
    let mut out = BTreeMap::new();
    if cell_count == 0 {
        return out;
    }
    for n in 1..=cell_count {
        // groupes = liste de sets, initialement singletons
        let mut groups: Vec<BTreeSet<usize>> =
            (0..cell_count).map(|c| BTreeSet::from([c])).collect();
        while groups.len() > n {
            // paire adjacente de charge combinée minimale ; le scan a croissant puis
            // b croissant + comparaison stricte `<` garantit le tie-break (a, b) minimal.
            let mut best: Option<(f64, usize, usize)> = None;
            for a in 0..groups.len() {
                for b in (a + 1)..groups.len() {
                    let adjacent = adjacency.iter().any(|&(x, y)| {
                        (groups[a].contains(&x) && groups[b].contains(&y))
                            || (groups[a].contains(&y) && groups[b].contains(&x))
                    });
                    if !adjacent {
                        continue;
                    }
                    let combined: f64 = groups[a]
                        .iter()
                        .chain(groups[b].iter())
                        .map(|&c| loads[c])
                        .sum();
                    if best.is_none_or(|(bl, _, _)| combined < bl) {
                        best = Some((combined, a, b));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let gb = groups.remove(b);
                groups[a].extend(gb);
            } else {
                // graphe déconnecté : fusion des deux plus petits groupes restants
                groups.sort_by_key(|g| g.len());
                let gb = groups.remove(1);
                groups[0].extend(gb);
            }
        }
        let mut pattern: Vec<Vec<usize>> =
            groups.iter().map(|g| g.iter().copied().collect()).collect();
        pattern.sort();
        out.insert(n, pattern);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n1_is_all_cells_in_one_group() {
        let adj = vec![(0, 1), (1, 2)];
        let pats = assignment_patterns(3, &adj, &[1.0; 3]);
        assert_eq!(pats[&1], vec![vec![0, 1, 2]]);
    }

    #[test]
    fn nk_is_one_cell_per_group() {
        let adj = vec![(0, 1), (1, 2)];
        let pats = assignment_patterns(3, &adj, &[1.0; 3]);
        assert_eq!(pats[&3].len(), 3);
        assert!(pats[&3].iter().all(|g| g.len() == 1));
    }

    #[test]
    fn every_pattern_covers_all_cells_once() {
        let adj = vec![(0, 1), (1, 2), (2, 3)];
        let pats = assignment_patterns(4, &adj, &[1.0; 4]);
        for groups in pats.values() {
            let mut all: Vec<usize> = groups.iter().flatten().copied().collect();
            all.sort();
            assert_eq!(all, vec![0, 1, 2, 3]);
        }
    }

    #[test]
    fn groups_are_contiguous() {
        let adj = vec![(0, 1), (1, 2), (2, 3)];
        let pats = assignment_patterns(4, &adj, &[1.0; 4]);
        // Pour N=2 sur une chaîne 0-1-2-3, chaque groupe doit être connexe.
        for g in &pats[&2] {
            assert!(is_connected(g, &adj), "groupe non contigu: {g:?}");
        }
    }

    #[test]
    fn coldest_adjacent_pair_merges_first() {
        // Chaîne 0-1-2 : cellule 0 chaude (10.0), cellules 1 et 2 froides (1.0).
        // L'ancien algorithme fusionnait la première paire par index → [[0,1],[2]] ;
        // le nouveau doit fusionner la paire adjacente la plus froide (1,2) → [[0],[1,2]].
        let adj = vec![(0, 1), (1, 2)];
        let loads = vec![10.0, 1.0, 1.0];
        let pats = assignment_patterns(3, &adj, &loads);
        assert_eq!(pats[&2], vec![vec![0], vec![1, 2]]);
    }
}
