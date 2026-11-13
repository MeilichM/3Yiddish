//! 3Yiddish desktop: a native window around the same Rust core the terminal app uses.
//! Everything runs in this process: microphone, model download and transcription. No server,
//! no terminal. Nothing is sent anywhere unless the person says yes to sharing or presses
//! Send under Contribute (`contribute.rs`).
mod contribute;
mod ptt;

use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use three_yiddish::{
    asr::{self, Compute, Engine},
    audio::Recorder,
    model, paths,
};

/// Started by the system at sign-in: stay in the tray instead of opening the window.
const HIDDEN: &str = "--hidden";

/// Bars in a voice note's wave.
const PEAKS: usize = 56;

#[derive(Default)]
pub(crate) struct Core {
    engine: Mutex<Option<Arc<Engine>>>,
    /// The take being recorded (from the window or from push-to-talk) and when it began.
    recording: Mutex<Option<(Recorder, Instant)>>,
    /// The take was started by the push-to-talk shortcut.
    ptt: AtomicBool,
    /// The push-to-talk shortcut in use, and why the saved one could not be used.
    shortcut: Mutex<Option<String>>,
    shortcut_error: Mutex<Option<String>>,
    /// The last decoded audio, so importing then transcribing a file decodes it once.
    decoded: Mutex<Option<(PathBuf, Arc<Vec<f32>>)>>,
    contribute: contribute::Contribute,
}

type Shared<'a> = State<'a, Arc<Core>>;
type Out<T> = Result<T, String>;

fn fail(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}
fn plain(e: anyhow::Error) -> String {
    model::plain_error(&e)
}

impl Core {
    /// The loaded engine, loading the active model on first use.
    fn engine(&self) -> anyhow::Result<Arc<Engine>> {
        let mut slot = self.engine.lock().map_err(|_| anyhow::anyhow!("engine lock"))?;
        if let Some(engine) = slot.as_ref() {
            return Ok(engine.clone());
        }
        let (_, model, vad) = model::current(&paths::models())
            .ok_or_else(|| anyhow::anyhow!("No speech model yet. Download it first."))?;
        let engine = Arc::new(Engine::load(&model, vad.as_deref(), asr::preferred())?);
        *slot = Some(engine.clone());
        Ok(engine)
    }
    fn drop_engine(&self) {
        if let Ok(mut slot) = self.engine.lock() {
            *slot = None;
        }
    }
    /// 16 kHz mono samples of an audio file, reusing the last decode.
    fn decode(&self, path: &Path) -> anyhow::Result<Arc<Vec<f32>>> {
        if let Ok(cache) = self.decoded.lock()
            && let Some((p, pcm)) = cache.as_ref()
            && p == path
        {
            return Ok(pcm.clone());
        }
        let pcm = Arc::new(asr::decode(std::fs::read(path)?)?);
        if let Ok(mut cache) = self.decoded.lock() {
            *cache = Some((path.to_owned(), pcm.clone()));
        }
        Ok(pcm)
    }
}

fn recordings() -> anyhow::Result<PathBuf> {
    let dir = std::env::current_dir()?.join("state/desktop/recordings");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn new_id() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}-{:03}", t.as_secs(), t.subsec_millis())
}

