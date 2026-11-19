//! Chat on this computer. Two helper programs do the work, both downloaded only when someone
//! asks for chat, and started only while chat is in use:
//!
//! - the thinker: a language model that answers in English (Gemma), run by llama.cpp's
//!   `llama-server` (the official build for this platform, pinned by checksum);
//! - the translator: our NLLB model trained on heimish Yiddish, run by `3yiddish-translate`.
//!
//! What to download is one list, `chat.json`, beside the speech models on GitHub. Each file
//! names its role, its size and checksum, where to get it (`url`, when it is not beside the
//! list) and, for programs, the platform it is for. Locally: state/chat/<version>/ holds the
//! files and state/chat/current names the version in use.
use crate::model::{File, Manifest, download, sha256_file};
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

/// The name the thinker answers to.
pub const THINKER: &str = "gemma-4-e4b";

fn index_url() -> String {
    std::env::var("THREE_YIDDISH_CHAT_INDEX")
        .unwrap_or_else(|_| crate::model::DEFAULT_INDEX.replace("/model.json", "/chat.json"))
}

/// The downloaded chat: state/chat.
pub fn root() -> PathBuf {
    PathBuf::from("state/chat")
}

/// Which build of the helper programs this computer takes, or None where chat does not run
/// (phones). A graphics-card build of the app takes the graphics-card thinker.
pub fn platform() -> Option<&'static str> {
    let gpu = cfg!(feature = "vulkan");
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if gpu => "linux-x86_64-vulkan",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "aarch64") => "macos-aarch64",
        // Intel Macs took minutes per answer in testing: no chat there for now.
        ("windows", "x86_64") if gpu => "windows-x86_64-vulkan",
        ("windows", "x86_64") => "windows-x86_64",
        _ => return None,
    })
}

/// The files of the list that this computer needs.
fn wanted(manifest: &Manifest, platform: &str) -> Vec<File> {
    manifest
        .files
        .iter()
        .filter(|f| f.platform.as_deref().is_none_or(|p| p == platform))
        .cloned()
        .collect()
}

/// Where a file of the list goes inside its version's folder.
fn place(dir: &Path, file: &File) -> PathBuf {
    match file.role.as_str() {
        "translator" => dir.join("translator").join(&file.name),
        "translator-program" | "runtime" => dir.join("bin").join(&file.name),
        _ => dir.join(&file.name),
    }
}

/// A chat download that is complete: where its parts are.
#[derive(Clone, Debug)]
pub struct Installed {
    pub thinker: PathBuf,
    pub translator: PathBuf,
    pub translator_program: PathBuf,
    pub server: PathBuf,
}

/// A quick look, for the status asked every second: a chat download is marked as in use.
/// (`installed` checks every part.)
pub fn ready() -> bool {
    std::fs::read_to_string(root().join("current"))
        .is_ok_and(|v| root().join(v.trim()).join("chat.json").is_file())
}

/// The chat in use, if it is all here.
pub fn installed() -> Option<Installed> {
    // Whole paths: the thinker is started from its own folder.
    let root = std::path::absolute(root()).ok()?;
    let version = std::fs::read_to_string(root.join("current")).ok()?;
    let dir = root.join(version.trim());
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(dir.join("chat.json")).ok()?).ok()?;
    let files = wanted(&manifest, platform()?);
    let of = |role: &str| {
        files
            .iter()
            .find(|f| f.role == role)
            .map(|f| place(&dir, f))
    };
    let server_dir = dir.join("runtime");
    let server = find(
        &server_dir,
        if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        },
    )?;
    let found = Installed {
        thinker: of("thinker").filter(|p| p.is_file())?,
        translator: dir.join("translator"),
        translator_program: of("translator-program").filter(|p| p.is_file())?,
        server,
    };
    found
        .translator
        .join("model.bin")
        .is_file()
        .then_some(found)
}

/// The first file called `name` under `dir`.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        } else if entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

/// The published chat list.
fn published() -> Result<(String, Manifest)> {
    let index = index_url();
    let base = index
        .rsplit_once('/')
        .map_or(index.as_str(), |(b, _)| b)
        .to_owned();
    let manifest: Manifest = serde_json::from_reader(crate::model::http_get(&index)?.take(1 << 20))
        .context("The chat list is not valid")?;
    Ok((base, manifest))
}

/// How many bytes chat takes to download here; None where chat does not run.
pub fn size() -> Result<Option<u64>> {
    let Some(platform) = platform() else {
        return Ok(None);
    };
    let (_, manifest) = published()?;
    let files = wanted(&manifest, platform);
    if !files.iter().any(|f| f.role == "runtime") {
        return Ok(None);
    }
    Ok(Some(files.iter().map(|f| f.size).sum()))
}

