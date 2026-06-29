use std::process::Command;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    for schema in ["protocol.fbs", "internal.fbs"] {
        println!("cargo:rerun-if-changed=schema/{schema}");
        let status = Command::new("flatc")
            .args(["--rust", "-o", &out_dir, &format!("schema/{schema}")])
            .status()
            .expect("échec d'exécution de flatc — installer avec `brew install flatbuffers`");
        assert!(status.success(), "flatc a échoué sur {schema}");
    }
}
