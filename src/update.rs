//! Self-update from GitHub releases. Each release carries one raw binary per platform
//! (`3yiddish-<target>[.exe]`), a graphics-card (Vulkan) build beside it for Windows and
//! Linux x86_64 (`3yiddish-<target>-vulkan[.exe]`), and a `SHA256SUMS` file.
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub const REPO: &str = "TripleU613/3Yiddish";

/// This build's platform, matching the release asset names.
pub fn target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("android", "aarch64") => "aarch64-linux-android",
        _ => "unsupported",
    }
}

/// Which build this is: "vulkan" for the graphics-card build, "" for the regular one.
/// macOS always uses the graphics card (Metal) and has a single build.
pub fn variant() -> &'static str {
    if cfg!(feature = "vulkan") {
        "vulkan"
    } else {
        ""
    }
}

/// The release file name for a platform. A Vulkan build keeps updating to Vulkan builds,
/// which exist only for Windows and Linux x86_64; anything else gets the regular file.
pub fn asset_name(target: &str, vulkan: bool) -> String {
    let gpu = vulkan
        && matches!(
            target,
            "x86_64-unknown-linux-gnu" | "x86_64-pc-windows-msvc"
        );
    let variant = if gpu { "-vulkan" } else { "" };
    let exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("3yiddish-{target}{variant}{exe}")
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("3yiddish/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// The newest released version, from the /releases/latest redirect (no rate-limited API).
pub fn latest() -> Result<String> {
    let response = client()?
        .get(format!("https://github.com/{REPO}/releases/latest"))
        .send()?;
    let location = response
        .headers()
        .get("location")
        .and_then(|l| l.to_str().ok())
        .context("GitHub did not name a latest release")?;
    let tag = location.rsplit('/').next().unwrap_or_default();
    Ok(tag.trim_start_matches('v').to_owned())
}

pub fn newer(have: &str, want: &str) -> bool {
    let parse = |v: &str| {
        v.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(want) > parse(have)
}

/// Replace the running binary with the latest release. Returns the new version, or None
/// when already up to date.
pub fn update() -> Result<Option<String>> {
    let version = latest()?;
    if !newer(env!("CARGO_PKG_VERSION"), &version) {
        return Ok(None);
    }
    let target = target();
    if target == "unsupported" {
        bail!("No release build for this platform");
    }
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let asset = asset_name(target, variant() == "vulkan");
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let download = reqwest::blocking::Client::builder()
        .user_agent(concat!("3yiddish/", env!("CARGO_PKG_VERSION")))
        .timeout(None)
        .build()?;
    let sums = download
        .get(format!("{base}/SHA256SUMS"))
        .send()?
        .error_for_status()?
        .text()?;
    let want = sums
        .lines()
        .find_map(|l| {
            l.split_once("  ")
                .filter(|(_, name)| name.trim() == asset)
                .map(|(h, _)| h.to_owned())
        })
        .context("The release has no checksum for this platform")?;
    let bytes = download
        .get(format!("{base}/{asset}"))
        .send()?
        .error_for_status()?
        .bytes()?;
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        bail!("The downloaded update failed its checksum; nothing was changed");
    }
    let tmp = std::env::temp_dir().join(format!("3yiddish-update-{}{exe}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    let result = self_replace::self_replace(&tmp);
    let _ = std::fs::remove_file(&tmp);
    result.context("Cannot replace the program file (is it installed somewhere read-only?)")?;
    Ok(Some(version))
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare_numerically() {
        assert!(super::newer("0.1.9", "0.1.10") && !super::newer("0.2.0", "0.2.0"));
        assert_ne!(super::target(), "");
    }

    #[test]
    fn asset_names_follow_the_build_variant() {
        use super::asset_name;
        let linux = "x86_64-unknown-linux-gnu";
        let windows = "x86_64-pc-windows-msvc";
        assert_eq!(asset_name(linux, false), format!("3yiddish-{linux}"));
        assert_eq!(asset_name(linux, true), format!("3yiddish-{linux}-vulkan"));
        assert_eq!(
            asset_name(windows, false),
            format!("3yiddish-{windows}.exe")
        );
        assert_eq!(
            asset_name(windows, true),
            format!("3yiddish-{windows}-vulkan.exe")
        );
        // No Vulkan release exists for these, so they always name the regular file.
        for target in [
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-linux-android",
        ] {
            assert_eq!(asset_name(target, true), format!("3yiddish-{target}"));
        }
        assert_eq!(super::variant() == "vulkan", cfg!(feature = "vulkan"));
    }
}
