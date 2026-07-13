//! Endpoint métriques minimal (texte Prometheus), un par process (Gateway, Shard). Aucune
//! dépendance HTTP ajoutée au workspace — implémentation volontairement minimale : une seule
//! route fixe, réponse identique quels que soient le chemin/la méthode demandés (suffisant pour
//! un scraper Prometheus).

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Compteurs partagés, mis à jour par la boucle appelante, lus par le endpoint HTTP.
#[derive(Default)]
pub struct Metrics {
    pub players: AtomicU64,
    pub shards_loaded: AtomicU64,
    pub last_tick_micros: AtomicI64,
    pub max_snapshot_age_ticks: AtomicU64,
    pub rejected_messages_total: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Rend l'état courant au format texte Prometheus (`# HELP`/`# TYPE` + une ligne par
    /// métrique).
    pub fn render(&self) -> String {
        format!(
            "# HELP tessera_players Nombre de joueurs actuellement suivis par ce process.\n\
             # TYPE tessera_players gauge\n\
             tessera_players {}\n\
             # HELP tessera_shards_loaded Nombre de shards chargés (Gateway ; 0 pour un Shard).\n\
             # TYPE tessera_shards_loaded gauge\n\
             tessera_shards_loaded {}\n\
             # HELP tessera_last_tick_micros Durée du dernier tick, en microsecondes (Shard ; 0 pour un Gateway).\n\
             # TYPE tessera_last_tick_micros gauge\n\
             tessera_last_tick_micros {}\n\
             # HELP tessera_snapshot_age_ticks Âge du plus vieux snapshot rediffusé depuis un shard (0 = frais).\n\
             # TYPE tessera_snapshot_age_ticks gauge\n\
             tessera_snapshot_age_ticks {}\n\
             # HELP tessera_rejected_messages_total Total cumulé de messages rejetés (flood, anti-triche, serveur plein).\n\
             # TYPE tessera_rejected_messages_total counter\n\
             tessera_rejected_messages_total {}\n",
            self.players.load(Ordering::Relaxed),
            self.shards_loaded.load(Ordering::Relaxed),
            self.last_tick_micros.load(Ordering::Relaxed),
            self.max_snapshot_age_ticks.load(Ordering::Relaxed),
            self.rejected_messages_total.load(Ordering::Relaxed),
        )
    }
}

/// Sert `metrics.render()` sur toute requête HTTP reçue sur `addr`.
pub async fn serve(addr: &str, metrics: Arc<Metrics>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (mut sock, _) = listener.accept().await?;
        let metrics = metrics.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await; // contenu de la requête ignoré : route unique
            let body = metrics.render();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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

    #[test]
    fn render_includes_all_three_metrics_with_current_values() {
        let m = Metrics::default();
        m.players.store(5, Ordering::Relaxed);
        m.shards_loaded.store(2, Ordering::Relaxed);
        m.last_tick_micros.store(1234, Ordering::Relaxed);
        let text = m.render();
        assert!(text.contains("tessera_players 5"));
        assert!(text.contains("tessera_shards_loaded 2"));
        assert!(text.contains("tessera_last_tick_micros 1234"));
    }

    #[test]
    fn render_includes_max_snapshot_age_ticks() {
        let m = Metrics::default();
        m.max_snapshot_age_ticks.store(7, Ordering::Relaxed);
        let out = m.render();
        assert!(out.contains("tessera_snapshot_age_ticks 7"));
    }

    #[test]
    fn render_includes_rejected_messages_total() {
        let m = Metrics::default();
        m.rejected_messages_total.fetch_add(3, Ordering::Relaxed);
        let out = m.render();
        assert!(out.contains("tessera_rejected_messages_total 3"));
    }

    #[tokio::test]
    async fn serve_responds_with_prometheus_text_containing_current_values() {
        use tokio::net::TcpStream;

        let metrics = Metrics::new();
        metrics.players.store(3, Ordering::Relaxed);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener); // libère le port : `serve` le re-binde lui-même

        let m = metrics.clone();
        let addr_clone = addr.clone();
        tokio::spawn(async move { serve(&addr_clone, m).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let mut sock = TcpStream::connect(&addr).await.unwrap();
        sock.write_all(b"GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            sock.read_to_end(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("200 OK"));
        assert!(text.contains("tessera_players 3"));
    }
}
