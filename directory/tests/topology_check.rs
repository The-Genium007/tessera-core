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
    let dir = tempfile::tempdir().unwrap();
    let bad_manifest = dir.path().join("bad.toml");
    let base = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../server/server.example.toml"),
    )
    .unwrap();
    let bad = base.replace(r#"right = "shard-b""#, r#"right = "shard-ghost""#);
    std::fs::write(&bad_manifest, bad).unwrap();

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
}
