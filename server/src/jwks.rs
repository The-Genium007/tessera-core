//! Cache local des clés publiques JWKS ZITADEL — vérification JWT hors-ligne, jamais d'appel
//! réseau synchrone par connexion client (design 2026-07-09, §2 : le serveur de jeu ne dépend
//! jamais synchroniquement de la plateforme).

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// Audience d'un JWT (`aud`, RFC 7519 §4.1.3) : **soit une chaîne unique, soit un tableau de
/// chaînes** — la RFC autorise les deux, et ZITADEL émet un tableau dès que plusieurs audiences
/// sont autorisées sur le projet (id_token launcher réel du 2026-07-17 : trois entrées).
///
/// Root cause du kick « session invalide, reconnectez-vous » à chaque Join (2026-07-17) : ce champ
/// était typé `String`. `jsonwebtoken::decode::<Claims>` désérialise le payload dans `Claims`
/// AVANT de valider l'audience — serde échouait donc sur le tableau, et l'erreur (`ErrorKind::Json`)
/// tombait dans le bras `_ =>` de `verify`, qui la traduisait en `InvalidSignature` : une signature
/// parfaitement valide accusée à tort, puis présentée au joueur comme une session expirée.
///
/// L'angle mort qui l'a laissé passer : les tests encodaient `Claims` (donc `aud` toujours
/// sérialisé en chaîne) puis le redécodaient — round-trip Rust→Rust vert des deux côtés pendant
/// que le fil réel était cassé (cf. CLAUDE.md, « Tester le FIL, pas chaque côté »).
/// `verify_accepts_token_whose_aud_is_an_array` encode désormais le payload en JSON brut, comme
/// le vrai IdP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Audience {
    Single(String),
    Multiple(Vec<String>),
}

impl From<&str> for Audience {
    fn from(value: &str) -> Self {
        Audience::Single(value.to_string())
    }
}

impl From<String> for Audience {
    fn from(value: String) -> Self {
        Audience::Single(value)
    }
}

/// Claims extraites d'un id_token vérifié. Seul `sub` est réellement consommé (clé de persistance,
/// cf. `gateway::resolve_join_key`) ; `aud`/`exp` sont validés par `jsonwebtoken` lui-même via
/// `Validation` (sa propre struct interne, indépendante de celle-ci) et sont ici surtout pour que
/// les tests puissent encoder des tokens réalistes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub aud: Audience,
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

        // `ErrorKind::Json` = le payload ne rentre pas dans `Claims` (forme inattendue), PAS une
        // signature forgée. Le distinguer n'est pas cosmétique : c'est ce bras `_ =>` fourre-tout
        // qui a fait passer l'incident du 2026-07-17 (aud en tableau) pour une attaque sur la
        // signature, et envoyé enquêter sur ZITADEL pendant une nuit. Un défaut de forme doit se
        // dire `Malformed` et se voir dans les logs, jamais se déguiser en `InvalidSignature`.
        let token_data = decode::<Claims>(token, key, &validation).map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::Expired,
            jsonwebtoken::errors::ErrorKind::InvalidAudience => JwtError::WrongAudience,
            jsonwebtoken::errors::ErrorKind::Json(cause) => {
                // Jamais le token ni le payload (secret) — seulement la raison structurelle.
                tracing::warn!(%cause, "JWT rejeté : payload illisible (forme des claims)");
                JwtError::Malformed
            }
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

    /// Vérité terrain (incident 2026-07-17, kick « session invalide » à chaque Join) : ZITADEL
    /// émet `aud` comme un **tableau** dès que plusieurs audiences sont autorisées sur le projet.
    /// L'id_token réel du launcher en portait trois, dont le client_id attendu.
    ///
    /// Tous les autres tests de ce module encodent une struct `Claims` (`aud: String`) puis la
    /// redécodent : round-trip Rust→Rust où `aud` est TOUJOURS sérialisé en chaîne — le tableau
    /// n'y apparaît jamais, le fil réel n'est jamais exercé (cf. CLAUDE.md, « Tester le FIL, pas
    /// chaque côté »). Ce test encode donc le payload en JSON brut, comme le fait le vrai IdP.
    #[test]
    fn verify_accepts_token_whose_aud_is_an_array() {
        let (encoding_key, jwks_cache) = test_key_pair_and_cache();
        // Payload calqué sur la FORME EXACTE d'un id_token ZITADEL réel capturé le 2026-07-17
        // (identifiants remplacés par des valeurs factices — on ne fige jamais un vrai token dans
        // le source : il porte un `sub` réel et une signature valide) :
        //   - `aud` est un TABLEAU et l'audience attendue n'est PAS la première entrée ;
        //   - le payload porte 8 claims que `Claims` ne déclare pas (`iss`, `iat`, `auth_time`,
        //     `amr`, `azp`, `client_id`, `at_hash`, `sid`). Elles doivent être ignorées : ce test
        //     échouerait si quelqu'un ajoutait un jour `#[serde(deny_unknown_fields)]` à `Claims`.
        let payload = serde_json::json!({
            "iss": "https://auth.tesserasynth.net",
            "sub": "381635808245383541",
            "aud": ["000000000000000001", "launcher", "000000000000000002"],
            "exp": far_future_timestamp(),
            "iat": 1_784_271_777,
            "auth_time": 1_784_138_569,
            "amr": ["pwd"],
            "azp": "launcher",
            "client_id": "launcher",
            "at_hash": "AzDTQ0bmPjQH0PqeqF6gOg",
            "sid": "381920845678772631",
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        let token = encode(&header, &payload, &encoding_key).expect("encodage du token de test");

        let claims = jwks_cache
            .verify(&token, "launcher")
            .expect("un aud en tableau contenant l'audience attendue doit être accepté");
        assert_eq!(claims.sub, "381635808245383541");
    }

    /// Corollaire du test ci-dessus : un `aud` en tableau qui NE contient PAS l'audience attendue
    /// doit toujours être rejeté. Sans ce test, on pourrait « réparer » le tableau en désactivant
    /// la validation d'audience — ce qui ferait accepter le token d'un autre client ZITADEL.
    ///
    /// Assertion sur la VARIANTE (`WrongAudience`), pas un simple `is_err()` : avant le correctif
    /// « aud en tableau », ce test passait déjà — mais parce que la désérialisation échouait
    /// (`Malformed`), pas parce que l'audience était refusée. Un `is_err()` serait donc resté vert
    /// même si le correctif avait accidentellement désactivé la validation d'audience.
    #[test]
    fn verify_rejects_array_aud_without_expected_audience() {
        let (encoding_key, jwks_cache) = test_key_pair_and_cache();
        let payload = serde_json::json!({
            "sub": "user-123",
            "aud": ["381641683642679674", "381641646514635130"],
            "exp": far_future_timestamp(),
        });
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(TEST_KID.to_string());
        let token = encode(&header, &payload, &encoding_key).expect("encodage du token de test");

        assert!(
            matches!(
                jwks_cache.verify(&token, "launcher"),
                Err(JwtError::WrongAudience)
            ),
            "doit être refusé pour AUDIENCE, pas pour une erreur de forme des claims"
        );
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
