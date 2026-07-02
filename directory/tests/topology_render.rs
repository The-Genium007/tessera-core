use std::process::Command;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_tessera-directory")
}

#[test]
fn render_produces_non_empty_svg() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../server/server.example.toml");
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("zones.svg");

    let result = Command::new(bin_path())
        .args([
            "topology",
            "render",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);

    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("<svg"));
    assert!(content.contains("shard-a") || content.contains("127.0.0.1:27030"));
}
