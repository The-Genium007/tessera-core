mod attestation_verify;
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

    let attested_official = resolve_attestation(&manifest);

    let entry = derive_entry(&manifest, attested_official);
    let bytes = serde_json::to_vec_pretty(&vec![entry])?;
    let key = signing_key()?;
    let sig = signing::sign_detached_b64(&key, &bytes);

    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join("servers.json"), &bytes)?;
    std::fs::write(out_dir.join("servers.json.sig"), sig)?;
    println!("Publié dans {}", out_dir.display());
    Ok(())
}

/// Résout si CE manifeste peut légitimement être republié "official" : interroge l'endpoint
/// interne du serveur pour son token d'attestation courant, le vérifie contre la clé publique
/// statique de l'autorité d'attestation (CMS, EdDSA — plus de JWKS/issuer ZITADEL depuis
/// 2026-07-16), vérifie que le `sub` du token correspond au slug déclaré du manifeste (défense en
/// profondeur), puis confirme via le CMS que ce slug n'est pas révoqué. `false` pour toute étape
/// manquante/en échec — jamais bloquant pour la publication (spec §objectif : repli silencieux,
/// pas d'erreur fatale).
fn resolve_attestation(manifest: &server::manifest::Manifest) -> bool {
    if manifest.identity.kind != server::manifest::ServerKind::Official {
        // Pas la peine d'interroger quoi que ce soit si le manifeste ne déclare même pas
        // "official" — évite un appel réseau inutile à chaque publication d'un serveur
        // community, le cas le plus courant.
        return false;
    }

    let internal_attestation_url = match std::env::var("TESSERA_INTERNAL_ATTESTATION_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "TESSERA_INTERNAL_ATTESTATION_URL absente — impossible de vérifier l'attestation, kind rétrogradé à community"
            );
            return false;
        }
    };
    let public_key_pem = match std::env::var("TESSERA_ATTESTATION_PUBLIC_KEY") {
        Ok(v) => v.replace("\\n", "\n"), // tolère les \n échappés en env
        Err(_) => {
            eprintln!("TESSERA_ATTESTATION_PUBLIC_KEY absente — kind rétrogradé à community");
            return false;
        }
    };
    let cms_url = match std::env::var("TESSERA_CMS_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("TESSERA_CMS_URL absente — kind rétrogradé à community");
            return false;
        }
    };

    let client = reqwest::blocking::Client::new();
    let token = match client
        .get(&internal_attestation_url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<serde_json::Value>())
    {
        Ok(body) => match body.get("token").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => {
                eprintln!("aucun token d'attestation disponible sur {internal_attestation_url}");
                return false;
            }
        },
        Err(e) => {
            eprintln!(
                "endpoint d'attestation interne injoignable ({internal_attestation_url}) : {e}"
            );
            return false;
        }
    };

    let sub = match attestation_verify::verify_attestation(&token, &public_key_pem, "tessera-cms") {
        Some(s) => s,
        None => {
            eprintln!("token d'attestation invalide (signature/issuer/expiration) — kind rétrogradé à community");
            return false;
        }
    };
    // Défense en profondeur : le sub du token DOIT être le slug déclaré du manifeste.
    if sub != manifest.identity.id {
        eprintln!(
            "sub du token ({sub}) ≠ identity.id ({}) — kind rétrogradé à community",
            manifest.identity.id
        );
        return false;
    }
    attestation_verify::confirm_official_server(&cms_url, &sub)
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
    let manifest = server::manifest::load(manifest_path).map_err(|e| anyhow::anyhow!(e))?;
    // Valide aussi la résolution complète de la topologie d'autorité (pas seulement les champs
    // scalaires du manifeste) : un `server_count` sans groupement correspondant dans
    // `assignment_patterns`, ou un `authority_artifact` introuvable/invalide, doivent faire
    // échouer `check` — c'est la référence pendante analogue à l'ancien schéma BSP
    // (`right`/`left` pointant vers un shard inexistant).
    server::manifest::load_authority_topology(
        &manifest.runtime.topology,
        manifest_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )
    .map_err(|e| anyhow::anyhow!(e))?;
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
        // register/heartbeat n'établissent pas l'attestation officielle (c'est `publish` qui la
        // résout, via resolve_attestation) — le metadata annoncé reste plafonné à "community".
        metadata: derive_entry(manifest, false),
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
mod attestation_resolution_tests {
    //! Seul test du chemin **positif** de bout en bout de l'attestation : token valide → vérifié
    //! contre la clé publique statique → confirmé par le CMS → `resolve_attestation` renvoie
    //! `true`. Tous les autres tests de la feature (attestation_verify.rs) couvrent des branches
    //! d'échec (fail-closed) ; celui-ci verrouille la seule branche où un bug importerait vraiment
    //! — la frontière de confiance qui accorde le badge `official`.
    //!
    //! Deux faux serveurs HTTP montés en local (aucun réseau externe, aucun vrai CMS), chacun sur
    //! son propre `127.0.0.1:0` dans son `std::thread` : `resolve_attestation` utilise
    //! `reqwest::blocking`, donc on reste en `std::net::TcpListener` synchrone (le crate
    //! `directory` ne dépend pas de tokio). Crypto Ed25519 réelle : la signature du JWT est
    //! réellement vérifiée contre la clé publique statique, ce n'est pas un raccourci.
    //!
    //! ISOLATION ENV : `resolve_attestation` lit 3 variables d'env process-globales. Aucun autre
    //! test du crate ne lit/écrit ces mêmes noms (`TESSERA_INTERNAL_ATTESTATION_URL`,
    //! `TESSERA_ATTESTATION_PUBLIC_KEY`, `TESSERA_CMS_URL` — vérifié par grep), et ce module ne
    //! contient qu'un seul test qui les écrit, donc pas de course inter-tests même en exécution
    //! parallèle par défaut. Pas de crate `serial_test` ajouté pour ça.
    use super::*;
    use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    use ed25519_dalek::pkcs8::EncodePublicKey;
    use ed25519_dalek::SigningKey;
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const TEST_ISSUER: &str = "tessera-cms";

