//! Logique de routage du Gateway (M3) : extraire la position du protocole client + assigner les
//! clients aux shards selon leur 1re position. Pur, testable sans GNS/TCP.

use crate::quant::{dq_pos, dq_yaw, q_pos};
use protocol::{
    ClientEnvelope, ClientEnvelopeArgs, ClientMsg, PositionUpdate, PositionUpdateArgs, QVec3,
};

/// Version courante du protocole FlatBuffers parlée par CE serveur (Task C3). Comparée au champ
/// `protocol_version` du `Join` — un client qui ne matche pas est kické avec un message explicite
/// avant toute logique métier, plutôt que de risquer une désérialisation qui dérive
/// silencieusement au fil des évolutions du schéma. À incrémenter à chaque changement de schéma
/// qui casse la compatibilité binaire (champ retiré/retypé, union réordonnée) — PAS à chaque ajout
/// additif rétrocompatible (un nouveau champ optionnel en fin de table, par exemple).
/// v2 : gel palier 2 (2026-07-23) — positions `Vec3`→`QVec3` fixed-point, yaw `float`→`ushort`,
/// `InteractionChoice.choice` `ubyte`→`uint` (retypages = cassure binaire assumée, un client v1
/// est kické au Join avec le message explicite plutôt que de lire des positions absurdes).
pub const CURRENT_PROTOCOL_VERSION: u32 = 2;

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

/// Décode un `ClientEnvelope` client ; si c'est un `PositionUpdate`, renvoie sa position en
/// mètres f32 (déquantifiée du `QVec3` fixed-point du fil — le cœur serveur reste en f32, la
/// conversion vit au bord, cf. `quant.rs`).
pub fn extract_position(client_payload: &[u8]) -> Option<(f32, f32, f32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::PositionUpdate {
        return None;
    }
    let pu = env.msg_as_position_update()?;
    let p = pu.position()?;
    Some((dq_pos(p.x()), dq_pos(p.y()), dq_pos(p.z())))
}

/// Comme `extract_position` mais renvoie le `yaw` du `PositionUpdate` en degrés f32 (déquantifié
/// du `ushort` du fil) — nécessaire pour renvoyer une `PositionCorrection` (rubber-band zone
/// rouge) fidèle à l'orientation du joueur. `None` si l'enveloppe n'est pas un `PositionUpdate`
/// décodable.
pub fn extract_position_yaw(client_payload: &[u8]) -> Option<f32> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::PositionUpdate {
        return None;
    }
    let pu = env.msg_as_position_update()?;
    Some(dq_yaw(pu.yaw()))
}

