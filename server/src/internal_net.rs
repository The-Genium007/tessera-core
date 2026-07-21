//! Transport interne : (dé)sérialise les messages internes et présente un `Transport`
//! au Shard. Le Gateway alimente `feed(octets bruts)` et consomme `take_outbound()`.

use crate::framing::{encode_frame, FrameReader};
use crate::transport::{ClientId, Transport, TransportEvent};
use flatbuffers::FlatBufferBuilder;
use protocol::internal::{
    ClientEvent, ClientEventArgs, EntityPositionReport, EntityPositionReportArgs, EventKind,
    InternalEnvelope, InternalEnvelopeArgs, InternalMsg, RouteReply, RouteReplyArgs, RouteRequest,
    RouteRequestArgs, ServerSend, ServerSendArgs,
};
use std::collections::VecDeque;

/// Encode un `ClientEvent` (Gateway → Shard) dans un `InternalEnvelope` framé.
pub fn encode_client_event(kind: EventKind, client_id: u64, payload: &[u8]) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let pl = b.create_vector(payload);
    let ce = ClientEvent::create(
        &mut b,
        &ClientEventArgs {
            kind,
            client_id,
            payload: Some(pl),
        },
    );
    let env = InternalEnvelope::create(
        &mut b,
        &InternalEnvelopeArgs {
            msg_type: InternalMsg::ClientEvent,
            msg: Some(ce.as_union_value()),
        },
    );
    b.finish(env, None);
    encode_frame(b.finished_data())
}

/// Décode un frame `InternalEnvelope`/`ClientEvent` (déjà déframé) en `TransportEvent`.
pub fn decode_client_event(body: &[u8]) -> Option<TransportEvent> {
    let env = flatbuffers::root::<InternalEnvelope>(body).ok()?;
    let ce = env.msg_as_client_event()?;
    let id = ce.client_id();
    match ce.kind() {
        EventKind::Connected => Some(TransportEvent::Connected(id)),
        EventKind::Disconnected => Some(TransportEvent::Disconnected(id)),
        EventKind::Message => {
            let data = ce.payload().map(|p| p.bytes().to_vec()).unwrap_or_default();
            Some(TransportEvent::Message { from: id, data })
        }
        _ => None,
    }
}

/// Traduit un `TransportEvent` (côté client) en `ClientEvent` framé (Gateway → Shard).
pub fn event_to_client_event_frame(ev: &TransportEvent) -> Vec<u8> {
    match ev {
        TransportEvent::Connected(id) => encode_client_event(EventKind::Connected, *id, &[]),
        TransportEvent::Disconnected(id) => encode_client_event(EventKind::Disconnected, *id, &[]),
        TransportEvent::Message { from, data } => {
            encode_client_event(EventKind::Message, *from, data)
        }
    }
}

/// Décode le corps déframé d'un `ServerSend` (Shard → Gateway) en (client, payload).
pub fn decode_server_send(body: &[u8]) -> Option<(ClientId, Vec<u8>)> {
    let env = flatbuffers::root::<InternalEnvelope>(body).ok()?;
    let ss = env.msg_as_server_send()?;
    let payload = ss.payload().map(|p| p.bytes().to_vec()).unwrap_or_default();
    Some((ss.client_id(), payload))
}

/// `RouteRequest` framé (Gateway → Router).
pub fn encode_route_request(client_id: u64, x: f32, y: f32, z: f32) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let rr = RouteRequest::create(&mut b, &RouteRequestArgs { client_id, x, y, z });
    let env = InternalEnvelope::create(
        &mut b,
        &InternalEnvelopeArgs {
            msg_type: InternalMsg::RouteRequest,
            msg: Some(rr.as_union_value()),
        },
    );
    b.finish(env, None);
    encode_frame(b.finished_data())
}

pub fn decode_route_request(body: &[u8]) -> Option<(u64, f32, f32, f32)> {
    let env = flatbuffers::root::<InternalEnvelope>(body).ok()?;
    let rr = env.msg_as_route_request()?;
    Some((rr.client_id(), rr.x(), rr.y(), rr.z()))
}

