//! The Transcribe model store. GitHub hosts the published models under the release tag
//! `model-latest`: `model.json` there is the main model (version, files, sizes, SHA-256 sums)
//! and names, in `also`, the same kind of file for each further model.
//! Locally: models/<version>/ holds the files, models/current names the active version,
//! models/previous the one before it (kept for an instant rollback) and models/index the
//! versions that were published at the last download (kept, and offered for switching).
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const DEFAULT_INDEX: &str =
    "https://github.com/TripleU613/3Yiddish/releases/download/model-latest/model.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub files: Vec<File>,
    #[serde(default)]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub scores: serde_json::Value,
    /// The model was trained to write plain text, without time marks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plain_text: bool,
    /// How hard the word check pushes towards the words in the "words" file; absent: off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_check: Option<f32>,
    /// Stop a phrase from being written a third time in a row.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub loop_guard: bool,
    /// The kind of model: absent for Whisper (a ggml file), "omnilingual" for an Omnilingual
    /// one (an ONNX file, with its letter table under the role "letters").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// A model put here by hand: the check for a newer published model leaves it alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
    /// The name shown in the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// In the published main model only: the files (beside it) that describe further models.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct File {
    /// Role: "transcribe" (whisper.cpp ggml model), "vad" (Silero VAD) or "words" (the word
    /// list for the word check, one word per line).
    pub role: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    /// Where to download it, when it is not beside the list (chat's files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The build of the app it is for (chat's programs); absent: every build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
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

/// How the model in this file wants to be run: what the `model.json` beside it says. A model
/// file on its own (no `model.json`) runs with the plain settings.
pub fn settings(model: &Path) -> crate::asr::Settings {
    let Some(dir) = model.parent() else {
        return Default::default();
    };
    let Some(manifest) = std::fs::read(dir.join("model.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
    else {
        return Default::default();
    };
    let file = |role: &str| {
        manifest
            .files
            .iter()
            .find(|f| f.role == role)
            .map(|f| dir.join(&f.name))
            .filter(|p| p.is_file())
    };
    crate::asr::Settings {
        plain_text: manifest.plain_text,
        word_weight: manifest.word_check.unwrap_or(0.0),
        words: file("words"),
        loop_guard: manifest.loop_guard,
        omnilingual: manifest.engine.as_deref() == Some("omnilingual"),
        letters: file("letters"),
    }
}

fn manifest_of(dir: &Path) -> Option<Manifest> {
    serde_json::from_slice(&std::fs::read(dir.join("model.json")).ok()?).ok()
}

/// Can this build run the model? The Omnilingual engine is not in every build.
fn runs_here(manifest: &Manifest) -> bool {
    match manifest.engine.as_deref() {
        None => true,
        Some("omnilingual") => cfg!(feature = "omni"),
        Some(_) => false,
    }
}

/// The versions that were published at the last download; none before the first one.
fn listed(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("index"))
        .map(|text| text.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Every model on disk that can be run and switched to: (version, the name to show), in a
/// fixed order. Once a download has said what is published, an older model kept only for
/// rolling back is not among them.
pub fn installed(root: &Path) -> Vec<(String, String)> {
    let listed = listed(root);
    let now = std::fs::read_to_string(root.join("current")).unwrap_or_default();
    let mut found: Vec<(String, String)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let manifest = manifest_of(&entry.path())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !runs_here(&manifest)
                || !(listed.is_empty()
                    || manifest.local
                    || name == now.trim()
                    || listed.contains(&name))
            {
                return None;
            }
            let model = manifest.files.iter().find(|f| f.role == "transcribe")?;
            entry.path().join(&model.name).is_file().then(|| {
                let label = manifest.label.unwrap_or_else(|| manifest.version.clone());
                (entry.file_name().to_string_lossy().into_owned(), label)
            })
        })
        .collect();
    found.sort();
    found
}

/// Whether the active model was put here by hand (not the published one).
pub fn current_is_local(root: &Path) -> bool {
    current(root)
        .and_then(|(version, ..)| manifest_of(&root.join(version)))
        .is_some_and(|m| m.local)
}

/// Make the next installed model the active one; returns its (version, name). Nothing is
/// deleted. With one model installed it stays as it is.
pub fn switch_next(root: &Path) -> Result<(String, String)> {
    let all = installed(root);
    let now = current(root).map(|(v, ..)| v);
    let at = all.iter().position(|(v, _)| Some(v) == now.as_ref());
    let next = all
        .get(at.map_or(0, |i| (i + 1) % all.len().max(1)))
        .context("No model installed")?
        .clone();
    if Some(&next.0) != now.as_ref() {
        if let Some(old) = now {
            write_atomic(&root.join("previous"), &old)?;
        }
        write_atomic(&root.join("current"), &next.0)?;
    }
    Ok(next)
}

pub(crate) fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && !name.starts_with('.')
}

