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
