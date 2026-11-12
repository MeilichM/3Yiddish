//! Termux only: a font in which Yiddish reads well.
//!
//! Termux draws every character in a cell of one width. The phone's own font has no letters
//! of that width for Yiddish, so Android borrows them from another font and Termux stretches
//! each one to fill its cell: the slim letters (vav, yud, final nun) come out as blobs. A
//! font that has the letters at the cell's width fixes it. Termux takes its font from
//! `~/.termux/font.ttf`, which is the user's own setting for the whole terminal, so it is
//! only written when the user asks (the "Get font" button, or `3yiddish font`), and never
//! over a font that is already there.
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

/// Cousine (SIL Open Font License 1.1) as built by the Nerd Fonts project, release v3.5.1:
/// one fixed width for Latin and Yiddish letters, plus the line and dot characters the app
/// draws with.
const URL: &str = "https://raw.githubusercontent.com/ryanoasis/nerd-fonts/v3.5.1/patched-fonts/Cousine/CousineNerdFontMono-Regular.ttf";
const SIZE: u64 = 2_768_760;
const SHA256: &str = "6b00669c50a3d02fd73488325d8ce6a92b5b78a6630fb81220c4f4153799ee9b";

/// Running in Termux on an Android phone.
pub fn termux() -> bool {
    cfg!(target_os = "android") || std::env::var_os("TERMUX_VERSION").is_some()
}

/// Where Termux looks for the user's font.
fn file() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".termux/font.ttf"))
}

/// In Termux with the phone's own font: worth offering the font.
pub fn wanted() -> bool {
    termux() && file().is_some_and(|f| !f.exists())
}

/// Download the font, check it, put it in place and tell Termux to use it. `progress` gets
/// (bytes done, bytes total).
pub fn install(progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    let file = file().context("No home folder")?;
    if file.exists() {
        bail!("Termux already has a font of your own; it was left alone");
    }
    let folder = file.parent().context("No home folder")?;
    std::fs::create_dir_all(folder)?;
    let part = folder.join("font.ttf.part");
    // Another address for tests; the file must still match the pinned checksum.
    let url = std::env::var("THREE_YIDDISH_FONT_URL").unwrap_or_else(|_| URL.to_owned());
    crate::model::download(&url, &part, SIZE, &mut |done| progress(done, SIZE))
        .context("The font download did not finish")?;
    if crate::model::sha256_file(&part)? != SHA256 {
        let _ = std::fs::remove_file(&part);
        bail!("The font failed its checksum; try again");
    }
    std::fs::rename(&part, &file)?;
    // Termux picks the new font up at once (part of Termux itself, no add-on needed). If
    // that fails, it shows the next time Termux is opened.
    let _ = std::process::Command::new("termux-reload-settings")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(())
}

/// How to go back to the phone's own font.
pub const UNDO: &str = "rm ~/.termux/font.ttf; termux-reload-settings";
