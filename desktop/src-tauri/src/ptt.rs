//! Push-to-talk: hold a shortcut in any app, speak, let go, and the Yiddish is typed where the
//! cursor is (and added to the app's history). The shortcut is the person's choice, saved in
//! state/desktop/settings.json; F9 until they pick another or switch it off.
use crate::{Clip, Core, Transcript, pill, save_take, transcribe_blocking};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
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
    held::watch(app, 0)?;
    if let Some(s) = shortcut {
        if let Some(mask) = held::parse(s) {
            return held::watch(app, mask);
        }
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
    match event.state {
        ShortcutState::Pressed => pressed(app),
        ShortcutState::Released => released(app),
    }
}

fn pressed(app: &AppHandle) {
    press(app, app.state::<Arc<Core>>().inner())
}

fn released(app: &AppHandle) {
    release(app.clone(), app.state::<Arc<Core>>().inner().clone())
}

/// Another key was pressed with the shortcut (Ctrl + Win + Right to change desktops): that
/// was not talking, so the take goes without a word.
fn cancelled(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    if !core.ptt.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Ok(mut slot) = core.recording.lock() {
        *slot = None;
    }
    let _ = app.emit("ptt", Event::Cancelled { reason: String::new() });
    pill::hide(app, Duration::ZERO);
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
    pill::show(app);
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
                pill::hide(&app, pill::MESSAGE);
                return;
            }
        };
        let id = clip.id.clone();
        let path = clip.path.clone();
        let _ = app.emit("ptt", Event::Transcribing { clip });
        match transcribe_blocking(&core, &path, true, None) {
            Ok(transcript) => {
                // Typed into the app the person is in; when that is this window, the text is
                // already on screen and pasting it would go nowhere useful.
                let here = app
                    .get_webview_window("main")
                    .is_some_and(|w| w.is_focused().unwrap_or(false));
                let typed = !transcript.text.is_empty()
                    && !here
                    && three_yiddish::dictate::type_text(&transcript.text).is_ok();
                // Nothing heard: the pill says so for a moment.
                let after = if transcript.text.is_empty() { pill::MESSAGE } else { Duration::ZERO };
                let _ = app.emit("ptt", Event::Done { id, transcript, typed });
                pill::hide(&app, after);
            }
            Err(error) => {
                let _ = app.emit("ptt", Event::Error { id, error });
                pill::hide(&app, pill::MESSAGE);
            }
        }
    });
}

