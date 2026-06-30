//! Binaire Gateway. Usage (feature gns) :
//!   cargo run -p server --features gns --bin gateway -- [listen_addr] [router_addr]
//! Défauts : 0.0.0.0:27020 (GNS public) et 127.0.0.1:27040 (router).

#[cfg(feature = "gns")]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();
    let listen = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:27020".to_string());
    let router = std::env::args().nth(2).unwrap_or_else(|| "127.0.0.1:27040".to_string());
    server::gateway::gateway_main(&listen, &router).await
}

#[cfg(not(feature = "gns"))]
fn main() {
    eprintln!("Recompiler avec --features gns");
}