/// `RouteReply` framé (Router → Gateway).
pub fn encode_route_reply(shard_addr: &str) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let addr = b.create_string(shard_addr);
    let rr = RouteReply::create(
        &mut b,
        &RouteReplyArgs {
            shard_addr: Some(addr),
        },
    );
    let env = InternalEnvelope::create(
        &mut b,
        &InternalEnvelopeArgs {
            msg_type: InternalMsg::RouteReply,
            msg: Some(rr.as_union_value()),
        },
    );
    b.finish(env, None);
    encode_frame(b.finished_data())
}

pub fn decode_route_reply(body: &[u8]) -> Option<String> {
    let env = flatbuffers::root::<InternalEnvelope>(body).ok()?;
    let rr = env.msg_as_route_reply()?;
    Some(rr.shard_addr()?.to_string())
}

/// `EntityPositionReport` framé (Shard -> Gateway). Primitive générique de pont cross-shard pour
/// toute entité simulée côté Shard — voir `shard_boundary_bridge.rs` pour l'utilisation.
pub fn encode_entity_position_report(
    entity_id: u64,
    x: f32,
    y: f32,
    z: f32,
    speed: f32,
) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let epr = EntityPositionReport::create(
        &mut b,
        &EntityPositionReportArgs {
            entity_id,
            x,
            y,
            z,
            speed,
        },
    );
    let env = InternalEnvelope::create(
        &mut b,
        &InternalEnvelopeArgs {
            msg_type: InternalMsg::EntityPositionReport,
            msg: Some(epr.as_union_value()),
        },
    );
    b.finish(env, None);
    encode_frame(b.finished_data())
}

pub fn decode_entity_position_report(body: &[u8]) -> Option<(u64, f32, f32, f32, f32)> {
    let env = flatbuffers::root::<InternalEnvelope>(body).ok()?;
    let epr = env.msg_as_entity_position_report()?;
    Some((epr.entity_id(), epr.x(), epr.y(), epr.z(), epr.speed()))
}

/// Transport branché sur le lien interne TCP. Le Shard l'utilise via `Server::tick`.
/// `feed` reçoit les octets BRUTS du socket (frames length-préfixés, éventuellement
/// partiels) ; le `FrameReader` interne est persistant → un frame coupé entre deux
/// appels `feed` est correctement réassemblé.
#[derive(Default)]
pub struct InternalTransport {
    reader: FrameReader,
    inbound: VecDeque<TransportEvent>,
    outbound: Vec<Vec<u8>>, // frames ServerSend prêts pour le socket
}

impl InternalTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pousse les octets bruts reçus du socket. Renvoie `false` si le frame en cours annonce
    /// une longueur au-delà de `framing::MAX_FRAME_LEN` — l'appelant doit alors fermer la
    /// connexion plutôt que de continuer à lire (frame malveillant/buggé, jamais complet).
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.reader.push(bytes);
        if self
            .reader
            .declared_len_exceeds(crate::framing::MAX_FRAME_LEN)
        {
            return false;
        }
        while let Some(body) = self.reader.next_frame() {
            if let Some(ev) = decode_client_event(&body) {
                self.inbound.push_back(ev);
            }
        }
        true
    }

    /// Récupère (et vide) les frames sortants à écrire sur le socket.
    pub fn take_outbound(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outbound)
    }
}

impl Transport for InternalTransport {
    fn poll(&mut self) -> Vec<TransportEvent> {
        self.inbound.drain(..).collect()
    }

    fn send(&mut self, to: ClientId, data: &[u8]) {
        let mut b = FlatBufferBuilder::new();
        let pl = b.create_vector(data);
        let ss = ServerSend::create(
            &mut b,
            &ServerSendArgs {
                client_id: to,
                payload: Some(pl),
            },
        );
        let env = InternalEnvelope::create(
            &mut b,
            &InternalEnvelopeArgs {
                msg_type: InternalMsg::ServerSend,
                msg: Some(ss.as_union_value()),
            },
        );
        b.finish(env, None);
        self.outbound.push(encode_frame(b.finished_data()));
    }

    /// No-op : `InternalTransport` est le lien interne Gateway↔Shard, pas la connexion
    /// publique d'un client. Kicker un client est une décision du Gateway (serveur plein,
    /// flood), qui agit sur son propre transport client-facing (`GnsTransport`), jamais ici.
    fn disconnect(&mut self, _to: ClientId) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::FrameReader;
    use crate::transport::{Transport, TransportEvent};
    use protocol::internal::EventKind;

