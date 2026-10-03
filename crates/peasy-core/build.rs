// Embed catalogues automatically: adding a language needs no Rust UI changes.
use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=src/locales");
    let mut files: Vec<_> = fs::read_dir("src/locales")
        .expect("locales directory")
        .map(|entry| entry.expect("locale entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    let mut source = String::from("const CATALOGUES: &[(&str, &str)] = &[\n");
    for path in files {
        let code = path.file_stem().unwrap().to_str().unwrap();
        assert!(
            code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "invalid locale code"
        );
        source.push_str(&format!(
            "({code:?}, include_str!({path:?})),\n",
            path = path.canonicalize().unwrap()
        ));
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("locales.rs"),
        source,
    )
    .unwrap();
}
