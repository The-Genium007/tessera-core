//! Vérifie un token d'attestation ZITADEL récupéré depuis `/internal/attestation` d'un serveur,
//! puis confirme via le CMS que le `sub` correspond à une entrée `officialServers` connue.
//! Duplique volontairement le mécanisme JWKS déjà présent côté launcher
//! (`launcher/src-tauri/src/lib.rs`) et site (`zitadelVerify.ts`) — troisième implémentation
//! indépendante, cf. spec §points-ouverts (un crate partagé reste une extension possible mais
//! pas nécessaire pour cette première version : les trois usages ont des contraintes de runtime
//! différentes — async Tokio, async Nuxt/Nitro, et ici du `reqwest::blocking` synchrone dans un
//! outil CLI ponctuel).
//!
//! `fetch_jwks`/`verify_attestation`/`confirm_official_server` ne sont pas encore appelées
//! depuis `main.rs` (câblage dans `cmd_publish` = tâche suivante du plan) : `dead_code` autorisé
//! localement à ce module le temps de cette transition, pas globalement au crate.
#![allow(dead_code)]

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct JwksDocumentRaw {
    keys: Vec<JwkEntry>,
}

#[derive(Debug, Deserialize)]
struct JwkEntry {
    kid: String,
    kty: String,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
}

pub struct JwksDocument {
    keys: HashMap<String, DecodingKey>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AttestationClaims {
    sub: String,
    iss: String,
    exp: u64,
}

/// Récupère et parse le document JWKS ZITADEL. Entrées malformées/non-RSA ignorées avec un
/// avertissement (même tolérance que `server::jwks::JwksCache::refresh`), jamais une erreur
/// bloquante pour une seule clé invalide dans un trousseau par ailleurs valide.
pub fn fetch_jwks(jwks_url: &str) -> Result<JwksDocument, String> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get(jwks_url)
        .send()
        .map_err(|e| format!("JWKS injoignable ({jwks_url}) : {e}"))?;
    let document: JwksDocumentRaw = response
        .json()
        .map_err(|e| format!("JWKS illisible ({jwks_url}) : {e}"))?;

    let mut keys = HashMap::with_capacity(document.keys.len());
    for entry in document.keys {
        if entry.kty != "RSA" {
            eprintln!("clé JWKS ignorée (kid={}) : kty non supporté", entry.kid);
            continue;
        }
        let (Some(n), Some(e)) = (entry.n.as_deref(), entry.e.as_deref()) else {
            eprintln!(
                "clé JWKS ignorée (kid={}) : composants n/e manquants",
                entry.kid
            );
            continue;
        };
        match DecodingKey::from_rsa_components(n, e) {
            Ok(key) => {
                keys.insert(entry.kid, key);
            }
            Err(err) => eprintln!("clé JWKS ignorée (kid={}) : {err}", entry.kid),
        }
    }
    Ok(JwksDocument { keys })
}

/// Vérifie le token (signature via JWKS + issuer + expiration) et renvoie le `sub` si valide.
/// `None` pour toute défaillance (signature invalide, issuer différent, expiré, kid inconnu) —
/// jamais de panique, l'appelant retombe sur `kind = "community"` (spec §4).
pub fn verify_attestation(
    token: &str,
    jwks: &JwksDocument,
    expected_issuer: &str,
) -> Option<String> {
    let header = decode_header(token).ok()?;
    let kid = header.kid?;
    let key = jwks.keys.get(&kid)?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[expected_issuer]);
    validation.validate_exp = true;

    let token_data = decode::<AttestationClaims>(token, key, &validation).ok()?;
    Some(token_data.claims.sub)
}

#[derive(Deserialize)]
struct OfficialServerLookupResponse {
    found: bool,
}

/// Interroge `GET {cms_url}/api/public/official-server-by-zitadel-user?userId=...` (Task 4).
/// `false` pour toute erreur réseau/HTTP non-2xx — jamais bloquant pour le reste de la
/// publication (spec §4).
pub fn confirm_official_server(cms_url: &str, sub: &str) -> bool {
    let client = reqwest::blocking::Client::new();
    let url = format!("{cms_url}/api/public/official-server-by-zitadel-user?userId={sub}");
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
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::traits::PublicKeyParts;
    use rsa::{RsaPrivateKey, RsaPublicKey};

    const TEST_KID: &str = "test-key-1";
    const TEST_ISSUER: &str = "https://auth.example.com";

    struct TestKeyMaterial {
        encoding_key: EncodingKey,
        jwks: JwksDocument,
    }

    fn test_key_material() -> TestKeyMaterial {
        let mut rng = rand::rngs::OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap();
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();

        let n_b64 = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
        let decoding_key = DecodingKey::from_rsa_components(&n_b64, &e_b64).unwrap();

        let mut keys = HashMap::new();
        keys.insert(TEST_KID.to_string(), decoding_key);

        TestKeyMaterial {
            encoding_key,
            jwks: JwksDocument { keys },
        }
    }

    fn far_future() -> u64 {
        9_999_999_999
    }

    fn encode_test_token(claims: &AttestationClaims, key: &EncodingKey) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        encode(&header, claims, key).unwrap()
    }

    #[test]
    fn verify_attestation_accepts_valid_token_and_returns_sub() {
        let material = test_key_material();
        let claims = AttestationClaims {
            sub: "srv-user-1".into(),
            iss: TEST_ISSUER.into(),
            exp: far_future(),
        };
        let token = encode_test_token(&claims, &material.encoding_key);

        assert_eq!(
            verify_attestation(&token, &material.jwks, TEST_ISSUER),
            Some("srv-user-1".to_string())
        );
    }

    #[test]
    fn verify_attestation_rejects_wrong_issuer() {
        let material = test_key_material();
        let claims = AttestationClaims {
            sub: "srv-user-1".into(),
            iss: "https://not-the-real-issuer.example.com".into(),
            exp: far_future(),
        };
        let token = encode_test_token(&claims, &material.encoding_key);

        assert_eq!(
            verify_attestation(&token, &material.jwks, TEST_ISSUER),
            None
        );
    }

    #[test]
    fn verify_attestation_rejects_expired_token() {
        let material = test_key_material();
        let claims = AttestationClaims {
            sub: "srv-user-1".into(),
            iss: TEST_ISSUER.into(),
            exp: 1,
        };
        let token = encode_test_token(&claims, &material.encoding_key);

        assert_eq!(
            verify_attestation(&token, &material.jwks, TEST_ISSUER),
            None
        );
    }

    #[test]
    fn verify_attestation_rejects_token_signed_by_unknown_key() {
        let material = test_key_material();
        let other = test_key_material(); // paire RSA différente, jamais dans le jwks testé
        let claims = AttestationClaims {
            sub: "srv-user-1".into(),
            iss: TEST_ISSUER.into(),
            exp: far_future(),
        };
        let token = encode_test_token(&claims, &other.encoding_key);

        assert_eq!(
            verify_attestation(&token, &material.jwks, TEST_ISSUER),
            None
        );
    }
}
