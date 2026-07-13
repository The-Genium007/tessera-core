//! Cache local des clés publiques JWKS ZITADEL — vérification JWT hors-ligne, jamais d'appel
//! réseau synchrone par connexion client (design 2026-07-09, §2 : le serveur de jeu ne dépend
//! jamais synchroniquement de la plateforme).

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub aud: String,
    pub exp: u64,
}

#[derive(Debug)]
pub enum JwtError {
    UnknownKeyId,
    InvalidSignature,
    Expired,
    WrongAudience,
    Malformed,
}

#[derive(Debug)]
pub enum JwksError {
    FetchFailed(String),
    ParseFailed(String),
}

/// Document JWKS standard tel que servi par un endpoint `/oauth/v2/keys` OIDC.
#[derive(Debug, Deserialize)]
struct JwksDocument {
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

pub struct JwksCache {
    keys: RwLock<HashMap<String, DecodingKey>>,
}

impl Default for JwksCache {
    fn default() -> Self {
        Self::new()
    }
}

impl JwksCache {
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn with_key(kid: &str, key: DecodingKey) -> Self {
        let mut keys = HashMap::new();
        keys.insert(kid.to_string(), key);
        Self {
            keys: RwLock::new(keys),
        }
    }

    /// Récupère le document JWKS à `jwks_url` et remplace intégralement le cache de clés.
    ///
    /// Ne panique jamais sur une entrée malformée ou d'un type non supporté (ex. `kty` autre que
    /// `RSA`) : cette entrée est ignorée (avec un `warn` de log) et les autres clés valides du
    /// document sont conservées.
    pub async fn refresh(&self, jwks_url: &str) -> Result<(), JwksError> {
        let response = reqwest::get(jwks_url)
            .await
            .map_err(|e| JwksError::FetchFailed(e.to_string()))?;
        let document: JwksDocument = response
            .json()
            .await
            .map_err(|e| JwksError::ParseFailed(e.to_string()))?;

        let mut new_keys = HashMap::with_capacity(document.keys.len());
        for entry in document.keys {
            if entry.kty != "RSA" {
                tracing::warn!(kid = %entry.kid, kty = %entry.kty, "clé JWKS ignorée: kty non supporté");
                continue;
            }
            let (Some(n), Some(e)) = (entry.n.as_deref(), entry.e.as_deref()) else {
                tracing::warn!(kid = %entry.kid, "clé JWKS ignorée: composants n/e RSA manquants");
                continue;
            };
            match DecodingKey::from_rsa_components(n, e) {
                Ok(key) => {
                    new_keys.insert(entry.kid, key);
                }
                Err(err) => {
                    tracing::warn!(kid = %entry.kid, error = %err, "clé JWKS ignorée: composants RSA invalides");
                }
            }
        }

        let mut keys = self.keys.write().unwrap();
        *keys = new_keys;
        Ok(())
    }

    pub fn verify(&self, token: &str, expected_aud: &str) -> Result<Claims, JwtError> {
        let header = decode_header(token).map_err(|_| JwtError::Malformed)?;
        let kid = header.kid.ok_or(JwtError::Malformed)?;
        let keys = self.keys.read().unwrap();
        let key = keys.get(&kid).ok_or(JwtError::UnknownKeyId)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[expected_aud]);

