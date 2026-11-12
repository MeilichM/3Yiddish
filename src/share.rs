//! Sharing: what someone transcribes in the app, sent to 3Yiddish as logs so the owner can
//! see how the model does on real use. Off until the person says yes (the first-run question,
//! or the Contribute explanation, which says so), never on the owner's own computer, and
//! switched off with one click or `3yiddish share off`, which also deletes what is waiting.
//!
//! Each transcription is copied into a local queue (the recording or file, the text the
//! model wrote, a few technical facts). The queue goes to the relay in batches, in the
//! background: when enough is waiting, when something has waited an hour, and on quit.
//! Failures are silent; what was not taken stays queued for the next try.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// What sharing sends, where it goes and what it is for: the words a yes is given to (the
/// first-run question, the Sharing card and `3yiddish share on` all show them).
pub const WHAT: &str = "What you transcribe here, your recordings and files with the text the \
    app wrote, is sent to 3Yiddish with technical facts: app and model version, operating \
    system, GPU or CPU, how long it took, and errors. No name or computer id goes with it. It \
    is kept privately and used to test, and maybe later to improve, Yiddish speech to text.";
/// How to switch it off.
pub const OFF: &str =
    "You can switch it off any time with Sharing at the top, or: 3yiddish share off";

/// Items waiting that make a batch.
pub const BATCH: usize = 10;
/// Fewer go anyway once the oldest has waited this long.
const WAIT: Duration = Duration::from_secs(3600);
/// The local queue's caps; the oldest go first when it is full.
pub const MOST_ITEMS: usize = 100;
pub const MOST_BYTES: u64 = 200 * 1024 * 1024;
/// One item's audio: the service takes no more.
const MOST_AUDIO: u64 = 25 * 1024 * 1024;
/// One request to the relay (its Worker may make 50 requests of its own per request: 3 and
/// two per recording), and the most requests one try makes.
const MOST_PER_REQUEST: usize = 20;
const MOST_REQUEST_BYTES: u64 = 80 * 1024 * 1024;
const ROUNDS: usize = 5;
/// The audio formats the relay keeps; anything else is not queued.
pub const KINDS: [&str; 9] = [
    "wav", "mp3", "m4a", "ogg", "opus", "flac", "webm", "mp4", "aac",
];

/// The stored answer: yes, no, or never asked (no file).
pub fn answer_at(path: &Path) -> Option<bool> {
    match std::fs::read_to_string(path).ok()?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}
pub fn answer() -> Option<bool> {
    answer_at(&crate::paths::share())
}
/// Should the first-run question show: nobody answered it here, and this is not the owner's
/// own computer (which never shares).
pub fn ask() -> bool {
    !crate::phone::owner() && answer().is_none()
}
/// Sharing is switched on here.
pub fn on() -> bool {
    !crate::phone::owner() && answer() == Some(true)
}
/// Is anything to be queued and sent: sharing is on, and this is not demo mode, which never
/// sends anything unless a relay is named (the same rule as Send).
pub fn active() -> bool {
    allowed(
        answer(),
        crate::phone::owner(),
        std::env::var_os("THREE_YIDDISH_DEMO").is_some(),
        std::env::var_os("THREE_YIDDISH_RELAY").is_some(),
    )
}
pub fn allowed(answer: Option<bool>, owner: bool, demo: bool, relay_named: bool) -> bool {
    answer == Some(true) && !owner && (!demo || relay_named)
}

/// Store the answer. No (or off) also deletes the queue, and a send under way stops.
pub fn set(yes: bool) -> Result<()> {
    set_at(&crate::paths::share(), &Queue::here(), yes)
}
pub fn set_at(answer: &Path, queue: &Queue, yes: bool) -> Result<()> {
    if let Some(folder) = answer.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(answer, if yes { "yes\n" } else { "no\n" })
        .context("Cannot save the sharing answer")?;
    if !yes {
        queue.clear();
    }
    Ok(())
}

