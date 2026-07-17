//! Vérifie un JWT d'attestation « serveur officiel » signé par le CMS (EdDSA), contre une clé
//! publique statique — plus aucun JWKS/issuer ZITADEL (spec 2026-07-16). Renvoie le `sub` (slug)
//! si le token est valide (signature + iss + exp). Puis confirme via le CMS que ce slug est
//! toujours listé (révocation live). Jamais de panique : toute défaillance ⇒ community.
#![allow(dead_code)] // câblé depuis main.rs en Task 7

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AttestationClaims {
    sub: String,
}

/// Vérifie la signature EdDSA (contre `public_key_pem`, une clé publique statique — plus de
/// trousseau JWKS à rafraîchir), l'issuer et l'expiration. Renvoie le `sub` (slug du serveur) si
/// valide, `None` pour toute défaillance (signature, issuer, expiration, PEM malformé) — jamais
/// de panique, l'appelant retombe sur `kind = "community"` (spec §4).
pub fn verify_attestation(
    token: &str,
    public_key_pem: &str,
    expected_issuer: &str,
) -> Option<String> {
    let key = DecodingKey::from_ed_pem(public_key_pem.as_bytes()).ok()?;
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[expected_issuer]);
    validation.validate_exp = true;
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);
    let data = decode::<AttestationClaims>(token, &key, &validation).ok()?;
    Some(data.claims.sub)
}

#[derive(Deserialize)]
struct OfficialServerLookupResponse {
    found: bool,
}

/// Interroge `GET {cms_url}/api/public/official-server-by-slug?slug=...` (Task 3/4) pour
/// confirmer que le slug attesté est toujours listé (révocation live). `false` pour toute erreur
/// réseau/HTTP non-2xx/réponse illisible — jamais bloquant (spec §Objectif).
pub fn confirm_official_server(cms_url: &str, slug: &str) -> bool {
    let client = reqwest::blocking::Client::new();
    let url = format!("{cms_url}/api/public/official-server-by-slug?slug={slug}");
    let response = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("confirmation CMS injoignable ({url}) : {e}");
            return false;
        }
    };
    if !response.status().is_success() {
        eprintln!(
            "confirmation CMS refusée ({url}) : HTTP {}",
            response.status()
        );
        return false;
    }
    match response.json::<OfficialServerLookupResponse>() {
        Ok(body) => body.found,
        Err(e) => {
            eprintln!("réponse CMS illisible ({url}) : {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
    use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use ed25519_dalek::SigningKey;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;

    const ISSUER: &str = "tessera-cms";

    #[derive(Serialize)]
    struct Claims {
        iss: String,
        sub: String,
        iat: u64,
        exp: u64,
    }

    struct Pair {
        private_pem: String,
        public_pem: String,
    }

    fn gen_pair() -> Pair {
        let sk = SigningKey::generate(&mut rand::rngs::OsRng);
        Pair {
            private_pem: sk.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
            public_pem: sk
                .verifying_key()
                .to_public_key_pem(LineEnding::LF)
                .unwrap(),
        }
    }

    fn sign(private_pem: &str, sub: &str, iss: &str, exp: u64) -> String {
        let key = EncodingKey::from_ed_pem(private_pem.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some("JWT".into());
        encode(
            &header,
            &Claims {
                iss: iss.into(),
                sub: sub.into(),
                iat: 0,
                exp,
            },
            &key,
        )
        .unwrap()
    }

    #[test]
    fn accepts_valid_token_and_returns_sub() {
        let p = gen_pair();
        let t = sign(&p.private_pem, "srv-1", ISSUER, 9_999_999_999);
        assert_eq!(
            verify_attestation(&t, &p.public_pem, ISSUER),
            Some("srv-1".into())
        );
    }

    #[test]
    fn rejects_wrong_issuer() {
        let p = gen_pair();
        let t = sign(&p.private_pem, "srv-1", "someone-else", 9_999_999_999);
        assert_eq!(verify_attestation(&t, &p.public_pem, ISSUER), None);
    }

    #[test]
    fn rejects_expired() {
        let p = gen_pair();
        let t = sign(&p.private_pem, "srv-1", ISSUER, 1);
        assert_eq!(verify_attestation(&t, &p.public_pem, ISSUER), None);
    }

    #[test]
    fn rejects_unknown_key() {
        let signer = gen_pair();
        let other = gen_pair();
        let t = sign(&signer.private_pem, "srv-1", ISSUER, 9_999_999_999);
        assert_eq!(verify_attestation(&t, &other.public_pem, ISSUER), None);
    }

    #[test]
    fn verifies_real_cms_wire_token() {
        // Token produit par le VRAI signeur jose (fixtures, Task 6 step 1) — rougit si le contrat
        // de fil casse (leçon CLAUDE.md : tester le fil, pas chaque côté isolément).
        let token = include_str!("../tests/fixtures/wire-token.jwt").trim();
        let public_pem = include_str!("../tests/fixtures/wire-public.pem");
        assert_eq!(
            verify_attestation(token, public_pem, "tessera-cms"),
            Some("wire-fixture-slug".into())
        );
    }
}
