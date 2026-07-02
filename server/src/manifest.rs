//! Manifeste serveur (fichier TOML par opérateur) : identité publique (dérive servers.json) +
//! config runtime privée (topologie/spawn/rayons/store) consommée au boot du Gateway.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub identity: Identity,
    pub runtime: Runtime,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub description: String,
    pub region: String,
    pub language: String,
    pub max_players: u32,
    pub tags: Vec<String>,
    pub discord_url: String,
    pub website_url: String,
    pub required_modset: String,
    pub voice_required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Runtime {
    pub whitelist: bool,
    pub store_path: String,
    pub gateway: GatewayConfig,
    pub topology: TopologyConfig,
    pub radius: RadiusConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub listen_addr: String,
    pub advertise_addr: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TopologyConfig {
    #[serde(default)]
    pub active_preset: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RadiusConfig {
    pub base: f32,
    pub moderator: f32,
    pub game_master: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    UnsupportedFormatVersion(u32),
    EmptyField(&'static str),
    InvalidMaxPlayers,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormatVersion(v) => {
                write!(f, "format_version {v} non supportée (seule 1 est supportée)")
            }
            Self::EmptyField(name) => write!(f, "champ {name} vide"),
            Self::InvalidMaxPlayers => write!(f, "identity.max_players doit être > 0"),
        }
    }
}

fn validate_scalars(m: &Manifest) -> Result<(), ManifestError> {
    if m.format_version != 1 {
        return Err(ManifestError::UnsupportedFormatVersion(m.format_version));
    }
    if m.identity.id.is_empty() {
        return Err(ManifestError::EmptyField("identity.id"));
    }
    if m.identity.name.is_empty() {
        return Err(ManifestError::EmptyField("identity.name"));
    }
    if m.identity.max_players == 0 {
        return Err(ManifestError::InvalidMaxPlayers);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TOML: &str = r#"
        format_version = 1

        [identity]
        id = "tessera-dev-01"
        name = "Tessera Dev"
        description = "desc"
        region = "EU"
        language = "FR"
        max_players = 16
        tags = ["dev"]
        discord_url = ""
        website_url = ""
        required_modset = "0.1.0"
        voice_required = false

        [runtime]
        whitelist = false
        store_path = "players.json"

        [runtime.gateway]
        listen_addr = "0.0.0.0:27020"
        advertise_addr = "51.38.189.234:27020"

        [runtime.topology]
        active_preset = "2-shards"

        [runtime.radius]
        base = 25.0
        moderator = 50.0
        game_master = 75.0
    "#;

    #[test]
    fn parses_minimal_valid_toml() {
        let m: Manifest = toml::from_str(MINIMAL_TOML).expect("should parse");
        assert_eq!(m.format_version, 1);
        assert_eq!(m.identity.id, "tessera-dev-01");
        assert_eq!(m.runtime.gateway.advertise_addr, "51.38.189.234:27020");
        assert_eq!(m.runtime.radius.base, 25.0);
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let toml_str = MINIMAL_TOML.replace("format_version = 1", "format_version = 2");
        let m: Manifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            validate_scalars(&m),
            Err(ManifestError::UnsupportedFormatVersion(2))
        );
    }

    #[test]
    fn rejects_empty_id() {
        let toml_str = MINIMAL_TOML.replace(r#"id = "tessera-dev-01""#, r#"id = """#);
        let m: Manifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(validate_scalars(&m), Err(ManifestError::EmptyField("identity.id")));
    }

    #[test]
    fn rejects_zero_max_players() {
        let toml_str = MINIMAL_TOML.replace("max_players = 16", "max_players = 0");
        let m: Manifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(validate_scalars(&m), Err(ManifestError::InvalidMaxPlayers));
    }
}
