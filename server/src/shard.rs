//! Le Shard : simulation autoritaire d'une zone, pilotée par le Gateway via TCP interne.
//! Une seule connexion Gateway en v1 (M0-M1). Tick 20 Hz.

use crate::internal_net::InternalTransport;
use crate::server_loop::Server;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TICK: Duration = Duration::from_millis(50); // 20 Hz

pub async fn shard_main(addr: &str, aoi_radius: f32, metrics_addr: &str) -> std::io::Result<()> {
    let metrics = crate::metrics::Metrics::new();
    {
        let metrics = metrics.clone();
        let metrics_addr = metrics_addr.to_string();
        tokio::spawn(async move {
            if let Err(e) = crate::metrics::serve(&metrics_addr, metrics).await {
                tracing::warn!("endpoint métriques indisponible ({metrics_addr}): {e}");
            }
        });
    }

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Shard en écoute (interne) sur {addr}");
    loop {
        let (mut sock, peer) = listener.accept().await?;
        tracing::info!("Gateway connecté depuis {peer}");
        let mut server = Server::new(aoi_radius);
        let mut transport = InternalTransport::new();
        let mut buf = [0u8; 8192];
        let mut ticker = tokio::time::interval(TICK);

        loop {
            tokio::select! {
                // Lecture des frames du Gateway (events clients).
                read = sock.read(&mut buf) => {
                    let n = match read { Ok(0) | Err(_) => break, Ok(n) => n };
                    if !transport.feed(&buf[..n]) {
                        tracing::warn!("frame surdimensionné reçu du Gateway — connexion fermée");
                        break;
                    }
                }
                // Tick de simulation 20 Hz.
                _ = ticker.tick() => {
                    let tick_start = std::time::Instant::now();
                    server.tick(&mut transport);
                    metrics.last_tick_micros.store(
                        tick_start.elapsed().as_micros() as i64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    metrics.players.store(
                        server.player_count() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    for frame in transport.take_outbound() {
                        if sock.write_all(&frame).await.is_err() {
                            return Ok(()); // Gateway parti
                        }
                    }
                }
            }
        }
        tracing::info!("Gateway déconnecté, réinitialisation du shard");
    }
}