/// The loudest moment in each of `PEAKS` slices, scaled so the loudest bar is full height.
fn peaks(pcm: &[f32]) -> Vec<f32> {
    if pcm.is_empty() {
        return vec![0.0; PEAKS];
    }
    let size = pcm.len().div_ceil(PEAKS).max(1);
    let raw: Vec<f32> = pcm
        .chunks(size)
        .map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs())))
        .collect();
    let top = raw.iter().copied().fold(0f32, f32::max).max(1e-4);
    raw.into_iter().map(|v| (v / top).sqrt()).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    app_version: &'static str,
    model: Option<String>,
    compute: &'static str,
    gpu_available: bool,
    gpu_name: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Clip {
    id: String,
    path: String,
    name: String,
    seconds: f32,
    peaks: Vec<f32>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Transcript {
    text: String,
    ms: u64,
    compute: &'static str,
}

/// Keep a finished take as a WAV file: the voice note.
pub(crate) fn save_take(core: &Core, pcm: Vec<f32>) -> Out<Clip> {
    if pcm.len() < 16_000 / 4 {
        return Err("That was too short to hear anything.".into());
    }
    let id = new_id();
    let path = recordings().map_err(fail)?.join(format!("{id}.wav"));
    std::fs::write(&path, asr::wav(&pcm).map_err(fail)?).map_err(fail)?;
    let clip = Clip {
        id,
        name: "Recording".into(),
        path: path.to_string_lossy().into_owned(),
        seconds: pcm.len() as f32 / 16_000.0,
        peaks: peaks(&pcm),
    };
    *core.decoded.lock().map_err(fail)? = Some((path, Arc::new(pcm)));
    Ok(clip)
}

/// Transcribe one file. `recorded`: made in the app (else a file brought in), which sharing
/// is told when it is on.
pub(crate) fn transcribe_blocking(core: &Core, path: &str, recorded: bool) -> Out<Transcript> {
    let began = Instant::now();
    let mut engine_name = None;
    let result = (|| {
        let engine = core.engine().map_err(fail)?;
        engine_name = Some(engine.compute().name());
        let pcm = core.decode(Path::new(path)).map_err(fail)?;
        let began = Instant::now();
        let text = engine.transcribe(&pcm).map_err(fail)?;
        // The GPU may have given way to the CPU on first use.
        engine_name = Some(engine.compute().name());
        Ok(Transcript {
            text: text.trim().to_owned(),
            ms: began.elapsed().as_millis() as u64,
            compute: engine.compute().name(),
        })
    })();
    contribute::shared(
        Path::new(path),
        recorded,
        began.elapsed(),
        engine_name,
        &result.as_ref().map(|t| t.text.clone()).map_err(Clone::clone),
    );
    result
}

#[derive(Serialize, Clone)]
struct Progress {
    file: String,
    done: u64,
    total: u64,
}

#[derive(Serialize)]
struct Level {
    level: f32,
    seconds: f32,
    failed: Option<String>,
}

#[tauri::command]
fn status(core: Shared) -> Status {
    let loaded = core.engine.lock().ok().and_then(|e| e.as_ref().map(|e| e.compute()));
    let compute = loaded.unwrap_or_else(|| {
        if asr::gpu_usable() { asr::preferred() } else { Compute::Cpu }
    });
    Status {
        app_version: env!("CARGO_PKG_VERSION"),
        model: model::current(&paths::models()).map(|(v, ..)| v),
        compute: compute.name(),
        gpu_available: asr::gpu_usable(),
        gpu_name: asr::gpu_name(),
    }
}

/// A newer model version than the installed one, if GitHub has one. Silent when offline.
#[tauri::command]
async fn newer_model() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let latest = model::latest_version().ok()?;
        let have = model::current(&paths::models()).map(|(v, ..)| v);
        (have.as_deref() != Some(latest.as_str())).then_some(latest)
    })
    .await
    .ok()
    .flatten()
}

/// Download (or update to) the latest model, reporting progress as `model-progress` events.
#[tauri::command]
async fn pull_model(app: AppHandle, core: Shared<'_>) -> Out<String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = Instant::now();
        let version = model::pull(&paths::models(), &mut |file, done, total| {
            if done == total || last.elapsed().as_millis() > 100 {
                last = Instant::now();
                let _ = app.emit("model-progress", Progress { file: file.into(), done, total });
            }
        })
        .map_err(plain)?;
        core.drop_engine();
        Ok(version)
    })
    .await
    .map_err(fail)?
}

#[tauri::command]
fn rollback_model(core: Shared) -> Out<String> {
    let version = model::rollback(&paths::models()).map_err(fail)?;
    core.drop_engine();
    Ok(version)
}

/// Load the model in the background so the first transcription does not wait for it.
#[tauri::command]
async fn warm(core: Shared<'_>) -> Out<()> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.engine().map(|_| ()).map_err(fail))
        .await
        .map_err(fail)?
}

#[tauri::command]
fn set_compute(mode: String, core: Shared) -> Out<()> {
    let compute = Compute::parse(&mode).ok_or("mode must be gpu or cpu")?;
    if compute == Compute::Gpu && !asr::gpu_usable() {
        return Err("No usable graphics card on this computer.".into());
    }
    asr::remember(compute).map_err(fail)?;
    core.drop_engine();
    Ok(())
}

