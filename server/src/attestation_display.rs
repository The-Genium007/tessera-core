//! Décode le payload d'un JWT d'attestation POUR AFFICHAGE (bannière de boot) uniquement — AUCUNE
//! vérification de signature ici (c'est `directory` qui vérifie). Ne jamais s'en servir pour une
//! décision de confiance.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

/// Cause d'échec de `describe` — distinguée pour un diagnostic de boot précis (playtest
/// 2026-07-17 : `TESSERA_OFFICIAL_ATTESTATION présente mais illisible` ne disait pas POURQUOI).
#[derive(Debug, PartialEq, Eq)]
pub enum DescribeError {
    /// Pas un JWT à 3 segments séparés par `.`.
    NotAJwt,
    /// Le payload (2ᵉ segment) n'est pas décodable en base64url sans padding.
    BadBase64,
    /// Payload décodé en JSON mais `sub` (string) et/ou `exp` (u64) absents.
    MissingClaims,
}

/// Décode le payload POUR AFFICHAGE (bannière de boot). Aucune vérif de signature (c'est
/// `directory` qui vérifie). `Err` distingue la cause exacte pour un WARN de boot diagnosticable.
pub fn describe(jwt: &str) -> Result<(String, u64), DescribeError> {
    // 3 segments requis (header.payload.signature) — un `split('.').nth(1)` réussit déjà sur
    // "a.b", donc on exige explicitement les 3 pour ne pas confondre "pas un JWT" avec le reste.
    if jwt.split('.').count() != 3 {
        return Err(DescribeError::NotAJwt);
    }
    let payload_b64 = jwt.split('.').nth(1).ok_or(DescribeError::NotAJwt)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| DescribeError::BadBase64)?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| DescribeError::MissingClaims)?;
    let sub = v
        .get("sub")
        .and_then(|s| s.as_str())
        .ok_or(DescribeError::MissingClaims)?;
    let exp = v
        .get("exp")
        .and_then(|e| e.as_u64())
        .ok_or(DescribeError::MissingClaims)?;
    Ok((sub.to_string(), exp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64url(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    #[test]
    fn describe_reads_sub_and_exp_from_wire_token() {
        // token du fil (fixtures directory) : sub="wire-fixture-slug".
        let jwt = include_str!("../../directory/tests/fixtures/wire-token.jwt").trim();
        let (sub, exp) = describe(jwt).unwrap();
        assert_eq!(sub, "wire-fixture-slug");
        assert!(exp > 0);
    }

    #[test]
    fn describe_distinguishes_the_three_failure_modes() {
        // Pas 3 segments → NotAJwt.
        assert_eq!(describe("pas-un-jwt"), Err(DescribeError::NotAJwt));
        // 3 segments mais payload non décodable en base64url → BadBase64.
        assert_eq!(
            describe("aaa.!!!notbase64!!!.bbb"),
            Err(DescribeError::BadBase64)
        );
        // payload base64url valide mais JSON sans sub/exp → MissingClaims.
        let no_claims = format!("h.{}.s", b64url(b"{\"foo\":1}"));
        assert_eq!(describe(&no_claims), Err(DescribeError::MissingClaims));
    }
}
