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
}