pub(crate) fn version_newer(have: &str, want: &str) -> bool {
    let parse = |v: &str| {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    parse(want) > parse(have)
}

/// A download error in one plain line: no connection is said as such, without the chain of
/// causes underneath it.
pub fn plain_error(error: &anyhow::Error) -> String {
    let offline = error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|e| e.is_connect() || e.is_timeout())
    });
    if offline {
        "No internet connection; try again later.".into()
    } else {
        format!("{error:#}")
    }
}

/// The published models this build can run: where their files are, and the main one first.
fn published() -> Result<(String, Vec<Manifest>)> {
    let index = index_url();
    let base = index
        .rsplit_once('/')
        .map_or(index.as_str(), |(b, _)| b)
        .to_owned();
    let read = |url: &str| -> Result<Manifest> {
        serde_json::from_reader(http_get(url)?.take(1024 * 1024))
            .context("The model index is not valid")
    };
    let main = read(&index)?;
    let mut all = Vec::new();
    for name in &main.also {
        if !safe_name(name) {
            bail!("The model index has unsafe names");
        }
        all.push(read(&format!("{base}/{name}"))?);
    }
    all.insert(0, main);
    all.retain(runs_here);
    Ok((base, all))
}

/// Are all of the model's files on disk, at their full size?
fn complete(root: &Path, manifest: &Manifest) -> bool {
    let dir = root.join(&manifest.version);
    dir.join("model.json").is_file()
        && manifest.files.iter().all(|f| {
            std::fs::metadata(dir.join(&f.name)).is_ok_and(|m| m.is_file() && m.len() == f.size)
        })
}

/// The kinds of model on disk (None: Whisper).
fn kinds(root: &Path) -> Vec<Option<String>> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| manifest_of(&entry.path()))
        .map(|m| m.engine)
        .collect()
}

/// Is there a newer version of a kind of model already on disk? A model nobody downloaded
/// is not an update: it is offered when someone asks for it.
pub fn wanted(root: &Path) -> Result<bool> {
    let have = kinds(root);
    Ok(published()?
        .1
        .iter()
        .any(|m| have.contains(&m.engine) && !complete(root, m)))
}

/// A model that can be picked in the app: published, or already on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub version: String,
    /// The name shown in the app.
    pub label: String,
    /// Bytes to download (all of its files).
    pub size: u64,
    pub installed: bool,
}

/// Every model this build can run, published or on disk, the main published one first.
/// Without a connection, only what is on disk.
pub fn catalog(root: &Path) -> Vec<Listing> {
    let on_disk = installed(root);
    let mut list: Vec<Listing> = published()
        .map(|(_, all)| all)
        .unwrap_or_default()
        .into_iter()
        .map(|m| Listing {
            installed: complete(root, &m),
            label: m.label.clone().unwrap_or_else(|| m.version.clone()),
            size: m.files.iter().map(|f| f.size).sum(),
            version: m.version,
        })
        .collect();
    for (version, label) in on_disk {
        if !list.iter().any(|l| l.version == version) {
            list.push(Listing {
                version,
                label,
                size: 0,
                installed: true,
            });
        }
    }
    list
}

