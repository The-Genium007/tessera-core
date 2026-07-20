//! Logique de handoff (M4) : topologie des shards (zones AABB), calcul du placement d'un joueur
//! (shard autoritaire + shards en zone tampon), rayon par rang, et machine de chargement.
//! Pur et testable sans GNS/TCP.

use tessera_authority::geometry::{dist_point_polygon, point_in_polygon, Point};

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

/// Zone d'une cellule d'autorité, définie par une géométrie polygonale (issue de la
/// tessellation Voronoï d'autorité, `tessera-authority`). Une cellule peut porter plusieurs
/// anneaux (quartiers non contigus) — pas de fusion géométrique, appartenance testée anneau
/// par anneau (D4 de la spec district-topology, toujours valide).
#[derive(Debug, Clone)]
pub struct CellZone {
    pub boundary_rings: Vec<Vec<Point>>,
}

impl CellZone {
    /// Le point appartient à la cellule s'il est dans AU MOINS UN des anneaux.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let p: Point = [x as f64, y as f64];
        self.boundary_rings
            .iter()
            .any(|ring| point_in_polygon(p, ring))
    }

    /// Distance euclidienne du point à la cellule : le minimum, sur tous les anneaux, de la
    /// distance au polygone (`dist_point_polygon` — distance au segment le plus proche, PAS 0
    /// quand le point est dedans ; voir les tests `cellzone_dist_*` pour la convention exacte).
    pub fn dist(&self, x: f32, y: f32) -> f32 {
        let p: Point = [x as f64, y as f64];
        self.boundary_rings
            .iter()
            .map(|ring| dist_point_polygon(p, ring))
            .fold(f64::MAX, f64::min) as f32
    }
}

/// Un shard (ou "groupe" de cellules Voronoï simulées par un même process, décision 3 de la
/// spec câblage runtime tessellation d'autorité) et les cellules d'autorité dont il est
/// responsable. Chaque élément de `cells` porte sa géométrie (`CellZone`) et son rayon de tampon
/// déjà résolu en amont (artefact ou override manuel par cellule, décision 5 de la même spec) —
/// pas de rayon uniforme par shard : une cellule dense et une cellule périphérique du même groupe
/// ont des tampons différents.
///
/// **`id` et `addr` sont deux choses distinctes, ne pas les confondre** (séparation faite le
/// 2026-07-20, quand `addr` est devenue une vraie adresse réseau) :
/// - `id` — identifiant LOGIQUE (`"group-0"`…), le seul qui sorte du serveur : il part au client
///   dans `ShardAssignment` et il est publié dans `shard_map.json` via le directory. Stable,
///   indépendant du déploiement, sans valeur pour un attaquant.
/// - `addr` — `host:port` INTERNE, uniquement pour que le Gateway ouvre son lien TCP vers le
///   shard (`write_to_shard`). Ne doit jamais fuiter vers un client ni vers un artefact public.
///
/// `locate()` et tout ce qui en découle (`Placement`, `LoadAction`) manipulent des **id**.
/// La résolution id→addr se fait au seul moment de l'écriture, via `ShardTopology::addr_for`.
#[derive(Debug, Clone)]
pub struct ShardZone {
    pub id: String,
    pub addr: String,
    pub cells: Vec<(CellZone, f32)>,
}

/// L'ensemble des shards et leurs cellules (un groupe de cellules Voronoï par shard).
#[derive(Debug, Clone)]
pub struct ShardTopology {
    pub shards: Vec<ShardZone>,
}

impl ShardTopology {
    /// Adresse réseau interne du shard d'id logique `id`, pour ouvrir le lien Gateway→Shard.
    /// `None` si l'id est inconnu — l'appelant doit alors le signaler bruyamment plutôt que de
    /// laisser le client disparaître du monde en silence.
    pub fn addr_for(&self, id: &str) -> Option<&str> {
        self.shards
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.addr.as_str())
    }
}

/// Résultat du placement d'un joueur : un autoritaire + 0..n shards en zone tampon.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub authoritative: String,
    pub overlaps: Vec<String>,
}

/// Distance minimale d'un point à un shard : le minimum, sur toutes les cellules du groupe, de
/// `CellZone::dist` — un shard est "proche" du point dès qu'une de ses cellules l'est (même
/// logique de regroupement que l'ancien `ShardZone` mono-`Aabb`, étendue à N cellules).
fn shard_dist(shard: &ShardZone, x: f32, y: f32) -> f32 {
    shard
        .cells
        .iter()
        .map(|(cell, _)| cell.dist(x, y))
        .fold(f32::MAX, f32::min)
}

