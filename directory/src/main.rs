mod derive;
mod render;
mod server_identity;
mod shard_map;
mod signing;

use clap::{Parser, Subcommand};
use derive::derive_entry;
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Publish {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long = "out-dir")]
        out_dir: PathBuf,
    },
    Verify {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        sig: PathBuf,
        #[arg(long)]
        pubkey: String,
    },
    Topology {
        #[command(subcommand)]
        command: TopologyCommand,
    },
    Register {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long = "platform-url")]
        platform_url: String,
        #[arg(long = "identity-path")]
        identity_path: PathBuf,
    },
    Heartbeat {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long = "platform-url")]
        platform_url: String,
        #[arg(long = "identity-path")]
        identity_path: PathBuf,
        /// Endpoint métriques Prometheus du Gateway (fournit tessera_players).
        #[arg(long = "metrics-url", default_value = "http://127.0.0.1:9100")]
        metrics_url: String,
        #[arg(long = "interval-secs", default_value_t = 30)]
        interval_secs: u64,
        /// Un seul battement puis sortie (tests/vérification manuelle).
        #[arg(long, default_value_t = false)]
        once: bool,
    },
}

#[derive(Subcommand)]
enum TopologyCommand {
    Check {
        #[arg(long)]
        manifest: PathBuf,
    },
    Render {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Export {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Publish { manifest, out_dir } => cmd_publish(&manifest, &out_dir),
        Command::Verify { file, sig, pubkey } => cmd_verify(&file, &sig, &pubkey),
        Command::Topology { command } => match command {
            TopologyCommand::Check { manifest } => cmd_topology_check(&manifest),
            TopologyCommand::Render { manifest, out } => cmd_topology_render(&manifest, &out),
            TopologyCommand::Export { manifest, out } => cmd_topology_export(&manifest, &out),
        },
        Command::Register {
            manifest,
            platform_url,
            identity_path,
        } => cmd_register(&manifest, &platform_url, &identity_path),
        Command::Heartbeat {
            manifest,
            platform_url,
            identity_path,
            metrics_url,
            interval_secs,
            once,
        } => cmd_heartbeat(
            &manifest,
            &platform_url,
            &identity_path,
            &metrics_url,
            interval_secs,
            once,
        ),
    }
}

fn signing_key() -> anyhow::Result<ed25519_dalek::SigningKey> {
    let seed = std::env::var("TESSERA_DIRECTORY_SIGNING_KEY")
        .map_err(|_| anyhow::anyhow!("TESSERA_DIRECTORY_SIGNING_KEY non définie"))?;
    signing::signing_key_from_b64_seed(&seed).map_err(|e| anyhow::anyhow!(e))
}

fn cmd_publish(manifest_path: &std::path::Path, out_dir: &std::path::Path) -> anyhow::Result<()> {
    let manifest = server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    let entry = derive_entry(&manifest);
    let bytes = serde_json::to_vec_pretty(&vec![entry])?;
    let key = signing_key()?;
    let sig = signing::sign_detached_b64(&key, &bytes);

    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("servers.json"), &bytes)?;
    std::fs::write(out_dir.join("servers.json.sig"), sig)?;
    println!("Publié dans {}", out_dir.display());
    Ok(())
}

fn cmd_verify(
    file: &std::path::Path,
    sig: &std::path::Path,
    pubkey_b64: &str,
) -> anyhow::Result<()> {
    let bytes = std::fs::read(file)?;
    let sig_b64 = std::fs::read_to_string(sig)?;
    let key = signing::verifying_key_from_b64(pubkey_b64).map_err(|e| anyhow::anyhow!(e))?;
    signing::verify_detached_b64(&key, &bytes, sig_b64.trim()).map_err(|e| anyhow::anyhow!(e))?;
    println!("Signature valide.");
    Ok(())
}

fn cmd_topology_check(manifest_path: &std::path::Path) -> anyhow::Result<()> {
    server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    println!("Topologie valide.");
    Ok(())
}

