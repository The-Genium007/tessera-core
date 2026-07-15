//! Dérive une entrée `servers.json` (schéma déjà consommé par le launcher) depuis `[identity]`
//! du manifeste. `address`/`launchArgs` sont calculés depuis `advertise_addr`, jamais dupliqués
//! à la main. `launchArgs` porte aussi `-skipStartScreen` (spec playtest-shards §#3) : argument
//! de lancement du jeu, communautaire et documenté, qui saute la vidéo d'intro.

use serde::Serialize;
use server::manifest::{Manifest, ServerChannel, ServerKind};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DirectoryEntry {
    pub id: String,
    pub name: String,
    pub address: String,
    pub region: String,
    pub language: String,
    pub players: u32,
    #[serde(rename = "maxPlayers")]
    pub max_players: u32,
    pub ping: u32,
    pub tags: Vec<String>,
    pub status: String,
    pub description: String,
    #[serde(rename = "discordUrl")]
    pub discord_url: String,
    #[serde(rename = "websiteUrl")]
    pub website_url: String,
    #[serde(rename = "requiredModset")]
    pub required_modset: String,
    #[serde(rename = "voiceRequired")]
    pub voice_required: bool,
    pub public: bool,
    pub kind: String,
    pub whitelist: bool,
    pub channel: String,
    #[serde(rename = "launchArgs")]
    pub launch_args: Vec<String>,
}

pub fn derive_entry(m: &Manifest, attested_official: bool) -> DirectoryEntry {
    let addr = &m.runtime.gateway.advertise_addr;
    let (ip, port) = addr
        .rsplit_once(':')
        .expect("advertise_addr déjà validé comme SocketAddr par server::manifest::load");
    DirectoryEntry {
        id: m.identity.id.clone(),
        name: m.identity.name.clone(),
        address: addr.clone(),
        region: m.identity.region.clone(),
        language: m.identity.language.clone(),
        players: 0,
        max_players: m.identity.max_players,
        ping: 0,
        tags: m.identity.tags.clone(),
        status: "online".to_string(),
        description: m.identity.description.clone(),
        discord_url: m.identity.discord_url.clone(),
        website_url: m.identity.website_url.clone(),
        required_modset: m.identity.required_modset.clone(),
        voice_required: m.identity.voice_required,
        public: m.identity.public,
        kind: match m.identity.kind {
            ServerKind::Official if attested_official => "official".to_string(),
            // Un TOML qui déclare "official" sans attestation valide est silencieusement
            // rétrogradé à "community" (spec §objectif : jamais "official" par défaut, log
            // d'avertissement plutôt qu'erreur bloquante — un opérateur tiers qui copie un
            // manifeste existant ne doit pas planter son serveur pour ça).
            ServerKind::Official => {
                eprintln!(
                    "avertissement : identity.kind=\"official\" déclaré mais aucune attestation valide — rétrogradé à \"community\""
                );
                "community".to_string()
            }
            ServerKind::Community => "community".to_string(),
        },
        whitelist: m.runtime.whitelist,
        channel: match m.identity.channel {
            ServerChannel::Dev => "dev".to_string(),
            ServerChannel::Playtest => "playtest".to_string(),
            ServerChannel::Release => "release".to_string(),
        },
        launch_args: vec![
            "-skipStartScreen".to_string(),
            format!("--cyberverse-server-address={ip}"),
            format!("--cyberverse-server-port={port}"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_manifest() -> Manifest {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server.example.toml");
        server::manifest::load(&path).expect("example manifest should be valid")
    }

    #[test]
    fn derives_expected_json_shape() {
        let entry = derive_entry(&example_manifest(), false);
        let json = serde_json::to_value(&entry).unwrap();
        let expected = serde_json::json!({
            "id": "tessera-dev-01",
            "name": "Tessera Dev — Night City",
            "address": "51.38.189.234:27020",
            "region": "EU",
            "language": "FR",
            "players": 0,
            "maxPlayers": 16,
            "ping": 0,
            "tags": ["dev", "test"],
            "status": "online",
            "description": "Serveur de développement TesseraSynth (serveur Rust autoritaire, GameNetworkingSockets). Tranche verticale 0-D.",
            "discordUrl": "",
            "websiteUrl": "https://tesserasynth.net",
            "requiredModset": "0.1.0-dev10",
            "voiceRequired": false,
            "public": false,
            "kind": "community",
            "whitelist": false,
            "channel": "release",
            "launchArgs": [
                "-skipStartScreen",
                "--cyberverse-server-address=51.38.189.234",
                "--cyberverse-server-port=27020"
            ]
        });
        assert_eq!(json, expected);
    }

    #[test]
    fn derive_entry_propagates_runtime_whitelist_to_directory_entry() {
        let mut manifest = example_manifest();
        manifest.runtime.whitelist = true;
        let entry = derive_entry(&manifest, false);
        assert!(
            entry.whitelist,
            "runtime.whitelist=true doit se propager jusqu'à DirectoryEntry.whitelist"
        );
    }

    #[test]
    fn derive_entry_caps_declared_official_kind_to_community_without_attestation() {
        let mut manifest = example_manifest();
        manifest.identity.kind = ServerKind::Official;
        let entry = derive_entry(&manifest, false);
        assert_eq!(entry.kind, "community");
    }

    #[test]
    fn derive_entry_honors_official_kind_when_attested() {
        let mut manifest = example_manifest();
        manifest.identity.kind = ServerKind::Official;
        let entry = derive_entry(&manifest, true);
        assert_eq!(entry.kind, "official");
    }

    #[test]
    fn derive_entry_never_upgrades_a_declared_community_kind_even_when_attested() {
        // attested_official=true ne doit jamais forcer "official" si le TOML dit "community" —
        // l'attestation plafonne, elle n'élève jamais.
        let entry = derive_entry(&example_manifest(), true);
        assert_eq!(entry.kind, "community");
    }
}
