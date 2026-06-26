//! Point d'entrée du serveur (squelette Phase 0-A : log + affiche le tick rate).

fn main() {
    tracing_subscriber::fmt::init();
    let hz = server::default_tick_rate_hz();
    tracing::info!(
        tick_rate_hz = hz,
        "Cyberpunk RP server — squelette Phase 0-A"
    );
    println!("Cyberpunk RP server (squelette) — tick rate cible : {hz} Hz");
}
