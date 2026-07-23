//! Représentation de graphe de navigation, indépendante de la source (spec navigation PNJ §2 :
//! « le crate de nav serveur expose une interface stable... indépendante de la source réelle »).
//! Nœuds = points 3D marchables, arêtes = connexions pondérées (distance). Aucune donnée réelle de
//! jeu ici — chargé/construit soit depuis un graphe synthétique de test, soit (travail futur, hors
//! périmètre) depuis les smartobjects/locopaths extraits par WolvenKit (spec §2, source S2).

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn distance(&self, other: &Vec3) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// Graphe non orienté pondéré. Poids d'arête = distance euclidienne au moment de l'ajout (spec §3 :
/// A* sur le graphe retenu) — recalculée une fois à la construction, jamais à chaque requête.
#[derive(Debug, Default)]
pub struct NavGraph {
    positions: Vec<Vec3>,
    /// Liste d'adjacence : `edges[i]` = liste de `(voisin, poids)` pour le nœud `i`.
    edges: Vec<Vec<(NodeId, f32)>>,
}

impl NavGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, position: Vec3) -> NodeId {
        let id = self.positions.len();
        self.positions.push(position);
        self.edges.push(Vec::new());
        id
    }

    /// Ajoute une arête non orientée (les deux sens sont ajoutés). Poids = distance euclidienne
    /// entre les deux positions. Panique si `a`/`b` n'existent pas — erreur de programmation d'un
    /// appelant qui construit un graphe synthétique en dur, pas un cas d'exécution normale à gérer
    /// gracieusement (cohérent avec le fait que ce module ne charge encore aucune donnée externe
    /// non fiable — le chargement depuis un fichier réel, futur, devra sa propre validation).
    pub fn add_edge(&mut self, a: NodeId, b: NodeId) {
        let weight = self.positions[a].distance(&self.positions[b]);
        self.edges[a].push((b, weight));
        self.edges[b].push((a, weight));
    }

    pub fn position_of(&self, id: NodeId) -> Option<Vec3> {
        self.positions.get(id).copied()
    }

    pub fn neighbors(&self, id: NodeId) -> &[(NodeId, f32)] {
        self.edges.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn node_count(&self) -> usize {
        self.positions.len()
    }

    /// Nœud le plus proche d'un point donné (recherche linéaire — le graphe synthétique de ce plan
    /// reste petit ; une structure spatiale dédiée est un raffinement futur si le graphe réel
    /// s'avère volumineux, cf. spec §7 tuilage).
    pub fn nearest_node(&self, point: Vec3) -> Option<NodeId> {
        self.positions
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.distance(&point).partial_cmp(&b.distance(&point)).unwrap())
            .map(|(id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_graph_has_no_nodes() {
        assert_eq!(NavGraph::new().node_count(), 0);
    }

    #[test]
    fn add_node_returns_sequential_ids() {
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::new(0.0, 0.0, 0.0));
        let b = g.add_node(Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(g.node_count(), 2);
    }

    #[test]
    fn add_edge_is_bidirectional_with_the_euclidean_distance_as_weight() {
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::new(0.0, 0.0, 0.0));
        let b = g.add_node(Vec3::new(3.0, 4.0, 0.0)); // distance = 5.0
        g.add_edge(a, b);
        assert_eq!(g.neighbors(a), &[(b, 5.0)]);
        assert_eq!(g.neighbors(b), &[(a, 5.0)]);
    }

    #[test]
    fn a_node_with_no_edges_has_empty_neighbors() {
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::new(0.0, 0.0, 0.0));
        assert!(g.neighbors(a).is_empty());
    }

    #[test]
    fn nearest_node_finds_the_closest_position() {
        let mut g = NavGraph::new();
        let a = g.add_node(Vec3::new(0.0, 0.0, 0.0));
        let b = g.add_node(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(g.nearest_node(Vec3::new(1.0, 0.0, 0.0)), Some(a));
        assert_eq!(g.nearest_node(Vec3::new(9.0, 0.0, 0.0)), Some(b));
    }

    #[test]
    fn nearest_node_on_an_empty_graph_returns_none() {
        assert_eq!(NavGraph::new().nearest_node(Vec3::new(0.0, 0.0, 0.0)), None);
    }
}
