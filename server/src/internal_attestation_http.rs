//! Endpoint HTTP interne (loopback/réseau interne uniquement — JAMAIS exposé
//! publiquement, cf. spec §3) qui renvoie le JWT d'attestation « officiel » du serveur, tel quel.
//! Le serveur ne fait que relayer le token collé en variable d'env (spec 2026-07-16) : plus aucun
//! échange ZITADEL runtime. Pattern calqué sur `metrics.rs::serve` (une seule route fixe, réponse
//! identique quels que soient chemin/méthode).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub async fn serve(addr: &str, attestation: Option<String>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        let attestation = attestation.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await; // contenu de la requête ignoré : route unique
            let body = serde_json::to_string(&serde_json::json!({ "token": attestation }))
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
    use tokio::net::TcpStream;

    async fn serve_and_request(attestation: Option<String>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener); // libère le port pour serve() qui rebind au même endroit

        let addr_str = addr.to_string();
        tokio::spawn(async move { serve(&addr_str, attestation).await });
        // Laisse le temps au listener de démarrer avant de s'y connecter.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /internal/attestation HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }

    #[tokio::test]
    async fn serve_relays_the_attestation_jwt_when_present() {
        let response_text = serve_and_request(Some("un-jwt".to_string())).await;

        assert!(response_text.contains("200 OK"));
        assert!(response_text.contains("\"token\":\"un-jwt\""));
    }

    #[tokio::test]
    async fn serve_responds_with_token_null_when_no_attestation_available() {
        let response_text = serve_and_request(None).await;

        assert!(response_text.contains("200 OK"));
        assert!(response_text.contains("\"token\":null"));
    }
}