fn cmd_topology_render(
    manifest_path: &std::path::Path,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let manifest = server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    let zones = server::manifest::load_authority_topology(
        &manifest.runtime.topology,
        manifest_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let svg = render::render_svg(&manifest.runtime.topology, &zones);
    std::fs::write(out, svg)?;
    println!("Rendu dans {}", out.display());
    Ok(())
}

fn cmd_topology_export(
    manifest_path: &std::path::Path,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let manifest = server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    let zones = server::manifest::load_authority_topology(
        &manifest.runtime.topology,
        manifest_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let v = shard_map::shard_map_json(&manifest, &zones);
    std::fs::write(out, serde_json::to_vec_pretty(&v)?)?;
    println!("Carte des shards exportée dans {}", out.display());
    Ok(())
}

fn cmd_register(
    manifest_path: &std::path::Path,
    platform_url: &str,
    identity_path: &std::path::Path,
) -> anyhow::Result<()> {
    let manifest = server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    let key = server_identity::load_or_create(identity_path).map_err(|e| anyhow::anyhow!(e))?;
    let public_key_b64 = signing::public_b64(&key);
    let payload = build_register_payload(&manifest, public_key_b64);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{platform_url}/v1/servers/register"))
        .json(&payload)
        .send()?;

    if resp.status().is_success() {
        println!(
            "Serveur '{}' enregistré auprès de {platform_url}.",
            manifest.identity.id
        );
        Ok(())
    } else {
        anyhow::bail!("échec d'enregistrement : HTTP {}", resp.status())
    }
}

#[derive(serde::Serialize)]
struct RegisterPayload {
    id: String,
    name: String,
    public_key_b64: String,
    metadata: derive::DirectoryEntry,
}

fn build_register_payload(
    manifest: &server::manifest::Manifest,
    public_key_b64: String,
) -> RegisterPayload {
    RegisterPayload {
        id: manifest.identity.id.clone(),
        name: manifest.identity.name.clone(),
        public_key_b64,
        metadata: derive_entry(manifest),
    }
}

/// Extrait la gauge `tessera_players` du texte Prometheus rendu par server::metrics.
fn parse_players_metric(prometheus_text: &str) -> Option<i32> {
    prometheus_text
        .lines()
        .find_map(|l| l.strip_prefix("tessera_players ")?.trim().parse().ok())
}

/// Le message signé du heartbeat — format FIGÉ, dupliqué de platform-api/src/routes.rs.
fn heartbeat_message(id: &str, player_count: i32, timestamp_rfc3339: &str) -> String {
    format!("{id}|{player_count}|{timestamp_rfc3339}")
}

/// Récupère le nombre de joueurs depuis l'endpoint `/metrics` du Gateway.
/// En cas d'échec (requête injoignable, HTTP non-2xx, corps illisible ou gauge
/// absente/invalide), journalise la cause sur stderr et retombe sur 0 — le
/// heartbeat continue d'être envoyé, mais l'opérateur voit que la valeur est
/// dégradée plutôt qu'un simple "0 joueur" silencieux.
fn fetch_player_count(client: &reqwest::blocking::Client, metrics_url: &str) -> i32 {
    let resp = match client.get(metrics_url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("métriques injoignables ({metrics_url}) : {e}");
            return 0;
        }
    };
    if !resp.status().is_success() {
        eprintln!(
            "métriques refusées ({metrics_url}) : HTTP {}",
            resp.status()
        );
        return 0;
    }
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("corps des métriques illisible ({metrics_url}) : {e}");
            return 0;
        }
    };
    match parse_players_metric(&text) {
        Some(n) => n,
        None => {
            eprintln!(
                "gauge tessera_players absente ou invalide dans les métriques ({metrics_url})"
            );
            0
        }
    }
}