/// Construit le payload client d'un `PositionUpdate` — utilisé pour re-semer, sur un shard qui
/// vient de perdre son état, la dernière position connue d'un client par le Gateway (le client
/// réel n'a pas renvoyé cette position, elle est reconstruite depuis `last_pos`). Yaw à 0 : une
/// orientation temporairement fausse s'auto-corrige au prochain vrai `PositionUpdate` du client.
pub fn encode_position_update(pos: [f32; 3]) -> Vec<u8> {
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let p = QVec3::new(q_pos(pos[0]), q_pos(pos[1]), q_pos(pos[2]));
    let pu = PositionUpdate::create(
        &mut b,
        &PositionUpdateArgs {
            position: Some(&p),
            yaw: 0,
            locomotion: 0,
            move_dir: 0,
            flags: 0,
            // Repère monde (ADR 0013) : cette position re-semée vient de `last_pos` du Gateway,
            // toujours une position monde — frame/slot posés explicitement à 0 (= monde).
            frame: 0,
            slot: 0,
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
/// protocol_version, hwid_hash)` (voir `sanitize_display_name` — le nom brut n'est jamais fiable,
/// il vient du réseau). `token` est renvoyé tel quel, chaîne vide si absent (défaut FlatBuffers) —
/// sa vérification (JWT ZITADEL, requise seulement si `identity.public = true`) vit dans
/// `gateway::resolve_join_key`, pas ici : ce module reste pur décodage, sans connaissance du
/// `JwksCache`. `protocol_version` vaut 0 si absent (client trop ancien pour connaître le champ) —
/// sa comparaison à `CURRENT_PROTOCOL_VERSION` vit aussi côté `gateway.rs` (Task C3), pas ici.
/// `hwid_hash` (Task 3, robustesse opérationnelle) vaut chaîne vide si absent (client qui ne le
/// pose pas encore, ou schéma de travail non régénéré côté fork C++) — sa vérification contre les
/// bans actifs vit aussi côté `gateway.rs`, pas ici.
pub fn extract_join_fields(client_payload: &[u8]) -> Option<(String, String, u32, String)> {
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
    let hwid_hash = join.hwid_hash().unwrap_or_default().to_string();
    Some((name, token, join.protocol_version(), hwid_hash))
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

/// Décode un `ClientEnvelope` client ; si c'est un `EntityInteraction` (fondation PNJ, palier 2),
/// renvoie `(target, kind, param)` — l'entité visée, le type d'interaction rapporté (0=Menace/
/// attaque 1=Parle 2=Interagit) et son contexte. L'arbitrage (traduire ou non en transition FSM)
/// vit dans `NpcRecord::apply_interaction` (npc.rs), pas ici : ce module reste pur décodage.
pub fn extract_entity_interaction(client_payload: &[u8]) -> Option<(u64, u8, u32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::EntityInteraction {
        return None;
    }
    let ei = env.msg_as_entity_interaction()?;
    Some((ei.target(), ei.kind(), ei.param()))
}

/// Décode un `ClientEnvelope` ; si c'est un `InteractionChoice` (fondation d'interaction, palier 2),
/// renvoie `(session_id, choice, param)`. Pur décodage — l'arbitrage (session valide ? bon
/// propriétaire ?) vit dans `interaction_session.rs`, pas ici. `choice` élargi `u8`→`u32` au gel
/// (2026-07-23) : opaque et tourné vers un contenu inconnu, 256 choix par écran était une borne
/// arbitraire.
pub fn extract_interaction_choice(client_payload: &[u8]) -> Option<(u64, u32, u32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::InteractionChoice {
        return None;
    }
    let ic = env.msg_as_interaction_choice()?;
    Some((ic.session_id(), ic.choice(), ic.param()))
}

// ── Flux d'arrivée : dispatch personnage (palier 2, tranche A serveur) ────────────────────────
// Même patron que `extract_admin_command`/`encode_command_result` : décodage/encodage pur, sans
// connaissance du `CharacterStore` ni de la machine à états (celle-ci vit dans `gateway_main`).

/// Décode un `ClientEnvelope` client ; si c'est un `CreateCharacter`, renvoie le pseudonyme demandé.
pub fn extract_create_character(client_payload: &[u8]) -> Option<String> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::CreateCharacter {
        return None;
    }
    let cmd = env.msg_as_create_character()?;
    cmd.pseudonym().map(|s| s.to_string())
}

/// Décode un `ClientEnvelope` client ; si c'est un `SelectCharacter`, renvoie l'id du personnage
/// que le client souhaite incarner.
pub fn extract_select_character(client_payload: &[u8]) -> Option<u64> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::SelectCharacter {
        return None;
    }
    env.msg_as_select_character().map(|c| c.id())
}

/// Décode un `ClientEnvelope` client ; si c'est un `DeleteCharacter`, renvoie l'id du personnage
/// à supprimer.
pub fn extract_delete_character(client_payload: &[u8]) -> Option<u64> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::DeleteCharacter {
        return None;
    }
    env.msg_as_delete_character().map(|c| c.id())
}

/// Décode un `ClientEnvelope` client ; si c'est un `ElevatorCall` (modèle serveur ascenseur, palier
/// 2, Task 4), renvoie `(elevator_id, floor)`. Le serveur ne distingue pas le bouton intérieur de la
/// cabine du terminal de palier — c'est le même acte, et l'arbitrage (file SCAN, `ElevatorState`)
/// vit dans `elevator.rs`, pas ici : ce module reste pur décodage.
pub fn extract_elevator_call(client_payload: &[u8]) -> Option<(u64, i32)> {
    let env = flatbuffers::root::<ClientEnvelope>(client_payload).ok()?;
    if env.msg_type() != ClientMsg::ElevatorCall {
        return None;
    }
    let call = env.msg_as_elevator_call()?;
    Some((call.elevator_id(), call.floor()))
}

/// Construit le payload serveur d'un `CharacterResult` — réponse à un `CreateCharacter`/
/// `SelectCharacter`/`DeleteCharacter`. `reason` porte un code d'erreur stable (`pseudonym_taken`,
/// `slot_full`, `not_found`, `not_owner`, `database_error`) quand `success` est faux, chaîne vide
/// en cas de succès.
pub fn encode_character_result(success: bool, reason: &str) -> Vec<u8> {
    use protocol::{
        CharacterResult, CharacterResultArgs, ServerEnvelope, ServerEnvelopeArgs, ServerMsg,
    };
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let reason_off = b.create_string(reason);
    let res = CharacterResult::create(
        &mut b,
        &CharacterResultArgs {
            success,
            reason: Some(reason_off),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::CharacterResult,
            msg: Some(res.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Construit le payload serveur d'un `InteractionOpen` (fondation d'interaction, palier 2 —
/// ouverture de session). `payload` est copié tel quel (opaque, cf. commentaire du schéma) :
/// ni ce module ni le schéma n'en interprètent le contenu.
pub fn encode_interaction_open(
    session_id: u64,
    target: u64,
    ui_kind: u8,
    payload: &[u8],
) -> Vec<u8> {
    use protocol::{
        InteractionOpen, InteractionOpenArgs, ServerEnvelope, ServerEnvelopeArgs, ServerMsg,
    };
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let payload_off = b.create_vector(payload);
    let open = InteractionOpen::create(
        &mut b,
        &InteractionOpenArgs {
            session_id,
            target,
            ui_kind,
            payload: Some(payload_off),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::InteractionOpen,
            msg: Some(open.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Construit le payload serveur d'un `InteractionResult` (fondation d'interaction, palier 2 —
/// verdict de session). `ok=false` avec `payload` vide = refus sans détail transporté (session
/// inconnue/expirée) ; `ok=false` avec `payload` = raison in-fiction encodée par le contenu réel
/// (différé, cf. commentaire du schéma).
pub fn encode_interaction_result(session_id: u64, ok: bool, payload: &[u8]) -> Vec<u8> {
    use protocol::{
        InteractionResult, InteractionResultArgs, ServerEnvelope, ServerEnvelopeArgs, ServerMsg,
    };
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let payload_off = b.create_vector(payload);
    let result = InteractionResult::create(
        &mut b,
        &InteractionResultArgs {
            session_id,
            ok,
            payload: Some(payload_off),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::InteractionResult,
            msg: Some(result.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
}

/// Construit le payload serveur d'un `CharacterList` — envoyé automatiquement à l'entrée en
/// `AwaitingSelection` (liste des personnages du compte). `characters` est une liste de
/// `(id, pseudonym)` déjà résolue par l'appelant depuis le `CharacterStore`.
pub fn encode_character_list(characters: &[(u64, String)]) -> Vec<u8> {
    use protocol::{
        CharacterList, CharacterListArgs, CharacterSummary, CharacterSummaryArgs, ServerEnvelope,
        ServerEnvelopeArgs, ServerMsg,
    };
    let mut b = flatbuffers::FlatBufferBuilder::new();
    let summaries: Vec<_> = characters
        .iter()
        .map(|(id, pseudonym)| {
            let p = b.create_string(pseudonym);
            CharacterSummary::create(
                &mut b,
                &CharacterSummaryArgs {
                    id: *id,
                    pseudonym: Some(p),
                },
            )
        })
        .collect();
    let vec_off = b.create_vector(&summaries);
    let list = CharacterList::create(
        &mut b,
        &CharacterListArgs {
            characters: Some(vec_off),
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::CharacterList,
            msg: Some(list.as_union_value()),
        },
    );
    b.finish(env, None);
    b.finished_data().to_vec()
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
        let pos = QVec3::new(q_pos(x), q_pos(y), q_pos(z));
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw: 0,
                locomotion: 0,
                move_dir: 0,
                flags: 0,
                frame: 0,
                slot: 0,
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
        let hwid_hash = b.create_string("");
        let join = Join::create(
            &mut b,
            &JoinArgs {
                display_name: Some(name),
                token,
                protocol_version,
                hwid_hash: Some(hwid_hash),
                space_id: 0, // monde ouvert (l'instancing est un chantier palier 3)
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
    fn extract_position_yaw_reads_yaw_and_ignores_join() {
        let mut b = FlatBufferBuilder::new();
        let pos = QVec3::new(q_pos(1.0), q_pos(2.0), q_pos(3.0));
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                // 90° = 16384 crans exactement (65536/4) — le round-trip est sans perte sur les
                // quarts de tour, cf. quant.rs.
                position: Some(&pos),
                yaw: crate::quant::q_yaw(90.0),
                locomotion: 0,
                move_dir: 0,
                flags: 0,
                frame: 0,
                slot: 0,
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
        let payload = b.finished_data().to_vec();

        assert_eq!(extract_position_yaw(&payload), Some(90.0));
        assert_eq!(extract_position_yaw(&client_join()), None);
        assert_eq!(extract_position_yaw(&[0, 1, 2]), None); // garbage → None
    }

    #[test]
    fn extract_join_fields_reads_display_name_and_token() {
        assert_eq!(
            extract_join_fields(&client_join()),
            Some((
                "v".to_string(),
                String::new(),
                CURRENT_PROTOCOL_VERSION,
                String::new()
            ))
        );
        assert_eq!(
            extract_join_fields(&client_join_with_token("v", Some("jwt-abc"))),
            Some((
                "v".to_string(),
                "jwt-abc".to_string(),
                CURRENT_PROTOCOL_VERSION,
                String::new()
            ))
        );
        assert_eq!(extract_join_fields(&client_position(1.0, 2.0, 3.0)), None); // pas un Join
        assert_eq!(extract_join_fields(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_join_fields_reads_the_protocol_version_field() {
        assert_eq!(
            extract_join_fields(&client_join_with_version("v", None, 999)),
            Some(("v".to_string(), String::new(), 999, String::new()))
        );
    }

    #[test]
    fn extract_join_fields_reads_hwid_hash() {
        let mut b = flatbuffers::FlatBufferBuilder::new();
        let name = b.create_string("Lucas");
        let token = b.create_string("");
        let hwid = b.create_string("hashed-value");
        let join = Join::create(
            &mut b,
            &JoinArgs {
                display_name: Some(name),
                token: Some(token),
                protocol_version: 1,
                hwid_hash: Some(hwid),
                space_id: 0,
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
        let (_, _, _, hwid_hash) = extract_join_fields(b.finished_data()).unwrap();
        assert_eq!(hwid_hash, "hashed-value");
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
        let (name, token, version, hwid_hash) =
            extract_join_fields(CPP_CLIENT_JOIN_V1).expect("le Join du client C++ doit se décoder");
        assert_eq!(name, "Lucas");
        assert_eq!(token, ""); // pas encore câblé côté client (JWT ZITADEL à venir)
        assert_eq!(version, 1);
        // `hwid_hash` n'existait pas dans le schéma au moment où ces octets ont été capturés
        // (2026-07-15) — champ additif en fin de table, défaut FlatBuffers vide attendu tant que
        // le fork C++ n'est pas régénéré pour le poser (Task 3, robustesse opérationnelle).
        assert_eq!(hwid_hash, "");
    }

    /// Octets d'un `Join` v2 produits par le VRAI encodeur C++ (gel palier 2) : même séquence
    /// d'appels que `NetworkGameSystem::SendJoin` du fork, même `protocol_generated.h` régénéré
    /// (flatc 25.12.19, headers flatbuffers 25.12.19 vendorés du fork), compilé MSVC et exécuté le
    /// 2026-07-23 (outil `dump_wire_v2.cpp`). Join{display_name="Lucas", token="",
    /// protocol_version=2} — hwid_hash/space_id non posés (défauts, comme le client actuel).
    const CPP_CLIENT_JOIN_V2: &[u8] = &[
        0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0e, 0x00, 0x07, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x10, 0x00,
        0x04, 0x00, 0x08, 0x00, 0x0c, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x08,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x05, 0x00, 0x00, 0x00, 0x4c, 0x75, 0x63, 0x61, 0x73, 0x00, 0x00, 0x00,
    ];

    /// Octets d'un `PositionUpdate` v2 produits par le VRAI encodeur C++ (même provenance que
    /// `CPP_CLIENT_JOIN_V2`) : position (2387, -1295, 63) m quantifiée QVec3, yaw 90° (=0x4000),
    /// frame/slot 0 — la séquence exacte de `NetworkGameSystem::SendPositionUpdate` après gel.
    const CPP_CLIENT_POSITION_UPDATE_V2: &[u8] = &[
        0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0c, 0x00, 0x07, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x02, 0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x14, 0x00, 0x08, 0x00,
        0x06, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0xa6, 0x12, 0x00,
        0x00, 0xe2, 0xf5, 0x00, 0x00, 0x7e, 0x00,
    ];

    #[test]
    fn server_decodes_a_v2_join_encoded_by_the_real_cpp_client_and_accepts_it() {
        // Le fil, pas chaque côté : ces octets viennent du header C++ régénéré au gel. Si le
        // schéma bouge sans régénérer le fork (ou l'inverse), ce test rougit en `cargo test` —
        // sans PC Windows, sans lancer le jeu (piège 2026-07-15/16).
        let (name, token, version, hwid_hash) = extract_join_fields(CPP_CLIENT_JOIN_V2)
            .expect("le Join v2 du client C++ doit se décoder");
        assert_eq!(name, "Lucas");
        assert_eq!(token, "");
        assert_eq!(version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(hwid_hash, ""); // non posé par le client actuel (câblage launcher à venir)
        assert!(crate::gateway::resolve_protocol_version(version).is_ok());
    }

    #[test]
    fn server_decodes_a_v2_position_update_encoded_by_the_real_cpp_client() {
        // 2387/-1295/63 m sont représentables exactement sur la grille fixed-point (quant.rs) :
        // la déquantization redonne les valeurs d'origine bit-à-bit, pas « à epsilon près ».
        assert_eq!(
            extract_position(CPP_CLIENT_POSITION_UPDATE_V2),
            Some((2387.0, -1295.0, 63.0))
        );
        assert_eq!(
            extract_position_yaw(CPP_CLIENT_POSITION_UPDATE_V2),
            Some(90.0)
        );
    }

    #[test]
    fn a_v1_cpp_client_join_still_decodes_but_is_rejected_by_the_version_check() {
        // Gel palier 2 (v2) : `Join` est resté APPEND-ONLY précisément pour ça — les octets v1 du
        // vrai client C++ doivent encore SE DÉCODER (sinon le serveur ne peut pas lire leur
        // protocol_version et kicker avec un message explicite), mais la vérification de version
        // doit désormais les REFUSER (les positions v1 sont des Vec3 float, illisibles en v2).
        // Les octets v2 du vrai encodeur C++ seront figés en 1.4 (test frère de celui-ci).
        let (_, _, version, _) = extract_join_fields(CPP_CLIENT_JOIN_V1).unwrap();
        assert_eq!(version, 1);
        assert_ne!(version, CURRENT_PROTOCOL_VERSION);
        assert!(
            crate::gateway::resolve_protocol_version(version).is_err(),
            "un client v1 doit être kické au Join (message explicite), jamais laissé parler"
        );
    }

    // ── Fil figé ShardAssignment / PositionCorrection (server → client) ──────────────────────
    //
    // NUANCE DE DIRECTION vs le `Join` ci-dessus. `Join` va client→C++ → serveur→Rust : on fige
    // les octets du VRAI encodeur C++ et on prouve que le décodeur Rust les lit — le fil traverse
    // bien deux langages dans le test. `ShardAssignment` et `PositionCorrection` vont dans l'AUTRE
    // sens : serveur→Rust (encode) → client→C++ (décode). Refiger ici des octets et les redécoder
    // en Rust serait exactement la tautologie « encode+décode dans le même langage » que le test
    // Join dénonce — ça ne prouverait rien sur le fil réel.
    //
    // Ce que ce test PEUT garder depuis macOS, sans PC : (1) l'encodeur serveur reste stable
    // octet-pour-octet — tout changement de layout devient un choix conscient qui met à jour ces
    // vecteurs ; (2) auto-cohérence de décodage Rust. Ce qu'il NE peut PAS prouver seul : que le
    // décodeur C++ régénéré (`protocol_generated.h`, flatc 25.12.19) lit bien ces mêmes octets —
    // ça, c'est le sens serveur→client, validé de bout en bout EN JEU (Étape 3 du round-trip :
    // le client applique réellement la téléportation et le HUD affiche le shard serveur). Un test
    // unitaire C++ figeant ces mêmes octets serait le garde idéal, mais le fork n'a pas encore de
    // cible de test C++ (CMake ne build que le plugin) — suivi documenté, pas bloquant.
    // Régénérer ces vecteurs : petit test `dump` appelant les deux encodeurs (cf. historique).
    const SHARD_ASSIGNMENT_G1_OVL02: &[u8] = &[
        0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0c, 0x00, 0x07, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x07, 0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0c, 0x00, 0x04, 0x00,
        0x08, 0x00, 0x08, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02,
        0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00,
        0x67, 0x72, 0x6f, 0x75, 0x70, 0x2d, 0x32, 0x00, 0x07, 0x00, 0x00, 0x00, 0x67, 0x72, 0x6f,
        0x75, 0x70, 0x2d, 0x30, 0x00, 0x07, 0x00, 0x00, 0x00, 0x67, 0x72, 0x6f, 0x75, 0x70, 0x2d,
        0x31, 0x00,
    ];

    // PositionCorrection{ position=(100.5, -20.25, 3.0) m, yaw=90.0°, reason=0 } — layout v2
    // (gel palier 2) : QVec3 fixed-point (100.5 m = 0x00C90000, -20.25 = 0xFFD78000,
    // 3.0 = 0x00060000, little-endian) + yaw ushort (90° = 0x4000 = 16384 crans). Régénéré le
    // 2026-07-23 depuis l'encodeur Rust après migration (vérifié à la main octet par octet).
    const POSITION_CORRECTION_SPAWN: &[u8] = &[
        0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x0c, 0x00, 0x07, 0x00, 0x08, 0x00, 0x08, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x06, 0x0c, 0x00, 0x00, 0x00, 0x08, 0x00, 0x14, 0x00, 0x08, 0x00,
        0x06, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0xc9, 0x00, 0x00,
        0x80, 0xd7, 0xff, 0x00, 0x00, 0x06, 0x00,
    ];

    #[test]
    fn shard_assignment_wire_is_stable_and_matches_the_shared_vector() {
        // (1) L'encodeur serveur produit EXACTEMENT le vecteur partagé (tripwire de dérive de
        // layout — le test C++ du fork fige les mêmes octets).
        assert_eq!(
            crate::gateway::encode_shard_assignment(
                "group-1",
                &["group-0".to_string(), "group-2".to_string()]
            ),
            SHARD_ASSIGNMENT_G1_OVL02,
            "layout ShardAssignment modifié : régénérer ce vecteur (test dump) et revalider le décodage en jeu"
        );
        // (2) Auto-cohérence : les octets se décodent aux bons champs.
        let env = flatbuffers::root::<protocol::ServerEnvelope>(SHARD_ASSIGNMENT_G1_OVL02).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::ShardAssignment);
        let sa = env.msg_as_shard_assignment().unwrap();
        assert_eq!(sa.authoritative(), Some("group-1"));
        let overlaps: Vec<&str> = sa.overlaps().unwrap().iter().collect();
        assert_eq!(overlaps, vec!["group-0", "group-2"]);
    }

    #[test]
    fn position_correction_wire_is_stable_and_matches_the_shared_vector() {
        assert_eq!(
            crate::gateway::encode_position_correction([100.5, -20.25, 3.0], 90.0, 0),
            POSITION_CORRECTION_SPAWN,
            "layout PositionCorrection modifié : régénérer ce vecteur (test dump) et revalider le décodage en jeu"
        );
        let env = flatbuffers::root::<protocol::ServerEnvelope>(POSITION_CORRECTION_SPAWN).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::PositionCorrection);
        let pc = env.msg_as_position_correction().unwrap();
        let pos = pc.position().unwrap();
        // 100.5/-20.25/3.0 m et 90° sont représentables exactement sur la grille (quant.rs) —
        // la déquantization redonne les valeurs bit-à-bit.
        assert_eq!(
            (dq_pos(pos.x()), dq_pos(pos.y()), dq_pos(pos.z())),
            (100.5, -20.25, 3.0)
        );
        assert_eq!(dq_yaw(pc.yaw()), 90.0);
        assert_eq!(pc.reason(), 0);
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
    fn extract_entity_interaction_reads_target_kind_and_param() {
        let mut b = FlatBufferBuilder::new();
        let ei = EntityInteraction::create(
            &mut b,
            &EntityInteractionArgs {
                target: 42,
                kind: 1,
                param: 7,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::EntityInteraction,
                msg: Some(ei.as_union_value()),
            },
        );
        b.finish(env, None);
        assert_eq!(
            extract_entity_interaction(b.finished_data()),
            Some((42, 1, 7))
        );
        assert_eq!(extract_entity_interaction(&client_join()), None); // pas un EntityInteraction
        assert_eq!(extract_entity_interaction(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_interaction_choice_reads_session_id_choice_and_param() {
        let mut b = FlatBufferBuilder::new();
        let ic = InteractionChoice::create(
            &mut b,
            &InteractionChoiceArgs {
                session_id: 99,
                choice: 2,
                param: 5,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::InteractionChoice,
                msg: Some(ic.as_union_value()),
            },
        );
        b.finish(env, None);
        assert_eq!(
            extract_interaction_choice(b.finished_data()),
            Some((99, 2, 5))
        );
        assert_eq!(extract_interaction_choice(&client_join()), None); // pas un InteractionChoice
        assert_eq!(extract_interaction_choice(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_create_character_reads_the_pseudonym() {
        let mut b = FlatBufferBuilder::new();
        let p = b.create_string("Nyx");
        let cmd = CreateCharacter::create(&mut b, &CreateCharacterArgs { pseudonym: Some(p) });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::CreateCharacter,
                msg: Some(cmd.as_union_value()),
            },
        );
        b.finish(env, None);
        assert_eq!(
            extract_create_character(b.finished_data()),
            Some("Nyx".to_string())
        );
        assert_eq!(extract_create_character(&client_join()), None); // pas un CreateCharacter
        assert_eq!(extract_create_character(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_select_character_reads_the_id() {
        let mut b = FlatBufferBuilder::new();
        let cmd = SelectCharacter::create(&mut b, &SelectCharacterArgs { id: 42 });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::SelectCharacter,
                msg: Some(cmd.as_union_value()),
            },
        );
        b.finish(env, None);
        assert_eq!(extract_select_character(b.finished_data()), Some(42));
        assert_eq!(extract_select_character(&client_join()), None); // pas un SelectCharacter
        assert_eq!(extract_select_character(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_delete_character_reads_the_id() {
        let mut b = FlatBufferBuilder::new();
        let cmd = DeleteCharacter::create(&mut b, &DeleteCharacterArgs { id: 7 });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::DeleteCharacter,
                msg: Some(cmd.as_union_value()),
            },
        );
        b.finish(env, None);
        assert_eq!(extract_delete_character(b.finished_data()), Some(7));
        assert_eq!(extract_delete_character(&client_join()), None); // pas un DeleteCharacter
        assert_eq!(extract_delete_character(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn extract_elevator_call_reads_both_fields() {
        let mut b = FlatBufferBuilder::new();
        let call = ElevatorCall::create(
            &mut b,
            &ElevatorCallArgs {
                elevator_id: 9939278384122899325,
                floor: 3,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::ElevatorCall,
                msg: Some(call.as_union_value()),
            },
        );
        b.finish(env, None);
        assert_eq!(
            extract_elevator_call(b.finished_data()),
            Some((9939278384122899325, 3))
        );
        assert_eq!(extract_elevator_call(&client_join()), None); // pas un ElevatorCall
        assert_eq!(extract_elevator_call(&[9, 9, 9]), None); // garbage
    }

    #[test]
    fn encode_character_result_roundtrips() {
        let bytes = encode_character_result(false, "pseudonym_taken");
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::CharacterResult);
        let res = env.msg_as_character_result().unwrap();
        assert!(!res.success());
        assert_eq!(res.reason(), Some("pseudonym_taken"));
    }

    #[test]
    fn encode_interaction_open_roundtrips() {
        let bytes = encode_interaction_open(7, 42, 1, &[1, 2, 3]);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::InteractionOpen);
        let open = env.msg_as_interaction_open().unwrap();
        assert_eq!(open.session_id(), 7);
        assert_eq!(open.target(), 42);
        assert_eq!(open.ui_kind(), 1);
        assert_eq!(open.payload().unwrap().bytes(), &[1, 2, 3]);
    }

    #[test]
    fn encode_interaction_result_roundtrips() {
        let bytes = encode_interaction_result(7, false, &[9]);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::InteractionResult);
        let res = env.msg_as_interaction_result().unwrap();
        assert_eq!(res.session_id(), 7);
        assert!(!res.ok());
        assert_eq!(res.payload().unwrap().bytes(), &[9]);
    }

    #[test]
    fn encode_character_list_roundtrips_multiple_summaries() {
        let bytes = encode_character_list(&[(1, "Nyx".to_string()), (2, "Vex".to_string())]);
        let env = flatbuffers::root::<protocol::ServerEnvelope>(&bytes).unwrap();
        assert_eq!(env.msg_type(), protocol::ServerMsg::CharacterList);
        let list = env.msg_as_character_list().unwrap();
        let chars = list.characters().unwrap();
        assert_eq!(chars.len(), 2);
        assert_eq!(chars.get(0).id(), 1);
        assert_eq!(chars.get(0).pseudonym(), Some("Nyx"));
        assert_eq!(chars.get(1).id(), 2);
        assert_eq!(chars.get(1).pseudonym(), Some("Vex"));
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
