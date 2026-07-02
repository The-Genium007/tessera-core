use std::process::Command;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_tessera-directory")
}

// Même vecteur de test que directory/src/signing.rs et tools/release/src/signing.rs.
const TEST_SEED: &str = "ghSvoGCgqRCrzJuGqFKnG1g55jjmH5lrEX7neX7vfag=";
const TEST_PUB: &str = "Xb4m8qh/yoACil6zvR3npGOJppaYjPxrEuhp5r74dGg=";

#[test]
fn publish_then_verify_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server.example.toml");

    let publish = Command::new(bin_path())
        .args([
            "publish",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--out-dir",
            dir.path().to_str().unwrap(),
        ])
        .env("TESSERA_DIRECTORY_SIGNING_KEY", TEST_SEED)
        .output()
        .unwrap();
    assert!(publish.status.success(), "{:?}", publish);

    let servers_json = dir.path().join("servers.json");
    let sig = dir.path().join("servers.json.sig");
    assert!(servers_json.exists());
    assert!(sig.exists());

    let verify = Command::new(bin_path())
        .args([
            "verify",
            "--file",
            servers_json.to_str().unwrap(),
            "--sig",
            sig.to_str().unwrap(),
            "--pubkey",
            TEST_PUB,
        ])
        .output()
        .unwrap();
    assert!(verify.status.success(), "{:?}", verify);
}
