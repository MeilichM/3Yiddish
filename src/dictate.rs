//! Push-to-talk dictation: hold a key anywhere, speak, release, and the Yiddish is typed into
//! whatever text field has focus.
//!
//! - Windows, macOS, Linux X11: the key is watched globally (rdev).
//! - Wayland (e.g. Hyprland): apps cannot watch the keyboard, so the compositor calls
//!   `3yiddish dictate start` on key press and `3yiddish dictate stop` on release.
//!
//! Either way the running `3yiddish dictate` process owns the microphone; `start`/`stop` reach it
//! over a loopback control port.
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
};

/// Loopback port for `3yiddish dictate start|stop|toggle`.
pub const CONTROL: &str = "127.0.0.1:8792";

/// Where the current take is being recorded.
enum Take {
    Pc(crate::audio::Recorder),
    /// The paired phone (PCs without a microphone); `revision` is the transcript counter
    /// before this take, so the new transcript is recognised.
    Phone {
        id: String,
        revision: u64,
    },
}

struct Dictation {
    recorder: Option<Take>,
    server: String,
    engine: Option<crate::asr::Engine>,
}

type Shared = Arc<Mutex<Dictation>>;

/// Run the dictation helper until killed. `key` is the hold-to-talk key (e.g. "F9", "CapsLock").
pub fn run(key: &str, server: &str) -> Result<()> {
    let state: Shared = Arc::new(Mutex::new(Dictation {
        recorder: None,
        server: server.trim_end_matches('/').to_owned(),
        engine: None,
    }));
    crate::audio::warm();
    let listener = TcpListener::bind(CONTROL).context("Dictation is already running")?;
    let control = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = handle(stream, &control);
        }
    });
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        eprintln!(
            "3Yiddish dictation is ready. On Linux, give it a key in your desktop's keyboard shortcuts:\n  \
             Hyprland:  bind = , {key}, exec, 3yiddish dictate start\n             \
             bindr = , {key}, exec, 3yiddish dictate stop      (hold to talk)\n  \
             GNOME/KDE/others: a custom shortcut running `3yiddish dictate toggle` (press, talk, press again)"
        );
        loop {
            std::thread::park();
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    listen(key, state)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn listen(key: &str, state: Shared) -> Result<()> {
    let want = parse_key(key)?;
    eprintln!("3Yiddish dictation is ready: hold {key}, speak, let go.");
    let keys = state;
    rdev::listen(move |event| match event.event_type {
        rdev::EventType::KeyPress(k) if k == want => press(&keys),
        rdev::EventType::KeyRelease(k) if k == want => release(&keys),
        _ => {}
    })
    .map_err(|e| anyhow::anyhow!("Cannot watch the keyboard: {e:?}"))
}

/// `3yiddish dictate start|stop|toggle`: tell the running helper.
pub fn send(command: &str) -> Result<()> {
    let mut stream = TcpStream::connect(CONTROL)
        .context("Dictation is not running; start it with `3yiddish dictate`")?;
    writeln!(stream, "{command}")?;
    Ok(())
}

fn handle(stream: TcpStream, state: &Shared) -> Result<()> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    match line.trim() {
        "start" => press(state),
        "stop" => release(state),
        "toggle" => {
            if state.lock().map(|s| s.recorder.is_some()).unwrap_or(false) {
                release(state)
            } else {
                press(state)
            }
        }
        _ => {}
    }
    Ok(())
}

fn press(state: &Shared) {
    let Ok(mut s) = state.lock() else { return };
    if s.recorder.is_some() {
        return; // key auto-repeat
    }
    match crate::audio::Recorder::start() {
        Ok(r) => s.recorder = Some(Take::Pc(r)),
        // Developer machines only (no PC mic there): the paired phone stands in.
        Err(pc) if !std::path::Path::new("state/read-aloud.txt").is_file() => eprintln!("{pc}"),
        Err(pc) => match phone_start(&s.server) {
            Ok(take) => s.recorder = Some(take),
            Err(phone) => eprintln!("{pc}\n(Phone: {phone})"),
        },
    }
}

fn http() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?)
}

/// Start recording on the paired phone, if one is connected with its mic on.
fn phone_start(server: &str) -> Result<Take> {
    let pair: serde_json::Value = http()?.get(format!("{server}/v1/pair")).send()?.json()?;
    if pair["connected"] != true || pair["mic_ready"] != true {
        bail!("no phone paired with its microphone on");
    }
    let id = pair["id"].as_str().context("no pairing id")?.to_owned();
    let revision = pair["transcript_revision"].as_u64().unwrap_or(0);
    if pair["record_requested"] != true {
        http()?
            .post(format!("{server}/v1/pair/{id}/record"))
            .send()?
            .error_for_status()?;
    }
    Ok(Take::Phone { id, revision })
}

