//! The Transcribe model store. GitHub hosts exactly one model, the latest, under the release
//! tag `model-latest`; `model.json` there names its version, files, sizes and SHA-256 sums.
//! Locally: models/<version>/ holds the files, models/current names the active version and
//! models/previous the one before it (kept for an instant rollback).
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const DEFAULT_INDEX: &str =
    "https://github.com/TripleU613/3Yiddish/releases/download/model-latest/model.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub files: Vec<File>,
    #[serde(default)]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub scores: serde_json::Value,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct File {
    /// Role: "transcribe" (whisper.cpp ggml model) or "vad" (Silero VAD).
    pub role: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

fn index_url() -> String {
    std::env::var("THREE_YIDDISH_MODEL_INDEX").unwrap_or_else(|_| DEFAULT_INDEX.into())
}

/// The active model: (version, transcribe model path, VAD model path if shipped).
pub fn current(root: &Path) -> Option<(String, PathBuf, Option<PathBuf>)> {
    let version = std::fs::read_to_string(root.join("current"))
        .ok()?
        .trim()
        .to_owned();
    let dir = root.join(&version);
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(dir.join("model.json")).ok()?).ok()?;
    let path = |role: &str| {
        manifest
            .files
            .iter()
            .find(|f| f.role == role)
            .map(|f| dir.join(&f.name))
    };
    let model = path("transcribe").filter(|p| p.is_file())?;
    Some((version, model, path("vad").filter(|p| p.is_file())))
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !name.starts_with('.')
}

fn version_newer(have: &str, want: &str) -> bool {
    let parse = |v: &str| {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(want) > parse(have)
}

/// The version the GitHub model index currently names.
pub fn latest_version() -> Result<String> {
    let manifest: Manifest = serde_json::from_reader(http_get(&index_url())?.take(1024 * 1024))?;
    Ok(manifest.version)
}

/// Check GitHub for a newer model; download, verify and activate it. Returns the version now
/// active. `progress` gets (file name, bytes done, bytes total).
pub fn pull(root: &Path, progress: &mut dyn FnMut(&str, u64, u64)) -> Result<String> {
    let index = index_url();
    let manifest: Manifest = serde_json::from_reader(http_get(&index)?.take(1024 * 1024))
        .context("The model index is not valid")?;
    if !safe_name(&manifest.version) || manifest.files.iter().any(|f| !safe_name(&f.name)) {
        bail!("The model index has unsafe names");
    }
    if let Some(min) = &manifest.min_app_version
        && version_newer(env!("CARGO_PKG_VERSION"), min)
    {
        bail!("This model needs 3Yiddish {min} or newer; run `3yiddish update` first");
    }
    if current(root).is_some_and(|(v, ..)| v == manifest.version) {
        return Ok(manifest.version);
    }
    let base = index
        .rsplit_once('/')
        .map_or(index.as_str(), |(b, _)| b)
        .to_owned();
    let dir = root.join(&manifest.version);
    std::fs::create_dir_all(&dir)?;
    for f in &manifest.files {
        let target = dir.join(&f.name);
        if target.is_file() && sha256_file(&target)? == f.sha256 {
            continue;
        }
        let part = dir.join(format!("{}.part", f.name));
        download(&format!("{base}/{}", f.name), &part, f.size, &mut |done| {
            progress(&f.name, done, f.size)
        })?;
        let got = sha256_file(&part)?;
        if got != f.sha256 {
            let _ = std::fs::remove_file(&part);
            bail!("{} failed its checksum; try again", f.name);
        }
        std::fs::rename(&part, &target)?;
    }
    std::fs::write(
        dir.join("model.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    activate(root, &manifest.version)?;
    Ok(manifest.version)
}

/// Point `current` at a version, keep the old one as `previous`, delete anything older.
fn activate(root: &Path, version: &str) -> Result<()> {
    let old = std::fs::read_to_string(root.join("current"))
        .ok()
        .map(|s| s.trim().to_owned());
    if let Some(old) = old.as_deref().filter(|o| *o != version) {
        write_atomic(&root.join("previous"), old)?;
    }
    write_atomic(&root.join("current"), version)?;
    let keep = [Some(version.to_owned()), old];
    for entry in std::fs::read_dir(root)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && !keep.iter().flatten().any(|k| *k == name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

/// Swap back to the previous model.
pub fn rollback(root: &Path) -> Result<String> {
    let previous = std::fs::read_to_string(root.join("previous"))
        .context("No previous model to roll back to")?;
    let previous = previous.trim().to_owned();
    if !root.join(&previous).is_dir() {
        bail!("The previous model is no longer on disk");
    }
    activate(root, &previous)?;
    Ok(previous)
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Blocking HTTPS GET that follows GitHub's redirect to its download CDN.
fn http_get(url: &str) -> Result<Box<dyn Read + Send>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("3yiddish/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get(url)
        .send()?
        .error_for_status()
        .with_context(|| format!("Cannot reach {url}"))?;
    Ok(Box::new(response))
}

/// Download with resume: continues a `.part` file with an HTTP Range request.
fn download(url: &str, part: &Path, size: u64, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have > size {
        std::fs::remove_file(part)?;
    }
    let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have == size {
        return Ok(());
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("3yiddish/", env!("CARGO_PKG_VERSION")))
        .timeout(None)
        .build()?;
    let mut response = client
        .get(url)
        .header("Range", format!("bytes={have}-"))
        .send()?
        .error_for_status()?;
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(true)
        .truncate(!resumed)
        .open(part)?;
    let mut done = if resumed { have } else { 0 };
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = response.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done > size {
            bail!("Download is larger than the index says");
        }
        progress(done);
    }
    out.flush()?;
    if done != size {
        bail!("Download stopped early; run it again to resume");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activates_keeps_previous_and_rolls_back() {
        let root = std::env::temp_dir().join(format!("3y-models-{}", uuid::Uuid::new_v4()));
        for v in ["1", "2", "3"] {
            let dir = root.join(v);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("m.bin"), v).unwrap();
            let manifest = Manifest {
                version: v.into(),
                files: vec![File {
                    role: "transcribe".into(),
                    name: "m.bin".into(),
                    size: 1,
                    sha256: String::new(),
                }],
                min_app_version: None,
                scores: serde_json::Value::Null,
            };
            std::fs::write(
                dir.join("model.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            activate(&root, v).unwrap();
        }
        assert_eq!(current(&root).unwrap().0, "3");
        assert!(
            !root.join("1").exists(),
            "only current and previous are kept"
        );
        assert_eq!(rollback(&root).unwrap(), "2");
        assert_eq!(current(&root).unwrap().0, "2");
        assert!(safe_name("ggml-q5_0.bin") && !safe_name("../x") && !safe_name(".hidden"));
        assert!(version_newer("0.1.0", "0.2.0") && !version_newer("0.2.0", "0.1.9"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
