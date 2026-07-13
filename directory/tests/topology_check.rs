use std::process::Command;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_tessera-directory")
}

#[test]
fn check_accepts_valid_example_manifest() {
    let manifest_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server.example.toml");
    let out = Command::new(bin_path())
        .args([
            "topology",
            "check",
            "--manifest",
            manifest_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out);
}

#[test]
fn check_rejects_manifest_with_dangling_reference() {
    // Analogue de l'ancienne « référence pendante » (BSP : `right`/`left` pointant vers un shard
    // inexistant) pour le nouveau schéma Voronoï (Groupe G) : un `server_count` sans groupement
    // correspondant dans `assignment_patterns` de l'artefact — l'artefact réel (`authority.json`,
    // copié ici) n'a des patterns que pour N=1..10.
    let dir = tempfile::tempdir().unwrap();
    let bad_manifest = dir.path().join("bad.toml");
    let base = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server.example.toml"),
    )
    .unwrap();
    let bad = base.replace("server_count = 2", "server_count = 99");
    std::fs::write(&bad_manifest, bad).unwrap();
    // authority_artifact = "authority.json" est un chemin relatif au manifeste : copié à côté du
    // manifeste cassé pour isoler précisément la cause de l'échec (server_count invalide, pas un
    // fichier introuvable).
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/authority.json"),
        dir.path().join("authority.json"),
    )
    .unwrap();

    let out = Command::new(bin_path())
        .args([
            "topology",
            "check",
            "--manifest",
            bad_manifest.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("server_count=99") || stderr.contains("assignment_pattern"),
        "message d'erreur inattendu : {stderr}"
    );
}