/// Stop the phone recording and wait for its transcript.
fn phone_finish(server: &str, id: &str, revision: u64) -> Result<String> {
    let client = http()?;
    let pair: serde_json::Value = client.get(format!("{server}/v1/pair")).send()?.json()?;
    if pair["record_requested"] == true || pair["recording"] == true {
        client
            .post(format!("{server}/v1/pair/{id}/record"))
            .send()?
            .error_for_status()?;
    }
    for _ in 0..240 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let pair: serde_json::Value = client.get(format!("{server}/v1/pair")).send()?.json()?;
        if pair["transcript_revision"].as_u64().unwrap_or(0) > revision {
            return Ok(pair["transcript"].as_str().unwrap_or_default().to_owned());
        }
    }
    bail!("the phone's recording did not come back within a minute")
}

fn release(state: &Shared) {
    let recorder = state.lock().ok().and_then(|mut s| s.recorder.take());
    let Some(take) = recorder else { return };
    let state = state.clone();
    std::thread::spawn(move || {
        let server = state.lock().map(|s| s.server.clone()).unwrap_or_default();
        let result = match take {
            Take::Pc(recorder) => recorder.stop().and_then(|pcm| {
                if pcm.len() < 16_000 / 4 {
                    bail!("too short");
                }
                transcribe(&state, &pcm)
            }),
            Take::Phone { id, revision } => phone_finish(&server, &id, revision),
        };
        match result {
            Ok(text) if !text.trim().is_empty() => {
                if let Err(e) = type_text(&format!("{} ", text.trim())) {
                    eprintln!("Cannot type the text: {e}");
                }
            }
            Ok(_) => {}
            Err(e) if e.to_string() == "too short" => {}
            Err(e) => eprintln!("Transcription failed: {e}"),
        }
    });
}

/// Prefer the running 3Yiddish service (it may use a faster engine); else run the model here.
fn transcribe(state: &Shared, pcm: &[f32]) -> Result<String> {
    let wav = crate::asr::wav(pcm)?;
    let server = state.lock().map(|s| s.server.clone()).unwrap_or_default();
    let via_service = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?
        .post(format!("{server}/v1/audio/transcriptions"))
        .multipart(reqwest::blocking::multipart::Form::new().part(
            "file",
            reqwest::blocking::multipart::Part::bytes(wav).file_name("dictation.wav"),
        ))
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json::<serde_json::Value>().ok())
        .and_then(|v| v["text"].as_str().map(str::to_owned));
    if let Some(text) = via_service {
        return Ok(text);
    }
    let mut s = state
        .lock()
        .map_err(|_| anyhow::anyhow!("dictation state poisoned"))?;
    if s.engine.is_none() {
        let (_, model, vad) = crate::model::current(&crate::paths::models())
            .context("No model installed; run `3yiddish model pull`")?;
        s.engine = Some(crate::asr::Engine::load(
            &model,
            vad.as_deref(),
            crate::asr::preferred(),
        )?);
    }
    s.engine.as_ref().unwrap().transcribe(pcm)
}

/// Put the text into the focused field. Wayland: `wtype` types it directly. Elsewhere: copy it
/// and press Ctrl+V (Cmd+V on macOS), then restore the old clipboard. Pasting is far more
/// reliable for Yiddish letters than simulating each key.
fn type_text(text: &str) -> Result<()> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let status = std::process::Command::new("wtype")
            .arg("--")
            .arg(text)
            .status()
            .context("Install `wtype` to type into apps on Wayland")?;
        if !status.success() {
            bail!("wtype failed");
        }
        return Ok(());
    }
    #[cfg(target_os = "android")]
    {
        let _ = text;
        bail!("Typing into other apps is not available on Android");
    }
    #[cfg(not(target_os = "android"))]
    type_on_desktop(text)
}

#[cfg(not(target_os = "android"))]
fn type_on_desktop(text: &str) -> Result<()> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut clipboard = arboard::Clipboard::new()?;
    let previous = clipboard.get_text().ok();
    clipboard.set_text(text.to_owned())?;
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Some(old) = previous {
        let _ = clipboard.set_text(old);
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
/// Key names people type: F1..F12, CapsLock, ScrollLock, Pause, RightAlt, RightCtrl, Insert.
fn parse_key(name: &str) -> Result<rdev::Key> {
    use rdev::Key::*;
    Ok(match name.to_ascii_lowercase().as_str() {
        "f1" => F1,
        "f2" => F2,
        "f3" => F3,
        "f4" => F4,
        "f5" => F5,
        "f6" => F6,
        "f7" => F7,
        "f8" => F8,
        "f9" => F9,
        "f10" => F10,
        "f11" => F11,
        "f12" => F12,
        "capslock" | "caps" => CapsLock,
        "scrolllock" => ScrollLock,
        "pause" => Pause,
        "rightalt" | "altgr" => AltGr,
        "rightctrl" | "rightcontrol" => ControlRight,
        "insert" => Insert,
        _ => bail!(
            "Unknown key {name}; use F1-F12, CapsLock, ScrollLock, Pause, RightAlt, RightCtrl or Insert"
        ),
    })
}

#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
mod tests {
    #[test]
    fn keys_parse() {
        assert!(super::parse_key("F9").is_ok() && super::parse_key("capslock").is_ok());
        assert!(super::parse_key("banana").is_err());
    }
}
