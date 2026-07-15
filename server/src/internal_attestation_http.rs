//! Endpoint HTTP interne (loopback/réseau Dokploy interne uniquement — JAMAIS exposé
//! publiquement, cf. spec §3) qui renvoie le token d'attestation courant en cache. `directory`
//! l'interroge au moment de publier l'entrée d'annuaire de ce serveur précis. Pattern calqué sur
//! `metrics.rs::serve` (une seule route fixe, réponse identique quels que soient chemin/méthode).

use crate::attestation::AttestationCache;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub async fn serve(addr: &str, cache: Arc<AttestationCache>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        let cache = cache.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await; // contenu de la requête ignoré : route unique
            let token = cache.current_token().await;
            let body = serde_json::to_string(&serde_json::json!({ "token": token }))
                .unwrap_or_else(|_| "{\"token\":null}".to_string());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(response.as_bytes()).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::RsaPrivateKey;

    fn unreachable_cache() -> Arc<AttestationCache> {
        // Issuer injoignable (port 1) -> current_token() renvoie toujours None, exercice le
        // chemin "pas de token disponible" sans dépendre d'un vrai ZITADEL.
        let mut rng = rand::rngs::OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap();
        let key_json = serde_json::to_string(&serde_json::json!({
            "userId": "u", "keyId": "k", "key": pem.as_str(),
        }))
        .unwrap();
        Arc::new(AttestationCache::new("http://127.0.0.1:1", &key_json).unwrap())
    }

    #[tokio::test]
    async fn serve_responds_with_token_null_when_no_attestation_available() {
        use tokio::net::TcpStream;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // libère le port pour serve() qui rebind au même endroit

        let cache = unreachable_cache();
        let addr_str = addr.to_string();
        tokio::spawn(async move { serve(&addr_str, cache).await });
        // Laisse le temps au listener de démarrer avant de s'y connecter.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /internal/attestation HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response_text = String::from_utf8_lossy(&response);

        assert!(response_text.contains("200 OK"));
        assert!(response_text.contains("\"token\":null"));
    }
}
