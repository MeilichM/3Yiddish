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
    link_clang_runtime();
    tauri_build::build()
}

/// whisper.cpp's Metal code asks at runtime whether the Mac has macOS 15 (`@available`).
/// Release builds target an older macOS, so that check stays in and needs
/// `__isPlatformVersionAtLeast` from clang's runtime, which Rust's linking leaves out:
/// link it in from the clang that compiled whisper.cpp.
fn link_clang_runtime() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Ok(out) = std::process::Command::new("clang").arg("--print-resource-dir").output() else {
        return;
    };
    let dir = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("lib/darwin");
    if dir.join("libclang_rt.osx.a").exists() {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    }
}
