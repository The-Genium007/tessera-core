//! Logique de routage du Gateway (M3) : extraire la position du protocole client + assigner les
//! clients aux shards selon leur 1re position. Pur, testable sans GNS/TCP.

use protocol::{
    ClientEnvelope, ClientEnvelopeArgs, ClientMsg, PositionUpdate, PositionUpdateArgs, Vec3,
};

/// Version courante du protocole FlatBuffers parlée par CE serveur (Task C3). Comparée au champ
/// `protocol_version` du `Join` — un client qui ne matche pas est kické avec un message explicite
/// avant toute logique métier, plutôt que de risquer une désérialisation qui dérive
/// silencieusement au fil des évolutions du schéma. À incrémenter à chaque changement de schéma
/// qui casse la compatibilité binaire (champ retiré/retypé, union réordonnée) — PAS à chaque ajout
/// additif rétrocompatible (un nouveau champ optionnel en fin de table, par exemple).
pub const CURRENT_PROTOCOL_VERSION: u32 = 1;

/// Décode un `ClientEnvelope` client ; si c'est un `ClientTimeReport`, renvoie (heure, minute,
/// seconde) tels qu'observés localement par le client — diagnostic de dérive d'horloge (playtest).
pub fn extract_time_report(client_payload: &[u8]) -> Option<(u8, u8, u8)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::ClientTimeReport {
        return None;
    }
    let report = env.msg_as_client_time_report()?;
    Some((report.hour(), report.minute(), report.second()))
}

/// Décode un `ClientEnvelope` client ; si c'est un `PositionUpdate`, renvoie sa position.
pub fn extract_position(client_payload: &[u8]) -> Option<(f32, f32, f32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::PositionUpdate {
        return None;
    }
    let pu = env.msg_as_position_update()?;
    let p = pu.position()?;
    Some((p.x(), p.y(), p.z()))
}

/// Construit le payload client d'un `PositionUpdate` — utilisé pour re-semer, sur un shard qui
/// vient de perdre son état, la dernière position connue d'un client par le Gateway (le client
/// réel n'a pas renvoyé cette position, elle est reconstruite depuis `last_pos`). Yaw à 0 : une
/// orientation temporairement fausse s'auto-corrige au prochain vrai `PositionUpdate` du client.
pub fn encode_position_update(pos: [f32; 3]) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let p = Vec3::new(pos[0], pos[1], pos[2]);
    let pu = PositionUpdate::create(
        &mut b,
        &PositionUpdateArgs {
            position: Some(&p),
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

/// Longueur max d'un nom de joueur après nettoyage — évite qu'un nom énorme gonfle le journal de
/// session ou la persistance (le nom sert de clé dans le store JSON, cf. persistence.rs).
const MAX_DISPLAY_NAME_LEN: usize = 32;

/// Nettoie un nom de joueur reçu du client (non fiable, brut du réseau) avant qu'il n'atteigne les
/// logs (`tracing::info!` — dont certains sont maintenant lisibles à distance via l'API Dokploy,
/// cf. gateway.rs) ou le store de persistance. Sans ça, un nom contenant des retours à la ligne ou
/// des séquences d'échappement ANSI peut forger de fausses lignes de log ou perturber l'affichage
/// d'un terminal qui les lit (log injection). Ne garde que les caractères imprimables ASCII,
/// remplace tout le reste par `_`, et tronque à une longueur raisonnable.
fn sanitize_display_name(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .take(MAX_DISPLAY_NAME_LEN)
        .collect()
}

/// Décode un `ClientEnvelope` client ; si c'est un `Join`, renvoie `(display_name nettoyé, token,
/// protocol_version)` (voir `sanitize_display_name` — le nom brut n'est jamais fiable, il vient du
/// réseau). `token` est renvoyé tel quel, chaîne vide si absent (défaut FlatBuffers) — sa
/// vérification (JWT ZITADEL, requise seulement si `identity.public = true`) vit dans
/// `gateway::resolve_join_key`, pas ici : ce module reste pur décodage, sans connaissance du
/// `JwksCache`. `protocol_version` vaut 0 si absent (client trop ancien pour connaître le champ) —
/// sa comparaison à `CURRENT_PROTOCOL_VERSION` vit aussi côté `gateway.rs` (Task C3), pas ici.
pub fn extract_join_fields(client_payload: &[u8]) -> Option<(String, String, u32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::Join {
        return None;
    }
    let join = env.msg_as_join()?;
    let name = join
        .display_name()
        .map(sanitize_display_name)
        .unwrap_or_default();
    let token = join.token().unwrap_or_default().to_string();
    Some((name, token, join.protocol_version()))
}

/// Décode un `ClientEnvelope` client ; si c'est un `Leave` (départ volontaire, Task C3), renvoie
/// `Some(())` — table vide, seul le type du message compte. Distinct d'une coupure réseau/crash
/// (`TransportEvent::Disconnected`, jamais annoncé par le client) : permet au Gateway de libérer
/// immédiatement l'état par-client plutôt que d'attendre un timeout de transport.
pub fn extract_leave(client_payload: &[u8]) -> Option<()> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::Leave {
        return None;
    }
    env.msg_as_leave()?;
    Some(())
}

/// Décode un `ClientEnvelope` client ; si c'est un `AdminCommand`, renvoie son texte brut
/// (le parsing/la validation vivent dans `admin_commands::parse`, pas ici).
pub fn extract_admin_command(client_payload: &[u8]) -> Option<String> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::AdminCommand {
        return None;
    }
    let cmd = env.msg_as_admin_command()?;
    cmd.text().map(|s| s.to_string())
}