    /// Répond à UNE connexion HTTP avec `body` (JSON), puis termine. Renvoie l'adresse écoutée.
    /// Sépare volontairement la lecture (best-effort, on ignore la requête : chaque faux endpoint
    /// n'attend qu'un GET) de l'écriture de la réponse — même esprit que le mock de `server::jwks`.
    fn spawn_json_once(body: String) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind du faux serveur HTTP");
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept du client HTTP");
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes());
        });
        addr
    }

    #[test]
    fn resolve_attestation_returns_true_for_a_fully_valid_chain() {
        // 0. Manifeste déclarant kind=official (sinon resolve_attestation court-circuite) — le
        // `sub` du token DOIT correspondre à `identity.id` (défense en profondeur récemment
        // ajoutée), donc on le lit ici plutôt que d'utiliser un slug de test arbitraire.
        let manifest_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server.example.toml");
        let mut manifest =
            server::manifest::load(&manifest_path).expect("manifeste exemple valide");
        manifest.identity.kind = server::manifest::ServerKind::Official;
        let sub = manifest.identity.id.clone();

        // 1. Paire Ed25519 de test générée à la volée (jamais de vraie clé CMS en dur).
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let private_pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("encodage PKCS8 PEM")
            .to_string();
        let public_pem = signing_key
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("encodage SPKI PEM");

        // 2. Un vrai JWT fraîchement signé (issuer correct, exp lointaine) → réellement vérifié.
        let claims = serde_json::json!({
            "sub": sub,
            "iss": TEST_ISSUER,
            "iat": 0,
            "exp": 9_999_999_999u64,
        });
        let encoding_key = EncodingKey::from_ed_pem(private_pem.as_bytes())
            .expect("clé Ed25519 illisible par jsonwebtoken");
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some("JWT".into());
        let token = encode(&header, &claims, &encoding_key).expect("encodage du JWT de test");

        // 3. Deux faux endpoints locaux, chacun sur son port, chacun dans son thread.
        let attestation_addr = spawn_json_once(format!("{{\"token\":\"{token}\"}}"));
        // confirm_official_server appelle GET {cms_url}/api/public/... ; notre mock répond quel
        // que soit le chemin, il suffit qu'il renvoie {"found": true}.
        let cms_addr = spawn_json_once("{\"found\":true}".to_string());

        // 4. Les 3 variables d'env pointent vers nos faux serveurs / la clé publique de test.
        std::env::set_var(
            "TESSERA_INTERNAL_ATTESTATION_URL",
            format!("http://{attestation_addr}/internal/attestation"),
        );
        std::env::set_var("TESSERA_ATTESTATION_PUBLIC_KEY", &public_pem);
        std::env::set_var("TESSERA_CMS_URL", format!("http://{cms_addr}"));

        assert!(
            resolve_attestation(&manifest),
            "une chaîne d'attestation entièrement valide doit résoudre à official (true)"
        );
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
