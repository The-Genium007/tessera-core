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
    // Enregistré UNE SEULE FOIS avant la boucle, comme côté Gateway (cf. shutdown.rs) — une
    // reconstruction par itération perdrait un signal reçu entre deux itérations.
    let mut shutdown = crate::shutdown::ShutdownSignal::new();
    loop {
        let (mut sock, peer) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = shutdown.recv() => {
                tracing::info!("Arrêt propre : extinction du Shard");
                return Ok(());
            }
        };
        tracing::info!("Gateway connecté depuis {peer}");
        let mut server = Server::new(aoi_radius);
        let mut transport = InternalTransport::new();
        let mut buf = [0u8; 8192];
        let mut ticker = tokio::time::interval(TICK);
        // Défaut tokio = Burst : un tick en retard déclenche une rafale de rattrapage — chaque
        // tick de rattrapage ré-encode et renvoie un snapshot complet à chaque joueur, dépensant
        // donc PLUS de CPU/réseau juste après un pic de charge. Skip saute les ticks manqués
        // (le dernier état écrase les précédents, cf. audit prod 2026-07-03 §4.3/§C.2).
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

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
                // Arrêt propre : ne pas attendre la fin de la connexion Gateway pour répondre
                // au signal (sinon un SIGTERM/SIGINT reste bloqué tant qu'un Gateway est connecté).
                _ = shutdown.recv() => {
                    tracing::info!("Arrêt propre : extinction du Shard");
                    return Ok(());
                }
            }
        }
        tracing::info!("Gateway déconnecté, réinitialisation du shard");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    // Envoie un signal Unix réel au process de test courant — sûr ici car `ShutdownSignal`
    // installe un handler qui remplace la disposition par défaut (le process ne meurt pas),
    // le futur `recv()` se contente de se résoudre. Passe par la commande `kill` plutôt qu'une
    // dépendance `libc` pour ne pas toucher Cargo.toml (hors périmètre de cette tâche).
    #[cfg(unix)]
    fn send_self_sigterm() {
        let pid = std::process::id().to_string();
        let status = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("kill -TERM devrait pouvoir s'exécuter");
        assert!(status.success(), "kill -TERM a échoué");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shard_flushes_nothing_but_exits_cleanly_on_shutdown_signal() {
        // Le Shard n'a pas de persistance propre (state Gateway-autoritaire) — le test vérifie
        // seulement que la boucle sort proprement (retourne Ok(())) sur réception du signal,
        // sans laisser de connexion Gateway en attente indéfiniment (ici : aucune connexion du
        // tout — la course doit gagner dès la boucle d'accept externe).
        let addr = "127.0.0.1:27131";
        let handle =
            tokio::spawn(async move { super::shard_main(addr, 1000.0, "127.0.0.1:0").await });

        // Laisse le shard se binder et enregistrer son ShutdownSignal avant d'envoyer le signal.
        tokio::time::sleep(Duration::from_millis(200)).await;

        send_self_sigterm();

        let result = tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("shard_main aurait dû sortir avant le timeout au lieu d'attendre une connexion")
            .expect("la tâche du shard n'aurait pas dû paniquer");

        assert!(
            result.is_ok(),
            "shard_main devrait retourner Ok(()) sur arrêt propre"
        );
    }
}
