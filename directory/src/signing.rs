//! Signature Ed25519 **détachée** sur octets bruts, indépendante de `tools/release` (clé et
//! usage différents — cf. ADR 0006 pour le contrat octet-pour-octet).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

pub fn signing_key_from_b64_seed(seed_b64: &str) -> Result<SigningKey, String> {
    let raw = B64
        .decode(seed_b64.trim())
        .map_err(|e| format!("seed base64 invalide: {e}"))?;
    let seed: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| format!("le seed doit faire 32 octets, reçu {}", raw.len()))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn verifying_key_from_b64(pub_b64: &str) -> Result<VerifyingKey, String> {
    let raw = B64
        .decode(pub_b64.trim())
        .map_err(|e| format!("pubkey base64 invalide: {e}"))?;
    let key: [u8; 32] = raw
        .as_slice()
        .try_into()
        .map_err(|_| format!("la pubkey doit faire 32 octets, reçu {}", raw.len()))?;
    VerifyingKey::from_bytes(&key).map_err(|e| format!("pubkey ed25519 invalide: {e}"))
}

pub fn public_b64(key: &SigningKey) -> String {
    B64.encode(key.verifying_key().to_bytes())
}

pub fn sign_detached_b64(key: &SigningKey, message: &[u8]) -> String {
    B64.encode(key.sign(message).to_bytes())
}

pub fn verify_detached_b64(
    key: &VerifyingKey,
    message: &[u8],
    sig_b64: &str,
) -> Result<(), String> {
    let raw = B64
        .decode(sig_b64.trim())
        .map_err(|e| format!("signature base64 invalide: {e}"))?;
    let bytes: [u8; 64] = raw
        .as_slice()
        .try_into()
        .map_err(|_| format!("la signature doit faire 64 octets, reçu {}", raw.len()))?;
    key.verify_strict(message, &Signature::from_bytes(&bytes))
        .map_err(|e| format!("signature invalide: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Copié depuis tools/release/src/signing.rs (même vecteur de test, clé jetable hors-prod
    // générée via OpenSSL) — prouve l'interopérabilité octet-pour-octet entre les deux crates.
    const TEST_SEED: &str = "ghSvoGCgqRCrzJuGqFKnG1g55jjmH5lrEX7neX7vfag=";
    const TEST_PUB: &str = "Xb4m8qh/yoACil6zvR3npGOJppaYjPxrEuhp5r74dGg=";
    const TEST_MSG: &[u8] = b"tessera-release test vector v1";
    const TEST_SIG: &str =
        "gEPVeiXXvex2FcJVZ1FtTSbYLVQV9VWxcrosCSe0rV0Gjbk/MX5729U5Fw0lm4/r3BaWUhpA8sfNlg1ZtPf/Aw==";

    #[test]
    fn pubkey_derivation_matches_reference() {
        let key = signing_key_from_b64_seed(TEST_SEED).unwrap();
        assert_eq!(public_b64(&key), TEST_PUB);
    }

    #[test]
    fn signing_is_deterministic_and_matches_reference() {
        let key = signing_key_from_b64_seed(TEST_SEED).unwrap();
        assert_eq!(sign_detached_b64(&key, TEST_MSG), TEST_SIG);
    }

    #[test]
    fn verify_accepts_valid_signature() {
        let pk = verifying_key_from_b64(TEST_PUB).unwrap();
        assert!(verify_detached_b64(&pk, TEST_MSG, TEST_SIG).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_message() {
        let pk = verifying_key_from_b64(TEST_PUB).unwrap();
        assert!(verify_detached_b64(&pk, b"tampered", TEST_SIG).is_err());
    }

    #[test]
    fn roundtrip_sign_then_verify() {
        let key = signing_key_from_b64_seed(TEST_SEED).unwrap();
        let pk = key.verifying_key();
        let msg = b"un manifeste arbitraire {\"x\":1}";
        let sig = sign_detached_b64(&key, msg);
        assert!(verify_detached_b64(&pk, msg, &sig).is_ok());
    }

    #[test]
    fn rejects_wrong_length_seed() {
        assert!(signing_key_from_b64_seed("aGVsbG8=").is_err());
    }
}
