use std::path::Path;

const DEFAULT_API_BASE: &str = "http://localhost:3000";

fn load_dotenv() {
    let mut api_base = DEFAULT_API_BASE.to_string();

    if let Ok(contents) = std::fs::read_to_string(".env") {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if key.trim() == "VITADECK_API_BASE" && !value.is_empty() {
                api_base = value.to_string();
            }
        }
    }

    let api_base = api_base.trim_end_matches('/');
    println!("cargo:rustc-env=VITADECK_API_BASE={api_base}");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/dummy.c");
    if Path::new(".env").exists() {
        println!("cargo:rerun-if-changed=.env");
    }
    load_dotenv();
    cc::Build::new()
        .file("src/dummy.c")
        .compile("dummy_c");
}
