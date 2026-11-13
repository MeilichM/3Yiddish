fn main() {
    // The core's version (the repository's Cargo.toml), which Send tells 3Yiddish, as the
    // terminal app does.
    let core = std::path::Path::new("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", core.display());
    let version = std::fs::read_to_string(core)
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix("version = \"")?.strip_suffix('"').map(str::to_owned))
        })
        .unwrap_or_else(|| "0.0.0".into());
    println!("cargo:rustc-env=CORE_VERSION={version}");
    tauri_build::build()
}
