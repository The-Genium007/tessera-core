//! Couture de transport (poll-based, calquée sur GameNetworkingSockets).
//! Le serveur est générique sur ce trait ; on le teste avec `InMemoryTransport`.

use std::collections::{HashMap, VecDeque};

pub type ClientId = u64;

#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    Connected(ClientId),
    Disconnected(ClientId),
    Message { from: ClientId, data: Vec<u8> },
}

/// Transport réseau abstrait. `poll` draine les events en attente (non bloquant) ;
/// `send` envoie des octets de façon fiable à un client.
pub trait Transport {
    fn poll(&mut self) -> Vec<TransportEvent>;
    fn send(&mut self, to: ClientId, data: &[u8]);
}

/// Transport déterministe en mémoire, pour les tests (aucun réseau).
#[derive(Default)]
pub struct InMemoryTransport {
    incoming: VecDeque<TransportEvent>,
    sent: HashMap<ClientId, Vec<Vec<u8>>>,
}

impl InMemoryTransport {
    pub fn new() -> Self {
        Self::default()
    }
    /// Simule l'arrivée d'un event côté serveur (depuis un test).
    pub fn inject(&mut self, ev: TransportEvent) {
        self.incoming.push_back(ev);
    }
    /// Récupère (et vide) les messages que le serveur a envoyés à `to`, pour assertions.
    pub fn take_sent(&mut self, to: ClientId) -> Vec<Vec<u8>> {
        self.sent.remove(&to).unwrap_or_default()
    }
}

impl Transport for InMemoryTransport {
    fn poll(&mut self) -> Vec<TransportEvent> {
        self.incoming.drain(..).collect()
    }
    fn send(&mut self, to: ClientId, data: &[u8]) {
        self.sent.entry(to).or_default().push(data.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_transport_round_trips_events_and_sends() {
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: vec![9, 9],
        });

        let events = t.poll();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], TransportEvent::Connected(1));
        assert!(t.poll().is_empty(), "poll doit vider la file");

        t.send(1, &[7, 7]);
        assert_eq!(t.take_sent(1), vec![vec![7, 7]]);
        assert!(t.take_sent(1).is_empty(), "take_sent doit vider");
    }
}
