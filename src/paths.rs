//! Where 3Yiddish keeps its files.
use anyhow::{Context, Result};
use std::path::PathBuf;

/// The app's home: `$THREE_YIDDISH_HOME`, else the current folder when it already has
/// `state/` (running from a checkout), else the per-user data folder
/// (~/.local/share/3yiddish, %APPDATA%\3yiddish, ~/Library/Application Support/3yiddish),
/// or the folder named in that folder's `home` file.
pub fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("THREE_YIDDISH_HOME") {
        return Ok(PathBuf::from(h));
    }
    let here = std::env::current_dir()?;
    let data = directories::ProjectDirs::from("", "", "3yiddish")
        .context("No home folder for this user")?
        .data_dir()
        .to_path_buf();
    if here.join("state").is_dir() && here != data {
        return Ok(here);
    }
    // A `home` file in the per-user folder points the app at another folder, e.g. a
    // developer checkout, so it finds the same data from wherever it is started.
    if let Ok(target) = std::fs::read_to_string(data.join("home")) {
        let target = PathBuf::from(target.trim());
        if target.join("state").is_dir() {
            return Ok(target);
        }
    }
    Ok(data)
}

/// Move into the home folder so every `state/...` path resolves there.
// ponytail: one chdir at startup instead of threading a base path through every file path.
pub fn enter_home() -> Result<PathBuf> {
    let home = home()?;
    std::fs::create_dir_all(home.join("state"))?;
    std::env::set_current_dir(&home)?;
    Ok(home)
}

/// Downloaded models: state/models/<version>/ plus `current`/`previous` version files.
pub fn models() -> PathBuf {
    PathBuf::from("state/models")
}

/// The user's saved GPU/CPU choice (`gpu` or `cpu`); no file means automatic.
pub fn compute() -> PathBuf {
    PathBuf::from("state/compute")
}

/// Present where the playground (the phone benchmark) is switched on:
/// `3yiddish playground on`.
pub fn playground_marker() -> PathBuf {
    PathBuf::from("state/playground-on")
}

/// Written when someone reads the Contribute explanation and accepts it (the number of the
/// text they accepted). From then on the app's Contribute button opens the playground.
pub fn contribute_accepted() -> PathBuf {
    PathBuf::from("state/contribute-accepted")
}

/// The name a contributor last sent takes under.
pub fn contributor_name() -> PathBuf {
    PathBuf::from("state/contributor-name")
}

/// A contributor's takes, kept apart from the owner's own set in `free_bench`.
pub fn contributions() -> PathBuf {
    PathBuf::from("state/contributions")
}

/// The playground's takes: `items.jsonl` and `audio/`. A held-out exam, never training data.
pub fn free_bench() -> PathBuf {
    PathBuf::from("state/free-bench")
}

/// The code in the phone's pairing link (owner-only file).
pub fn pair_code() -> PathBuf {
    PathBuf::from("state/phone-pair-code")
}

/// The Tailscale HTTPS port a pairing set up, so it can be taken down after a crash.
pub fn pair_serve() -> PathBuf {
    PathBuf::from("state/phone-pair-serve")
}

/// The answer to "Share what you transcribe?": `yes` or `no`; no file means never asked.
pub fn share() -> PathBuf {
    PathBuf::from("state/share")
}

/// What sharing has queued and not sent yet (deleted when sharing is switched off).
pub fn share_queue() -> PathBuf {
    PathBuf::from("state/share-queue")
}