fn cmd_heartbeat(
    manifest_path: &std::path::Path,
    platform_url: &str,
    identity_path: &std::path::Path,
    metrics_url: &str,
    interval_secs: u64,
    once: bool,
) -> anyhow::Result<()> {
    let manifest = server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    let key = server_identity::load_or_create(identity_path).map_err(|e| anyhow::anyhow!(e))?;
    let id = manifest.identity.id.clone();
    let client = reqwest::blocking::Client::new();

    // Register d'abord (upsert idempotent) : le serveur s'annonce dès son lancement.
    let public_key_b64 = signing::public_b64(&key);
    let payload = build_register_payload(&manifest, public_key_b64);
    let resp = client
        .post(format!("{platform_url}/v1/servers/register"))
        .json(&payload)
        .send()?;
    if !resp.status().is_success() {
        anyhow::bail!("échec d'enregistrement : HTTP {}", resp.status());
    }
    println!("Serveur '{id}' enregistré auprès de {platform_url} — heartbeat toutes les {interval_secs}s.");

    loop {
        let players = fetch_player_count(&client, metrics_url);
        let ts = chrono::Utc::now().to_rfc3339();
        let sig = signing::sign_detached_b64(&key, heartbeat_message(&id, players, &ts).as_bytes());
        let sent = client
            .post(format!("{platform_url}/v1/heartbeat"))
            .json(&serde_json::json!({
                "id": id, "player_count": players, "timestamp": ts, "signature_b64": sig
            }))
            .send();
        match sent {
            Ok(r) if r.status().is_success() => println!("heartbeat ok · players={players}"),
            Ok(r) => eprintln!("heartbeat refusé : HTTP {}", r.status()),
            Err(e) => eprintln!("heartbeat injoignable : {e}"),
        }
        if once {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval_secs));
    }
    Ok(())
}

#[cfg(test)]
mod register_tests {
    use super::*;

    fn sample_manifest() -> server::manifest::Manifest {
        server::manifest::load(std::path::Path::new("../server/server.example.toml"))
            .expect("server.example.toml doit être chargeable depuis tessera-core/directory")
    }

    #[test]
    fn build_register_payload_uses_manifest_identity() {
        let manifest = sample_manifest();
        let payload = build_register_payload(&manifest, "fake-pubkey-b64".to_string());
        assert_eq!(payload.id, manifest.identity.id);
        assert_eq!(payload.name, manifest.identity.name);
        assert_eq!(payload.public_key_b64, "fake-pubkey-b64");
    }

    #[test]
    fn register_payload_carries_full_directory_entry_as_metadata() {
        let manifest = sample_manifest();
        let payload = build_register_payload(&manifest, "fake-pubkey-b64".to_string());
        let json = serde_json::to_value(&payload).unwrap();
        // Le metadata doit être l'entrée launcher camelCase, dérivée du manifeste.
        assert_eq!(
            json["metadata"]["address"],
            manifest.runtime.gateway.advertise_addr
        );
        assert_eq!(
            json["metadata"]["maxPlayers"],
            manifest.identity.max_players
        );
        assert_eq!(
            json["metadata"]["requiredModset"],
            manifest.identity.required_modset
        );
        assert!(json["metadata"]["launchArgs"].is_array());
    }
}

#[cfg(test)]
mod heartbeat_tests {
    use super::*;

    #[test]
    fn parse_players_metric_reads_the_gauge_from_prometheus_text() {
        let text = "# HELP tessera_players Nombre de joueurs actuellement suivis par ce process.\n\
                    # TYPE tessera_players gauge\n\
                    tessera_players 7\n\
                    tessera_shards_loaded 2\n";
        assert_eq!(parse_players_metric(text), Some(7));
    }

    #[test]
    fn parse_players_metric_is_none_when_absent_or_invalid() {
        assert_eq!(parse_players_metric(""), None);
        assert_eq!(parse_players_metric("tessera_players abc\n"), None);
    }

    #[test]
    fn heartbeat_message_matches_platform_contract() {
        // Format figé côté platform-api (routes.rs) : "{id}|{player_count}|{timestamp_rfc3339}".
        assert_eq!(
            heartbeat_message("srv-1", 3, "2026-07-05T12:00:00+00:00"),
            "srv-1|3|2026-07-05T12:00:00+00:00"
        );
    }
}
