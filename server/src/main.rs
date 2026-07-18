//! Point d'entrée du serveur : boucle de tick à fréquence fixe.
//!
//! Sans la feature `gns`, le serveur tourne à vide (aucun transport réseau réel n'est branché).
//! Avec `--features gns`, passer une adresse en argument pour activer GnsTransport :
//!   `cargo run -p server --features gns -- 127.0.0.1:27020`

use std::time::{Duration, Instant};

fn main() {
    tracing_subscriber::fmt::init();

    // Attestation « serveur officiel » (spec 2026-07-16) : le JWT signé par le CMS, collé en env.
    // Absente = ce serveur ne sera jamais republié "official" (community). Jamais un prérequis dur.
    if let Ok(attestation) = std::env::var("TESSERA_OFFICIAL_ATTESTATION") {
        let listen_addr = std::env::var("TESSERA_INTERNAL_ATTESTATION_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:9090".to_string());
        match server::attestation_display::describe(&attestation) {
            Ok((sub, exp)) => {
                tracing::info!(slug = %sub, exp_epoch = exp, "attestation officielle active")
            }
            Err(e) => tracing::warn!(
                "TESSERA_OFFICIAL_ATTESTATION illisible : {e:?} \
                 (attendu : JWT 3 segments, payload base64url no-pad avec sub+exp)"
            ),
        }
        let token = attestation.clone();
        let rt = tokio::runtime::Runtime::new()
            .expect("runtime tokio pour le serveur d'attestation interne");
        std::thread::spawn(move || {
            rt.block_on(async move {
                if let Err(e) =
                    server::internal_attestation_http::serve(&listen_addr, Some(token)).await
                {
                    tracing::error!(error = %e, "serveur d'attestation interne arrêté");
                }
            });
        });
    }

    let hz = server::default_tick_rate_hz();
    let period = Duration::from_secs_f64(1.0 / hz as f64);
    tracing::info!(
        tick_rate_hz = hz,
        "Cyberpunk RP server — démarrage boucle de tick"
    );

    // Binaire monolithique historique (pré-Gateway/Shard) — rayon AoI en dur, son sort réel
    // (suppression ou conteneurisation) est traité au chantier B.
    let mut srv = server::server_loop::Server::new(100.0);

    // Transport : en 0-C, le transport réseau réel (GNS) est derrière la feature `gns`.
    // Sans elle, on tourne à vide pour valider la cadence de tick.
    #[cfg(feature = "gns")]
    {
        use server::gns_transport::GnsTransport;
        use std::net::SocketAddr;

        // Lire l'adresse d'écoute en argument, ou utiliser un défaut.
        let addr_str = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "127.0.0.1:27020".to_string());
        let sock_addr: SocketAddr = addr_str
            .parse()
            .expect("adresse invalide (ex: 127.0.0.1:27020)");

        tracing::info!(addr = %sock_addr, "GnsTransport — écoute activée");
        let mut transport = GnsTransport::listen(sock_addr.ip(), sock_addr.port())
            .expect("GnsTransport::listen failed");

        loop {
            let start = Instant::now();
            srv.tick(&mut transport);
            let elapsed = start.elapsed();
            if let Some(rem) = period.checked_sub(elapsed) {
                std::thread::sleep(rem);
            }
        }
    }

    #[cfg(not(feature = "gns"))]
    {
        tracing::warn!(
            "mode sans transport réseau — compiler avec --features gns pour activer GnsTransport"
        );
        loop {
            let start = Instant::now();
            // srv.tick(&mut transport);  // activé quand un Transport concret est fourni
            let _ = &mut srv;
            let elapsed = start.elapsed();
            if let Some(rem) = period.checked_sub(elapsed) {
                std::thread::sleep(rem);
            }
        }
    }
}
