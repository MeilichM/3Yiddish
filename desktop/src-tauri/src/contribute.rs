//! Contribute and sharing: the terminal app's playground (record a phrase in your own words,
//! read what the model wrote, mark what is wrong, Send) in the desktop window. It keeps the
//! same files and rules as the terminal app and the phone page (`freebench`, `phone`, `share`
//! in the core), so takes and answers made in one show in the others.
use crate::{Clip, Core, Out, Shared, fail, peaks};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use three_yiddish::{
    asr,
    freebench::{self, Store},
    model, paths, phone,
    share::{self, Queue},
};

/// The Record screen's ideas: the phone page's list.
const IDEAS: &str = include_str!("../../../assets/phone/ideas.txt");
fn ideas() -> Vec<&'static str> {
    IDEAS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// The most takes in one request to the relay, and the most requests one Send makes (the
/// terminal app's numbers).
const SEND_MOST: usize = 20;
const SEND_ROUNDS: usize = 10;

#[derive(Default)]
pub(crate) struct Contribute {
    /// One change to the takes at a time.
    store: Mutex<()>,
    /// The idea on the Record screen (a line of `IDEAS`).
    idea: Mutex<Option<usize>>,
    /// A take whose draft failed, to try again without recording it again.
    retry: Mutex<Option<Vec<f32>>>,
}

/// The owner's own set (a fixed number of takes, never sent) or a contributor's takes.
fn store() -> Store {
    Store::new(if phone::owner() { paths::free_bench() } else { paths::contributions() })
}
fn goal() -> usize {
    if phone::owner() { freebench::GOAL } else { 0 }
}
/// A take as the window gets it: the stored fields plus the draft split into words.
fn shown(take: &freebench::Take) -> Value {
    let mut value = serde_json::to_value(take).unwrap_or_default();
    value["words"] = json!(freebench::words(&take.draft));
    value
}
fn idea(c: &Contribute) -> &'static str {
    let ideas = ideas();
    if ideas.is_empty() {
        return "";
    }
    let mut slot = c.idea.lock().unwrap_or_else(|e| e.into_inner());
    let at = *slot.get_or_insert_with(|| (uuid_ish() % ideas.len() as u128) as usize);
    ideas[at % ideas.len()]
}
fn uuid_ish() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(7, |d| d.as_nanos())
}

#[tauri::command]
pub(crate) fn contribute_state(core: Shared) -> Out<Value> {
    let c = &core.contribute;
    let takes = {
        let _guard = c.store.lock().map_err(fail)?;
        store().load().map_err(fail)?
    };
    Ok(json!({
        "accepted": phone::playground(),
        "owner": phone::owner(),
        "goal": goal(),
        "canSend": goal() == 0,
        "items": takes.iter().map(shown).collect::<Vec<_>>(),
        "idea": idea(c),
        "name": std::fs::read_to_string(paths::contributor_name()).unwrap_or_default().trim(),
        "sharing": share::on(),
        "askShare": share::ask(),
    }))
}

/// "I accept" under the Contribute explanation: remembered, and sharing switched on (which
/// the text says). Is sharing on now.
#[tauri::command]
pub(crate) fn contribute_accept() -> Out<bool> {
    share::accept_contribute().map_err(fail)
}

/// The answer to "Share what you transcribe?" (the first-run question, or Settings). No also
/// deletes what is waiting to be sent.
#[tauri::command]
pub(crate) fn share_set(yes: bool) -> Out<bool> {
    let yes = yes && !phone::owner();
    share::set(yes).map_err(fail)?;
    if yes {
        nudge(false);
    }
    Ok(yes)
}

/// Another idea, never the same one twice in a row.
#[tauri::command]
pub(crate) fn idea_next(core: Shared) -> String {
    let c = &core.contribute;
    let count = ideas().len();
    if count > 1 {
        let mut slot = c.idea.lock().unwrap_or_else(|e| e.into_inner());
        let step = 1 + (uuid_ish() % (count as u128 - 1)) as usize;
        *slot = Some(slot.map_or(step, |now| now + step) % count);
    }
    idea(c).to_owned()
}

