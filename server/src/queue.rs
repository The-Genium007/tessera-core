//! File d'attente pure — priorité par rôle, slots réservés, détecteur AFK. Aucune I/O, aucune
//! dépendance réseau : testable en isolation complète.

use std::collections::VecDeque;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Public = 0,
    Whitelist = 1,
    Premium = 2,
    Staff = 3,
}

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub client_id: u64,
    pub priority: Priority,
    pub joined_at: Instant,
}

#[derive(Default)]
pub struct PriorityQueue {
    entries: VecDeque<QueueEntry>,
}

impl PriorityQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insère en respectant l'ordre de priorité (plus haute priorité d'abord), FIFO au sein
    /// d'une même priorité.
    pub fn enqueue(&mut self, client_id: u64, priority: Priority) {
        let pos = self
            .entries
            .iter()
            .position(|e| e.priority < priority)
            .unwrap_or(self.entries.len());
        self.entries.insert(
            pos,
            QueueEntry {
                client_id,
                priority,
                joined_at: Instant::now(),
            },
        );
    }

    /// Position 1-indexée du client dans la file (None s'il n'y est pas).
    pub fn position_of(&self, client_id: u64) -> Option<u32> {
        self.entries
            .iter()
            .position(|e| e.client_id == client_id)
            .map(|i| (i + 1) as u32)
    }

    /// Retire et renvoie le prochain client à admettre (tête de file).
    pub fn dequeue_next(&mut self) -> Option<u64> {
        self.entries.pop_front().map(|e| e.client_id)
    }

    /// Retire un client de la file (départ volontaire via Leave, ou timeout AFK).
    pub fn remove(&mut self, client_id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.client_id != client_id);
        self.entries.len() != before
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl From<crate::handoff::Rank> for Priority {
    fn from(rank: crate::handoff::Rank) -> Self {
        match rank {
            crate::handoff::Rank::GameMaster => Priority::Staff,
            crate::handoff::Rank::Moderator => Priority::Staff,
            crate::handoff::Rank::Player => Priority::Public,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_orders_by_priority_staff_first() {
        let mut q = PriorityQueue::new();
        q.enqueue(1, Priority::Public);
        q.enqueue(2, Priority::Staff);
        q.enqueue(3, Priority::Premium);
        assert_eq!(
            q.dequeue_next(),
            Some(2),
            "staff en tête malgré l'ordre d'arrivée"
        );
        assert_eq!(q.dequeue_next(), Some(3));
        assert_eq!(q.dequeue_next(), Some(1));
    }

    #[test]
    fn enqueue_is_fifo_within_the_same_priority() {
        let mut q = PriorityQueue::new();
        q.enqueue(1, Priority::Public);
        q.enqueue(2, Priority::Public);
        assert_eq!(q.dequeue_next(), Some(1));
        assert_eq!(q.dequeue_next(), Some(2));
    }

    #[test]
    fn position_of_reflects_priority_ordering_not_arrival_order() {
        let mut q = PriorityQueue::new();
        q.enqueue(1, Priority::Public);
        q.enqueue(2, Priority::Staff);
        assert_eq!(
            q.position_of(2),
            Some(1),
            "le staff arrivé après doit être en position 1"
        );
        assert_eq!(q.position_of(1), Some(2));
    }

    #[test]
    fn position_of_returns_none_for_unknown_client() {
        let q = PriorityQueue::new();
        assert_eq!(q.position_of(999), None);
    }

    #[test]
    fn remove_takes_a_client_out_of_the_queue() {
        let mut q = PriorityQueue::new();
        q.enqueue(1, Priority::Public);
        q.enqueue(2, Priority::Public);
        assert!(q.remove(1));
        assert_eq!(q.position_of(1), None);
        assert_eq!(q.position_of(2), Some(1));
    }

    #[test]
    fn remove_returns_false_when_client_not_in_queue() {
        let mut q = PriorityQueue::new();
        assert!(!q.remove(999));
    }

    #[test]
    fn dequeue_next_on_empty_queue_returns_none() {
        let mut q = PriorityQueue::new();
        assert_eq!(q.dequeue_next(), None);
    }

    #[test]
    fn game_master_and_moderator_both_map_to_staff_priority() {
        use crate::handoff::Rank;
        assert_eq!(Priority::from(Rank::GameMaster), Priority::Staff);
        assert_eq!(Priority::from(Rank::Moderator), Priority::Staff);
    }

    #[test]
    fn player_maps_to_public_priority() {
        use crate::handoff::Rank;
        assert_eq!(Priority::from(Rank::Player), Priority::Public);
    }
}
