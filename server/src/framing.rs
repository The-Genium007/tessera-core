//! Framing TCP : chaque message interne est préfixé de sa longueur (u32 big-endian).

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
}