/// Someone accepted the Contribute explanation: the number of the text accepted (a later,
/// different one is shown again), and sharing on, which that text says (never on the
/// owner's own computer). Is sharing on now.
pub fn accept_contribute() -> Result<bool> {
    accept_contribute_at(
        &crate::paths::contribute_accepted(),
        &crate::paths::share(),
        &Queue::here(),
        crate::phone::owner(),
    )
}
pub fn accept_contribute_at(
    accepted: &Path,
    answer: &Path,
    queue: &Queue,
    owner: bool,
) -> Result<bool> {
    std::fs::write(accepted, crate::phone::CONSENT)?;
    if owner {
        return Ok(false);
    }
    set_at(answer, queue, true)?;
    Ok(true)
}

fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
/// An item's name starts with the millisecond it was queued and a count, so names sort in
/// the order items came, also two in the same millisecond.
fn next_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:014}-{:06}{}",
        millis(),
        COUNT.fetch_add(1, Ordering::Relaxed) % 1_000_000,
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    )
}

/// One transcription's facts, as the relay keeps them. `kind` is "recording" (made in the
/// app) or "file" (attached); the file's own name is never sent.
pub fn facts(
    kind: &str,
    seconds: Option<f32>,
    took: Duration,
    engine: Option<&str>,
    result: &Result<Value, String>,
) -> Value {
    let (text, model, error) = match result {
        Ok(value) => (
            // The relay keeps up to 20000 letters of a transcript.
            value["text"]
                .as_str()
                .map(|t| t.chars().take(20_000).collect::<String>()),
            value["model"].as_str().map(str::to_owned),
            None,
        ),
        Err(error) => (
            None,
            None,
            Some(error.chars().take(500).collect::<String>()),
        ),
    };
    json!({
        "kind": kind,
        "app": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "engine": engine,
        "model": model,
        "seconds": seconds.map(|s| (f64::from(s) * 100.0).round() / 100.0),
        "took": (took.as_secs_f64() * 100.0).round() / 100.0,
        "text": text,
        "error": error,
    })
}

