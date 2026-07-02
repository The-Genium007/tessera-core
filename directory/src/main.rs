mod derive;
mod render;
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Publish { manifest, out_dir } => cmd_publish(&manifest, &out_dir),
        Command::Verify { file, sig, pubkey } => cmd_verify(&file, &sig, &pubkey),
        Command::Topology { command } => match command {
            TopologyCommand::Check { manifest } => cmd_topology_check(&manifest),
            TopologyCommand::Render { manifest, out } => cmd_topology_render(&manifest, &out),
        },
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
    let zones = server::manifest::flatten_topology(&manifest.runtime.topology)
        .map_err(|e| anyhow::anyhow!(e))?;
    let svg = render::render_svg(&manifest.runtime.topology, &zones);
    std::fs::write(out, svg)?;
    println!("Rendu dans {}", out.display());
    Ok(())
}
