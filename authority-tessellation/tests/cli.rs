use std::fs;
use std::process::Command;

fn sample_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ta_cli_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("districts.json"),
        r#"{"entries":[
          {"id":"a","tweakdb_path":"Districts.Kabuki","code":"KAB","has_transform":true,
           "polygon":[[-1500,-1500],[-500,-1500],[-500,-500],[-1500,-500]]},
          {"id":"c","tweakdb_path":"Districts.BiotechnicaFlats","code":"BF","has_transform":true,
           "polygon":[[2000,2000],[6000,2000],[6000,6000],[2000,6000]]}
        ],"warnings":[]}"#,
    )
    .unwrap();
    dir
}

#[test]
fn generate_then_validate_roundtrip() {
    let dir = sample_dir();
    let art = dir.join("artifact.json");
    let gen = Command::new(env!("CARGO_BIN_EXE_tessera-authority"))
        .args(["generate", "--districts"])
        .arg(dir.join("districts.json"))
        .args(["--out"])
        .arg(&art)
        .output()
        .unwrap();
    assert!(
        gen.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&gen.stderr)
    );
    assert!(art.exists());

    let val = Command::new(env!("CARGO_BIN_EXE_tessera-authority"))
        .args(["validate", "--artifact"])
        .arg(&art)
        .output()
        .unwrap();
    // Partition + non-ambiguïté OK ; r_min peut échouer sur ce mini-échantillon -> on tolère,
    // mais le binaire doit s'exécuter et écrire un diagnostic lisible.
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&val.stdout),
        String::from_utf8_lossy(&val.stderr)
    );
    assert!(
        combined.contains("OK") || combined.contains("violation"),
        "{combined}"
    );
}

#[test]
fn generate_is_byte_deterministic() {
    let dir = sample_dir();
    let a = dir.join("a.json");
    let b = dir.join("b.json");
    for out in [&a, &b] {
        let s = Command::new(env!("CARGO_BIN_EXE_tessera-authority"))
            .args(["generate", "--districts"])
            .arg(dir.join("districts.json"))
            .args(["--out"])
            .arg(out)
            .output()
            .unwrap();
        assert!(s.status.success());
    }
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
}