/// Stop the microphone on a take and write down what was said: the new take, to review.
#[tauri::command]
pub(crate) async fn take_finish(core: Shared<'_>) -> Out<Value> {
    let (rec, _) = core
        .recording
        .lock()
        .map_err(fail)?
        .take()
        .ok_or("Not recording")?;
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let pcm = rec.stop().map_err(fail)?;
        draft(&core, pcm)
    })
    .await
    .map_err(fail)?
}

/// "Send it again": the take whose draft failed.
#[tauri::command]
pub(crate) async fn take_retry(core: Shared<'_>) -> Out<Value> {
    let pcm = core
        .contribute
        .retry
        .lock()
        .map_err(fail)?
        .take()
        .ok_or("Nothing to send again. Record the take.")?;
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || draft(&core, pcm))
        .await
        .map_err(fail)?
}

/// A new take: our own installed model's draft, kept with the recording, not reviewed yet.
fn draft(core: &Core, pcm: Vec<f32>) -> Out<Value> {
    let seconds = pcm.len() as f32 / 16_000.0;
    if seconds < 0.5 {
        return Err("That was too short. Speak, then stop.".into());
    }
    if seconds > 600.0 {
        return Err("Too long: keep a take under ten minutes.".into());
    }
    let written = core.engine().and_then(|engine| {
        let text = engine.transcribe(&pcm)?;
        Ok((text, engine.compute().name()))
    });
    let (text, engine) = match written {
        Ok(done) => done,
        Err(e) => {
            *core.contribute.retry.lock().map_err(fail)? = Some(pcm);
            return Err(fail(e));
        }
    };
    let version = model::current(&paths::models()).map(|(v, ..)| v).unwrap_or_default();
    let model = json!({"name": format!("3Yiddish-Transcribe {version}"), "engine": engine});
    let _guard = core.contribute.store.lock().map_err(fail)?;
    let take = store().add(&pcm, &text, model).map_err(fail)?;
    Ok(shown(&take))
}

/// Save what the speaker decided about a take: its marks, the text typed out, or Toss.
#[tauri::command]
pub(crate) fn take_review(n: u32, review: Value, core: Shared) -> Out<Value> {
    let review: freebench::Review = serde_json::from_value(review).map_err(fail)?;
    let _guard = core.contribute.store.lock().map_err(fail)?;
    store().review(n, review).map(|t| shown(&t)).map_err(fail)
}

/// Redo: the take and its recording are thrown away.
#[tauri::command]
pub(crate) fn take_delete(n: u32, core: Shared) -> Out<()> {
    let _guard = core.contribute.store.lock().map_err(fail)?;
    store().delete(n).map_err(fail)
}

/// A take's recording as a voice note.
#[tauri::command]
pub(crate) async fn take_clip(n: u32) -> Out<Clip> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = std::env::current_dir().map_err(fail)?.join(store().audio(n));
        let pcm = asr::decode(std::fs::read(&path).map_err(|_| "The recording is gone.")?)
            .map_err(fail)?;
        Ok(Clip {
            id: format!("take-{n}"),
            name: format!("Take {n}"),
            path: path.to_string_lossy().into_owned(),
            seconds: pcm.len() as f32 / 16_000.0,
            peaks: peaks(&pcm),
        })
    })
    .await
    .map_err(fail)?
}