/// Download chat (resuming, checking every file), unpack the thinker's program and make it
/// the chat in use. `progress` gets (bytes done, bytes in all).
pub fn pull(progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    let platform = platform().context("Chat does not run on this device")?;
    let (base, manifest) = published()?;
    ensure!(
        crate::model::safe_name(&manifest.version)
            && manifest
                .files
                .iter()
                .all(|f| crate::model::safe_name(&f.name)),
        "The chat list has unsafe names"
    );
    if let Some(min) = &manifest.min_app_version
        && crate::model::version_newer(env!("CARGO_PKG_VERSION"), min)
    {
        bail!("Chat needs 3Yiddish {min} or newer; update the app first");
    }
    let files = wanted(&manifest, platform);
    ensure!(
        files.iter().any(|f| f.role == "runtime"),
        "Chat does not run on this device yet"
    );
    let total: u64 = files.iter().map(|f| f.size).sum();
    let root = root();
    let dir = root.join(&manifest.version);
    let mut before = 0u64;
    for file in &files {
        let target = place(&dir, file);
        std::fs::create_dir_all(target.parent().context("No folder")?)?;
        if !(target.is_file() && sha256_file(&target)? == file.sha256) {
            let part = target.with_file_name(format!("{}.part", file.name));
            let url = file
                .url
                .clone()
                .unwrap_or_else(|| format!("{base}/{}", file.name));
            download(&url, &part, file.size, &mut |done| {
                progress(before + done, total)
            })?;
            if sha256_file(&part)? != file.sha256 {
                let _ = std::fs::remove_file(&part);
                bail!("{} failed its checksum; try again", file.name);
            }
            std::fs::rename(&part, &target)?;
        }
        before += file.size;
        progress(before, total);
        match file.role.as_str() {
            "runtime" => unpack(&target, &dir.join("runtime"))?,
            "translator-program" => executable(&target)?,
            _ => {}
        }
    }
    std::fs::write(dir.join("chat.json"), serde_json::to_vec_pretty(&manifest)?)?;
    stop();
    std::fs::write(root.join("current"), &manifest.version)?;
    // Older chat downloads go.
    for entry in std::fs::read_dir(&root)?.flatten() {
        if entry.path().is_dir() && entry.file_name() != manifest.version.as_str() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    installed().context("Chat did not install")?;
    Ok(())
}

fn executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = path;
    Ok(())
}

/// Unpack the thinker's program (a .tar.gz, or a .zip on Windows) into `to`, fresh.
fn unpack(archive: &Path, to: &Path) -> Result<()> {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to)?;
    let name = archive.to_string_lossy();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        zip.extract(to)?;
    } else {
        let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
        let mut tar = tar::Archive::new(gz);
        tar.set_preserve_permissions(true);
        tar.unpack(to)?;
    }
    let server = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let found = find(to, server).context("The thinker's program is missing from its download")?;
    executable(&found)
}

/// The helpers while they run.
struct Running {
    thinker: Child,
    translator: Child,
    thinker_url: String,
    translate_url: String,
    used: Instant,
}