impl ShardTopology {
    /// Place un joueur en `(x,y)` : le shard autoritaire (celui dont une cellule contient le
    /// point ; tie-break = adresse minimale) et les shards en zone tampon (tout autre shard dont
    /// une cellule est à <= [rayon de tampon de la cellule autoritaire + `rank_bonus`] du point).
    ///
    /// `rank_bonus` est le bonus de rayon selon le rang du joueur (`RadiusPolicy`, inchangé,
    /// non remplacé) ; il est combiné en interne avec le rayon de tampon déjà résolu (artefact ou
    /// override) de la cellule où se trouve le joueur.
    pub fn locate(&self, x: f32, y: f32, rank_bonus: f32) -> Placement {
        // Autoritaire : parmi les shards ayant une cellule contenant le point, l'id minimal.
        // On retient aussi le rayon de tampon de LA CELLULE QUI CONTIENT LE POINT
        // (pas d'une autre cellule du même shard) — c'est ce rayon, pas celui d'un voisin, qui
        // sert de seuil de zone tampon ci-dessous.
        let containing_id = self
            .shards
            .iter()
            .filter(|s| s.cells.iter().any(|(cell, _)| cell.contains(x, y)))
            .map(|s| s.id.clone())
            .min();

        let (authoritative, own_cell_buffer) = match containing_id {
            Some(id) => {
                let shard = self
                    .shards
                    .iter()
                    .find(|s| s.id == id)
                    .expect("id retenu ci-dessus provient de self.shards");
                let buffer = shard
                    .cells
                    .iter()
                    .find(|(cell, _)| cell.contains(x, y))
                    .map(|(_, b)| *b)
                    .unwrap_or(0.0);
                (id, buffer)
            }
            None => {
                // Hors couverture (aucune cellule ne contient le point — ne devrait pas arriver
                // avec la couverture raster réelle de l'artefact v3) : fallback au shard le plus
                // proche (min sur toutes ses cellules), tie-break id minimal — comportement
                // de fallback préservé fidèlement de l'ancienne implémentation `Aabb`. Le rayon de
                // tampon retenu est alors celui de la cellule la plus proche de ce shard.
                match self.shards.iter().min_by(|a, b| {
                    shard_dist(a, x, y)
                        .total_cmp(&shard_dist(b, x, y))
                        .then(a.id.cmp(&b.id))
                }) {
                    Some(shard) => {
                        let buffer = shard
                            .cells
                            .iter()
                            .min_by(|(ca, _), (cb, _)| ca.dist(x, y).total_cmp(&cb.dist(x, y)))
                            .map(|(_, b)| *b)
                            .unwrap_or(0.0);
                        (shard.id.clone(), buffer)
                    }
                    None => (String::new(), 0.0),
                }
            }
        };

        // Seuil de zone tampon : le rayon de tampon résolu de LA CELLULE OÙ SE TROUVE LE JOUEUR
        // (`own_cell_buffer`, calculé ci-dessus), PAS celui de la cellule voisine candidate.
        // Décision 5 de la spec câblage runtime tessellation d'autorité
        // (docs/superpowers/specs/2026-07-09-authority-tessellation-runtime-wiring-design.md) :
        // « locate() utilise le b de la cellule où se trouve le joueur, combiné au rayon de rang
        // existant ». L'exemple illustratif du plan (Task G2, Step 4) utilise par erreur le
        // buffer du voisin candidat — en cas de doute d'architecture la spec fait foi (contrainte
        // globale du plan). Ne PAS "corriger" ceci vers le buffer du voisin sans relire cette note.
        let threshold = own_cell_buffer + rank_bonus;

        let mut overlaps: Vec<String> = self
            .shards
            .iter()
            .filter(|s| s.id != authoritative)
            .filter(|s| shard_dist(s, x, y) <= threshold)
            .map(|s| s.id.clone())
            .collect();
        overlaps.sort();
        Placement {
            authoritative,
            overlaps,
        }
    }
}

/// Rang d'un client. En M4 c'est un stub (défaut `Player`) ; l'authentification réelle viendra
/// plus tard. Le rang module la largeur de la zone tampon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rank {
    Player,
    Moderator,
    GameMaster,
}

/// Rayon de zone tampon selon le rang. Valeurs en dur en M4, destinées au fichier serveur (M6).
#[derive(Debug, Clone, Copy)]
pub struct RadiusPolicy {
    pub base: f32,
    pub moderator: f32,
    pub game_master: f32,
}

impl RadiusPolicy {
    pub fn radius_for(&self, rank: Rank) -> f32 {
        match rank {
            Rank::Player => self.base,
            Rank::Moderator => self.moderator,
            Rank::GameMaster => self.game_master,
        }
    }
}

use crate::internal_net::event_to_client_event_frame;
use crate::transport::{ClientId, TransportEvent};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

