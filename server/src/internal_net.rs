//! Transport interne : (dé)sérialise les messages internes et présente un `Transport`
//! au Shard. Le Gateway alimente `feed(octets bruts)` et consomme `take_outbound()`.

use crate::framing::{encode_frame, FrameReader};
use crate::transport::{ClientId, Transport, TransportEvent};
use flatbuffers::FlatBufferBuilder;
use protocol::internal::{
    ClientEvent, ClientEventArgs, EventKind, InternalEnvelope, InternalEnvelopeArgs, InternalMsg,
    ServerSend, ServerSendArgs,
};
use std::collections::VecDeque;

/// Encode un `ClientEvent` (Gateway → Shard) dans un `InternalEnvelope` framé.
pub fn encode_client_event(kind: EventKind, client_id: u64, payload: &[u8]) -> Vec<u8> {
    let mut b = FlatBufferBuilder::new();
    let pl = b.create_vector(payload);
    let ce = ClientEvent::create(
        &mut b,
        &ClientEventArgs { kind, client_id, payload: Some(pl) },
    );
    let env = InternalEnvelope::create(
        &mut b,
        &InternalEnvelopeArgs { msg_type: InternalMsg::ClientEvent, msg: Some(ce.as_union_value()) },
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

    /// Pousse les octets bruts reçus du socket (0, 1 ou plusieurs frames, et/ou un
    /// frame partiel conservé pour le prochain appel).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.reader.push(bytes);
        while let Some(body) = self.reader.next_frame() {
            if let Some(ev) = decode_client_event(&body) {
                self.inbound.push_back(ev);
            }
        }
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
        let ss = ServerSend::create(&mut b, &ServerSendArgs { client_id: to, payload: Some(pl) });
        let env = InternalEnvelope::create(
            &mut b,
            &InternalEnvelopeArgs { msg_type: InternalMsg::ServerSend, msg: Some(ss.as_union_value()) },
        );
        b.finish(env, None);
        self.outbound.push(encode_frame(b.finished_data()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Transport, TransportEvent};
    use crate::framing::FrameReader;
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
        assert_eq!(evs[1], TransportEvent::Message { from: 1, data: vec![7, 7] });
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
        assert_eq!(evs[0], TransportEvent::Message { from: 7, data: vec![1, 2, 3] });
    }
}