/// Shortcuts of modifier keys alone ("Control+Super", held like Wispr Flow's Ctrl + Win)
/// cannot be global shortcuts, which need one ordinary key, so a keyboard hook watches for
/// them instead (Windows only).
pub mod held {
    use super::{cancelled, pressed, released};
    use std::sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering::SeqCst},
        mpsc,
    };
    use tauri::AppHandle;

    const CONTROL: u8 = 1;
    const ALT: u8 = 2;
    const SHIFT: u8 = 4;
    const SUPER: u8 = 8;

    /// The modifiers watched for (0: none).
    static WANT: AtomicU8 = AtomicU8::new(0);
    /// The person is choosing a shortcut in Settings.
    static CHOOSING: AtomicBool = AtomicBool::new(false);
    static STARTED: OnceLock<()> = OnceLock::new();

    /// A shortcut of two or more modifiers and nothing else, as their bits.
    pub fn parse(shortcut: &str) -> Option<u8> {
        let mut mask = 0;
        for part in shortcut.split('+') {
            mask |= match part.trim().to_ascii_lowercase().as_str() {
                "control" | "ctrl" => CONTROL,
                "alt" | "option" => ALT,
                "shift" => SHIFT,
                "super" | "meta" | "win" | "command" | "cmd" => SUPER,
                _ => return None,
            };
        }
        (mask.count_ones() >= 2).then_some(mask)
    }

    /// Hold these modifiers to talk (0: stop watching).
    pub fn watch(app: &AppHandle, mask: u8) -> Result<(), String> {
        if mask != 0 {
            start(app)?;
        }
        WANT.store(mask, SeqCst);
        Ok(())
    }

    /// While a shortcut is being chosen, Win or Alt let go does not open the Start menu (which
    /// would take the window away mid-choice).
    pub fn choosing(app: &AppHandle, on: bool) {
        if on && start(app).is_err() {
            return;
        }
        CHOOSING.store(on, SeqCst);
    }

    #[cfg(not(windows))]
    fn start(_: &AppHandle) -> Result<(), String> {
        let _ = (&STARTED, cancelled, pressed, released, mpsc::channel::<()>);
        Err("Modifier keys alone work as a shortcut on Windows only. Add a key to them.".into())
    }

    #[cfg(windows)]
    fn start(app: &AppHandle) -> Result<(), String> {
        STARTED.get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            win::hook(tx);
            let app = app.clone();
            std::thread::spawn(move || follow(app, rx));
        });
        Ok(())
    }

    /// The modifier a key is, as its bit.
    #[cfg(windows)]
    fn modifier(vk: u16) -> u8 {
        match vk {
            0x11 | 0xA2 | 0xA3 => CONTROL,
            0x12 | 0xA4 | 0xA5 => ALT,
            0x10 | 0xA0 | 0xA1 => SHIFT,
            0x5B | 0x5C => SUPER,
            _ => 0,
        }
    }

    /// Follows the keys (away from the hook, which must answer at once): the take starts when
    /// exactly the shortcut's modifiers are held, ends when one is let go, and is dropped when
    /// another key joins them.
    #[cfg(windows)]
    fn follow(app: AppHandle, keys: mpsc::Receiver<(u16, bool)>) {
        let (mut held, mut active, mut spoiled) = (0u8, false, false);
        for (vk, down) in keys {
            let want = WANT.load(SeqCst);
            // A key let go where the hook cannot see (the lock screen after Win + L) must not
            // stay held.
            held &= win::held();
            let m = modifier(vk);
            if m == 0 {
                if down {
                    if active {
                        active = false;
                        cancelled(&app);
                    }
                    spoiled |= held != 0;
                }
            } else {
                if down {
                    if held & m == 0 && CHOOSING.load(SeqCst) && m & (ALT | SUPER) != 0 {
                        win::mask();
                    }
                    held |= m;
                } else {
                    held &= !m;
                }
                if active && (want == 0 || held & want != want) {
                    active = false;
                    released(&app);
                } else if !active && !spoiled && want != 0 && held == want {
                    active = true;
                    if want & (ALT | SUPER) != 0 {
                        win::mask();
                    }
                    pressed(&app);
                }
            }
            if held == 0 {
                spoiled = false;
            }
        }
    }

    #[cfg(windows)]
    mod win {
        use super::{ALT, CONTROL, SHIFT, SUPER};
        use std::sync::{OnceLock, mpsc::Sender};
        use windows_sys::Win32::{
            Foundation::{LPARAM, LRESULT, WPARAM},
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
                    SendInput,
                },
                WindowsAndMessaging::{
                    CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW,
                    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
                },
            },
        };

        /// An unassigned key. Pressed while Win or Alt is down, it tells Windows they were
        /// used with another key, so letting go opens no Start menu or menu bar.
        const MASK: u16 = 0xE8;

        static SINK: OnceLock<Sender<(u16, bool)>> = OnceLock::new();

        pub fn hook(sink: Sender<(u16, bool)>) {
            if SINK.set(sink).is_err() {
                return;
            }
            std::thread::spawn(|| unsafe {
                let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), std::ptr::null_mut(), 0);
                if hook.is_null() {
                    return;
                }
                // The hook is called from this thread's message loop.
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            });
        }

        unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            if code == HC_ACTION as i32 {
                let key = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
                let vk = key.vkCode as u16;
                if vk != MASK {
                    let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                    if let Some(sink) = SINK.get() {
                        let _ = sink.send((vk, down));
                    }
                }
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
        }

        pub fn mask() {
            let key = |flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT { wVk: MASK, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
                },
            };
            let inputs = [key(0), key(KEYEVENTF_KEYUP)];
            unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        }

        /// The modifiers Windows sees held now, as bits.
        pub fn held() -> u8 {
            let down = |vk: i32| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0;
            let mut bits = 0;
            if down(0x11) {
                bits |= CONTROL;
            }
            if down(0x12) {
                bits |= ALT;
            }
            if down(0x10) {
                bits |= SHIFT;
            }
            if down(0x5B) || down(0x5C) {
                bits |= SUPER;
            }
            bits
        }
    }
}
