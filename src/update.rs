//! Self-update from GitHub releases. Each release carries one raw binary per platform
//! (`3yiddish-<target>[.exe]`) and a `SHA256SUMS` file; nothing else is needed.
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
        _ => "unsupported",
    }
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
    let asset = format!("3yiddish-{target}{exe}");
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
}