use crate::internal_net::event_to_client_event_frame;
use crate::transport::{ClientId, TransportEvent};
use std::collections::HashMap;

/// Action décidée par l'assigneur pour un événement client.
#[derive(Debug)]
pub enum AssignAction {
    /// Événement bufferisé (client pas encore assigné, pas de position).
    Buffered,
    /// Une position est arrivée pour un client non assigné → le caller doit interroger le Router
    /// avec (x,y,z) puis appeler `assign(client_id, shard)`.
    NeedRoute {
        client_id: ClientId,
        x: f32,
        y: f32,
        z: f32,
    },
    /// Frames `ClientEvent` à écrire au shard indiqué.
    Forward { shard: String, frames: Vec<Vec<u8>> },
}

/// Suit l'état de chaque client : non assigné (buffer) → assigné à un shard.
#[derive(Default)]
pub struct ShardAssigner {
    assigned: HashMap<ClientId, String>,
    buffer: HashMap<ClientId, Vec<TransportEvent>>,
}

impl ShardAssigner {
    pub fn new() -> Self {
        Self::default()
    }

    fn client_of(ev: &TransportEvent) -> ClientId {
        match ev {
            TransportEvent::Connected(id) | TransportEvent::Disconnected(id) => *id,
            TransportEvent::Message { from, .. } => *from,
        }
    }

    pub fn feed(&mut self, ev: TransportEvent) -> AssignAction {
        let id = Self::client_of(&ev);

        // Déjà assigné → on relaie directement vers son shard.
        if let Some(shard) = self.assigned.get(&id) {
            return AssignAction::Forward {
                shard: shard.clone(),
                frames: vec![event_to_client_event_frame(&ev)],
            };
        }

        // Non assigné : si c'est une position, on bufferise ET on demande un routage.
        if let TransportEvent::Message { from, data } = &ev {
            if let Some((x, y, z)) = extract_position(data) {
                let from = *from;
                self.buffer.entry(id).or_default().push(ev);
                return AssignAction::NeedRoute {
                    client_id: from,
                    x,
                    y,
                    z,
                };
            }
        }

        // Sinon (Connected/Join/autre) : on bufferise et on attend.
        self.buffer.entry(id).or_default().push(ev);
        AssignAction::Buffered
    }

    pub fn assign(&mut self, client_id: ClientId, shard: String) -> AssignAction {
        self.assigned.insert(client_id, shard.clone());
        let pending = self.buffer.remove(&client_id).unwrap_or_default();
        let frames = pending.iter().map(event_to_client_event_frame).collect();
        AssignAction::Forward { shard, frames }
    }