        let token_data = decode::<Claims>(token, key, &validation).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
            jsonwebtoken::errors::ErrorKind::InvalidAudience => JwtError::WrongAudience,
            _ => JwtError::InvalidSignature,
        })?;
        Ok(token_data.claims)
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

    /// Matériel de clé RSA de test généré à la volée (jamais une vraie clé ZITADEL, jamais de
    /// clé embarquée en dur dans le source — cf. `platform-api/src/signing.rs` qui génère aussi
    /// sa paire Ed25519 de test avec `SigningKey::generate(&mut OsRng)`, même esprit).
    struct TestRsaKeyMaterial {
        encoding_key: EncodingKey,
        decoding_key: DecodingKey,
        n_b64: String,
        e_b64: String,
    }

    fn generate_test_rsa_key_material() -> TestRsaKeyMaterial {
        let mut rng = rand::rngs::OsRng;
        let private_key =
            RsaPrivateKey::new(&mut rng, 2048).expect("génération de la clé RSA de test");
        let public_key = RsaPublicKey::from(&private_key);

        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("encodage PKCS1 PEM de la clé de test");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .expect("clé RSA de test illisible par jsonwebtoken");

        let n_b64 = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());
        let decoding_key = DecodingKey::from_rsa_components(&n_b64, &e_b64)
            .expect("composants RSA de test invalides");

        TestRsaKeyMaterial {
            encoding_key,
            decoding_key,
            n_b64,
            e_b64,
        }
    }

    fn test_key_pair_and_cache() -> (EncodingKey, JwksCache) {
        let material = generate_test_rsa_key_material();
        (
            material.encoding_key,
            JwksCache::with_key(TEST_KID, material.decoding_key),
        )
    }

    /// Clé privée d'une paire RSA totalement différente de celle connue du cache — sert à
    /// simuler un token signé par un tiers non autorisé.
    fn other_encoding_key() -> EncodingKey {
        generate_test_rsa_key_material().encoding_key
    }

    fn far_future_timestamp() -> u64 {
        9_999_999_999 // an. 2286 — largement suffisant pour ne jamais expirer en test
    }

    fn encode_test_token(claims: &Claims, key: &EncodingKey) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        encode(&header, claims, key).expect("échec de l'encodage du token de test")
    }

    #[test]
    fn verify_accepts_token_signed_with_matching_key() {
        let (encoding_key, jwks_cache) = test_key_pair_and_cache();
        let claims = Claims {
            sub: "user-123".into(),
            aud: "launcher".into(),
            exp: far_future_timestamp(),
        };
        let token = encode_test_token(&claims, &encoding_key);

        let result = jwks_cache.verify(&token, "launcher");
        assert_eq!(result.unwrap().sub, "user-123");
    }

    #[test]
    fn verify_rejects_token_signed_with_wrong_key() {
        let (_, jwks_cache) = test_key_pair_and_cache();
        let other_encoding_key = other_encoding_key();
        let claims = Claims {
            sub: "user-123".into(),
            aud: "launcher".into(),
            exp: far_future_timestamp(),
        };
        let token = encode_test_token(&claims, &other_encoding_key);

        assert!(jwks_cache.verify(&token, "launcher").is_err());
    }

    #[test]
    fn verify_rejects_expired_token() {
        let (encoding_key, jwks_cache) = test_key_pair_and_cache();
        let claims = Claims {
            sub: "user-123".into(),
            aud: "launcher".into(),
            exp: 1, // 1970
        };
        let token = encode_test_token(&claims, &encoding_key);

        assert!(jwks_cache.verify(&token, "launcher").is_err());
    }

    #[test]
    fn verify_rejects_wrong_audience() {
        let (encoding_key, jwks_cache) = test_key_pair_and_cache();
        let claims = Claims {
            sub: "user-123".into(),
            aud: "other-client".into(),
            exp: far_future_timestamp(),
        };
        let token = encode_test_token(&claims, &encoding_key);

        assert!(jwks_cache.verify(&token, "launcher").is_err());
    }

    #[tokio::test]
    async fn refresh_populates_cache_from_jwks_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let material = generate_test_rsa_key_material();
        let body = format!(
            "{{\"keys\":[{{\"kid\":\"{kid}\",\"kty\":\"RSA\",\"n\":\"{n}\",\"e\":\"{e}\"}}]}}",
            kid = TEST_KID,
            n = material.n_b64,
            e = material.e_b64,
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind du serveur JWKS mocké");
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept du client HTTP");
            let mut buf = [0u8; 1024];
            // Lecture best-effort de la requête : on n'en a pas besoin, un seul GET est attendu.
            let _ = socket.read(&mut buf).await;

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("écriture de la réponse JWKS mockée");
            socket.shutdown().await.ok();
        });

        let cache = JwksCache::new();
        cache
            .refresh(&format!("http://{addr}/jwks"))
            .await
            .expect("refresh doit réussir contre le serveur mocké");

        server.await.expect("tâche du serveur mocké");

        let claims = Claims {
            sub: "user-123".into(),
            aud: "launcher".into(),
            exp: far_future_timestamp(),
        };
        let token = encode_test_token(&claims, &material.encoding_key);

        let result = cache.verify(&token, "launcher");
        assert_eq!(result.unwrap().sub, "user-123");
    }
}
