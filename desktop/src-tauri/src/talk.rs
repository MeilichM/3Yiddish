//! Chat: talk to 3Yiddish in Yiddish or English and it answers in Yiddish. The work is the
//! core's (`three_yiddish::chat` downloads and runs the helpers, `service::App::chat` turns a
//! conversation into an answer), so the desktop app answers exactly as the terminal app does.
use crate::{Core, Out, Shared, fail};
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use three_yiddish::{
    chat, config::Config, model,
    service::{App, ChatRequest, Message},
};

/// The chat helpers stop after this long without a message, so an idle app holds no model in
/// memory (they start again with the next one).
const IDLE: Duration = Duration::from_secs(600);
/// The last turns sent with each message: what a small model can keep in mind.
const TURNS: usize = 12;

/// The core's service, made on the first message (and again after CPU/GPU changes).
static SERVICE: Mutex<Option<App>> = Mutex::new(None);

fn service() -> anyhow::Result<App> {
    let mut slot = SERVICE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(app) = slot.as_ref() {
        return Ok(app.clone());
    }
    let app = App::new(Config::load_or_default(Path::new("config.toml"))?)?;
    *slot = Some(app.clone());
    Ok(app)
}

/// The graphics-card choice changed: the next message starts the helpers the new way.
pub(crate) fn forget() {
    *SERVICE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    chat::stop();
}

/// At start: helpers a crashed run left behind go, and from then on idle ones stop.
pub(crate) fn start() {
    std::thread::spawn(|| {
        chat::stop_stale();
        loop {
            std::thread::sleep(Duration::from_secs(60));
            chat::rest_if_idle(IDLE);
        }
    });
}

/// At quit: the helpers end with the app.
pub(crate) fn stop() {
    chat::stop();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatState {
    /// Chat runs on this computer (not on an Intel Mac).
    available: bool,
    /// Chat is downloaded.
    ready: bool,
}

#[tauri::command]
pub(crate) fn chat_state() -> ChatState {
    ChatState { available: chat::platform().is_some(), ready: chat::ready() }
}

/// How many bytes chat takes to download here; None offline.
#[tauri::command]
pub(crate) async fn chat_size() -> Option<u64> {
    tauri::async_runtime::spawn_blocking(|| chat::size().ok().flatten())
        .await
        .ok()
        .flatten()
}

#[derive(Serialize, Clone)]
struct ChatProgress {
    done: u64,
    total: u64,
}

/// Download chat, telling the window how far it got (`chat-progress`, ten times a second at most).
#[tauri::command]
pub(crate) async fn chat_download(app: AppHandle) -> Out<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = Instant::now();
        chat::pull(&mut |done, total| {
            if done == total || last.elapsed().as_millis() > 100 {
                last = Instant::now();
                let _ = app.emit("chat-progress", ChatProgress { done, total });
            }
        })
        .map_err(|e| model::plain_error(&e))
    })
    .await
    .map_err(fail)?
}

/// The answer, in Yiddish, to the conversation so far (oldest first, ending with the person's
/// message). The first message also starts the helpers, which can take a while.
#[tauri::command]
pub(crate) async fn chat_send(messages: Vec<Message>) -> Out<String> {
    let app = service().map_err(fail)?;
    let mut from = messages.len().saturating_sub(TURNS);
    // Start at one of the person's messages, as the conversation did.
    while from < messages.len() && messages[from].role != "user" {
        from += 1;
    }
    let messages: Vec<Message> = messages
        .into_iter()
        .skip(from)
        .filter(|m| !m.content.trim().is_empty())
        .collect();
    let answer = app
        .chat(ChatRequest { model: None, messages, max_tokens: None, stream: false })
        .await
        // "503 Service Unavailable: Chat could not start: …": the words after the status.
        .map_err(|e| {
            let text = e.to_string();
            text.split_once(": ").map_or(text.clone(), |(_, rest)| rest.to_owned())
        })?;
    answer
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_owned())
        .ok_or_else(|| "No answer came back.".into())
}

/// A spoken message: its words, for the chat. Not a transcription, so Contribute never sends it,
/// and the recording is deleted once heard.
#[tauri::command]
pub(crate) async fn chat_hear(path: String, core: Shared<'_>) -> Out<String> {
    let core: Arc<Core> = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let heard = (|| {
            let engine = core.engine()?;
            let pcm = core.decode(Path::new(&path))?;
            engine.transcribe(&pcm)
        })();
        // Only the app's own recordings are deleted, never a file brought in.
        if let Ok(dir) = crate::recordings()
            && Path::new(&path).parent() == Some(dir.as_path())
        {
            let _ = std::fs::remove_file(&path);
        }
        heard.map(|t| t.trim().to_owned()).map_err(fail)
    })
    .await
    .map_err(fail)?
}