    #[test]
    fn feed_decodes_events_and_send_produces_framed_serversend() {
        let mut t = InternalTransport::new();

        // Le Gateway relaie : client 1 se connecte, puis envoie des octets [7,7].
        t.feed(&encode_client_event(EventKind::Connected, 1, &[]));
        t.feed(&encode_client_event(EventKind::Message, 1, &[7, 7]));

        let evs = t.poll();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0], TransportEvent::Connected(1));
        assert_eq!(
            evs[1],
            TransportEvent::Message {
                from: 1,
                data: vec![7, 7]
            }
        );
        assert!(t.poll().is_empty(), "poll doit vider la file");

        // Le Shard envoie un snapshot [9,9] au client 1.
        t.send(1, &[9, 9]);
        let out = t.take_outbound();
        assert_eq!(out.len(), 1);

        // Le frame sortant est un ServerSend{client_id:1, payload:[9,9]}, framé.
        let mut r = FrameReader::new();
        r.push(&out[0]);
        let body = r.next_frame().expect("un frame complet");
        let env = flatbuffers::root::<protocol::internal::InternalEnvelope>(&body).unwrap();
        let ss = env.msg_as_server_send().unwrap();
        assert_eq!(ss.client_id(), 1);
        assert_eq!(ss.payload().unwrap().bytes(), &[9, 9]);
        assert!(t.take_outbound().is_empty(), "take_outbound doit vider");
    }

    #[test]
    fn feed_reassembles_a_frame_split_across_two_calls() {
        let mut t = InternalTransport::new();
        let framed = encode_client_event(EventKind::Message, 7, &[1, 2, 3]);
        let mid = framed.len() / 2;
        // Le frame arrive en deux morceaux (lecture TCP partielle).
        t.feed(&framed[..mid]);
        assert!(t.poll().is_empty(), "frame incomplet : aucun event encore");
        t.feed(&framed[mid..]);
        let evs = t.poll();
        assert_eq!(evs.len(), 1);
        assert_eq!(
            evs[0],
            TransportEvent::Message {
                from: 7,
                data: vec![1, 2, 3]
            }
        );
    }

    #[test]
    fn event_to_client_event_frame_round_trips_via_decode() {
        use crate::framing::FrameReader;
        // Message
        let framed = event_to_client_event_frame(&TransportEvent::Message {
            from: 3,
            data: vec![8, 9],
        });
        let mut r = FrameReader::new();
        r.push(&framed);
        let body = r.next_frame().unwrap();
        assert_eq!(
            decode_client_event(&body),
            Some(TransportEvent::Message {
                from: 3,
                data: vec![8, 9]
            })
        );
        // Connected
        let framed = event_to_client_event_frame(&TransportEvent::Connected(7));
        let mut r = FrameReader::new();
        r.push(&framed);
        assert_eq!(
            decode_client_event(&r.next_frame().unwrap()),
            Some(TransportEvent::Connected(7))
        );
    }

    #[test]
    fn decode_server_send_extracts_client_and_payload() {
        let mut t = InternalTransport::new();
        t.send(5, &[1, 2, 3]); // produit un ServerSend framé dans outbound
        let framed = t.take_outbound().remove(0);
        let mut r = crate::framing::FrameReader::new();
        r.push(&framed);
        let body = r.next_frame().unwrap();
        assert_eq!(decode_server_send(&body), Some((5, vec![1, 2, 3])));
    }

    #[test]
    fn route_request_and_reply_round_trip() {
        use crate::framing::FrameReader;
        let framed = encode_route_request(11, 1.0, 2.0, 3.0);
        let mut r = FrameReader::new();
        r.push(&framed);
        assert_eq!(
            decode_route_request(&r.next_frame().unwrap()),
            Some((11, 1.0, 2.0, 3.0))
        );

        let framed = encode_route_reply("127.0.0.1:27030");
        let mut r = FrameReader::new();
        r.push(&framed);
        assert_eq!(
            decode_route_reply(&r.next_frame().unwrap()),
            Some("127.0.0.1:27030".to_string())
        );
    }

    #[test]
    fn entity_position_report_round_trip() {
        use crate::framing::FrameReader;
        let framed = encode_entity_position_report(42, 1.0, 2.0, 3.0, 8.5);
        let mut r = FrameReader::new();
        r.push(&framed);
        assert_eq!(
            decode_entity_position_report(&r.next_frame().unwrap()),
            Some((42, 1.0, 2.0, 3.0, 8.5))
        );
    }
}