/// A WAV file's length in seconds, from its header; None for anything else.
pub fn wav_seconds(path: &Path) -> Option<f32> {
    use std::io::Read;
    let mut head = vec![0; 4096];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let head = &head[..n];
    if head.get(0..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    let total = std::fs::metadata(path).ok()?.len();
    let (mut at, mut rate) = (12, None);
    while at + 8 <= head.len() {
        let size = u32::from_le_bytes(head[at + 4..at + 8].try_into().ok()?) as u64;
        match &head[at..at + 4] {
            b"fmt " => {
                rate = Some(u32::from_le_bytes(
                    head.get(at + 16..at + 20)?.try_into().ok()?,
                ))
            }
            b"data" => {
                let bytes = size.min(total.saturating_sub(at as u64 + 8));
                return Some(bytes as f32 / rate.filter(|r| *r > 0)? as f32);
            }
            _ => {}
        }
        at += 8 + size as usize + (size as usize & 1);
    }
    None
}

/// The local queue: `<time>-<id>.json` (the facts) next to `<time>-<id>.<kind>` (the audio).
pub struct Queue(PathBuf);
pub struct Item {
    pub name: String,
    pub facts: Value,
    pub audio: PathBuf,
    pub bytes: u64,
    /// When it was queued, in milliseconds since 1970.
    pub at: u64,
}
impl Queue {
    pub fn at(folder: impl Into<PathBuf>) -> Self {
        Self(folder.into())
    }
    pub fn here() -> Self {
        Self::at(crate::paths::share_queue())
    }
    /// Queue a copy of `audio` with its facts; the queue then sheds its oldest items until it
    /// is within its caps. A file in a format the relay does not keep, or too large, is left out.
    pub fn add(&self, audio: &Path, mut facts: Value) -> Result<()> {
        let kind = audio
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|e| KINDS.contains(&e.as_str()))
            .context("Not an audio format that is shared")?;
        let bytes = std::fs::metadata(audio)?.len();
        if bytes == 0 || bytes > MOST_AUDIO {
            anyhow::bail!("Audio too large to share");
        }
        std::fs::create_dir_all(&self.0)?;
        // Recordings and their text: for this user only, like the app's own recordings.
        let private = |path: &Path| -> Result<()> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if path.is_dir() { 0o700 } else { 0o600 };
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
            }
            let _ = path;
            Ok(())
        };
        private(&self.0)?;
        let name = next_name();
        facts["audio"] = json!(kind);
        // The audio first, the facts last: an item is whole once its facts are there.
        let copy = self.0.join(format!("{name}.{kind}"));
        std::fs::copy(audio, &copy)?;
        private(&copy)?;
        let part = self.0.join(format!("{name}.json.part"));
        std::fs::write(&part, serde_json::to_vec(&facts)?)?;
        private(&part)?;
        std::fs::rename(part, self.0.join(format!("{name}.json")))?;
        self.trim();
        Ok(())
    }
    /// The whole items, oldest first.
    pub fn items(&self) -> Vec<Item> {
        let Ok(entries) = std::fs::read_dir(&self.0) else {
            return Vec::new();
        };
        let mut items: Vec<Item> = entries
            .flatten()
            .filter_map(|entry| {
                let file = entry.file_name().to_str()?.to_owned();
                let name = file.strip_suffix(".json")?.to_owned();
                let facts: Value =
                    serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok()?;
                let audio = self.0.join(format!("{name}.{}", facts["audio"].as_str()?));
                let bytes = std::fs::metadata(&audio).ok()?.len();
                let at = name.split('-').next()?.parse().ok()?;
                Some(Item {
                    name,
                    facts,
                    audio,
                    bytes,
                    at,
                })
            })
            .collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items
    }
    pub fn remove(&self, item: &Item) {
        let _ = std::fs::remove_file(self.0.join(format!("{}.json", item.name)));
        let _ = std::fs::remove_file(&item.audio);
    }
    /// Keep within the caps (oldest go first), and take away halves of items a crash left.
    fn trim(&self) {
        let mut items = self.items();
        let mut bytes: u64 = items.iter().map(|i| i.bytes).sum();
        while items.len() > MOST_ITEMS || bytes > MOST_BYTES {
            let oldest = items.remove(0);
            bytes -= oldest.bytes;
            self.remove(&oldest);
        }
        let whole: std::collections::HashSet<String> = items.into_iter().map(|i| i.name).collect();
        let old = millis().saturating_sub(10 * 60 * 1000);
        for entry in std::fs::read_dir(&self.0).into_iter().flatten().flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let name = file.split('.').next().unwrap_or_default();
            let at: u64 = name
                .split('-')
                .next()
                .and_then(|t| t.parse().ok())
                .unwrap_or(0);
            if at > 0 && at < old && !whole.contains(name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    /// Everything waiting goes, and the folder with it.
    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
    /// Is a send due: a batch is waiting, the oldest has waited an hour, or (quit, `share
    /// now`) anything at all.
    pub fn due(&self, all: bool) -> bool {
        let items = self.items();
        let oldest = items.first().map(|i| i.at);
        items.len() >= BATCH
            || oldest
                .is_some_and(|at| all || millis().saturating_sub(at) >= WAIT.as_millis() as u64)
    }
}

/// One request's body: 4 bytes (the manifest's length), the manifest (each item's facts with
/// its recording's size and SHA-256), then the recordings one after another, as `/send` has it.
pub fn body(items: &[Item]) -> Result<Vec<u8>> {
    use sha2::Digest;
    let mut entries = Vec::new();
    let mut sound = Vec::new();
    for item in items {
        let bytes = std::fs::read(&item.audio)?;
        let sum: String = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut entry = item.facts.clone();
        entry["size"] = json!(bytes.len());
        entry["sha256"] = json!(sum);
        entries.push(entry);
        sound.extend(bytes);
    }
    let manifest = serde_json::to_vec(&json!({
        "app": env!("CARGO_PKG_VERSION"),
        "items": entries,
    }))?;
    let mut body = (manifest.len() as u32).to_be_bytes().to_vec();
    body.extend(manifest);
    body.extend(sound);
    Ok(body)
}

/// Send what is due to `relay` (`all`: whatever is waiting), a batch per request, while
/// `still_on` says sharing is on: a send under way is dropped within half a second of
/// switching off. Items leave the queue only once the relay took them. How many went.
pub async fn send(queue: &Queue, relay: &str, all: bool, still_on: fn() -> bool) -> usize {
    // One sender at a time, also across two open windows; a lock older than ten minutes was
    // left by a sender that died.
    let lock = queue.0.join(".sending");
    let holder: Option<u32> = std::fs::read_to_string(&lock)
        .ok()
        .and_then(|t| t.trim().parse().ok());
    let old = std::fs::metadata(&lock)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(600));
    if old || holder.is_some_and(|pid| !alive(pid)) {
        let _ = std::fs::remove_file(&lock);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
    else {
        return 0;
    };
    let _ = std::io::Write::write_all(&mut file, std::process::id().to_string().as_bytes());
    struct Unlock(PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _unlock = Unlock(lock);
    // A client of its own: the app's carries the local service's key, which stays home.
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
    else {
        return 0;
    };
    let mut sent = 0;
    for _ in 0..ROUNDS {
        if !still_on() || !queue.due(all) {
            break;
        }
        let mut batch = Vec::new();
        let mut bytes = 0;
        for item in queue.items() {
            if batch.len() == MOST_PER_REQUEST || bytes + item.bytes > MOST_REQUEST_BYTES {
                break;
            }
            bytes += item.bytes;
            batch.push(item);
        }
        let Ok(body) = body(&batch) else { break };
        let request = client
            .post(format!("{relay}/log"))
            .header("content-type", "application/octet-stream")
            .body(body)
            .send();
        let switched_off = async {
            while still_on() {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        };
        let status = tokio::select! {
            response = request => response.map(|r| r.status().as_u16()).unwrap_or(0),
            _ = switched_off => 0,
        };
        // Taken; or refused as unreadable (400, 413), which no later try changes: dropped,
        // so it never holds up what comes after it.
        let taken = (200..300).contains(&status);
        if !(taken || status == 400 || status == 413) || !still_on() {
            break;
        }
        for item in &batch {
            queue.remove(item);
        }
        if taken {
            sent += batch.len();
        }
    }
    sent
}

/// Is process `pid` still running?
pub(crate) fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only asks whether the process exists.
        let found = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        pid == std::process::id()
    }
}

/// Where the logs go: the relay Send uses (`$THREE_YIDDISH_RELAY` names another one).
pub fn relay() -> String {
    crate::phone::relay()
}

/// Queue one transcription the app made (when sharing is on), then send if a batch is due.
/// Runs off the screen's thread.
pub fn record(audio: &Path, facts: Value) {
    if !active() {
        return;
    }
    if Queue::here().add(audio, facts).is_ok() {
        nudge(false);
    }
}
/// Send in the background if a send is due (`all`: anything waiting).
pub fn nudge(all: bool) {
    if !active() || !Queue::here().due(all) {
        return;
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move { send(&Queue::here(), &relay(), all, active).await });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A folder of its own, gone after the test.
    pub(crate) struct Scratch(PathBuf);
    impl Scratch {
        pub(crate) fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("3y-share-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    pub(crate) fn wav(seconds: f32) -> Vec<u8> {
        crate::asr::wav(&vec![0.0; (16_000.0 * seconds) as usize]).unwrap()
    }

    #[test]
    fn the_default_is_no_and_only_a_yes_turns_it_on() {
        let dir = Scratch::new();
        let answer = dir.path().join("share");
        // Never asked: nothing is shared.
        assert_eq!(answer_at(&answer), None);
        assert!(!allowed(answer_at(&answer), false, false, false));
        let queue = Queue::at(dir.path().join("queue"));
        set_at(&answer, &queue, false).unwrap();
        assert_eq!(answer_at(&answer), Some(false));
        assert!(!allowed(answer_at(&answer), false, false, false));
        set_at(&answer, &queue, true).unwrap();
        assert!(allowed(answer_at(&answer), false, false, false));
        // The owner's own computer never shares; demo mode only to a named relay.
        assert!(!allowed(Some(true), true, false, true));
        assert!(!allowed(Some(true), false, true, false));
        assert!(allowed(Some(true), false, true, true));
    }

    /// No first, then accepting Contribute (whose text says so): sharing is on; the owner's
    /// own computer stays off.
    #[test]
    fn accepting_contribute_after_no_turns_sharing_on() {
        let dir = Scratch::new();
        let (accepted, answer) = (dir.path().join("accepted"), dir.path().join("share"));
        let queue = Queue::at(dir.path().join("queue"));
        set_at(&answer, &queue, false).unwrap();
        assert!(accept_contribute_at(&accepted, &answer, &queue, false).unwrap());
        assert_eq!(answer_at(&answer), Some(true));
        assert!(crate::phone::accepted(&accepted));
        set_at(&answer, &queue, false).unwrap();
        assert!(!accept_contribute_at(&accepted, &answer, &queue, true).unwrap());
        assert_eq!(answer_at(&answer), Some(false));
    }

    #[test]
    fn off_deletes_the_queue() {
        let dir = Scratch::new();
        let audio = dir.path().join("a.wav");
        std::fs::write(&audio, wav(1.0)).unwrap();
        let queue = Queue::at(dir.path().join("queue"));
        let answer = dir.path().join("share");
        set_at(&answer, &queue, true).unwrap();
        queue.add(&audio, json!({"text": "x"})).unwrap();
        assert_eq!(queue.items().len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let item = &queue.items()[0];
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&item.audio), 0o600);
            assert_eq!(mode(&item.audio.with_extension("json")), 0o600);
            assert_eq!(mode(&dir.path().join("queue")), 0o700);
        }
        set_at(&answer, &queue, false).unwrap();
        assert!(queue.items().is_empty());
        assert!(!dir.path().join("queue").exists());
        // The person's own file is untouched.
        assert!(audio.exists());
    }

    #[test]
    fn the_queue_keeps_to_its_caps_oldest_first_and_only_audio() {
        let dir = Scratch::new();
        let audio = dir.path().join("a.wav");
        std::fs::write(&audio, wav(0.1)).unwrap();
        let queue = Queue::at(dir.path().join("queue"));
        for n in 0..MOST_ITEMS + 5 {
            queue.add(&audio, json!({"n": n})).unwrap();
        }
        let items = queue.items();
        assert_eq!(items.len(), MOST_ITEMS);
        assert_eq!(items[0].facts["n"], 5);
        assert_eq!(items.last().unwrap().facts["n"], MOST_ITEMS + 4);
        // Bytes: a few large files push the oldest out.
        let big = dir.path().join("big.wav");
        std::fs::write(&big, vec![0u8; 24 * 1024 * 1024]).unwrap();
        for _ in 0..9 {
            queue.add(&big, json!({"big": true})).unwrap();
        }
        let items = queue.items();
        assert!(items.iter().map(|i| i.bytes).sum::<u64>() <= MOST_BYTES);
        assert!(items.iter().all(|i| i.facts["big"] == true));
        // Not audio: not queued.
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        assert!(queue.add(&text, json!({})).is_err());
    }

    #[test]
    fn a_batch_is_due_at_ten_or_after_an_hour_or_on_quit() {
        let dir = Scratch::new();
        let audio = dir.path().join("a.wav");
        std::fs::write(&audio, wav(0.1)).unwrap();
        let queue = Queue::at(dir.path().join("queue"));
        assert!(!queue.due(true));
        queue.add(&audio, json!({})).unwrap();
        assert!(!queue.due(false));
        assert!(queue.due(true));
        // An item queued over an hour ago.
        let old = queue.items().remove(0);
        let name = format!("{:014}-oldoldol", millis() - 3_700_000);
        std::fs::rename(&old.audio, dir.path().join(format!("queue/{name}.wav"))).unwrap();
        std::fs::rename(
            dir.path().join(format!("queue/{}.json", old.name)),
            dir.path().join(format!("queue/{name}.json")),
        )
        .unwrap();
        assert!(queue.due(false));
        queue.clear();
        for _ in 0..BATCH - 1 {
            queue.add(&audio, json!({})).unwrap();
        }
        assert!(!queue.due(false));
        queue.add(&audio, json!({})).unwrap();
        assert!(queue.due(false));
    }

    #[test]
    fn facts_say_what_ran_and_never_the_file_name() {
        let ok = facts(
            "file",
            Some(2.0),
            Duration::from_millis(1500),
            Some("cpu"),
            &Ok(json!({"text": "גוט", "model": "3Yiddish-Transcribe v1"})),
        );
        assert_eq!(ok["text"], "גוט");
        assert_eq!(ok["model"], "3Yiddish-Transcribe v1");
        assert_eq!(ok["engine"], "cpu");
        assert_eq!(ok["took"], 1.5);
        assert_eq!(ok["os"], std::env::consts::OS);
        assert!(ok["error"].is_null());
        let failed = facts(
            "recording",
            None,
            Duration::ZERO,
            None,
            &Err("x".repeat(900)),
        );
        assert_eq!(failed["error"].as_str().unwrap().len(), 500);
        assert!(failed["text"].is_null());
        let keys: Vec<&String> = ok.as_object().unwrap().keys().collect();
        assert!(
            !keys
                .iter()
                .any(|k| k.contains("name") || k.contains("path"))
        );
        let dir = Scratch::new();
        let audio = dir.path().join("a.wav");
        std::fs::write(&audio, wav(2.5)).unwrap();
        assert!((wav_seconds(&audio).unwrap() - 2.5).abs() < 0.01);
    }

    /// A stand-in for the relay: each request's manifest; answers `status`.
    async fn stub(status: u16) -> (String, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
        use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};
        type Seen = std::sync::Arc<std::sync::Mutex<Vec<Value>>>;
        let seen: Seen = Default::default();
        let app = Router::new()
            .route(
                "/log",
                post(
                    |State((seen, status)): State<(Seen, u16)>, body: Bytes| async move {
                        let size = u32::from_be_bytes(body[..4].try_into().unwrap()) as usize;
                        let manifest: Value = serde_json::from_slice(&body[4..4 + size]).unwrap();
                        let audio: u64 = manifest["items"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|i| i["size"].as_u64().unwrap())
                            .sum();
                        assert_eq!(4 + size as u64 + audio, body.len() as u64);
                        seen.lock().unwrap().push(manifest);
                        StatusCode::from_u16(status).unwrap()
                    },
                ),
            )
            .with_state((seen.clone(), status));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        (url, seen)
    }

    #[tokio::test]
    async fn waiting_items_go_in_batches_and_stay_when_refused() {
        let dir = Scratch::new();
        let audio = dir.path().join("a.wav");
        std::fs::write(&audio, wav(0.2)).unwrap();
        let queue = Queue::at(dir.path().join("queue"));
        let on = || true;
        let (url, seen) = stub(200).await;
        // Fewer than a batch, and recent: nothing goes yet.
        for _ in 0..BATCH - 1 {
            queue.add(&audio, json!({"text": "x"})).unwrap();
        }
        assert_eq!(send(&queue, &url, false, on).await, 0);
        assert!(seen.lock().unwrap().is_empty());
        // A batch: one request takes them all.
        for _ in 0..BATCH + 15 {
            queue.add(&audio, json!({"text": "x"})).unwrap();
        }
        assert_eq!(send(&queue, &url, false, on).await, 2 * BATCH + 14);
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            assert_eq!(seen[0]["items"].as_array().unwrap().len(), MOST_PER_REQUEST);
            assert_eq!(seen[0]["items"][0]["audio"], "wav");
            assert!(seen[0]["items"][0]["sha256"].as_str().unwrap().len() == 64);
        }
        assert!(queue.items().is_empty());
        // Switched off: nothing goes.
        queue.add(&audio, json!({})).unwrap();
        assert_eq!(send(&queue, &url, true, || false).await, 0);
        assert_eq!(seen.lock().unwrap().len(), 2);
        // Refused: the items stay for the next try.
        let (refusing, tried) = stub(502).await;
        assert_eq!(send(&queue, &refusing, true, on).await, 0);
        assert_eq!(tried.lock().unwrap().len(), 1);
        assert_eq!(queue.items().len(), 1);
        // Nobody there: the same, and quietly.
        assert_eq!(send(&queue, "http://127.0.0.1:9", true, on).await, 0);
        assert_eq!(queue.items().len(), 1);
        // Unreadable to the relay: dropped, so it never holds up the rest.
        let (unreadable, _) = stub(400).await;
        assert_eq!(send(&queue, &unreadable, true, on).await, 0);
        assert!(queue.items().is_empty());
    }
}