/// Send: the reviewed takes not sent yet go to 3Yiddish under the name given; only what the
/// person pressed Send for, and never the owner's own set. The terminal app's request, byte
/// for byte (see `phone::send`).
#[tauri::command]
pub(crate) async fn send_takes(name: String, core: Shared<'_>) -> Out<Value> {
    if goal() != 0 {
        return Err("This set is not sent anywhere.".into());
    }
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 40 {
        return Err("Give a name, up to 40 letters (any name will do).".into());
    }
    let _ = std::fs::write(paths::contributor_name(), &name);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(fail)?;
    let unsent = || -> Out<Vec<freebench::Take>> {
        let _guard = core.contribute.store.lock().map_err(fail)?;
        store().unsent().map_err(fail)
    };
    let mut sent = 0;
    for round in 0..SEND_ROUNDS {
        let mut takes = unsent()?;
        takes.truncate(SEND_MOST);
        if takes.is_empty() {
            break;
        }
        match send_round(&client, &name, &takes).await {
            Ok(place) => {
                let done: Vec<(u32, String)> =
                    takes.iter().map(|t| (t.n, t.updated.clone())).collect();
                let _guard = core.contribute.store.lock().map_err(fail)?;
                store().mark_sent(&done, &place).map_err(fail)?;
                sent += takes.len();
            }
            // What went before this round stays sent; the rest goes with the next Send.
            Err(_) if round > 0 => break,
            Err(error) => return Err(error),
        }
    }
    if sent == 0 {
        return Err("Nothing new to send.".into());
    }
    Ok(json!({"sent": sent, "left": unsent()?.len()}))
}

/// One request: 4 bytes (the manifest's length), the manifest (who, and each take with its
/// recording's size and SHA-256), then the recordings. Says where they went.
async fn send_round(client: &reqwest::Client, name: &str, takes: &[freebench::Take]) -> Out<String> {
    use sha2::Digest;
    let mut entries = Vec::new();
    let mut sound = Vec::new();
    for take in takes {
        let bytes = std::fs::read(store().audio(take.n)).map_err(fail)?;
        let sum: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        entries.push(json!({"take": take, "size": bytes.len(), "sha256": sum}));
        sound.extend(bytes);
    }
    let manifest = serde_json::to_vec(&json!({
        "name": name,
        "app": env!("CORE_VERSION"),
        "takes": entries,
    }))
    .map_err(fail)?;
    let mut body = (manifest.len() as u32).to_be_bytes().to_vec();
    body.extend(manifest);
    body.extend(sound);
    let response = client
        .post(format!("{}/send", share::relay()))
        .header("content-type", "application/octet-stream")
        .body(body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                "Could not reach 3Yiddish. Check the internet connection and try again.".to_owned()
            } else {
                format!("Sending failed: {e}")
            }
        })?;
    let status = response.status();
    let answer: Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = answer["error"].as_str().unwrap_or("try again later");
        return Err(format!("3Yiddish did not take them: {why}"));
    }
    Ok(answer["id"].as_str().unwrap_or("sent").to_owned())
}

// ---- Sharing what is transcribed ----

/// One transcription the app made, queued when sharing is on (the terminal app does the same
/// for its chat), then sent in the background once a batch is due.
pub(crate) fn shared(path: &Path, recorded: bool, took: Duration, engine: Option<&str>, result: &Result<String, String>) {
    if !share::active() {
        return;
    }
    let version = model::current(&paths::models()).map(|(v, ..)| v).unwrap_or_default();
    let result = result
        .as_ref()
        .map(|text| json!({"text": text, "model": format!("3Yiddish-Transcribe {version}")}))
        .map_err(Clone::clone);
    let facts = share::facts(
        if recorded { "recording" } else { "file" },
        share::wav_seconds(path),
        took,
        engine,
        &result,
    );
    let path: PathBuf = path.to_owned();
    // Copying a large file takes a moment: not on the way of the transcript.
    std::thread::spawn(move || {
        if Queue::here().add(&path, facts).is_ok() {
            nudge(false);
        }
    });
}

/// Send in the background if a batch is due (`all`: anything waiting).
pub(crate) fn nudge(all: bool) {
    if share::active() && Queue::here().due(all) {
        tauri::async_runtime::spawn(async move {
            share::send(&Queue::here(), &share::relay(), all, share::active).await;
        });
    }
}

/// On quit: whatever is waiting goes now, for a few seconds at most.
pub(crate) fn flush() {
    if share::active() && Queue::here().due(true) {
        tauri::async_runtime::block_on(async {
            let _ = tokio::time::timeout(
                Duration::from_secs(8),
                share::send(&Queue::here(), &share::relay(), true, share::active),
            )
            .await;
        });
    }
}