/// Make an installed model the active one.
pub fn use_version(root: &Path, version: &str) -> Result<()> {
    if !installed(root).iter().any(|(v, _)| v == version) {
        bail!("That model is not on this computer");
    }
    activate(root, version)
}

/// Download one published model and make it the active one.
pub fn pull_one(
    root: &Path,
    version: &str,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<String> {
    let (base, all) = published()?;
    if !all.iter().any(|m| m.version == version) {
        bail!("That model is no longer published; try again later");
    }
    install(
        root,
        &all,
        Some(&|m: &Manifest| m.version == version),
        &mut |file, part, progress| {
            download(&format!("{base}/{}", file.name), part, file.size, progress)
        },
        progress,
    )?;
    // Asked for by name: it is the one wanted now.
    activate(root, version)?;
    Ok(version.to_owned())
}

/// Download the newer versions of the kinds of model on disk (the main one when there is
/// none yet), keeping the model in use where it is still published.
pub fn pull_updates(root: &Path, progress: &mut dyn FnMut(&str, u64, u64)) -> Result<String> {
    let (base, all) = published()?;
    let have = kinds(root);
    let main = all.first().map(|m| m.version.clone());
    let pick = |m: &Manifest| {
        if have.is_empty() {
            Some(&m.version) == main.as_ref()
        } else {
            have.contains(&m.engine)
        }
    };
    install(
        root,
        &all,
        Some(&pick),
        &mut |file, part, progress| {
            download(&format!("{base}/{}", file.name), part, file.size, progress)
        },
        progress,
    )
}

/// Check GitHub for published models that are not here; download and verify them. A new
/// main model becomes the active one, except for someone using a model that is still
/// published (it stays) or one of another kind that has its own newer version (that one
/// takes over). Returns the version now active. `progress` gets (file name, bytes done,
/// bytes total).
pub fn pull(root: &Path, progress: &mut dyn FnMut(&str, u64, u64)) -> Result<String> {
    let (base, all) = published()?;
    install(
        root,
        &all,
        None,
        &mut |file, part, progress| {
            download(&format!("{base}/{}", file.name), part, file.size, progress)
        },
        progress,
    )
}

/// Gets one file of a model into the given place, reporting the bytes done.
type Fetch<'a> = &'a mut dyn FnMut(&File, &Path, &mut dyn FnMut(u64)) -> Result<()>;

/// Which of the published models to download; None: all of them.
type Pick<'a> = Option<&'a dyn Fn(&Manifest) -> bool>;

/// Put the published models `all` (main one first) on disk, getting each file with `fetch`.
/// With `pick`, only the models it picks.
fn install(
    root: &Path,
    all: &[Manifest],
    pick: Pick,
    fetch: Fetch,
    progress: &mut dyn FnMut(&str, u64, u64),
) -> Result<String> {
    let main = all.first().context("The model index names no model")?;
    for manifest in all {
        if !safe_name(&manifest.version) || manifest.files.iter().any(|f| !safe_name(&f.name)) {
            bail!("The model index has unsafe names");
        }
        if let Some(min) = &manifest.min_app_version
            && version_newer(env!("CARGO_PKG_VERSION"), min)
        {
            bail!("This model needs 3Yiddish {min} or newer; run `3yiddish update` first");
        }
    }
    std::fs::create_dir_all(root)?;
    let versions: Vec<&str> = all.iter().map(|m| m.version.as_str()).collect();
    write_atomic(&root.join("index"), &versions.join("\n"))?;
    let chosen: Vec<&Manifest> = all.iter().filter(|m| pick.is_none_or(|p| p(m))).collect();
    for &manifest in &chosen {
        let dir = root.join(&manifest.version);
        if !complete(root, manifest) {
            std::fs::create_dir_all(&dir)?;
            for f in &manifest.files {
                let target = dir.join(&f.name);
                if target.is_file() && sha256_file(&target)? == f.sha256 {
                    continue;
                }
                let part = dir.join(format!("{}.part", f.name));
                fetch(f, &part, &mut |done| progress(&f.name, done, f.size))?;
                let got = sha256_file(&part)?;
                if got != f.sha256 {
                    let _ = std::fs::remove_file(&part);
                    bail!("{} failed its checksum; try again", f.name);
                }
                std::fs::rename(&part, &target)?;
            }
            std::fs::write(dir.join("model.json"), serde_json::to_vec_pretty(manifest)?)?;
        }
        // The first model is usable (and active) before the next download starts.
        let now = current(root).map(|(v, ..)| v);
        let takes_over = match &now {
            None => true,
            Some(v) if versions.contains(&v.as_str()) => false,
            Some(v) => {
                let kind = manifest_of(&root.join(v)).and_then(|m| m.engine);
                manifest.engine == kind
                    || (manifest.version == main.version && !all.iter().any(|m| m.engine == kind))
            }
        };
        if takes_over {
            activate(root, &manifest.version)?;
        }
    }
    tidy(root);
    current(root)
        .map(|(v, ..)| v)
        .context("The model did not install")
}

/// Point `current` at a version, keep the old one as `previous`, delete what is left over.
fn activate(root: &Path, version: &str) -> Result<()> {
    let old = std::fs::read_to_string(root.join("current"))
        .ok()
        .map(|s| s.trim().to_owned());
    if let Some(old) = old.as_deref().filter(|o| *o != version) {
        write_atomic(&root.join("previous"), old)?;
    }
    write_atomic(&root.join("current"), version)?;
    tidy(root);
    Ok(())
}

/// Delete the models nobody needs: not the active one, not the one before it, not one that
/// is published, and never one put here by hand.
fn tidy(root: &Path) {
    let read = |name: &str| {
        std::fs::read_to_string(root.join(name))
            .ok()
            .map(|s| s.trim().to_owned())
    };
    let mut keep = listed(root);
    keep.extend(read("current"));
    keep.extend(read("previous"));
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let local = manifest_of(&entry.path()).is_some_and(|m| m.local);
        if entry.path().is_dir() && !local && !keep.contains(&name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
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

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
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
pub(crate) fn http_get(url: &str) -> Result<Box<dyn Read + Send>> {
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
pub(crate) fn download(
    url: &str,
    part: &Path,
    size: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
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
                    ..Default::default()
                }],
                min_app_version: None,
                scores: serde_json::Value::Null,
                plain_text: v == "3",
                word_check: (v == "3").then_some(2.5),
                loop_guard: v == "3",
                engine: None,
                local: false,
                label: None,
                also: vec![],
            };
            std::fs::write(
                dir.join("model.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            activate(&root, v).unwrap();
        }
        assert_eq!(current(&root).unwrap().0, "3");
        // Each model carries its own way of being run; an older one has none.
        let new = settings(&root.join("3").join("m.bin"));
        assert!(new.plain_text && new.loop_guard && new.word_weight == 2.5);
        assert!(new.words.is_none(), "no word list file was shipped");
        let old = settings(&root.join("2").join("m.bin"));
        assert!(!old.plain_text && !old.loop_guard && old.word_weight == 0.0);
        assert!(
            !root.join("1").exists(),
            "only current and previous are kept"
        );
        assert_eq!(rollback(&root).unwrap(), "2");
        assert_eq!(current(&root).unwrap().0, "2");
        // Switching walks through what is installed and back, deleting nothing.
        assert_eq!(installed(&root).len(), 2);
        assert_eq!(switch_next(&root).unwrap().0, "3");
        assert_eq!(switch_next(&root).unwrap().0, "2");
        assert!(root.join("3").exists() && !current_is_local(&root));
        assert!(safe_name("ggml-q5_0.bin") && !safe_name("../x") && !safe_name(".hidden"));
        assert!(version_newer("0.1.0", "0.2.0") && !version_newer("0.2.0", "0.1.9"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A published model whose one file holds its own name.
    fn published_model(version: &str, engine: Option<&str>) -> Manifest {
        let sum = Sha256::digest(version.as_bytes());
        Manifest {
            version: version.into(),
            files: vec![File {
                role: "transcribe".into(),
                name: format!("{version}.bin"),
                size: version.len() as u64,
                sha256: sum.iter().map(|b| format!("{b:02x}")).collect(),
                ..Default::default()
            }],
            min_app_version: None,
            scores: serde_json::Value::Null,
            plain_text: false,
            word_check: None,
            loop_guard: false,
            engine: engine.map(str::to_owned),
            local: false,
            label: None,
            also: vec![],
        }
    }

    /// Install `all`; returns the active version and how many files were downloaded.
    fn get(root: &Path, all: &[Manifest]) -> (String, usize) {
        let mut fetched = 0;
        let active = install(
            root,
            all,
            None,
            &mut |file, part, _| {
                fetched += 1;
                Ok(std::fs::write(part, file.name.trim_end_matches(".bin"))?)
            },
            &mut |_, _, _| {},
        )
        .unwrap();
        (active, fetched)
    }

    #[test]
    fn two_published_models_live_side_by_side() {
        let root = std::env::temp_dir().join(format!("3y-models-{}", uuid::Uuid::new_v4()));
        let omni = Some("omnilingual");
        // Someone with the one old model: the new main model takes over, the second comes too.
        let old = published_model("old", None);
        assert_eq!(get(&root, std::slice::from_ref(&old)), ("old".into(), 1));
        let first = [published_model("w1", None), published_model("o1", omni)];
        assert_eq!(get(&root, &first), ("w1".into(), 2));
        assert!(root.join("o1/o1.bin").is_file() && root.join("old").is_dir());
        // The old one is kept for rolling back, but is not one of the models to switch to.
        let shown = if cfg!(feature = "omni") { 2 } else { 1 };
        assert_eq!(installed(&root).len(), shown);
        assert!(installed(&root).iter().all(|(v, _)| v != "old"));
        // Nothing to get, and whoever uses the second model stays on it.
        write_atomic(&root.join("current"), "o1").unwrap();
        assert_eq!(get(&root, &first), ("o1".into(), 0));
        assert!(root.join("w1/w1.bin").is_file());
        // A file that went missing is fetched again without changing the model in use.
        std::fs::remove_file(root.join("w1/w1.bin")).unwrap();
        assert_eq!(get(&root, &first), ("o1".into(), 1));
        // New versions of both: the second model's user moves to its new version.
        let second = [published_model("w2", None), published_model("o2", omni)];
        assert_eq!(get(&root, &second), ("o2".into(), 2));
        assert!(root.join("w2").is_dir() && root.join("o1").is_dir());
        assert!(!root.join("w1").exists() && !root.join("old").exists());
        // Rolling back keeps everything that is published.
        assert_eq!(rollback(&root).unwrap(), "o1");
        assert!(root.join("w2").is_dir() && root.join("o2").is_dir());
        // A main model alone again: the second model's user moves to it.
        let third = [published_model("w3", None)];
        assert_eq!(get(&root, &third), ("w3".into(), 1));
        // A first install: the main model is the active one.
        let fresh = root.join("fresh");
        assert_eq!(get(&fresh, &second), ("w2".into(), 2));
        // A file that does not match its checksum is refused and nothing is activated.
        let empty = root.join("empty");
        let result = install(
            &empty,
            &third,
            None,
            &mut |_, part, _| Ok(std::fs::write(part, "something else")?),
            &mut |_, _, _| {},
        );
        assert!(result.is_err() && current(&empty).is_none());
        // Picking one model downloads only that one; the rest stay published, not fetched.
        let picked = root.join("picked");
        let mut fetched = 0;
        install(
            &picked,
            &second,
            Some(&|m: &Manifest| m.engine.is_none()),
            &mut |file, part, _| {
                fetched += 1;
                Ok(std::fs::write(part, file.name.trim_end_matches(".bin"))?)
            },
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(fetched, 1);
        assert!(root.join("picked/w2").is_dir() && !root.join("picked/o2").exists());
        assert_eq!(kinds(&picked), vec![None]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