#[tauri::command]
fn record_start(core: Shared) -> Out<()> {
    let mut slot = core.recording.lock().map_err(fail)?;
    if slot.is_some() {
        return Err("Already recording.".into());
    }
    *slot = Some((Recorder::start_now(), Instant::now()));
    Ok(())
}

#[tauri::command]
fn record_level(core: Shared) -> Level {
    let mut slot = match core.recording.lock() {
        Ok(slot) => slot,
        Err(_) => return Level { level: 0.0, seconds: 0.0, failed: None },
    };
    match slot.as_mut() {
        Some((rec, began)) => Level {
            level: rec.level(),
            seconds: began.elapsed().as_secs_f32(),
            failed: rec.failed(),
        },
        None => Level { level: 0.0, seconds: 0.0, failed: None },
    }
}

#[tauri::command]
fn record_cancel(core: Shared) {
    if let Ok(mut slot) = core.recording.lock() {
        *slot = None;
    }
    core.ptt.store(false, Ordering::SeqCst);
}

/// Stop the microphone and keep the take as a WAV file: the voice note.
#[tauri::command]
async fn record_stop(core: Shared<'_>) -> Out<Clip> {
    let (rec, _) = core
        .recording
        .lock()
        .map_err(fail)?
        .take()
        .ok_or("Not recording")?;
    core.ptt.store(false, Ordering::SeqCst);
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_take(&core, rec.stop().map_err(fail)?))
    .await
    .map_err(fail)?
}

/// A file from disk (drag and drop): decode it to check it is audio and draw its wave.
#[tauri::command]
async fn import_path(path: String, core: Shared<'_>) -> Out<Clip> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let pcm = core
            .decode(&path)
            .map_err(|_| "That file is not audio this app can read (try WAV, MP3, M4A, OGG or FLAC).")?;
        Ok(Clip {
            id: new_id(),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            seconds: pcm.len() as f32 / 16_000.0,
            peaks: peaks(&pcm),
        })
    })
    .await
    .map_err(fail)?
}

/// A file handed over as bytes (the + button or Ctrl+V): saved next to the recordings first.
#[tauri::command]
async fn import_bytes(request: tauri::ipc::Request<'_>, core: Shared<'_>) -> Out<Clip> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected the file's bytes".into());
    };
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(|v| percent_decode(v))
        .unwrap_or_else(|| "audio".into());
    let ext = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin")
        .to_owned();
    let path = recordings().map_err(fail)?.join(format!("{}.{ext}", new_id()));
    std::fs::write(&path, bytes).map_err(fail)?;
    let mut clip = import_path(path.to_string_lossy().into_owned(), core).await?;
    clip.name = name;
    Ok(clip)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
async fn transcribe(path: String, recorded: bool, core: Shared<'_>) -> Out<Transcript> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || transcribe_blocking(&core, &path, recorded))
        .await
        .map_err(fail)?
}

/// Let the window stream one audio file (a voice note, or a file brought in) for playback.
/// The asset scope starts empty; only files the app shows are added.
#[tauri::command]
fn allow_audio(path: String, app: AppHandle) -> Out<()> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err("The recording is no longer on this computer.".into());
    }
    app.asset_protocol_scope().allow_file(&path).map_err(fail)
}

