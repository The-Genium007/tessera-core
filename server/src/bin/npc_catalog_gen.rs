//! Générateur de catalogue PNJ d'ambiance (pont Phase A→B, spec ambiance §4).
//!
//! Lit une liste de NOMS d'archétypes CDPR (un par ligne, le pool trié en Phase A), dérive les
//! tags de curation de chaque nom (`npc_tags::derive_tags`), et émet un `npc-catalog.toml` valide
//! prêt à être consommé par le serveur — chaque archétype avec un id stable, la brique d'ambiance
//! par défaut, et ses tags.
//!
//! ⚠️ ENTRÉE ET SORTIE SONT DÉRIVÉES DES DONNÉES CDPR : elles restent LOCALES (le pool est
//! gitignore, le catalogue généré aussi). Ce binaire est l'outil ; c'est lui qui est versionné,
//! pas ce qu'il produit. Un opérateur régénère son catalogue depuis son propre install de jeu.
//!
//! NON-SILENCE (spec §5) : tout nom dont `derive_tags` ne tire AUCUN tag est écrit sur stderr et
//! N'EST PAS émis — un archétype sans tags serait inexcluable, donc un piège muet. Le compte de
//! noms ignorés est affiché en fin de run pour que la couverture soit visible, jamais supposée.
//!
//! Usage :
//!   tessera-npc-catalog-gen <pool.txt> [> npc-catalog.toml]

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(pool_path) = args.next() else {
        eprintln!("usage : npc_catalog_gen <fichier-pool.txt>  (un nom d'archétype par ligne)");
        return ExitCode::from(2);
    };

    let content = match std::fs::read_to_string(&pool_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lecture de {pool_path} échouée : {e}");
            return ExitCode::from(2);
        }
    };

    // Noms : lignes non vides, non commentées. On enlève un éventuel préfixe `Character.` et le
    // suffixe `.ent` que certaines entrées portent, pour garder un nom d'archétype nu.
    let names: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.strip_prefix("Character.")
                .unwrap_or(l)
                .strip_suffix(".ent")
                .unwrap_or_else(|| l.strip_prefix("Character.").unwrap_or(l))
                .to_string()
        })
        .collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(
        out,
        "# Catalogue PNJ d'ambiance — GÉNÉRÉ par tessera-npc-catalog-gen (pont Phase A->B).\n\
         # NE PAS éditer à la main : régénérer depuis le pool. Dérivé des noms CDPR => reste LOCAL.\n\
         format_version = 1\n"
    );

    let mut emitted = 0u32;
    let mut skipped: Vec<String> = Vec::new();
    // Ids stables et déterministes : ordre alphabétique du pool -> id croissant. Régénérer le même
    // pool redonne les mêmes ids (important pour que le serveur ne « change » pas de PNJ au reboot).
    let mut sorted = names;
    sorted.sort();
    sorted.dedup();

    for name in &sorted {
        let tags = server::npc_tags::derive_tags(name);
        if tags.is_empty() {
            skipped.push(name.clone());
            continue;
        }
        emitted += 1;
        let tags_toml = tags
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "[[archetype]]\nid = {emitted}\nname = \"{name}\"\nbriques = [\"flaner-sur-place\"]\ntags = [{tags_toml}]\n"
        );
    }

    // Rapport de couverture sur stderr — visible sans polluer le TOML redirigé vers un fichier.
    eprintln!("généré : {emitted} archétypes tagués.");
    if !skipped.is_empty() {
        eprintln!(
            "IGNORÉS ({}) — aucun tag dérivé, donc NON émis (inexcluables sinon) :",
            skipped.len()
        );
        for n in &skipped {
            eprintln!("  - {n}");
        }
        eprintln!("=> ajoute une règle à npc_tags::derive_tags pour chacun, ou retire-le du pool.");
    }
    ExitCode::SUCCESS
}
