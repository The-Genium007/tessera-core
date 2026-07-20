//! Endpoint métriques minimal (texte Prometheus), un par process (Gateway, Shard). Aucune
//! dépendance HTTP ajoutée au workspace — implémentation volontairement minimale : une seule
//! route fixe, réponse identique quels que soient le chemin/la méthode demandés (suffisant pour
//! un scraper Prometheus).

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Seuils (en microsecondes) des buckets cumulatifs de l'histogramme `tessera_tick_duration` —
/// alignés sur le budget de tick 50ms (20 Hz) : <10/25/40/50ms puis +Inf. Cumulatif au sens
/// Prometheus (`le="0.04"` compte aussi les ticks tombés dans les buckets <10/<25) : chaque
/// bucket ci-dessous DOIT inclure le compte de tous les buckets de seuil inférieur, cf.
/// `record_tick_duration_micros`.
const TICK_BUCKET_THRESHOLDS_MICROS: [u64; 4] = [10_000, 25_000, 40_000, 50_000];

/// Au-delà de ce seuil (microsecondes), un tick est compté comme un overrun — dépassement du
/// budget de tick 50ms (20 Hz) qui dégrade la fluidité perçue par les joueurs de ce shard.
const OVERRUN_THRESHOLD_MICROS: u64 = 50_000;