/// Seuil de stabilité avant bascule d'autorité d'écriture — distinct du double-chargement AoI
/// (immédiat, cf. `overlaps`) : l'écriture ne bascule que si le joueur reste côté nouveau shard
/// un minimum de temps, pour absorber un joueur oscillant à la frontière (design stockage
/// 2026-07-09, décision explicite contre la double-écriture multi-maître).
const WRITE_AUTHORITY_STABILITY_THRESHOLD: Duration = Duration::from_secs(2);

/// Décide si l'autorité d'ÉCRITURE de l'état chaud (`HotStateCache`, Task D2) d'un joueur doit
/// basculer du shard `current_authoritative` vers `new_placement.authoritative`. Un seul écrivain
/// à la fois : tant que le seuil de stabilité n'est pas atteint, le shard courant reste seul à
/// écrire — le shard voisin en zone tampon peut lire (`HotStateCache::read`) mais n'écrit jamais
/// avant la bascule. Pure, sans réseau : `time_since_boundary_cross` est fourni par l'appelant
/// (horloge du chargement de shard, hors scope ici).
pub fn should_transfer_write_authority(
    current_authoritative: &str,
    new_placement: &Placement,
    time_since_boundary_cross: Duration,
) -> bool {
    if new_placement.authoritative == current_authoritative {
        return false;
    }
    time_since_boundary_cross >= WRITE_AUTHORITY_STABILITY_THRESHOLD
}

/// Action de chargement : des frames `ClientEvent` à écrire à un shard donné.
#[derive(Debug)]
pub enum LoadAction {
    Forward { shard: String, frames: Vec<Vec<u8>> },
}

#[derive(Default)]
struct ClientState {
    /// Connected + Join (et tout message reçu avant la 1re position) — rejoués au chargement.
    preamble: Vec<TransportEvent>,
    /// Shards actuellement chargés (triés → actions déterministes).
    loaded: BTreeSet<String>,
    has_position: bool,
}

/// Suit, par client, l'ensemble de shards chargés, et produit charge/décharge/relai selon le
/// placement calculé à chaque position.
#[derive(Default)]
pub struct ShardLoader {
    clients: HashMap<ClientId, ClientState>,
}

impl ShardLoader {
    pub fn new() -> Self {
        Self::default()
    }

    fn client_of(ev: &TransportEvent) -> ClientId {
        match ev {
            TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
            TransportEvent::Message { from, .. } => *from,
        }
    }

    pub fn feed(&mut self, ev: TransportEvent, placement: Option<Placement>) -> Vec<LoadAction> {
        let id = Self::client_of(&ev);

        // Déconnexion totale : décharge tous les shards chargés, puis oublie le client.
        if matches!(ev, TransportEvent::Disconnected(_)) {
            let st = self.clients.remove(&id).unwrap_or_default();
            let d = event_to_client_event_frame(&TransportEvent::Disconnected(id));
            return st
                .loaded
                .into_iter()
                .map(|shard| LoadAction::Forward {
                    shard,
                    frames: vec![d.clone()],
                })
                .collect();
        }

        let st = self.clients.entry(id).or_default();

        match placement {
            Some(p) => {
                let mut desired = BTreeSet::new();
                desired.insert(p.authoritative.clone());
                desired.extend(p.overlaps.iter().cloned());

                let pos_frame = event_to_client_event_frame(&ev);
                let preamble: Vec<Vec<u8>> = st
                    .preamble
                    .iter()
                    .map(event_to_client_event_frame)
                    .collect();

                let mut actions = Vec::new();
                // Nouveaux shards désirés : charger (préambule + position).
                for shard in desired.difference(&st.loaded) {
                    let mut frames = preamble.clone();
                    frames.push(pos_frame.clone());
                    actions.push(LoadAction::Forward {
                        shard: shard.clone(),
                        frames,
                    });
                }
                // Shards déjà chargés et toujours désirés : relayer la position.
                for shard in st.loaded.intersection(&desired) {
                    actions.push(LoadAction::Forward {
                        shard: shard.clone(),
                        frames: vec![pos_frame.clone()],
                    });
                }
                // Shards chargés mais plus désirés : décharger.
                let leave = event_to_client_event_frame(&TransportEvent::Disconnected(id));
                for shard in st.loaded.difference(&desired) {
                    actions.push(LoadAction::Forward {
                        shard: shard.clone(),
                        frames: vec![leave.clone()],
                    });
                }
                st.loaded = desired;
                st.has_position = true;
                actions
            }
            None => {
                if st.has_position {
                    // Déjà chargé : relayer ce message à tous les shards chargés.
                    let frame = event_to_client_event_frame(&ev);
                    st.loaded
                        .iter()
                        .map(|shard| LoadAction::Forward {
                            shard: shard.clone(),
                            frames: vec![frame.clone()],
                        })
                        .collect()
                } else {
                    // Avant la 1re position : bufferiser dans le préambule.
                    st.preamble.push(ev);
                    Vec::new()
                }
            }
        }
    }

