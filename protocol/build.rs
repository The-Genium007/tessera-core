use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    println!("cargo:rerun-if-changed=schema/protocol.fbs");
    let status = Command::new("flatc")
        .args(["--rust", "-o", &out_dir, "schema/protocol.fbs"])
        .status()
        .expect("échec d'exécution de flatc — installer avec `brew install flatbuffers`");
    assert!(status.success(), "flatc a échoué");
}
