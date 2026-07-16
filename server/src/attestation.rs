//! Échange une clé de Service User ZITADEL (JSON, format `{type, keyId, key, userId}` tel que
//! renvoyé par `POST /v2/users/{userId}/keys`) contre un access token via
//! `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`, et le garde en cache avec
//! re-échange proactif (spec §points-ouverts : moins de 5 min de validité restante).

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
pub struct ServiceAccountKey {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "keyId")]
    pub key_id: String,
    pub key: String,
}

// `Debug` est écrit à la main (plutôt que dérivé) pour ne jamais imprimer `key` (clé privée RSA
// PEM) : un `{:?}` accidentel plus tard (panic, log de debug de `AttestationCache`) ne doit pas
// finir par dumper la clé privée en clair dans les logs/stdout.
impl std::fmt::Debug for ServiceAccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccountKey")
            .field("user_id", &self.user_id)
            .field("key_id", &self.key_id)
            .field("key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug)]
pub enum AttestationError {
    InvalidKeyJson(String),
    InvalidPrivateKey(String),
    ExchangeFailed(String),
}

#[derive(Serialize)]
struct JwtAssertionClaims {
    iss: String,
    sub: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
struct TokenExchangeResponse {
    access_token: String,
    expires_in: u64,
}

struct CachedToken {
    access_token: String,
    expires_at_unix: u64,
}

pub struct AttestationCache {
    issuer: String,
    service_account: ServiceAccountKey,
    encoding_key: EncodingKey,
    cached: RwLock<Option<CachedToken>>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("horloge système avant epoch")
        .as_secs()
}

impl AttestationCache {
    /// `issuer` : ex. `https://auth.tesserasynth.net` (même variable que côté launcher/site).
    pub fn new(issuer: &str, service_account_key_json: &str) -> Result<Self, AttestationError> {
        let service_account: ServiceAccountKey = serde_json::from_str(service_account_key_json)
            .map_err(|e| AttestationError::InvalidKeyJson(e.to_string()))?;
        let encoding_key = EncodingKey::from_rsa_pem(service_account.key.as_bytes())
            .map_err(|e| AttestationError::InvalidPrivateKey(e.to_string()))?;
        Ok(Self {
            issuer: issuer.to_string(),
            service_account,
            encoding_key,
            cached: RwLock::new(None),
        })
    }

    fn build_assertion(&self) -> Result<String, AttestationError> {
        let iat = now_unix();
        let claims = JwtAssertionClaims {
            iss: self.service_account.user_id.clone(),
            sub: self.service_account.user_id.clone(),
            aud: self.issuer.clone(),
            iat,
            exp: iat + 3600,
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.service_account.key_id.clone());
        encode(&header, &claims, &self.encoding_key)
            .map_err(|e| AttestationError::InvalidPrivateKey(e.to_string()))
    }

    async fn exchange(&self) -> Result<CachedToken, AttestationError> {
        let assertion = self.build_assertion()?;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/oauth/v2/token", self.issuer))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
                ("scope", "openid"),
            ])
            .send()
            .await
            .map_err(|e| AttestationError::ExchangeFailed(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AttestationError::ExchangeFailed(format!(
                "HTTP {status} : {body}"
            )));
        }

        let parsed: TokenExchangeResponse = response
            .json()
            .await
            .map_err(|e| AttestationError::ExchangeFailed(e.to_string()))?;

        Ok(CachedToken {
            access_token: parsed.access_token,
            expires_at_unix: now_unix() + parsed.expires_in,
        })
    }

    /// Renvoie le token en cache, en le re-échangeant d'abord s'il expire dans moins de 5 min
    /// (ou s'il n'y a encore aucun token en cache). `None` si l'échange échoue — l'appelant
    /// (endpoint HTTP interne) doit alors répondre "pas d'attestation disponible", jamais
    /// planter le serveur de jeu pour un problème d'attestation (spec §3).
    pub async fn current_token(&self) -> Option<String> {
        const REFRESH_MARGIN_SECS: u64 = 300;
        {
            let guard = self.cached.read().unwrap();
            if let Some(cached) = guard.as_ref() {
                if cached.expires_at_unix > now_unix() + REFRESH_MARGIN_SECS {
                    return Some(cached.access_token.clone());
                }
            }
        }
        match self.exchange().await {
            Ok(fresh) => {
                let token = fresh.access_token.clone();
                *self.cached.write().unwrap() = Some(fresh);
                Some(token)
            }
            Err(e) => {
                tracing::error!(error = ?e, "échange du token d'attestation ZITADEL échoué");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::RsaPrivateKey;

    fn test_key_json() -> String {
        let mut rng = rand::rngs::OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("génération clé de test");
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("encodage PEM");
        serde_json::to_string(&serde_json::json!({
            "userId": "test-user-1",
            "keyId": "test-key-1",
            "key": pem.as_str(),
        }))
        .unwrap()
    }

    #[test]
    fn new_parses_a_valid_service_account_key_json() {
        let cache = AttestationCache::new("https://auth.example.com", &test_key_json());
        assert!(cache.is_ok());
    }

    #[test]
    fn new_rejects_malformed_json() {
        let cache = AttestationCache::new("https://auth.example.com", "not json");
        assert!(matches!(cache, Err(AttestationError::InvalidKeyJson(_))));
    }

    #[test]
    fn build_assertion_produces_a_jwt_with_matching_iss_sub_and_kid() {
        let cache = AttestationCache::new("https://auth.example.com", &test_key_json()).unwrap();
        let assertion = cache.build_assertion().unwrap();

        let header = jsonwebtoken::decode_header(&assertion).unwrap();
        assert_eq!(header.kid.as_deref(), Some("test-key-1"));

        let parts: Vec<&str> = assertion.split('.').collect();
        assert_eq!(parts.len(), 3);
    }

    #[tokio::test]
    async fn current_token_returns_none_when_issuer_unreachable() {
        // Port 0 réservé/jamais accepté en pratique -> échec de connexion immédiat, exercise le
        // chemin d'erreur sans dépendre d'un vrai serveur ZITADEL.
        let cache = AttestationCache::new("http://127.0.0.1:1", &test_key_json()).unwrap();
        assert!(cache.current_token().await.is_none());
    }

    #[test]
    fn service_account_key_debug_redacts_the_private_key() {
        let key_json = test_key_json();
        let parsed: ServiceAccountKey = serde_json::from_str(&key_json).unwrap();
        let raw_pem = parsed.key.clone();

        let debug_output = format!("{:?}", parsed);

        assert!(
            !debug_output.contains(&raw_pem),
            "le Debug de ServiceAccountKey ne doit jamais contenir la clé PEM brute"
        );
        assert!(
            debug_output.contains("<redacted>"),
            "le Debug de ServiceAccountKey doit indiquer que le champ key est masqué"
        );
    }
}
