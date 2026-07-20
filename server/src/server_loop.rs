//! Boucle serveur : draine les events transport, met à jour le World, diffuse les snapshots.
//! Générique sur `Transport` → testable avec `InMemoryTransport`, branché sur GNS en prod.

use crate::transport::{ClientId, Transport, TransportEvent};
use crate::world::{Pose, World};
use flatbuffers::FlatBufferBuilder;
use protocol::*;

pub struct Server {
    world: World,
    aoi_radius: f32,
    /// File d'événements one-shot du tick courant (actor, kind, action, param) — accumulée par
    /// `apply_client_message`, drainée et relayée aux voisins AoI en fin de `tick()`.
    pending_events: Vec<(ClientId, u8, u8, u32)>,
}

impl Server {
    pub fn new(aoi_radius: f32) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
        }
    }

    /// Nombre de joueurs actuellement dans le monde de ce Shard — pour l'endpoint métriques.
    pub fn player_count(&self) -> usize {
        self.world.player_ids().len()
    }

    /// Un tick : applique les events entrants, avance le monde, envoie un snapshot à chaque client.
    pub fn tick<T: Transport>(&mut self, transport: &mut T) {
        for ev in transport.poll() {
            match ev {
                TransportEvent::Connected(id) => self.world.add_player(id),
                TransportEvent::Disconnected(id) => self.world.remove_player(id),
                TransportEvent::Message { from, data } => self.apply_client_message(from, &data),
            }
        }
        self.world.advance_tick();
        let mut b = FlatBufferBuilder::new();
        for id in self.world.player_ids() {
            b.reset();
            let bytes = self.encode_snapshot_for(id, &mut b);
            transport.send(id, &bytes);
        }
        // Relais des événements one-shot du tick, filtré par le même AoI que les snapshots.
        for (actor, kind, action, param) in self.pending_events.drain(..) {
            let neighbors = self.world.snapshot_for(actor, self.aoi_radius);
            for (neighbor_id, _) in neighbors {
                b.reset();
                let ev = PlayerEvent::create(
                    &mut b,
                    &PlayerEventArgs {
                        actor,
                        kind,
                        action,
                        param,
                    },
                );
                let env = ServerEnvelope::create(
                    &mut b,
                    &ServerEnvelopeArgs {
                        msg_type: ServerMsg::PlayerEvent,
                        msg: Some(ev.as_union_value()),
                    },
                );
                b.finish(env, None);
                transport.send(neighbor_id, b.finished_data());
            }
        }
    }

    fn apply_client_message(&mut self, from: ClientId, data: &[u8]) {
        let Ok(env) = flatbuffers::root::<ClientEnvelope>(data) else {
            return;
        };
        match env.msg_type() {
            ClientMsg::Join => { /* TODO(Phase-1): stocker le display_name */ }
            ClientMsg::PositionUpdate => {
                if let Some(pu) = env.msg_as_position_update() {
                    if let Some(p) = pu.position() {
                        self.world.set_pose(
                            from,
                            Pose {
                                x: p.x(),
                                y: p.y(),
                                z: p.z(),
                                yaw: pu.yaw(),
                                ..self.world.pose_of(from).unwrap_or_default()
                            },
                        );
                    }
                    self.world
                        .set_locomotion(from, pu.locomotion(), pu.move_dir(), pu.flags());
                }
            }
            ClientMsg::EmoteReport => {
                if let Some(er) = env.msg_as_emote_report() {
                    let emote = if er.start() { er.emote() } else { 0 };
                    self.world.set_sustained(from, emote);
                }
            }
            ClientMsg::PlayerActionReport => {
                if let Some(ar) = env.msg_as_player_action_report() {
                    // kind=0=Action (seul type existant pour l'instant, cf. schéma). Relayé en fin
                    // de tick, filtré par AoI — jamais appliqué à la position/locomotion (canal
                    // cosmétique one-shot).
                    self.pending_events.push((from, 0, ar.action(), ar.param()));
                }
            }
            _ => {}
        }
    }

    fn encode_snapshot_for(&self, viewer: ClientId, b: &mut FlatBufferBuilder) -> Vec<u8> {
        let states: Vec<_> = self
            .world
            .snapshot_for(viewer, self.aoi_radius)
            .into_iter()
            .map(|(id, pose)| {
                let pos = Vec3::new(pose.x, pose.y, pose.z);
                PlayerState::create(
                    b,
                    &PlayerStateArgs {
                        id,
                        position: Some(&pos),
                        yaw: pose.yaw,
                        locomotion: pose.locomotion,
                        move_dir: pose.move_dir,
                        flags: pose.flags,
                        sustained: pose.sustained,
                    },
                )
            })
            .collect();
        let players = b.create_vector(&states);
        let snap = Snapshot::create(
            b,
            &SnapshotArgs {
                tick: self.world.tick(),
                players: Some(players),
            },
        );
        let env = ServerEnvelope::create(
            b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;

    fn encode_position(x: f32, y: f32, z: f32, yaw: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
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

    #[test]
    fn two_clients_see_each_other_move() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();

        // Deux clients se connectent.
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // Client 1 bouge en (5,0,0).
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(5.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        // Le client 2 doit recevoir un snapshot contenant le joueur 1 en x=5.
        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1, "un snapshot envoyé au client 2");
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(players.len(), 1);
        let p = players.get(0);
        assert_eq!(p.id(), 1);
        assert_eq!(p.position().unwrap().x(), 5.0);

        let sent_to_1 = t.take_sent(1);
        assert_eq!(sent_to_1.len(), 1, "un snapshot envoyé au client 1");
        let env1 = flatbuffers::root::<ServerEnvelope>(&sent_to_1[0]).unwrap();
        let snap1 = env1.msg_as_snapshot().unwrap();
        let players1 = snap1.players().unwrap();
        assert_eq!(players1.len(), 1);
        assert_eq!(players1.get(0).id(), 2); // client 1 voit client 2
    }

    fn encode_position_with_locomotion(
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        locomotion: u8,
        move_dir: u8,
    ) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
                locomotion,
                move_dir,
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

    #[test]
    fn position_update_carries_locomotion_into_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 3, 42),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.locomotion(), 3);
        assert_eq!(p.move_dir(), 42);
    }

    #[test]
    fn repeated_position_updates_do_not_reset_locomotion_to_idle() {
        // Piège identifié en Task 2 : set_pose remplace toute la Pose. Un deuxième PositionUpdate
        // (même sans nouveau champ de locomotion explicite envoyé par le client, qui renvoie
        // toujours son état courant à chaque update selon la spec §8.1) ne doit jamais faire
        // disparaître la valeur précédente si le client continue de la reporter correctement —
        // ce test vérifie surtout que l'ordre d'application (set_pose puis set_locomotion, ou une
        // fusion) ne perd pas le champ dans le MÊME message.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(6.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.position().unwrap().x(), 6.0);
        assert_eq!(p.locomotion(), 2);
    }

    #[test]
    fn position_update_never_touches_sustained() {
        // Le canal cosmétique continu (locomotion) et la pose tenue (sustained, pilotée par
        // EmoteReport UNIQUEMENT) doivent rester complètement indépendants — un PositionUpdate ne
        // doit jamais remettre sustained à 0 s'il était déjà posé.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // (Task 4 posera sustained via EmoteReport ; ici on vérifie juste qu'un PositionUpdate seul
        // sur un joueur au sustained par défaut à 0 le laisse à 0 - non-régression basique, le test
        // complet d'indépendance vraie est en Task 4 une fois EmoteReport câblé.)
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 1, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    fn encode_emote_report(emote: u32, start: bool) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let er = EmoteReport::create(&mut b, &EmoteReportArgs { emote, start });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::EmoteReport,
                msg: Some(er.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn emote_report_start_sets_sustained_in_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, true),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 7);
    }

    #[test]
    fn emote_report_stop_clears_sustained() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, true),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, false),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    #[test]
    fn sustained_emote_survives_a_subsequent_position_update() {
        // LE test clé du raffinement §5 de la spec : l'état continu (sustained) doit survivre à un
        // PositionUpdate qui suit — les deux canaux sont indépendants.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(9, true),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(1.0, 0.0, 0.0, 0.0, 0, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(
            p.sustained(),
            9,
            "la pose tenue doit survivre au PositionUpdate suivant"
        );
        assert_eq!(p.position().unwrap().x(), 1.0);
    }

    fn encode_player_action(action: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ar = PlayerActionReport::create(&mut b, &PlayerActionReportArgs { action, param });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PlayerActionReport,
                msg: Some(ar.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn decode_player_event(bytes: &[u8]) -> Option<(u64, u8, u8, u32)> {
        let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
        if env.msg_type() != ServerMsg::PlayerEvent {
            return None;
        }
        let pe = env.msg_as_player_event()?;
        Some((pe.actor(), pe.kind(), pe.action(), pe.param()))
    }

    #[test]
    fn player_action_report_relays_player_event_to_aoi_neighbor() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(5, 99),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        // sent_to_2 contient le Snapshot ET le PlayerEvent — filtrer par type.
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        let (actor, kind, action, param) = event.expect("le voisin doit recevoir le PlayerEvent");
        assert_eq!(actor, 1);
        assert_eq!(kind, 0);
        assert_eq!(action, 5);
        assert_eq!(param, 99);
    }

    #[test]
    fn player_action_report_not_relayed_outside_aoi_radius() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(500.0, 0.0, 0.0, 0.0, 0, 0),
        });
        server.tick(&mut t);
        t.take_sent(2); // vider le snapshot du premier tick
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(5, 99),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        assert!(
            event.is_none(),
            "le joueur 2 est hors AoI (500 > 50), ne doit rien recevoir"
        );
    }

    #[test]
    fn player_action_report_never_touches_position_or_locomotion() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(3.0, 0.0, 0.0, 0.0, 2, 0),
        });
        server.tick(&mut t);
        t.take_sent(2);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        }); // ex. Jump
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(
            sent_to_2
                .iter()
                .find(|b| {
                    flatbuffers::root::<ServerEnvelope>(b)
                        .map(|e| e.msg_type() == ServerMsg::Snapshot)
                        .unwrap_or(false)
                })
                .unwrap(),
        )
        .unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(
            p.position().unwrap().x(),
            3.0,
            "un PlayerActionReport ne doit jamais déplacer le joueur"
        );
        assert_eq!(p.locomotion(), 2, "ni changer sa locomotion continue");
    }

    #[test]
    fn players_far_apart_do_not_see_each_other() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();

        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(500.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(
            players.len(),
            0,
            "client 1 est à 500 unités, hors du rayon de 50 — ne doit pas apparaître"
        );
    }

    #[test]
    fn cosmetic_channel_events_never_change_player_count_or_connectivity() {
        // Aucun message du canal d'état (PositionUpdate enrichi, EmoteReport, PlayerActionReport)
        // ne doit jamais connecter/déconnecter un joueur ni modifier player_count.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        assert_eq!(server.player_count(), 2);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(1, true),
        });
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        });
        server.tick(&mut t);
        assert_eq!(
            server.player_count(),
            2,
            "le canal cosmétique ne doit jamais affecter la connectivité"
        );
    }

    #[test]
    fn late_aoi_joiner_learns_sustained_pose_from_snapshot_not_from_missed_event() {
        // Le test clé §5 de la spec, version bout-en-bout via Server (pas juste World, déjà couvert
        // en Task 4) : un joueur qui rejoint l'AoI APRÈS le début d'une pose tenue doit quand même
        // la voir dans son PREMIER snapshot (auto-cicatrisant), sans avoir reçu l'EmoteReport lui-même.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(3, true),
        });
        server.tick(&mut t);
        // Le joueur 2 arrive APRÈS le début de la pose.
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(
            sent_to_2
                .iter()
                .find(|b| {
                    flatbuffers::root::<ServerEnvelope>(b)
                        .map(|e| e.msg_type() == ServerMsg::Snapshot)
                        .unwrap_or(false)
                })
                .unwrap(),
        )
        .unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(
            snap.players().unwrap().get(0).sustained(),
            3,
            "un arrivant tardif doit lire la pose depuis le snapshot"
        );
    }

    #[test]
    fn one_shot_event_not_resent_to_late_joiner() {
        // Le contraste du test précédent : un ÉVÉNEMENT one-shot (PlayerActionReport→PlayerEvent)
        // n'est PAS auto-cicatrisant — un arrivant tardif ne le reçoit pas rétroactivement, c'est
        // le comportement voulu (rater un one-shot est inoffensif, cf. spec §5).
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        });
        server.tick(&mut t); // aucun voisin au moment de l'action — rien relayé, personne pour le recevoir
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        assert!(
            event.is_none(),
            "un one-shot manqué reste manqué, pas de rattrapage"
        );
    }
}
