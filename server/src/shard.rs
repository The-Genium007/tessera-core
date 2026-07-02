//! Le Shard : simulation autoritaire d'une zone, pilotée par le Gateway via TCP interne.
//! Une seule connexion Gateway en v1 (M0-M1). Tick 20 Hz.

use crate::internal_net::InternalTransport;
use crate::server_loop::Server;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TICK: Duration = Duration::from_millis(50); // 20 Hz

pub async fn shard_main(addr: &str, aoi_radius: f32) -> std::io::Result<()> {
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
                    transport.feed(&buf[..n]);
                }
                // Tick de simulation 20 Hz.
                _ = ticker.tick() => {
                    server.tick(&mut transport);
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
