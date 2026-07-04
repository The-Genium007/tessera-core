//! Identité Ed25519 propre à un serveur, persistée localement (jamais commitée — même
//! discipline que les autres secrets du projet). Distincte de TESSERA_DIRECTORY_SIGNING_KEY.

use crate::signing::{generate_signing_key, signing_key_from_b64_seed};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::SigningKey;
use std::path::Path;

/// Charge l'identité depuis `path` si le fichier existe, sinon en génère une nouvelle et
/// l'écrit. Idempotent : un 2e appel sur le même `path` renvoie la MÊME clé.
pub fn load_or_create(path: &Path) -> Result<SigningKey, String> {
    if path.exists() {
        let seed_b64 = std::fs::read_to_string(path)
            .map_err(|e| format!("lecture de {}: {e}", path.display()))?;
        signing_key_from_b64_seed(seed_b64.trim())
    } else {
        let key = generate_signing_key();
        let seed_b64 = B64.encode(key.to_bytes());
        create_new_identity_file(path, &seed_b64)?;
        Ok(key)
    }
}

/// Écrit une nouvelle identité sur disque. Sur Unix, restreint les permissions (0700 pour le
/// dossier parent, 0600 pour le fichier) car ce fichier contient une clé privée : les autres
/// utilisateurs locaux ne doivent pas pouvoir la lire. `create_new` évite en plus une race
/// TOCTOU entre le `path.exists()` de l'appelant et cette écriture.
#[cfg(unix)]
fn create_new_identity_file(path: &Path, seed_b64: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|e| format!("création de {}: {e}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("écriture de {}: {e}", path.display()))?;
    file.write_all(seed_b64.as_bytes())
        .map_err(|e| format!("écriture de {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn create_new_identity_file(path: &Path, seed_b64: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("création de {}: {e}", parent.display()))?;
    }
    std::fs::write(path, seed_b64).map_err(|e| format!("écriture de {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing::public_b64;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use tempfile::tempdir;

    fn b64_len_of(pub_b64: &str) -> usize {
        B64.decode(pub_b64).unwrap().len()
    }

    #[test]
    fn creates_a_new_identity_when_the_file_does_not_exist() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity.key");
        assert!(!path.exists());

        let key = load_or_create(&path).unwrap();
        assert!(path.exists());
        assert_eq!(b64_len_of(&public_b64(&key)), 32);
    }

    #[test]
    fn reuses_the_same_identity_on_a_second_call() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity.key");

        let first = load_or_create(&path).unwrap();
        let second = load_or_create(&path).unwrap();
        assert_eq!(public_b64(&first), public_b64(&second));
    }

    #[test]
    fn creates_parent_directories_if_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("identity.key");
        assert!(load_or_create(&path).is_ok());
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn creates_the_identity_file_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("identity.key");

        load_or_create(&path).unwrap();

        let file_mode = path.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);

        let dir_mode = path
            .parent()
            .unwrap()
            .metadata()
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }
}
