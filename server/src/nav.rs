//! Interface stable de navigation (spec §2 : `plan_path(from, to) -> Vec<Waypoint>`,
//! `is_walkable(point)`, indépendante de la source réelle du graphe — S2/smartobjects branché
//! plus tard derrière cette même interface, travail futur hors périmètre). A* écrite à la main
//! (BinaryHeap + distance euclidienne comme heuristique admissible sur un graphe géométrique 3D) —
//! cohérent avec le style du dépôt (pas de nouvelle dépendance Cargo, comme la grille spatiale de
//! `world.rs`).

use crate::nav_graph::{NavGraph, NodeId, Vec3};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    pub position: Vec3,
}

/// Entrée de la file de priorité A* — `f_score` inversé pour que `BinaryHeap` (max-heap) se
/// comporte comme une min-heap sur le coût total estimé.
#[derive(PartialEq)]
struct QueueEntry {
    node: NodeId,
    f_score: f32,
}
impl Eq for QueueEntry {}
impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_score
            .partial_cmp(&self.f_score)
            .unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Calcule un chemin entre deux points 3D sur `graph` (spec §3 : A* sur le graphe retenu). `from`/
/// `to` sont d'abord accrochés (`snap`) au nœud le plus proche via `NavGraph::nearest_node` — ce
/// plan ne fait pas de recherche de portail/projection sur arête, un raffinement possible si le
/// graphe réel s'avère trop épars. Retourne `None` si le graphe est vide ou si `to` est
/// inatteignable depuis `from` (composantes connexes disjointes).
pub fn plan_path(graph: &NavGraph, from: Vec3, to: Vec3) -> Option<Vec<Waypoint>> {
    let start = graph.nearest_node(from)?;
    let goal = graph.nearest_node(to)?;

    if start == goal {
        return Some(vec![Waypoint {
            position: graph.position_of(goal)?,
        }]);
    }

    let mut open = BinaryHeap::new();
    let mut g_score: HashMap<NodeId, f32> = HashMap::from([(start, 0.0)]);
    let mut came_from: HashMap<NodeId, NodeId> = HashMap::new();
    open.push(QueueEntry {
        node: start,
        f_score: heuristic(graph, start, goal),
    });

    while let Some(QueueEntry { node, .. }) = open.pop() {
        if node == goal {
            return Some(reconstruct_path(graph, &came_from, goal));
        }
        let current_g = *g_score.get(&node).unwrap_or(&f32::INFINITY);
        for &(neighbor, weight) in graph.neighbors(node) {
            let tentative_g = current_g + weight;
            if tentative_g < *g_score.get(&neighbor).unwrap_or(&f32::INFINITY) {
                came_from.insert(neighbor, node);
                g_score.insert(neighbor, tentative_g);
                open.push(QueueEntry {
                    node: neighbor,
                    f_score: tentative_g + heuristic(graph, neighbor, goal),
                });
            }
        }
    }
    None
}

fn heuristic(graph: &NavGraph, a: NodeId, b: NodeId) -> f32 {
    match (graph.position_of(a), graph.position_of(b)) {
        (Some(pa), Some(pb)) => pa.distance(&pb),
        _ => 0.0,
    }
}

fn reconstruct_path(
    graph: &NavGraph,
    came_from: &HashMap<NodeId, NodeId>,
    goal: NodeId,
) -> Vec<Waypoint> {
    let mut path = vec![goal];
    let mut current = goal;
    while let Some(&prev) = came_from.get(&current) {
        path.push(prev);
        current = prev;
    }
    path.reverse();
    path.into_iter()
        .filter_map(|id| graph.position_of(id).map(|position| Waypoint { position }))
        .collect()
}

/// `point` est-il à portée d'accrochage (`max_snap_distance`) d'un nœud marchable du graphe ? Spec
/// §2 : `is_walkable(point)`. Un graphe vide n'a aucun point marchable.
pub fn is_walkable(graph: &NavGraph, point: Vec3, max_snap_distance: f32) -> bool {
    graph
        .nearest_node(point)
        .and_then(|id| graph.position_of(id))
        .map(|pos| pos.distance(&point) <= max_snap_distance)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Graphe en ligne : 0 -- 1 -- 2 -- 3, chacun à 1 unité du suivant.
    fn line_graph() -> NavGraph {
        let mut g = NavGraph::new();
        let nodes: Vec<_> = (0..4)
            .map(|i| g.add_node(Vec3::new(i as f32, 0.0, 0.0)))
            .collect();
        for w in nodes.windows(2) {
            g.add_edge(w[0], w[1]);
        }
        g
    }

    #[test]
    fn plan_path_finds_a_plausible_path_between_two_points_of_a_district() {
        // Spec §9 (critère de sortie) : « plan_path calcule un chemin plausible entre deux points
        // d'un district ».
        let g = line_graph();
        let path = plan_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0)).unwrap();
        assert_eq!(path.len(), 4);
        assert_eq!(path[0].position, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(path[3].position, Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn plan_path_from_a_point_to_itself_returns_a_single_waypoint() {
        let g = line_graph();
        let path = plan_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.1, 0.0, 0.0)).unwrap();
        assert_eq!(path.len(), 1);
    }

    #[test]
    fn plan_path_prefers_the_shorter_of_two_routes() {
        // Losange : 0 -> 1 -> 3 (direct-ish, poids 1+1=2) vs 0 -> 2 -> 3 (poids 10+10=20).
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::new(0.0, 0.0, 0.0));
        let b = g.add_node(Vec3::new(1.0, 0.0, 0.0));
        let c = g.add_node(Vec3::new(0.0, 10.0, 0.0));
        let d = g.add_node(Vec3::new(2.0, 0.0, 0.0));
        g.add_edge(a, b);
        g.add_edge(b, d);
        g.add_edge(a, c);
        g.add_edge(c, d);
        let path = plan_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)).unwrap();
        assert_eq!(path.len(), 3, "doit passer par le chemin court a-b-d, pas a-c-d");
        assert_eq!(path[1].position, Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn plan_path_returns_none_when_the_target_is_in_a_disconnected_component() {
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::new(0.0, 0.0, 0.0));
        let _isolated = g.add_node(Vec3::new(100.0, 100.0, 100.0));
        let _ = a;
        assert!(plan_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 100.0, 100.0)).is_none());
    }

    #[test]
    fn plan_path_on_an_empty_graph_returns_none() {
        let g = NavGraph::new();
        assert!(plan_path(&g, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn is_walkable_is_true_within_snap_distance_of_a_node() {
        let g = line_graph();
        assert!(is_walkable(&g, Vec3::new(0.4, 0.0, 0.0), 0.5));
    }

    #[test]
    fn is_walkable_is_false_beyond_snap_distance() {
        let g = line_graph();
        assert!(!is_walkable(&g, Vec3::new(0.4, 0.0, 0.0), 0.1));
    }

    #[test]
    fn is_walkable_on_an_empty_graph_is_always_false() {
        assert!(!is_walkable(&NavGraph::new(), Vec3::new(0.0, 0.0, 0.0), 1000.0));
    }
}
