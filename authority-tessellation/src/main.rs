use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use tessera_authority::artifact::{generate, to_json, Artifact};
use tessera_authority::params::Params;
use tessera_authority::report::report;
use tessera_authority::svg::render_svg;
use tessera_authority::validate::validate;

#[derive(Parser)]
#[command(name = "tessera-authority")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Generate {
        #[arg(long)]
        districts: PathBuf,
        #[arg(long)]
        manual: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        svg: Option<PathBuf>,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    Validate {
        #[arg(long)]
        artifact: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Generate {
            districts,
            manual,
            out,
            svg,
            report: report_out,
        } => {
            let dj = fs::read_to_string(&districts)?;
            let mj = manual.map(fs::read_to_string).transpose()?;
            let p = Params::default();
            let art = generate(&dj, mj.as_deref(), &p)?;
            fs::write(&out, to_json(&art))?;
            if let Some(s) = svg {
                fs::write(s, render_svg(&art))?;
            }
            if let Some(r) = report_out {
                fs::write(r, report(&art))?;
            }
            println!("Généré : {} cellules -> {}", art.cells.len(), out.display());
        }
        Cmd::Validate { artifact } => {
            let s = fs::read_to_string(&artifact)?;
            let art: Artifact = serde_json::from_str(&s)?;
            let violations = validate(&art);
            if violations.is_empty() {
                println!("OK : artefact valide ({} cellules)", art.cells.len());
            } else {
                for v in &violations {
                    eprintln!("violation: {v}");
                }
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