/// Delete a recording the app made (never a file the user dropped in from elsewhere).
#[tauri::command]
fn forget_audio(path: String) -> Out<()> {
    let dir = recordings().map_err(fail)?;
    let path = PathBuf::from(path);
    if path.parent() == Some(dir.as_path()) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[derive(Serialize)]
struct ShortcutInfo {
    shortcut: Option<String>,
    default: &'static str,
    error: Option<String>,
}

#[tauri::command]
fn shortcut_get(core: Shared) -> ShortcutInfo {
    ShortcutInfo {
        shortcut: core.shortcut.lock().ok().and_then(|s| s.clone()),
        default: ptt::DEFAULT,
        error: core.shortcut_error.lock().ok().and_then(|e| e.clone()),
    }
}

/// Use a new push-to-talk shortcut (None: switch push-to-talk off) and remember it. When the
/// new one cannot be taken, the old one stays.
#[tauri::command]
fn shortcut_set(shortcut: Option<String>, app: AppHandle, core: Shared) -> Out<()> {
    let shortcut = shortcut.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let old = core.shortcut.lock().map_err(fail)?.clone();
    if let Err(e) = ptt::register(&app, shortcut.as_deref()) {
        let _ = ptt::register(&app, old.as_deref());
        return Err(e);
    }
    ptt::save(&ptt::Settings { push_to_talk: shortcut.clone() }).map_err(fail)?;
    *core.shortcut_error.lock().map_err(fail)? = None;
    set_tray_tooltip(&app, shortcut.as_deref());
    *core.shortcut.lock().map_err(fail)? = shortcut;
    Ok(())
}

/// Let go of the shortcut while the person presses keys to choose a new one, so pressing the
/// current one does not start a recording.
#[tauri::command]
fn shortcut_pause(paused: bool, app: AppHandle, core: Shared) -> Out<()> {
    let current = core.shortcut.lock().map_err(fail)?.clone();
    ptt::register(&app, if paused { None } else { current.as_deref() })
}

fn set_tray_tooltip(app: &AppHandle, shortcut: Option<&str>) {
    if let Some(tray) = app.tray_by_id("main") {
        let tip = match shortcut {
            Some(s) => format!("3Yiddish: hold {s} to talk"),
            None => "3Yiddish".into(),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

#[tauri::command]
fn autostart_get(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Start 3Yiddish (in the tray) when the person signs in, so push-to-talk is always there.
#[tauri::command]
fn autostart_set(on: bool, app: AppHandle) -> Out<()> {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    if on { launcher.enable() } else { launcher.disable() }.map_err(fail)
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The tray icon: it keeps push-to-talk reachable while the window is closed.
fn tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open 3Yiddish", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("3Yiddish")
        .menu(&Menu::with_items(app, &[&open, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // All of 3Yiddish's files (models, settings) live under its home folder, shared with the
    // terminal app, so a model downloaded by either is used by both.
    if let Err(e) = paths::enter_home() {
        eprintln!("3Yiddish: cannot open its data folder: {e:#}");
    }
    tauri::Builder::default()
        // Opening the app again while it runs in the tray brings back its window instead of
        // starting a second copy (which would fight over the microphone and the shortcut).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| ptt::handle(app, event))
                .build(),
        )
        .manage(Arc::new(Core::default()))
        .setup(|app| {
            tray(app)?;
            // The window is created hidden (no flash of an empty frame) and shown here,
            // unless the app was started at sign-in.
            if !std::env::args().any(|a| a == HIDDEN) {
                show_window(app.handle());
            }
            let core = app.state::<Arc<Core>>();
            let saved = ptt::load().push_to_talk;
            match ptt::register(app.handle(), saved.as_deref()) {
                Ok(()) => {
                    set_tray_tooltip(app.handle(), saved.as_deref());
                    *core.shortcut.lock().unwrap_or_else(|e| e.into_inner()) = saved;
                }
                Err(e) => *core.shortcut_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e),
            }
            // Shared transcriptions that have waited an hour go now.
            contribute::nudge(false);
            Ok(())
        })
        .on_window_event(|window, event| {
            // With push-to-talk on, closing the window keeps the app in the tray, so the
            // shortcut goes on working. Quit from the tray icon.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let core = window.state::<Arc<Core>>();
                let listening = core.shortcut.lock().is_ok_and(|s| s.is_some());
                if listening {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            autostart_get,
            autostart_set,
            shortcut_get,
            shortcut_set,
            shortcut_pause,
            status,
            newer_model,
            pull_model,
            rollback_model,
            warm,
            set_compute,
            record_start,
            record_level,
            record_cancel,
            record_stop,
            import_path,
            import_bytes,
            transcribe,
            allow_audio,
            forget_audio,
            contribute::contribute_state,
            contribute::contribute_accept,
            contribute::share_set,
            contribute::idea_next,
            contribute::take_finish,
            contribute::take_retry,
            contribute::take_review,
            contribute::take_delete,
            contribute::take_clip,
            contribute::send_takes,
        ])
        .build(tauri::generate_context!())
        .expect("error while building 3Yiddish")
        .run(|_app, event| {
            // Quitting: shared transcriptions still waiting go now (as the terminal app does).
            if let tauri::RunEvent::Exit = event {
                contribute::flush();
            }
        });
}
