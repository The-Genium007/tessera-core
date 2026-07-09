//! Signal d'arrêt propre (SIGTERM/SIGINT) partagé entre Gateway et Shard — un flux persistant
//! plutôt que recréé à chaque itération (une recréation par itération perdrait un signal reçu
//! pendant la partie synchrone entre deux itérations, cf. gateway.rs historique).

/// Détecteur d'arrêt propre : SIGINT (Ctrl+C) partout, plus SIGTERM (docker stop) sous Unix.
///
/// Doit être construit UNE SEULE FOIS avant la boucle principale et réutilisé (via `&mut`) à
/// chaque itération. Sur Unix, `recv()` réutilise le même flux `tokio::signal::unix::Signal` —
/// reconstruire ce flux à chaque itération (comme le faisait un appel `shutdown_signal()` frais
/// dans le `select!` de la boucle) rouvre une fenêtre où un signal arrivé entre deux itérations
/// (après le drop de l'ancien flux, avant la création du nouveau) n'est délivré à personne et est
/// silencieusement perdu — tokio ne bufferise pas un signal pour un récepteur qui n'existe pas
/// encore. `tokio::signal::ctrl_c()`, lui, n'a pas ce problème (canal partagé installé une seule
/// fois en interne par tokio dès le premier appel) : il reste donc appelé frais à chaque `recv()`.
pub struct ShutdownSignal {
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
}

// `new()` fait un travail non trivial (enregistrement d'un handler de signal OS) — un
// `Default` impliquerait la même chose sous un autre nom sans apporter de clarté ici, seul
// `new()` est appelé dans tout le codebase (Gateway et Shard).
#[allow(clippy::new_without_default)]
impl ShutdownSignal {
    #[cfg(unix)]
    pub fn new() -> Self {
        use tokio::signal::unix::{signal, SignalKind};
        Self {
            sigterm: signal(SignalKind::terminate()).expect("SIGTERM handler"),
        }
    }

    #[cfg(not(unix))]
    pub fn new() -> Self {
        Self {}
    }

    #[cfg(unix)]
    pub async fn recv(&mut self) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = self.sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    pub async fn recv(&mut self) {
        let _ = tokio::signal::ctrl_c().await;
    }
}