    pub fn loaded_shards(&self, client_id: ClientId) -> Vec<String> {
        self.clients
            .get(&client_id)
            .map(|s| s.loaded.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Frames `ClientEvent` (Connected/Join) à rejouer pour re-semer ce client sur un shard qui
    /// vient de perdre son état — vide pour un client inconnu.
    pub fn preamble_frames(&self, client_id: ClientId) -> Vec<Vec<u8>> {
        self.clients
            .get(&client_id)
            .map(|s| s.preamble.iter().map(event_to_client_event_frame).collect())
            .unwrap_or_default()
    }

    /// Tous les clients dont `shard_addr` fait partie des shards actuellement chargés.
    pub fn clients_loaded_on(&self, shard_addr: &str) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|(_, st)| st.loaded.contains(shard_addr))
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn forget(&mut self, client_id: ClientId) {
        self.clients.remove(&client_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::framing::FrameReader;
    use crate::internal_net::decode_client_event;
    use crate::transport::TransportEvent;
    use flatbuffers::FlatBufferBuilder;
    use protocol::*;

    fn join_payload() -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let name = b.create_string("v");
        let join = Join::create(
            &mut b,
            &JoinArgs {
                display_name: Some(name),
                token: None,
                protocol_version: 1,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::Join,
                msg: Some(join.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn pos_payload(x: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, 0.0, 0.0);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw: 0.0,
                locomotion: 0,
                move_dir: 0,
                flags: 0,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn decode_first(frame: &[u8]) -> Option<TransportEvent> {
        let mut r = FrameReader::new();
        r.push(frame);
        decode_client_event(&r.next_frame().unwrap())
    }

    fn place(auth: &str, overlaps: &[&str]) -> Placement {
        Placement {
            authoritative: auth.into(),
            overlaps: overlaps.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Retourne les frames de l'action Forward visant `shard` (helper anti-friction de pattern).
    fn frames_for<'a>(acts: &'a [LoadAction], shard: &str) -> &'a Vec<Vec<u8>> {
        acts.iter()
            .find_map(|a| {
                let LoadAction::Forward { shard: s, frames } = a;
                (s == shard).then_some(frames)
            })
            .unwrap_or_else(|| panic!("aucune action Forward vers {shard}"))
    }

    fn shards_of(acts: &[LoadAction]) -> Vec<String> {
        let mut v: Vec<String> = acts
            .iter()
            .map(|a| {
                let LoadAction::Forward { shard, .. } = a;
                shard.clone()
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn first_position_loads_authoritative_with_preamble() {
        let mut l = ShardLoader::new();
        assert!(l.feed(TransportEvent::Connected(1), None).is_empty());
        assert!(l
            .feed(
                TransportEvent::Message {
                    from: 1,
                    data: join_payload()
                },
                None
            )
            .is_empty());
        let acts = l.feed(
            TransportEvent::Message {
                from: 1,
                data: pos_payload(500.0),
            },
            Some(place("A", &[])),
        );
        assert_eq!(shards_of(&acts), vec!["A".to_string()]);
        let fa = frames_for(&acts, "A");
        assert_eq!(fa.len(), 3); // Connected + Join + Position
        assert_eq!(decode_first(&fa[0]), Some(TransportEvent::Connected(1)));
        assert_eq!(l.loaded_shards(1), vec!["A".to_string()]);
    }

    #[test]
    fn preamble_frames_replays_connected_and_join_for_a_loaded_client() {
        let mut l = ShardLoader::new();
        l.feed(TransportEvent::Connected(1), None);
        l.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        l.feed(
            TransportEvent::Message {
                from: 1,
                data: pos_payload(500.0),
            },
            Some(place("A", &[])),
        );

        let frames = l.preamble_frames(1);
        assert_eq!(frames.len(), 2); // Connected + Join — pas la Position
        assert_eq!(decode_first(&frames[0]), Some(TransportEvent::Connected(1)));
    }

    #[test]
    fn preamble_frames_is_empty_for_an_unknown_client() {
        let l = ShardLoader::new();
        assert!(l.preamble_frames(42).is_empty());
    }

    #[test]
    fn clients_loaded_on_lists_every_client_with_the_shard_in_its_loaded_set() {
        let mut l = ShardLoader::new();
        l.feed(TransportEvent::Connected(1), None);
        l.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        l.feed(
            TransportEvent::Message {
                from: 1,
                data: pos_payload(500.0),
            },
            Some(place("A", &[])),
        );

        l.feed(TransportEvent::Connected(2), None);
        l.feed(
            TransportEvent::Message {
                from: 2,
                data: join_payload(),
            },
            None,
        );
        l.feed(
            TransportEvent::Message {
                from: 2,
                data: pos_payload(2000.0),
            },
            Some(place("B", &[])),
        );

        assert_eq!(l.clients_loaded_on("A"), vec![1]);
        assert_eq!(l.clients_loaded_on("B"), vec![2]);
        assert!(l.clients_loaded_on("Z").is_empty());
    }

    #[test]
    fn entering_buffer_dual_loads_then_leaving_unloads() {
        let mut l = ShardLoader::new();
        l.feed(TransportEvent::Connected(1), None);
        l.feed(
            TransportEvent::Message {
                from: 1,
                data: join_payload(),
            },
            None,
        );
        l.feed(
            TransportEvent::Message {
                from: 1,
                data: pos_payload(500.0),
            },
            Some(place("A", &[])),
        ); // A seul

        // Entre dans le tampon : A + overlap B → CHARGE B (préambule+pos), relaie A.
        let acts = l.feed(
            TransportEvent::Message {
                from: 1,
                data: pos_payload(990.0),
            },
            Some(place("A", &["B"])),
        );
        let bf = frames_for(&acts, "B");
        assert_eq!(bf.len(), 3);
        assert_eq!(decode_first(&bf[0]), Some(TransportEvent::Connected(1)));
        let af = frames_for(&acts, "A");
        assert_eq!(af.len(), 1); // relai position seule
        assert_eq!(l.loaded_shards(1), vec!["A".to_string(), "B".to_string()]);

        // Franchit : B autoritaire, plus d'overlap A → DÉCHARGE A (Disconnected), relaie B.
        let acts2 = l.feed(
            TransportEvent::Message {
                from: 1,
                data: pos_payload(1100.0),
            },
            Some(place("B", &[])),
        );
        let af2 = frames_for(&acts2, "A");
        assert_eq!(decode_first(&af2[0]), Some(TransportEvent::Disconnected(1)));
        assert_eq!(l.loaded_shards(1), vec!["B".to_string()]);
    }

    #[test]
    fn disconnect_unloads_all_loaded_shards() {
        let mut l = ShardLoader::new();
        l.feed(TransportEvent::Connected(7), None);
        l.feed(
            TransportEvent::Message {
                from: 7,
                data: join_payload(),
            },
            None,
        );
        l.feed(
            TransportEvent::Message {
                from: 7,
                data: pos_payload(990.0),
            },
            Some(place("A", &["B"])),
        ); // A+B
        let acts = l.feed(TransportEvent::Disconnected(7), None);
        assert_eq!(shards_of(&acts), vec!["A".to_string(), "B".to_string()]);
        assert_eq!(
            decode_first(&frames_for(&acts, "A")[0]),
            Some(TransportEvent::Disconnected(7))
        );
        assert_eq!(
            decode_first(&frames_for(&acts, "B")[0]),
            Some(TransportEvent::Disconnected(7))
        );
        assert!(l.loaded_shards(7).is_empty());
    }

    /// Rectangle fermé (anneau `Vec<Point>`, premier == dernier point) — anti-friction pour
    /// éviter de répéter la construction de polygones dans chaque test (plusieurs tests de
    /// G1/G2 en ont besoin).
    fn rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<Point> {
        vec![
            [min_x, min_y],
            [max_x, min_y],
            [max_x, max_y],
            [min_x, max_y],
            [min_x, min_y],
        ]
    }

    /// Carré fermé de côté `side`, coin bas-gauche `(min_x, min_y)`.
    fn square(min_x: f64, min_y: f64, side: f64) -> Vec<Point> {
        rect(min_x, min_y, min_x + side, min_y + side)
    }

    // "Infini" pratique pour représenter un demi-plan par un polygone fini : assez grand pour
    // que les tests (qui ne sondent jamais qu'à quelques dizaines/centaines de mètres de la
    // frontière d'intérêt) ne puissent jamais atteindre le bord opposé du rectangle.
    const BIG: f64 = 1.0e7;

    // 2 shards : A = x<1000, B = x>=1000 (Y plein). Frontière à x=1000. Buffer de tampon
    // identique (25.0) sur les deux cellules — reproduit l'ancien radius uniforme passé par
    // l'appelant (Aabb) ; les tests appellent désormais `locate(x, y, 0.0)` (rank_bonus=0).
    fn two_shards() -> ShardTopology {
        ShardTopology {
            shards: vec![
                ShardZone {
                    id: "A".into(),
                    addr: "A".into(),
                    cells: vec![(
                        CellZone {
                            boundary_rings: vec![rect(-BIG, -BIG, 1000.0, BIG)],
                        },
                        25.0,
                    )],
                },
                ShardZone {
                    id: "B".into(),
                    addr: "B".into(),
                    cells: vec![(
                        CellZone {
                            boundary_rings: vec![rect(1000.0, -BIG, BIG, BIG)],
                        },
                        25.0,
                    )],
                },
            ],
        }
    }

    // 4 quadrants autour de (0,0) : coin où 4 shards se touchent. Buffer 5.0 sur chaque cellule
    // (reproduit l'ancien radius=5.0 uniforme).
    fn quad_shards() -> ShardTopology {
        ShardTopology {
            shards: vec![
                ShardZone {
                    id: "SW".into(),
                    addr: "SW".into(),
                    cells: vec![(
                        CellZone {
                            boundary_rings: vec![rect(-BIG, -BIG, 0.0, 0.0)],
                        },
                        5.0,
                    )],
                },
                ShardZone {
                    id: "SE".into(),
                    addr: "SE".into(),
                    cells: vec![(
                        CellZone {
                            boundary_rings: vec![rect(0.0, -BIG, BIG, 0.0)],
                        },
                        5.0,
                    )],
                },
                ShardZone {
                    id: "NW".into(),
                    addr: "NW".into(),
                    cells: vec![(
                        CellZone {
                            boundary_rings: vec![rect(-BIG, 0.0, 0.0, BIG)],
                        },
                        5.0,
                    )],
                },
                ShardZone {
                    id: "NE".into(),
                    addr: "NE".into(),
                    cells: vec![(
                        CellZone {
                            boundary_rings: vec![rect(0.0, 0.0, BIG, BIG)],
                        },
                        5.0,
                    )],
                },
            ],
        }
    }

    #[test]
    fn far_from_boundary_loads_only_authoritative() {
        let p = two_shards().locate(500.0, 0.0, 0.0);
        assert_eq!(p.authoritative, "A");
        assert!(p.overlaps.is_empty());
    }

    #[test]
    fn inside_buffer_dual_loads_neighbor() {
        // x=990 : autoritaire A, à 10 m de la frontière (<=25, buffer de la cellule A) → overlap B.
        let p = two_shards().locate(990.0, 0.0, 0.0);
        assert_eq!(p.authoritative, "A");
        assert_eq!(p.overlaps, vec!["B".to_string()]);
        // x=1000 (sur la frontière) → appartient à B (demi-ouvert, même convention que l'ancien
        // Aabb — vérifié : `point_in_polygon` sur un rectangle exclut le bord partagé du côté
        // dont ce bord est le max_x, l'inclut du côté dont il est le min_x), overlap A.
        let p2 = two_shards().locate(1000.0, 0.0, 0.0);
        assert_eq!(p2.authoritative, "B");
        assert_eq!(p2.overlaps, vec!["A".to_string()]);
    }

    #[test]
    fn junction_near_corner_loads_three_neighbors_but_edge_loads_one() {
        // Près du coin (-2,-2), buffer 5 (baked dans chaque cellule) : autoritaire SW, voisins
        // SE (bord x=0, d=2), NW (bord y=0, d=2), NE (coin, d=2.83) → les 3 dans le rayon.
        let corner = quad_shards().locate(-2.0, -2.0, 0.0);
        assert_eq!(corner.authoritative, "SW");
        assert_eq!(
            corner.overlaps,
            vec!["NE".to_string(), "NW".to_string(), "SE".to_string()]
        ); // triés

        // Loin du coin mais près d'un seul bord (-2,-50), buffer 5 : seul SE (x=0, d=2).
        // NW (y=0, d=50) et NE (coin, d>50) hors rayon → on NE charge PAS tous les voisins.
        let edge = quad_shards().locate(-2.0, -50.0, 0.0);
        assert_eq!(edge.authoritative, "SW");
        assert_eq!(edge.overlaps, vec!["SE".to_string()]);
    }

    #[test]
    fn locate_finds_authoritative_cell_among_multiple_in_same_shard_zone() {
        let zone_a = CellZone {
            boundary_rings: vec![square(0.0, 0.0, 10.0)],
        };
        let zone_b = CellZone {
            boundary_rings: vec![square(100.0, 100.0, 10.0)],
        };
        let shard = ShardZone {
            id: "shard-group-1".into(),
            addr: "shard-group-1".into(),
            cells: vec![(zone_a, 25.0), (zone_b, 25.0)],
        };
        let topology = ShardTopology {
            shards: vec![shard],
        };

        let placement = topology.locate(5.0, 5.0, 0.0);
        assert_eq!(placement.authoritative, "shard-group-1");
    }

    #[test]
    fn locate_uses_own_cell_buffer_not_neighbor_buffer_for_overlap_threshold() {
        // Décision 5 de la spec câblage runtime tessellation d'autorité
        // (docs/superpowers/specs/2026-07-09-authority-tessellation-runtime-wiring-design.md) :
        // le seuil de zone tampon utilise le `b` de LA CELLULE DU JOUEUR, pas celui du voisin
        // candidat. Deux ShardZone adjacentes à x=1000 : A (dense, buffer=25.0) et B (périphérie,
        // buffer=600.0).
        let a = ShardZone {
            id: "A".into(),
            addr: "A".into(),
            cells: vec![(
                CellZone {
                    boundary_rings: vec![rect(-BIG, -BIG, 1000.0, BIG)],
                },
                25.0,
            )],
        };
        let b = ShardZone {
            id: "B".into(),
            addr: "B".into(),
            cells: vec![(
                CellZone {
                    boundary_rings: vec![rect(1000.0, -BIG, BIG, BIG)],
                },
                600.0,
            )],
        };
        let topology = ShardTopology { shards: vec![a, b] };

        // Joueur dans A, à 30 m de la frontière (x=970). Si le seuil utilisait le buffer DE B
        // (600.0, l'interprétation erronée du plan), 30 <= 600 donnerait un overlap — mais le
        // seuil doit utiliser le buffer DE A (25.0, la cellule du joueur) : 30 > 25 → pas d'overlap.
        let p1 = topology.locate(970.0, 0.0, 0.0);
        assert_eq!(p1.authoritative, "A");
        assert!(
            p1.overlaps.is_empty(),
            "le seuil doit utiliser le buffer de A (25.0), pas celui de B (600.0)"
        );

        // Joueur dans B, à 30 m de la frontière (x=1030) : le buffer DE B (600.0, la cellule du
        // joueur) s'applique — 30 <= 600 → overlap A.
        let p2 = topology.locate(1030.0, 0.0, 0.0);
        assert_eq!(p2.authoritative, "B");
        assert_eq!(p2.overlaps, vec!["A".to_string()]);
    }

    #[test]
    fn radius_policy_widens_for_staff() {
        let pol = RadiusPolicy {
            base: 25.0,
            moderator: 50.0,
            game_master: 75.0,
        };
        assert_eq!(pol.radius_for(Rank::Player), 25.0);
        assert_eq!(pol.radius_for(Rank::Moderator), 50.0);
        assert_eq!(pol.radius_for(Rank::GameMaster), 75.0);
    }

    #[test]
    fn cellzone_contains_point_inside_simple_square() {
        // Carré [0,0]-[10,0]-[10,10]-[0,10]-[0,0] (anneau fermé, premier==dernier point).
        let zone = CellZone {
            boundary_rings: vec![square(0.0, 0.0, 10.0)],
        };
        assert!(zone.contains(5.0, 5.0));
        assert!(!zone.contains(15.0, 5.0));
    }

    #[test]
    fn cellzone_contains_checks_all_rings_for_multi_polygon_cell() {
        // Une cellule à deux quartiers disjoints (deux anneaux séparés) — le point appartient à la
        // cellule s'il est dans AU MOINS UN des anneaux (D4 de l'ancienne spec district-topology,
        // toujours valide : pas de fusion géométrique).
        let zone = CellZone {
            boundary_rings: vec![square(0.0, 0.0, 10.0), square(100.0, 100.0, 10.0)],
        };
        assert!(zone.contains(5.0, 5.0));
        assert!(zone.contains(105.0, 105.0));
        assert!(!zone.contains(50.0, 50.0));
    }

    #[test]
    fn cellzone_dist_is_distance_to_nearest_edge_even_when_point_is_inside() {
        // Convention réelle de `dist_point_polygon` (tools/authority-tessellation/src/geometry.rs) :
        // distance au segment le plus proche du polygone, PAS 0 quand le point est dedans (ce
        // n'est pas une distance signée). Vérifié en lisant l'implémentation + son test
        // `dist_to_polygon_edge` (point à 5 unités au-dessus d'un carré 10x10 → dist == 5.0), qui
        // s'applique symétriquement à un point à l'intérieur équidistant des 4 bords.
        let zone = CellZone {
            boundary_rings: vec![square(0.0, 0.0, 10.0)],
        };
        // Centre du carré : équidistant des 4 bords, chacun à 5.0.
        assert!((zone.dist(5.0, 5.0) - 5.0).abs() < 1e-6);
        // Point hors du polygone : distance normale au bord le plus proche.
        assert!((zone.dist(15.0, 5.0) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn cellzone_dist_takes_the_min_across_all_rings_for_multi_polygon_cell() {
        let zone = CellZone {
            boundary_rings: vec![square(0.0, 0.0, 10.0), square(100.0, 100.0, 10.0)],
        };
        // Point à x=50 : dist au 1er anneau (bord x=10) = 40 ; au 2e (bord x=100) = 50 → min = 40.
        assert!((zone.dist(50.0, 5.0) - 40.0).abs() < 1e-6);
    }

    #[test]
    fn write_authority_does_not_transfer_before_stability_threshold() {
        let placement = Placement {
            authoritative: "shard-b".into(),
            overlaps: vec!["shard-a".into()],
        };
        let should_transfer = should_transfer_write_authority(
            "shard-a",
            &placement,
            Duration::from_millis(500), // sous le seuil (proposé 1-2s dans la spec)
        );
        assert!(!should_transfer);
    }

    #[test]
    fn write_authority_transfers_after_stability_threshold() {
        let placement = Placement {
            authoritative: "shard-b".into(),
            overlaps: vec!["shard-a".into()],
        };
        let should_transfer =
            should_transfer_write_authority("shard-a", &placement, Duration::from_secs(2));
        assert!(should_transfer);
    }

    #[test]
    fn write_authority_never_transfers_if_still_same_shard() {
        let placement = Placement {
            authoritative: "shard-a".into(),
            overlaps: vec![],
        };
        let should_transfer =
            should_transfer_write_authority("shard-a", &placement, Duration::from_secs(10));
        assert!(!should_transfer); // rien à transférer, déjà autoritaire
    }

    #[test]
    fn rapid_flapping_never_triggers_transfer() {
        // Simule un joueur qui oscille toutes les 300ms entre shard-a et shard-b pendant 5 secondes
        // — à aucun moment le temps de stabilité continue ne dépasse le seuil, donc devrait rester
        // toujours sur shard-a (jamais de transfert). Vérifie la propriété centrale de la décision
        // "un seul écrivain + hystérésis" retenue contre la double-écriture.
        let mut current_authoritative = "shard-a".to_string();
        let mut time_stable = Duration::ZERO;
        for _tick in 0..17 {
            // 17 * 300ms ≈ 5.1s d'oscillation
            let placement = Placement {
                authoritative: "shard-b".into(),
                overlaps: vec!["shard-a".into()],
            };
            time_stable += Duration::from_millis(300);
            if should_transfer_write_authority(&current_authoritative, &placement, time_stable) {
                current_authoritative = "shard-b".into();
                time_stable = Duration::ZERO; // reset : on vient de transférer
            } else {
                time_stable = Duration::ZERO; // reset : oscillation = jamais 2 ticks d'affilée du même côté
            }
        }
        assert_eq!(current_authoritative, "shard-a");
    }

    /// Nécessite redis://127.0.0.1:6379 local — lancer manuellement avec `cargo test -- --ignored`.
    /// Aucun Redis n'est démarré automatiquement en CI ni via `docker-compose.yml` pour ce crate
    /// (même constat que les tests existants de `hot_state_cache.rs`, qui ne sont eux-mêmes PAS
    /// protégés par `#[ignore]` aujourd'hui — incohérence pré-existante signalée dans le rapport
    /// de la Task D4 ; corriger `hot_state_cache.rs` est hors du scope de fichiers de cette tâche).
    #[tokio::test]
    #[ignore]
    async fn neighbor_shard_reads_hot_state_during_buffer_before_authority_transfers() {
        use crate::hot_state_cache::HotStateCache;

        // Deux handles logiques (un par "shard"), même Redis de test — cohérent avec la note du
        // brief : pas besoin de deux process Redis séparés pour prouver la lecture croisée.
        let authoritative_shard_cache = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .unwrap();
        let neighbor_shard_cache = HotStateCache::connect("redis://127.0.0.1:6379")
            .await
            .unwrap();

        let subject = "test-subject-d4-buffer-read";
        let position = [42.0, 7.0, 0.0];

        // Le shard autoritaire écrit la position (seul écrivain à cet instant : le joueur vient
        // d'entrer en zone tampon, `should_transfer_write_authority` renverrait `false` ici).
        authoritative_shard_cache
            .write(subject, position)
            .await
            .unwrap();

        // Le shard voisin (en overlap) doit pouvoir LIRE cette position dès l'entrée en zone
        // tampon, sans jamais écrire lui-même tant que le seuil de stabilité n'est pas atteint.
        let placement = Placement {
            authoritative: "authoritative-shard".into(),
            overlaps: vec!["neighbor-shard".into()],
        };
        let should_neighbor_write = should_transfer_write_authority(
            "authoritative-shard",
            &placement,
            Duration::from_millis(500), // vient d'entrer dans le tampon, sous le seuil
        );
        assert!(
            !should_neighbor_write,
            "le voisin ne doit pas encore écrire pendant la fenêtre de stabilité"
        );

        let read_by_neighbor = neighbor_shard_cache.read(subject).await.unwrap();
        assert_eq!(
            read_by_neighbor,
            Some(position),
            "le shard voisin doit pouvoir lire l'état chaud écrit par l'autoritaire"
        );
    }
}