    pub fn forget(&mut self, client_id: ClientId) {
        self.assigned.remove(&client_id);
        self.buffer.remove(&client_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;
    use protocol::*;

    fn client_position(x: f32, y: f32, z: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
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

    fn client_join() -> Vec<u8> {
        client_join_with_token("v", None)
    }

    fn client_join_with_token(name: &str, token: Option<&str>) -> Vec<u8> {
        client_join_with_version(name, token, CURRENT_PROTOCOL_VERSION)
    }

    fn client_join_with_version(name: &str, token: Option<&str>, protocol_version: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let name = b.create_string(name);
        let token = token.map(|t| b.create_string(t));
        let join = Join::create(
            &mut b,
            &JoinArgs {
                display_name: Some(name),
                token,
                protocol_version,
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

    fn client_leave() -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let leave = Leave::create(&mut b, &LeaveArgs {});
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::Leave,
                msg: Some(leave.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn client_admin_command(text: &str) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let t = b.create_string(text);
        let cmd = AdminCommand::create(&mut b, &AdminCommandArgs { text: Some(t) });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::AdminCommand,
                msg: Some(cmd.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn encode_position_update_round_trips_through_extract_position() {
        let payload = encode_position_update([2387.0, -1295.0, 63.0]);
        assert_eq!(extract_position(&payload), Some((2387.0, -1295.0, 63.0)));
    }

    #[test]
    fn extract_position_reads_position_update_and_ignores_join() {
        assert_eq!(
            extract_position(&client_position(2387.0, -1295.0, 63.0)),
            Some((2387.0, -1295.0, 63.0))
        );
        assert_eq!(extract_position(&client_join()), None);
        assert_eq!(extract_position(&[0, 1, 2]), None); // garbage → None
    }

    #[test]
    fn extract_join_fields_reads_display_name_and_token() {
        assert_eq!(
            extract_join_fields(&client_join()),
            Some(("v".to_string(), String::new(), CURRENT_PROTOCOL_VERSION))
        );
        assert_eq!(
            extract_join_fields(&client_join_with_token("v", Some("jwt-abc"))),
            Some((
                "v".to_string(),
                "jwt-abc".to_string(),
                CURRENT_PROTOCOL_VERSION
            ))
        );
        assert_eq!(extract_join_fields(&client_position(1.0, 2.0, 3.0)), None); // pas un Join
        assert_eq!(extract_join_fields(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_join_fields_reads_the_protocol_version_field() {
        assert_eq!(
            extract_join_fields(&client_join_with_version("v", None, 999)),
            Some(("v".to_string(), String::new(), 999))
        );
    }

    /// Octets d'un `Join` produits par le VRAI encodeur C++ du client (fork Cyberverse,
    /// `NetworkGameSystem::SendJoin`, flatc 25.12.19), capturés le 2026-07-15 et figés ici.
    ///
    /// Pourquoi en dur plutôt que ré-encodés en Rust : tous les autres tests de ce module encodent
    /// ET décodent avec le même code Rust — ils prouvent que le serveur est cohérent avec
    /// lui-même, jamais qu'il s'entend avec le client. C'est exactement l'angle mort qui a rendu
    /// le jeu injouable le 2026-07-15 : `protocol_version` ajouté au schéma le 07-13, netcode
    /// publié le 06-29 dont l'en-tête généré ignorait le champ → `CreateJoin` ne le posait pas →
    /// le serveur lisait le défaut 0 → kick à chaque connexion. Les deux suites de tests étaient
    /// vertes ; le fil était cassé.
    ///
    /// Ce buffer transforme cette hypothèse implicite en assertion : si le schéma bouge sans que
    /// l'en-tête du fork soit régénéré (ou l'inverse), ce test rougit ici, en `cargo test` sur
    /// macOS — sans PC Windows, sans build du jeu, sans lancer une partie.
    ///
    /// Le regénérer : voir `kTesseraProtocolVersion` dans `NetworkGameSystem.cpp` (fork).
    const CPP_CLIENT_JOIN_V1: &[u8] = &[
        0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0e, 0x00, 0x07, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x0c, 0x00,
        0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x4c, 0x75, 0x63, 0x61, 0x73, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn server_decodes_a_join_encoded_by_the_real_cpp_client() {
        let (name, token, version) =
            extract_join_fields(CPP_CLIENT_JOIN_V1).expect("le Join du client C++ doit se décoder");
        assert_eq!(name, "Lucas");
        assert_eq!(token, ""); // pas encore câblé côté client (JWT ZITADEL à venir)
        assert_eq!(version, 1);
    }

    #[test]
    fn the_cpp_client_join_is_accepted_by_the_version_check() {
        // Le test qui aurait attrapé le kick du 2026-07-15 avant qu'il n'atteigne un joueur :
        // ce sont les octets réels du client, passés à la fonction réelle qui kicke.
        let (_, _, version) = extract_join_fields(CPP_CLIENT_JOIN_V1).unwrap();
        assert_eq!(
            version, CURRENT_PROTOCOL_VERSION,
            "le client C++ publié doit parler la version courante — sinon le Gateway le kicke \
             (« kick : version protocole incompatible ») et le jeu est injouable"
        );
        assert!(crate::gateway::resolve_protocol_version(version).is_ok());
    }

    #[test]
    fn extract_leave_reads_leave_and_ignores_other_messages() {
        assert_eq!(extract_leave(&client_leave()), Some(()));
        assert_eq!(extract_leave(&client_join()), None); // pas un Leave
        assert_eq!(extract_leave(&client_position(1.0, 2.0, 3.0)), None);
        assert_eq!(extract_leave(&[9, 9, 9]), None); // garbage
    }

    fn client_time_report(hour: u8, minute: u8, second: u8) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let report = ClientTimeReport::create(
            &mut b,
            &ClientTimeReportArgs {
                hour,
                minute,
                second,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::ClientTimeReport,
                msg: Some(report.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn extract_time_report_reads_hour_minute_second() {
        assert_eq!(
            extract_time_report(&client_time_report(14, 30, 5)),
            Some((14, 30, 5))
        );
        assert_eq!(extract_time_report(&client_join()), None); // pas un ClientTimeReport
        assert_eq!(extract_time_report(&[7, 7, 7]), None); // garbage
    }

    #[test]
    fn extract_admin_command_reads_text() {
        assert_eq!(
            extract_admin_command(&client_admin_command("/promote Compte1 moderator")),
            Some("/promote Compte1 moderator".to_string())
        );
        assert_eq!(extract_admin_command(&client_join()), None); // pas un AdminCommand
        assert_eq!(extract_admin_command(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn sanitize_display_name_strips_control_characters() {
        // Un nom client n'est jamais fiable : un nom contenant un retour à la ligne pourrait
        // forger une fausse ligne de log une fois inséré dans tracing::info! (log injection) —
        // confirmé par une revue de sécurité automatisée sur ce module, 2026-07-06.
        assert_eq!(
            sanitize_display_name("lucas\n[ERROR] fake log line"),
            "lucas_[ERROR] fake log line"
        );
    }

    #[test]
    fn sanitize_display_name_strips_ansi_escape_sequences() {
        assert_eq!(
            sanitize_display_name("lucas\x1b[31mred\x1b[0m"),
            "lucas_[31mred_[0m"
        );
    }

    #[test]
    fn sanitize_display_name_truncates_to_max_length() {
        let long_name = "a".repeat(200);
        assert_eq!(
            sanitize_display_name(&long_name).len(),
            MAX_DISPLAY_NAME_LEN
        );
    }

    #[test]
    fn sanitize_display_name_leaves_normal_names_untouched() {
        assert_eq!(sanitize_display_name("lucas"), "lucas");
        assert_eq!(sanitize_display_name("V 2.0"), "V 2.0");
    }

    use crate::framing::FrameReader;
    use crate::internal_net::decode_client_event;
    use crate::transport::TransportEvent;

    #[test]
    fn buffers_until_first_position_then_flushes_on_assign() {
        let mut a = ShardAssigner::new();
        // Connecté + Join : bufferisés, rien à router encore.
        assert!(matches!(
            a.feed(TransportEvent::Connected(1)),
            AssignAction::Buffered
        ));
        assert!(matches!(
            a.feed(TransportEvent::Message {
                from: 1,
                data: client_join()
            }),
            AssignAction::Buffered
        ));
        // 1re position : besoin de router (et l'événement est bufferisé).
        match a.feed(TransportEvent::Message {
            from: 1,
            data: client_position(500.0, 0.0, 0.0),
        }) {
            AssignAction::NeedRoute { client_id, x, .. } => {
                assert_eq!(client_id, 1);
                assert_eq!(x, 500.0);
            }
            other => panic!("attendu NeedRoute, eu {other:?}"),
        }
        // Le caller a interrogé le Router → shard "A". assign flushe les 3 événements framés.
        let act = a.assign(1, "A".to_string());
        let AssignAction::Forward { shard, frames } = act else {
            panic!("attendu Forward")
        };
        assert_eq!(shard, "A");
        assert_eq!(frames.len(), 3); // Connected + Join + Position
                                     // Le 1er frame est un ClientEvent décodable = Connected(1).
        let mut r = FrameReader::new();
        r.push(&frames[0]);
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Connected(1))
        );
    }

    #[test]
    fn assigned_client_forwards_directly_to_its_shard() {
        let mut a = ShardAssigner::new();
        a.feed(TransportEvent::Message {
            from: 2,
            data: client_position(2000.0, 0.0, 0.0),
        }); // NeedRoute
        a.assign(2, "B".to_string());
        // Désormais assigné : un nouvel événement part direct vers B.
        match a.feed(TransportEvent::Message {
            from: 2,
            data: vec![1, 2, 3],
        }) {
            AssignAction::Forward { shard, frames } => {
                assert_eq!(shard, "B");
                assert_eq!(frames.len(), 1);
            }
            other => panic!("attendu Forward, eu {other:?}"),
        }
    }
}