impl Drop for Running {
    fn drop(&mut self) {
        for child in [&mut self.thinker, &mut self.translator] {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(root().join("running"));
    }
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Where the helpers answer, starting them first if they are not running: (the thinker's
/// chat address, the translator's address). Blocks while they start (the thinker loads its
/// model, which takes a while the first time). `gpu`: let the thinker use the graphics card.
pub fn ensure(gpu: bool) -> Result<(String, String)> {
    let mut slot = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(running) = slot.as_mut() {
        let alive = running.thinker.try_wait().ok().flatten().is_none()
            && running.translator.try_wait().ok().flatten().is_none();
        if alive {
            running.used = Instant::now();
            return Ok((running.thinker_url.clone(), running.translate_url.clone()));
        }
        *slot = None;
    }
    let parts = installed().context("Chat is not downloaded")?;
    stop_stale();
    let logs = root().join("logs");
    std::fs::create_dir_all(&logs)?;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let translator_port = free_port()?;
    let translator = Command::new(&parts.translator_program)
        .arg("--model")
        .arg(&parts.translator)
        .arg("--listen")
        .arg(format!("127.0.0.1:{translator_port}"))
        .arg("--threads")
        .arg((threads / 2).clamp(1, 8).to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(logs.join("translator.log"))?)
        .spawn()
        .context("Cannot start the translator")?;
    // Anything started is stopped again if the rest fails (dropping a Child would leave it).
    let mut translator = Some(translator);
    let mut thinker = match start_thinker(&parts, gpu, threads, &logs) {
        Ok(started) => started,
        Err(error) => {
            if let Some(mut child) = translator.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(error);
        }
    };
    let mut running = Running {
        thinker_url: format!("http://127.0.0.1:{}/v1/chat/completions", thinker.1),
        translate_url: format!("http://127.0.0.1:{translator_port}/v1/translate"),
        thinker: thinker.0,
        translator: translator.take().context("No translator")?,
        used: Instant::now(),
    };
    let pids = format!("{} {}", running.thinker.id(), running.translator.id());
    let _ = std::fs::write(root().join("running"), pids);
    wait_ready(
        &mut running.translator,
        &format!("http://127.0.0.1:{translator_port}/healthz"),
        Duration::from_secs(120),
    )
    .context("The translator did not start")?;
    if let Err(error) = wait_ready(
        &mut running.thinker,
        &format!("http://127.0.0.1:{}/health", thinker.1),
        Duration::from_secs(300),
    ) {
        if !gpu {
            return Err(error).context("The thinking model did not start");
        }
        // The graphics card may lack room for it next to the speech model: the processor.
        let _ = running.thinker.kill();
        let _ = running.thinker.wait();
        thinker = start_thinker(&parts, false, threads, &logs)?;
        // The old one is already stopped; this one replaces it in the guard.
        let old = std::mem::replace(&mut running.thinker, thinker.0);
        drop(old);
        running.thinker_url = format!("http://127.0.0.1:{}/v1/chat/completions", thinker.1);
        wait_ready(
            &mut running.thinker,
            &format!("http://127.0.0.1:{}/health", thinker.1),
            Duration::from_secs(300),
        )
        .context("The thinking model did not start")?;
    }
    let urls = (running.thinker_url.clone(), running.translate_url.clone());
    *slot = Some(running);
    Ok(urls)
}

fn start_thinker(
    parts: &Installed,
    gpu: bool,
    threads: usize,
    logs: &Path,
) -> Result<(Child, u16)> {
    let port = free_port()?;
    let dir = parts.server.parent().context("No folder")?;
    let mut command = Command::new(&parts.server);
    command
        .current_dir(dir)
        .arg("-m")
        .arg(&parts.thinker)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args(["-c", "8192", "--reasoning", "off", "--alias", THINKER])
        .args(["-ngl", if gpu { "99" } else { "0" }])
        .args(["-t", &(threads / 2).max(1).to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(logs.join("thinker.log"))?);
    if cfg!(target_os = "linux") {
        // Its libraries sit beside it.
        command.env("LD_LIBRARY_PATH", dir);
    }
    let child = command.spawn().context("Cannot start the thinking model")?;
    Ok((child, port))
}

fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Wait until `url` answers 200, or the program ends, or time runs out.
fn wait_ready(child: &mut Child, url: &str, limit: Duration) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if let Some(status) = child.try_wait()? {
            bail!("it stopped ({status})");
        }
        if client
            .get(url)
            .send()
            .is_ok_and(|r| r.status().is_success())
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("it did not answer in time")
}

/// Stop the helpers (they start again with the next message).
pub fn stop() {
    *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Stop the helpers when chat has not been used for `after`.
pub fn rest_if_idle(after: Duration) {
    let mut slot = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|r| r.used.elapsed() >= after) {
        *slot = None;
    }
}

/// Helpers left running by a run of the app that ended without stopping them (a crash).
pub fn stop_stale() {
    let file = root().join("running");
    let Ok(pids) = std::fs::read_to_string(&file) else {
        return;
    };
    for pid in pids.split_whitespace() {
        // Only our own helpers, never another program that reused the number.
        let name = if cfg!(windows) {
            Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
                .output()
        } else {
            // The whole command line: on some systems the short name is cut off.
            Command::new("ps")
                .args(["-p", pid, "-o", "command="])
                .output()
        };
        let ours = name.is_ok_and(|out| {
            let text = String::from_utf8_lossy(&out.stdout);
            text.contains("llama-server") || text.contains("3yiddish-transl")
        });
        if ours {
            let _ = if cfg!(windows) {
                Command::new("taskkill").args(["/PID", pid, "/F"]).status()
            } else {
                Command::new("kill").arg(pid).status()
            };
        }
    }
    let _ = std::fs::remove_file(file);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(role: &str, name: &str, platform: Option<&str>) -> File {
        File {
            role: role.into(),
            name: name.into(),
            platform: platform.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn each_computer_takes_only_its_own_programs() {
        let manifest = Manifest {
            version: "chat-1".into(),
            files: vec![
                file("thinker", "gemma.gguf", None),
                file("translator", "model.bin", None),
                file("runtime", "llama-linux.tar.gz", Some("linux-x86_64")),
                file("runtime", "llama-mac.tar.gz", Some("macos-aarch64")),
                file(
                    "translator-program",
                    "3yiddish-translate",
                    Some("linux-x86_64"),
                ),
            ],
            ..Default::default()
        };
        let mine = wanted(&manifest, "linux-x86_64");
        let names: Vec<_> = mine.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "gemma.gguf",
                "model.bin",
                "llama-linux.tar.gz",
                "3yiddish-translate"
            ]
        );
        let dir = Path::new("v");
        assert_eq!(place(dir, &mine[1]), Path::new("v/translator/model.bin"));
        assert_eq!(place(dir, &mine[2]), Path::new("v/bin/llama-linux.tar.gz"));
        assert_eq!(place(dir, &mine[0]), Path::new("v/gemma.gguf"));
        assert!(
            wanted(&manifest, "windows-x86_64")
                .iter()
                .all(|f| f.platform.is_none())
        );
    }
}
