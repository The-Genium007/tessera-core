//! Framing TCP : chaque message interne est préfixé de sa longueur (u32 big-endian).

/// Taille maximale acceptée pour un frame interne (Gateway↔Shard), en octets — marge généreuse
/// par rapport au plus gros message du protocole. Un frame annoncé au-delà doit faire fermer la
/// connexion fautive plutôt que d'accumuler indéfiniment en attendant un corps qui ne viendra
/// jamais complet.
pub const MAX_FRAME_LEN: usize = 1 << 20; // 1 MiB

/// Préfixe `payload` de sa longueur (u32 BE) pour l'écrire sur un flux TCP.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Accumule des octets TCP et rend les payloads complets un par un.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        if self.buf.len() < 4 {
            return None;
        }
        let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if self.buf.len() < 4 + len {
            return None;
        }
        let payload = self.buf[4..4 + len].to_vec();
        self.buf.drain(..4 + len);
        Some(payload)
    }

    /// Vrai si la longueur annoncée du frame en cours d'assemblage dépasse `max`. Ne nécessite
    /// que le préfixe de 4 octets (pas le corps complet) — permet à l'appelant de refuser un
    /// frame malveillant/buggé avant d'accumuler son corps en mémoire.
    pub fn declared_len_exceeds(&self, max: usize) -> bool {
        if self.buf.len() < 4 {
            return false;
        }
        let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        len > max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_one_frame() {
        let framed = encode_frame(&[10, 20, 30]);
        let mut r = FrameReader::new();
        r.push(&framed);
        assert_eq!(r.next_frame(), Some(vec![10, 20, 30]));
        assert_eq!(r.next_frame(), None);
    }

    #[test]
    fn reassembles_across_partial_pushes_and_splits_multiple() {
        let a = encode_frame(b"AA");
        let b = encode_frame(b"BBB");
        let mut r = FrameReader::new();
        // pousser octet par octet le 1er frame + le début du 2e
        for byte in a.iter().chain(b.iter()) {
            r.push(&[*byte]);
        }
        assert_eq!(r.next_frame(), Some(b"AA".to_vec()));
        assert_eq!(r.next_frame(), Some(b"BBB".to_vec()));
        assert_eq!(r.next_frame(), None);
    }

    #[test]
    fn declared_len_exceeds_is_false_when_not_enough_bytes_yet() {
        let r = FrameReader::new();
        assert!(!r.declared_len_exceeds(1024));
    }

    #[test]
    fn declared_len_exceeds_detects_an_oversized_prefix_before_the_body_arrives() {
        let mut r = FrameReader::new();
        // Annonce une longueur au-delà de la limite — seul le préfixe de 4 octets est poussé,
        // pas le corps : c'est justement le point, détecter SANS accumuler le corps.
        let huge_len: u32 = MAX_FRAME_LEN as u32 + 1;
        r.push(&huge_len.to_be_bytes());
        assert!(r.declared_len_exceeds(MAX_FRAME_LEN));
        assert_eq!(r.next_frame(), None, "pas assez de données pour un frame complet");
    }

    #[test]
    fn declared_len_exceeds_is_false_exactly_at_the_limit() {
        let mut r = FrameReader::new();
        let len: u32 = MAX_FRAME_LEN as u32;
        r.push(&len.to_be_bytes());
        assert!(!r.declared_len_exceeds(MAX_FRAME_LEN));
    }

    #[test]
    fn frames_under_the_limit_still_round_trip_normally() {
        let framed = encode_frame(&[10, 20, 30]);
        let mut r = FrameReader::new();
        r.push(&framed);
        assert!(!r.declared_len_exceeds(MAX_FRAME_LEN));
        assert_eq!(r.next_frame(), Some(vec![10, 20, 30]));
    }
}