/// Compteurs partagés, mis à jour par la boucle appelante, lus par le endpoint HTTP.
#[derive(Default)]
pub struct Metrics {
    pub players: AtomicU64,
    pub shards_loaded: AtomicU64,
    pub last_tick_micros: AtomicI64,
    pub max_snapshot_age_ticks: AtomicU64,
    pub rejected_messages_total: AtomicU64,
    /// Buckets cumulatifs de l'histogramme de durée de tick, un compteur par seuil de
    /// `TICK_BUCKET_THRESHOLDS_MICROS` (<10/<25/<40/<50ms) — le 5e bucket implicite `+Inf` est
    /// déduit du nombre total d'observations (`tick_duration_count`), pas stocké séparément.
    pub tick_duration_buckets: [AtomicU64; 4],
    /// Nombre total d'observations de durée de tick — sert de compteur `_count` Prometheus et de
    /// valeur du bucket implicite `+Inf`.
    pub tick_duration_count: AtomicU64,
    /// Somme cumulée des durées de tick observées, en microsecondes — sert de compteur `_sum`
    /// Prometheus (converti en secondes au rendu, comme les seuils `le`).
    pub tick_duration_sum_micros: AtomicU64,
    /// Total cumulé de ticks dont la durée dépasse `OVERRUN_THRESHOLD_MICROS` (budget 50ms/20 Hz).
    pub overruns_total: AtomicU64,
    /// Durée de la dernière itération complète de la boucle Gateway, en microsecondes (0 pour un
    /// process Shard, qui n'a pas cette boucle).
    pub gateway_loop_duration_micros: AtomicI64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Enregistre la durée d'un tick (microsecondes) : incrémente chaque bucket cumulatif dont le
    /// seuil est atteint ou dépassé... non — chaque bucket dont le seuil est SUPÉRIEUR OU ÉGAL à
    /// la durée observée (sémantique standard `le` de Prometheus), plus `_count`/`_sum`, et
    /// `overruns_total` si la durée dépasse `OVERRUN_THRESHOLD_MICROS`.
    pub fn record_tick_duration_micros(&self, micros: u64) {
        for (bucket, &threshold) in self
            .tick_duration_buckets
            .iter()
            .zip(TICK_BUCKET_THRESHOLDS_MICROS.iter())
        {
            if micros <= threshold {
                bucket.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.tick_duration_count.fetch_add(1, Ordering::Relaxed);
        self.tick_duration_sum_micros
            .fetch_add(micros, Ordering::Relaxed);
        if micros > OVERRUN_THRESHOLD_MICROS {
            self.overruns_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Enregistre la durée d'une itération complète de la boucle Gateway (microsecondes).
    pub fn record_gateway_loop_duration_micros(&self, micros: i64) {
        self.gateway_loop_duration_micros
            .store(micros, Ordering::Relaxed);
    }

    /// Rend l'état courant au format texte Prometheus (`# HELP`/`# TYPE` + une ligne par
    /// métrique).
    pub fn render(&self) -> String {
        let mut out = format!(
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
        );

        // Histogramme de durée de tick (secondes, comme l'exige la convention Prometheus pour les
        // métriques de temps) — buckets cumulatifs `le="<seuil>"` puis `+Inf`, suivis de `_sum`/
        // `_count` (ordre standard d'un histogramme Prometheus).
        out.push_str(
            "# HELP tessera_tick_duration_seconds Durée des ticks de simulation, en secondes.\n\
             # TYPE tessera_tick_duration_seconds histogram\n",
        );
        let count = self.tick_duration_count.load(Ordering::Relaxed);
        for (bucket, &threshold) in self
            .tick_duration_buckets
            .iter()
            .zip(TICK_BUCKET_THRESHOLDS_MICROS.iter())
        {
            out.push_str(&format!(
                "tessera_tick_duration_bucket{{le=\"{}\"}} {}\n",
                threshold as f64 / 1_000_000.0,
                bucket.load(Ordering::Relaxed),
            ));
        }
        out.push_str(&format!(
            "tessera_tick_duration_bucket{{le=\"+Inf\"}} {}\n\
             tessera_tick_duration_sum {}\n\
             tessera_tick_duration_count {}\n",
            count,
            self.tick_duration_sum_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            count,
        ));

        out.push_str(&format!(
            "# HELP tessera_overruns_total Total cumulé de ticks dépassant le budget de 50ms (20 Hz).\n\
             # TYPE tessera_overruns_total counter\n\
             tessera_overruns_total {}\n\
             # HELP tessera_gateway_loop_duration_micros Durée de la dernière itération complète de la boucle Gateway, en microsecondes (0 pour un Shard).\n\
             # TYPE tessera_gateway_loop_duration_micros gauge\n\
             tessera_gateway_loop_duration_micros {}\n",
            self.overruns_total.load(Ordering::Relaxed),
            self.gateway_loop_duration_micros.load(Ordering::Relaxed),
        ));

        out
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

    #[test]
    fn tick_duration_bucket_increments_the_right_bucket() {
        let m = Metrics::new();
        m.record_tick_duration_micros(5_000); // 5ms -> bucket <10ms
        m.record_tick_duration_micros(35_000); // 35ms -> bucket <40ms
        let rendered = m.render();
        assert!(rendered.contains("tessera_tick_duration_bucket"));
    }

    #[test]
    fn overrun_counter_increments_on_ticks_beyond_50ms() {
        let m = Metrics::new();
        m.record_tick_duration_micros(60_000); // 60ms -> overrun
        assert_eq!(m.overruns_total.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn overrun_counter_does_not_increment_under_50ms() {
        let m = Metrics::new();
        m.record_tick_duration_micros(40_000);
        assert_eq!(m.overruns_total.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn gateway_loop_duration_is_recorded_as_a_gauge() {
        let m = Metrics::new();
        m.record_gateway_loop_duration_micros(1234);
        assert_eq!(m.gateway_loop_duration_micros.load(Ordering::Relaxed), 1234);
    }

    #[test]
    fn render_includes_tick_duration_bucket_lines_for_all_thresholds() {
        let m = Metrics::new();
        m.record_tick_duration_micros(60_000); // overrun, doit quand même peupler le bucket >50ms
        let out = m.render();
        assert!(out.contains("tessera_tick_duration_bucket{le=\"0.01\"}"));
        assert!(out.contains("tessera_tick_duration_bucket{le=\"0.025\"}"));
        assert!(out.contains("tessera_tick_duration_bucket{le=\"0.04\"}"));
        assert!(out.contains("tessera_tick_duration_bucket{le=\"0.05\"}"));
        assert!(out.contains("tessera_tick_duration_bucket{le=\"+Inf\"}"));
    }

    #[test]
    fn render_includes_overruns_total_and_gateway_loop_duration() {
        let m = Metrics::new();
        m.record_tick_duration_micros(60_000);
        m.record_gateway_loop_duration_micros(4321);
        let out = m.render();
        assert!(out.contains("tessera_overruns_total 1"));
        assert!(out.contains("tessera_gateway_loop_duration_micros 4321"));
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
