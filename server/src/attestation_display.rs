//! Décode le payload d'un JWT d'attestation POUR AFFICHAGE (bannière de boot) uniquement — AUCUNE
//! vérification de signature ici (c'est `directory` qui vérifie). Ne jamais s'en servir pour une
//! décision de confiance.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

/// Renvoie (sub, exp) si le payload est décodable. `None` sinon (jamais de panique).
pub fn describe(jwt: &str) -> Option<(String, u64)> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some((v.get("sub")?.as_str()?.to_string(), v.get("exp")?.as_u64()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn describe_reads_sub_and_exp_from_wire_token() {
        // token du fil (fixtures directory) : sub="wire-fixture-slug".
        let jwt = include_str!("../../directory/tests/fixtures/wire-token.jwt").trim();
        let (sub, exp) = describe(jwt).unwrap();
        assert_eq!(sub, "wire-fixture-slug");
        assert!(exp > 0);
    }
    #[test]
    fn describe_returns_none_on_garbage() {
        assert!(describe("pas-un-jwt").is_none());
    }
}
