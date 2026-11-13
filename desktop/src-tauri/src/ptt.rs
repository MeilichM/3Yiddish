//! Push-to-talk: hold a shortcut in any app, speak, let go, and the Yiddish is typed where the
//! cursor is (and added to the app's history). The shortcut is the person's choice, saved in
//! state/desktop/settings.json; F9 until they pick another or switch it off.
use crate::{Clip, Core, Transcript, save_take, transcribe_blocking};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use three_yiddish::audio::Recorder;

pub const DEFAULT: &str = "F9";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// None: push-to-talk is off. A missing field (a fresh install) means the default key.
    #[serde(default = "default_shortcut")]
    pub push_to_talk: Option<String>,
}
fn default_shortcut() -> Option<String> {
    Some(DEFAULT.into())
}
impl Default for Settings {
    fn default() -> Self {
        Self { push_to_talk: default_shortcut() }
    }
}

fn file() -> PathBuf {
    PathBuf::from("state/desktop/settings.json")
}
pub fn load() -> Settings {
    std::fs::read(file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    std::fs::create_dir_all("state/desktop")?;
    std::fs::write(file(), serde_json::to_vec_pretty(settings)?)?;
    Ok(())
}

/// Make `shortcut` the only global shortcut (None: none at all).
pub fn register(app: &AppHandle, shortcut: Option<&str>) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    if let Some(s) = shortcut {
        let parsed: Shortcut = s
            .parse()
            .map_err(|e| format!("“{s}” is not a shortcut this app can use ({e})."))?;
        shortcuts.register(parsed).map_err(|_| {
            format!("“{s}” could not be taken; another program may already use it. Try another.")
        })?;
    }
    Ok(())
}

/// What the window hears about a push-to-talk take, as `ptt` events.
#[derive(Serialize, Clone)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Event {
    Recording,
    Cancelled { reason: String },
    Transcribing { clip: Clip },
    Done { id: String, transcript: Transcript, typed: bool },
    Error { id: String, error: String },
}

pub fn handle(app: &AppHandle, event: ShortcutEvent) {
    let core = app.state::<Arc<Core>>().inner().clone();
    match event.state {
        ShortcutState::Pressed => press(app, &core),
        ShortcutState::Released => release(app.clone(), core),
    }
}

fn press(app: &AppHandle, core: &Core) {
    if core.ptt.load(Ordering::SeqCst) {
        return; // the key repeating while held
    }
    let Ok(mut slot) = core.recording.lock() else { return };
    if slot.is_some() {
        return; // a take started in the window is going
    }
    *slot = Some((Recorder::start_now(), Instant::now()));
    core.ptt.store(true, Ordering::SeqCst);
    let _ = app.emit("ptt", Event::Recording);
}

fn release(app: AppHandle, core: Arc<Core>) {
    if !core.ptt.swap(false, Ordering::SeqCst) {
        return;
    }
    let Some((recorder, _)) = core.recording.lock().ok().and_then(|mut s| s.take()) else {
        return;
    };
    std::thread::spawn(move || {
        let clip = match recorder
            .stop()
            .map_err(|e| format!("{e:#}"))
            .and_then(|pcm| save_take(&core, pcm))
        {
            Ok(clip) => clip,
            Err(reason) => {
                let _ = app.emit("ptt", Event::Cancelled { reason });
                return;
            }
        };
        let id = clip.id.clone();
        let path = clip.path.clone();
        let _ = app.emit("ptt", Event::Transcribing { clip });
        match transcribe_blocking(&core, &path) {
            Ok(transcript) => {
                // Typed into the app the person is in; when that is this window, the text is
                // already on screen and pasting it would go nowhere useful.
                let here = app
                    .get_webview_window("main")
                    .is_some_and(|w| w.is_focused().unwrap_or(false));
                let typed = !transcript.text.is_empty()
                    && !here
                    && three_yiddish::dictate::type_text(&transcript.text).is_ok();
                let _ = app.emit("ptt", Event::Done { id, transcript, typed });
            }
            Err(error) => {
                let _ = app.emit("ptt", Event::Error { id, error });
            }
        }
    });
}
