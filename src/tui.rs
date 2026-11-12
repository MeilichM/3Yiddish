//! Terminal-only playground. Models stay behind the same local middleware API.
use anyhow::{Context, Result, bail};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
const TABS: [&str; 7] = [
    "Schmooze",
    "Transcribe",
    "Speak",
    "Progress",
    "Models",
    "Benchmark",
    "Phone",
];
/// Tabs kept in code but hidden from the bar and keyboard cycling. Schmooze and Speak are
/// paused; Models and Benchmark only served them.
const PAUSED_TABS: [usize; 4] = [0, 2, 4, 5];
/// Developer (benchmark) machines have the read-aloud pool. Everyone else gets the plain app:
/// Transcribe with the PC microphone only; no Progress tab, no phone pairing.
static LAB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn lab() -> bool {
    LAB.load(std::sync::atomic::Ordering::Relaxed)
}
fn hidden(tab: usize) -> bool {
    PAUSED_TABS.contains(&tab) || (!lab() && matches!(tab, 3 | 6))
}
fn next_tab(from: usize, step: usize) -> usize {
    let mut tab = from;
    loop {
        tab = (tab + step) % TABS.len();
        if !hidden(tab) {
            return tab;
        }
    }
}
const BACKGROUND: Color = Color::Rgb(15, 20, 27);
const SURFACE: Color = Color::Rgb(23, 31, 42);
const INPUT_BG: Color = Color::Rgb(29, 39, 52);
const FOREGROUND: Color = Color::Rgb(230, 237, 245);
const MUTED: Color = Color::Rgb(143, 161, 180);
const OUTLINE: Color = Color::Rgb(54, 70, 90);
const ACCENT: Color = Color::Rgb(107, 224, 211);
const RECORDING: Color = Color::Rgb(255, 119, 143);
#[derive(Clone)]
struct Action {
    tab: usize,
    input: String,
    history: Vec<Value>,
    aliases: Vec<String>,
    pair_id: Option<String>,
    audio_path: Option<PathBuf>,
}
enum Update {
    Status(Value),
    Phone(Value),
    PhoneControl(Result<Value, String>),
    Done(usize, Result<Value, String>),
    /// One progress line from the conversation suite; `true` when the run ended.
    Eval(String, bool),
    /// Newer app release / newer (or first) model found by the background check.
    Available(Option<String>, Option<String>),
    /// Progress or result of an app/model update, shown as the notice line.
    Upgrade(String, bool),
    /// An audio file chosen with the attach button (None: cancelled or no picker).
    Picked(Option<String>),
    /// A finished take: its file, length in seconds and loudness over time (for the voice
    /// note's wave where the microphone gave no live level), or why it failed.
    Recorded(PathBuf, Result<(f32, Vec<f32>), String>),
    /// A download that did not finish: its button comes back, to try again (the model's
    /// label, or None for the font).
    Again(Option<String>),
    /// A one-line alert found out in the background.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    Notice(String),
    /// The service's answer to an engine switch: its new `compute` object.
    Compute(Result<Value, String>),
}
/// A sent recording or audio file that can be played back.
#[derive(Clone)]
struct Voice {
    /// Empty in demo mode (nothing to play; the slider still moves).
    path: PathBuf,
    /// 0 until known.
    seconds: f32,
    /// The wave, 0 to 7 per bar; empty for attached files.
    peaks: Vec<u8>,
}
struct VoicePlay {
    index: usize,
    started: std::time::Instant,
    /// Where playback stood when it last started or paused.
    offset: f32,
    paused: bool,
}
impl VoicePlay {
    fn position(&self) -> f32 {
        if self.paused {
            self.offset
        } else {
            self.offset + self.started.elapsed().as_secs_f32()
        }
    }
}
/// Columns the wave takes in a voice note, and how many loudness values are kept for it.
/// Two Spaces closer together than this are a held key repeating, not two taps.
const SPACE_REPEAT_MS: u128 = 150;
/// A held Space counts as released once its repeats have stopped for this long.
const SPACE_RELEASE_MS: u128 = 250;
const PEAKS: usize = 24;
const PEAK_VALUES: usize = 48;
/// Squeeze a recording's loudness history into the bars of a voice note.
fn peaks(levels: &[f32]) -> Vec<u8> {
    if levels.is_empty() {
        return Vec::new();
    }
    // Relative to the loudest moment, so a loud voice and a quiet one both show their shape.
    let loudest = levels.iter().fold(0.0f32, |p, v| p.max(*v)).max(0.02);
    (0..PEAK_VALUES)
        .map(|i| {
            let (a, b) = (
                i * levels.len() / PEAK_VALUES,
                ((i + 1) * levels.len() / PEAK_VALUES).max(1),
            );
            let peak = levels
                [a.min(levels.len() - 1)..b.min(levels.len()).max(a.min(levels.len() - 1) + 1)]
                .iter()
                .fold(0.0f32, |p, v| p.max(*v));
            ((peak / loudest).clamp(0.0, 1.0).powf(0.8) * 255.0) as u8
        })
        .collect()
}
/// Loudness (0 to 1) as a bar height 0 to 7; quiet speech still shows.
fn bar(level: f32) -> u8 {
    ((level.clamp(0.0, 1.0) * 4.0).sqrt().min(1.0) * 7.0).round() as u8
}
fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}
/// Start (or restart) listening to a voice note from `offset` seconds.
fn voice_start(state: &mut Screen, index: usize, offset: f32) {
    state.playback = None;
    state.voice_play = None;
    let Some(voice) = state.voices.get(&index).cloned() else {
        return;
    };
    if voice.path.is_file() {
        match crate::audio::Playback::start(&voice.path) {
            Ok(player) => {
                if let (Some(seconds), Some(v)) = (player.seconds(), state.voices.get_mut(&index)) {
                    v.seconds = seconds;
                }
                if offset > 0.0 {
                    player.seek(offset);
                }
                state.playback = Some(player);
            }
            Err(error) => {
                state.notice = format!("Cannot play it: {error}");
                return;
            }
        }
    } else if !voice.path.as_os_str().is_empty() {
        state.notice = "That recording is gone.".into();
        return;
    }
    state.voice_play = Some(VoicePlay {
        index,
        started: std::time::Instant::now(),
        offset,
        paused: false,
    });
}
fn voice_toggle(state: &mut Screen, index: usize) {
    if let Some(play) = state.voice_play.as_mut().filter(|p| p.index == index) {
        if play.paused {
            play.started = std::time::Instant::now();
            play.paused = false;
            if !crate::audio::SEEKS {
                // A phone's player starts the file over.
                play.offset = 0.0;
            }
        } else {
            play.offset = play.position();
            play.paused = true;
        }
        if let Some(player) = &state.playback {
            player.set_paused(play.paused);
        }
        return;
    }
    voice_start(state, index, 0.0);
}
fn voice_seek(state: &mut Screen, index: usize, permille: u16) {
    let seconds = state.voices.get(&index).map_or(0.0, |v| v.seconds);
    let target = seconds * permille as f32 / 1000.0;
    if !crate::audio::SEEKS {
        // A phone's player cannot jump: a tap on the wave plays from the start.
        voice_start(state, index, 0.0);
        return;
    }
    if let Some(play) = state.voice_play.as_mut().filter(|p| p.index == index) {
        play.offset = target;
        play.started = std::time::Instant::now();
        if let Some(player) = &state.playback {
            player.seek(target);
        }
        return;
    }
    voice_start(state, index, target);
}
#[cfg(not(target_os = "android"))]
type Clip = arboard::Clipboard;
#[cfg(target_os = "android")]
type Clip = ();
#[derive(Default)]
struct Screen {
    /// Plain app: this session's messages, newest last; `true` marks what the user sent
    /// (a file name, a recording), `false` a transcript.
    plain_log: Vec<(bool, String)>,
    /// Sent recordings and files, by their place in `plain_log`: shown as voice notes.
    voices: std::collections::HashMap<usize, Voice>,
    /// The voice note being listened to.
    voice_play: Option<VoicePlay>,
    /// Microphone loudness over the current recording, one value per screen refresh.
    levels: Vec<f32>,
    rec_started: Option<std::time::Instant>,
    /// Space to talk: the last Space seen, whether a hold (a run of repeats) is under way, and
    /// a lone Space that is either a tap to stop or the start of that run.
    space_last: Option<std::time::Instant>,
    space_hold: bool,
    space_tap: Option<std::time::Instant>,
    /// Demo mode's pretend recording.
    demo_rec: Option<std::time::Instant>,
    /// The response just copied by a click, for a moment of feedback.
    copied: Option<(usize, std::time::Instant)>,
    /// The notice on screen and when it appeared, so the plain app can clear it by itself.
    notice_since: Option<(String, std::time::Instant)>,
    /// Kept alive so copied text stays on the clipboard (Linux drops it with the owner).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    clipboard: Option<Clip>,
    tab: usize,
    input: String,
    drafts: [String; 7],
    outputs: [String; 7],
    status: Option<Value>,
    /// The engine (true: GPU) a click asked for, shown at once while the service switches.
    compute_switch: Option<bool>,
    /// Demo mode's pretend engine, for a service that does not report one.
    demo_gpu: Option<bool>,
    pair: Option<Value>,
    phone_status: Option<Value>,
    phone_transcript_revision: u64,
    phone_command_busy: bool,
    phone_audio: bool,
    pair_sync_busy: bool,
    eval_running: bool,
    /// Read-aloud game: the line on screen and this session's scores.
    read_line: Option<String>,
    /// The line being recorded right now; bound when recording starts.
    read_pending: Option<String>,
    read_scores: Vec<u32>,
    /// Last scored reading for the result cards (score, line, heard, missed, count, avg).
    read_result: Option<Value>,
    /// "My readings" list mode on the Transcribe tab, its selected row, and the cached list.
    read_list: bool,
    read_pick: usize,
    read_items: Vec<Value>,
    /// Benchmark tools (read-aloud game, readings, playground) exist only where the line
    /// pool does: the owner's machine. Everyone else gets plain Record → text.
    lab: bool,
    /// Playground review: on/off, items (state/playground/playground.json, unsure first),
    /// the shown item, and the speaker's decisions by reading number.
    pg_mode: bool,
    pg_items: Vec<Value>,
    pg_pick: usize,
    pg_review: std::collections::HashMap<u64, String>,
    /// Updates found at startup (app version, model label) and whether one is running.
    app_update: Option<String>,
    model_update: Option<String>,
    /// Termux with the phone's own font: offer the font in which Yiddish letters read well.
    font_offer: bool,
    upgrading: bool,
    /// First tap on "Delete temp recordings" arms it; the second deletes.
    delete_armed: bool,
    /// A reading the phone asked to hear; sent to the phone as playback.
    phone_play: Option<PathBuf>,
    /// Distinct lines in the training log, shown as progress toward READ_GOAL.
    read_done: usize,
    /// Lines read so far, oldest first; Back walks this to redo any of them.
    read_history: Vec<String>,
    /// Last game view sent to the phone, so it is only re-sent when it changes.
    published_game: Value,
    /// Last phone game tap applied; None until the first poll sets the baseline.
    phone_command_seen: Option<u64>,
    /// A key press requested by a phone tap, handled like the matching TUI button.
    pending_key: Option<KeyCode>,
    busy: bool,
    history: Vec<Value>,
    prompt: String,
    scroll: u16,
    comparison: Option<Value>,
    correction: Option<Value>,
    review_cursor: usize,
    review_available: bool,
    notice: String,
    audio_path: Option<PathBuf>,
    playback: Option<crate::audio::Playback>,
    recording: Option<crate::audio::Recorder>,
    recording_path: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Resume {
    #[serde(default)]
    drafts: [String; 7],
    #[serde(default)]
    review_cursor: usize,
    #[serde(default)]
    correction: Option<Value>,
    #[serde(default)]
    phone_audio: bool,
    tab: usize,
    input: String,
    outputs: [String; 7],
    history: Vec<Value>,
    pair: Option<Value>,
    phone_status: Option<Value>,
    phone_transcript_revision: u64,
    prompt: String,
    scroll: u16,
    comparison: Option<Value>,
    audio_path: Option<PathBuf>,
    #[serde(default)]
    read_line: Option<String>,
    #[serde(default)]
    read_pending: Option<String>,
    #[serde(default)]
    read_result: Option<Value>,
    #[serde(default)]
    read_history: Vec<String>,
    /// The regular app's chat: (sent by the user, text) for each message.
    #[serde(default)]
    plain_log: Vec<(bool, String)>,
    /// Its voice notes: (message number, file, seconds, wave).
    #[serde(default)]
    voices: Vec<(usize, PathBuf, f32, Vec<u8>)>,
}
impl Resume {
    fn capture(s: &Screen) -> Self {
        Self {
            review_cursor: s.review_cursor,
            correction: s.correction.clone(),
            phone_audio: s.phone_audio,
            drafts: s.drafts.clone(),
            tab: s.tab,
            input: s.input.clone(),
            outputs: s.outputs.clone(),
            history: s.history.clone(),
            pair: s.pair.clone(),
            phone_status: s.phone_status.clone(),
            phone_transcript_revision: s.phone_transcript_revision,
            prompt: s.prompt.clone(),
            scroll: s.scroll,
            comparison: s.comparison.clone(),
            audio_path: s.audio_path.clone(),
            read_line: s.read_line.clone(),
            read_pending: s.read_pending.clone(),
            read_result: s.read_result.clone(),
            read_history: s.read_history.clone(),
            plain_log: s.plain_log.clone(),
            voices: s
                .voices
                .iter()
                .map(|(index, v)| (*index, v.path.clone(), v.seconds, v.peaks.clone()))
                .collect(),
        }
    }
    fn apply(self, s: &mut Screen) {
        s.review_cursor = self.review_cursor;
        s.correction = self.correction;
        s.phone_audio = self.phone_audio;
        s.drafts = self.drafts;
        s.tab = self.tab.min(TABS.len() - 1);
        if hidden(s.tab) {
            s.tab = next_tab(s.tab, 1);
        }
        s.input = self.input;
        s.outputs = self.outputs;
        s.history = self.history;
        s.pair = self.pair;
        s.phone_status = self.phone_status;
        s.phone_transcript_revision = self.phone_transcript_revision;
        s.prompt = self.prompt;
        s.scroll = self.scroll;
        s.comparison = self.comparison;
        s.audio_path = self.audio_path;
        s.read_line = self.read_line;
        s.read_pending = self.read_pending;
        s.read_result = self.read_result;
        s.plain_log = self.plain_log;
        s.voices = self
            .voices
            .into_iter()
            .map(|(index, path, seconds, peaks)| {
                (
                    index,
                    Voice {
                        path,
                        seconds,
                        peaks,
                    },
                )
            })
            .collect();
        s.read_history = self.read_history;
    }
    fn save(&self) -> Result<PathBuf> {
        use std::io::Write;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > 8 * 1024 * 1024 {
            bail!("Workspace is too large to refresh safely");
        }
        let path =
            std::env::temp_dir().join(format!("3yiddish-refresh-{}.json", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        if let Err(error) = file.write_all(&bytes) {
            let _ = std::fs::remove_file(&path);
            return Err(error.into());
        }
        Ok(path)
    }
}
struct Exit {
    pair_id: Option<String>,
    refresh: Option<PathBuf>,
}
fn reload(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::{ffi::OsStrExt, process::CommandExt};
        // install replaces the binary inode; Linux marks the running path deleted.
        let current = std::env::current_exe()?;
        let bytes = current.as_os_str().as_bytes();
        let binary = Path::new(std::ffi::OsStr::from_bytes(
            bytes.strip_suffix(b" (deleted)").unwrap_or(bytes),
        ));
        let error = std::process::Command::new(binary)
            .args(std::env::args_os().skip(1))
            .env("THREE_YIDDISH_TUI_RESUME", path)
            .exec();
        Err(error).context("Cannot refresh the TUI executable")
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        bail!("In-place refresh currently requires Unix");
    }
}
/// Stop the PC microphone. Nothing here waits: the screen shows the voice note at once, and
/// a helper thread finishes the take (its last moments, the WAV file) and reports back with
/// `Update::Recorded`, which sends it to be transcribed.
fn finish_recording(state: &mut Screen, tx: &mpsc::UnboundedSender<Update>) {
    let Some(recorder) = state.recording.take() else {
        return;
    };
    let Some(path) = state.recording_path.take() else {
        return;
    };
    if !state.lab {
        // A voice note the user can listen back to; its exact length arrives with the file.
        state.voices.insert(
            state.plain_log.len(),
            Voice {
                path: path.clone(),
                seconds: state
                    .rec_started
                    .map_or(0.0, |at| at.elapsed().as_secs_f32()),
                peaks: peaks(&state.levels),
            },
        );
        state.plain_log.push((true, String::new()));
    }
    state.busy = true;
    let tx = tx.clone();
    std::thread::spawn(move || {
        let saved = recorder.stop().and_then(|pcm| {
            let mut options = std::fs::OpenOptions::new();
            options.create(true).truncate(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            std::io::Write::write_all(&mut options.open(&path)?, &crate::asr::wav(&pcm)?)?;
            // Loudness twenty times a second, like the live level while recording.
            let levels = pcm
                .chunks(800)
                .map(|c| c.iter().fold(0.0f32, |p, v| p.max(v.abs())))
                .collect();
            Ok((pcm.len() as f32 / 16_000.0, levels))
        });
        let _ = tx.send(Update::Recorded(path, saved.map_err(|e| e.to_string())));
    });
}
fn speak_destination(input: &str, directory: &Path) -> Result<(String, PathBuf)> {
    let (text, path) = if let Some((text, file)) = input.rsplit_once('|') {
        if file.trim().is_empty() {
            bail!("Output filename is empty");
        }
        (text.trim(), PathBuf::from(file.trim()))
    } else {
        std::fs::create_dir_all(directory)?;
        (
            input.trim(),
            directory.join(format!("speech-{}.wav", uuid::Uuid::new_v4())),
        )
    };
    if text.is_empty() {
        bail!("Enter text to speak");
    }
    if path.exists() {
        bail!("That audio file already exists; choose another filename");
    }
    Ok((text.to_owned(), path))
}

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            std::io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste
        );
        // Closing with a click or a tap: the button's release is still on its way, and
        // would be typed into the shell as stray characters. Take in what arrives shortly.
        let until = std::time::Instant::now() + Duration::from_millis(250);
        while std::time::Instant::now() < until {
            if event::poll(Duration::from_millis(60)).unwrap_or(false) {
                let _ = event::read();
            }
        }
        ratatui::restore();
    }
}
fn begin_correction(state: &mut Screen) {
    let Some(answer) = state
        .history
        .last()
        .filter(|m| m["role"] == "assistant")
        .and_then(|m| m["content"].as_str())
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let source = state
        .history
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or_default();
    state.correction = Some(
        json!({"source_message":source,"original_response":answer,"model":state.status.as_ref().map(|s|s["components"]["schmooze"]["model"].clone()),"previous_draft":state.input}),
    );
    state.input = clean(answer);
}
fn begin_data_review(state: &mut Screen) -> Result<bool> {
    use std::io::BufRead;
    let path = Path::new("state/training-review.jsonl");
    if std::fs::metadata(path)?.len() > 50 * 1024 * 1024 {
        bail!("Review queue exceeds 50 MiB");
    }
    let Some(line) = std::io::BufReader::new(std::fs::File::open(path)?)
        .lines()
        .nth(state.review_cursor)
    else {
        return Ok(false);
    };
    let row: Value = serde_json::from_str(&line?)?;
    let source = row["en"].as_str().context("No English source")?;
    let draft = row["draft_yi"].as_str().context("No draft translation")?;
    let target = row["yi"].as_str().context("No source translation")?;
    state.correction = Some(
        json!({"source_message":source,"original_response":draft,"model":row["draft_model"],"source_pair":row,"previous_draft":state.input,"previous_output":state.outputs[0]}),
    );
    state.outputs[0] = format!(
        "Review {}\n\nEnglish> {source}\n\nDraft> {draft}\n\nEdit the source translation below.",
        state.review_cursor + 1
    );
    state.input = clean(target);
    state.scroll = 0;
    Ok(true)
}
fn select_tab(state: &mut Screen, tab: usize) {
    state.drafts[state.tab] = state.input.clone();
    if state.tab == 0
        && let Some(context) = state.correction.as_mut()
    {
        context["edited_draft"] = json!(state.input);
    }
    state.tab = tab;
    state.input = if tab == 0 {
        state
            .correction
            .as_ref()
            .and_then(|c| {
                c["edited_draft"]
                    .as_str()
                    .or_else(|| c["original_response"].as_str())
            })
            .unwrap_or(&state.drafts[0])
            .into()
    } else {
        state.drafts[tab].clone()
    };
    state.scroll = 0;
    if tab == 1 && state.lab && state.read_line.is_none() {
        next_line(state);
    }
}
fn cancel_correction(state: &mut Screen) {
    if let Some(context) = state.correction.take() {
        if let Some(previous) = context["previous_output"].as_str() {
            state.outputs[0] = previous.into();
        }
        state.input = context["previous_draft"]
            .as_str()
            .unwrap_or_default()
            .into();
    }
}
fn save_correction(state: &mut Screen, path: &Path) -> Result<()> {
    use std::io::Write;
    let context = state
        .correction
        .as_ref()
        .context("No reply selected for correction")?;
    let rejected = context["decision"] == "reject";
    if state.input.trim().is_empty() && !rejected {
        bail!("A correction cannot be empty");
    }
    let original = context["original_response"].as_str().unwrap_or_default();
    let record = json!({"id":uuid::Uuid::new_v4().to_string(),"version":1,"created_unix":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),"task":if context.get("source_pair").is_some(){"repair_translation"}else{"correct_yiddish_reply"},"source_pair":context.get("source_pair"),"source_message":context["source_message"],"original_response":original,"corrected_response":if rejected{Value::Null}else{json!(state.input)},"model":context["model"],"review_kind":if rejected{"user_rejected"}else if original==state.input{"user_approved"}else{"user_corrected"},"native_review":"user-provided; not independently verified","sharing":"local-only; no automatic upload"});
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    // Windows refuses to lock an append-only handle (append overrides write there), so add
    // read access, which LockFileEx accepts.
    options.append(true).read(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.lock()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o077 != 0 {
            bail!("Correction file must be private (0600)");
        }
    }
    let mut bytes = serde_json::to_vec(&record)?;
    bytes.push(b'\n');
    file.write_all(&bytes)?;
    file.sync_data()?;
    if context.get("source_pair").is_some() {
        state.review_cursor += 1;
        cancel_correction(state);
        match begin_data_review(state) {
            Ok(true) => {}
            Ok(false) => state.outputs[0].push_str("\n\n✓ Review complete"),
            Err(e) => state.outputs[0].push_str(&format!("\n\nSaved. Next unavailable: {e}")),
        }
        return Ok(());
    }
    if let Some(last) = state
        .history
        .last_mut()
        .filter(|m| m["role"] == "assistant")
    {
        last["content"] = json!(state.input);
    }
    if let Some(offset) = state.outputs[0].rfind(original) {
        state.outputs[0].replace_range(offset..offset + original.len(), &state.input);
    }
    state.outputs[0].push_str("\n\n✓ Saved fix");
    cancel_correction(state);
    Ok(())
}
fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}
fn visual(text: &str, width: usize) -> String {
    let mut output = Vec::new();
    for paragraph in text.lines() {
        if paragraph.is_empty() {
            output.push(String::new());
            continue;
        }
        for line in textwrap::wrap(paragraph, width.max(1)) {
            let bidi = unicode_bidi::BidiInfo::new(&line, None);
            for paragraph in &bidi.paragraphs {
                output.push(
                    bidi.reorder_line(paragraph, paragraph.range.clone())
                        .into_owned(),
                );
            }
        }
    }
    output.join("\n")
}
fn distance<T: Eq>(a: &[T], b: &[T]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(old + 1)
                .min(diagonal + usize::from(x != y));
            diagonal = old;
        }
    }
    row[b.len()]
}
/// Folds spelling choices heimish writers make freely, applied to both sides alike, so a
/// score counts recognition mistakes rather than orthography: contracted pronouns and
/// titles (כ'/מ'/ס'/ר'), and the optional silent ה after ע/א (זעהן = זען, יאהרן = יארן).
fn heimish_variants(normalized: &str) -> String {
    normalized
        .split_whitespace()
        .map(|word| match word {
            "כ" => "איך".to_owned(),
            "מ" => "מען".to_owned(),
            "ס" => "עס".to_owned(),
            "ר" => "רבי".to_owned(),
            _ => {
                let chars: Vec<char> = word.chars().collect();
                chars
                    .iter()
                    .enumerate()
                    .filter(|&(i, &c)| {
                        !(c == 'ה'
                            && i > 0
                            && matches!(chars[i - 1], 'ע' | 'א')
                            && i + 1 < chars.len())
                    })
                    .map(|(_, &c)| c)
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn errors(reference: &str, hypothesis: &str) -> Result<(f64, f64)> {
    let a = heimish_variants(&crate::knowledge::normalize(reference));
    let b = heimish_variants(&crate::knowledge::normalize(hypothesis));
    let aw = a.split_whitespace().collect::<Vec<_>>();
    let bw = b.split_whitespace().collect::<Vec<_>>();
    // Character errors ignore word breaks: heimish spelling joins and splits words freely.
    let ac = a.chars().filter(|c| !c.is_whitespace()).collect::<Vec<_>>();
    let bc = b.chars().filter(|c| !c.is_whitespace()).collect::<Vec<_>>();
    if aw.is_empty() {
        bail!("A nonempty reference is required");
    }
    Ok((
        distance(&aw, &bw) as f64 / aw.len() as f64,
        distance(&ac, &bc) as f64 / ac.len() as f64,
    ))
}
async fn json_response(response: reqwest::Response) -> Result<Value> {
    let code = response.status();
    let value: Value = response.json().await.context("Invalid service response")?;
    if !code.is_success() {
        bail!(
            "HTTP {code}: {}",
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Request failed")
        );
    }
    Ok(value)
}
async fn execute(client: &reqwest::Client, base: &str, action: Action) -> Result<Value> {
    match action.tab {
        0 => {
            let mut history = action.history;
            history.push(json!({"role":"user","content":action.input}));
            let result = json_response(
                client
                    .post(format!("{base}/v1/chat/completions"))
                    .json(&json!({"messages":history}))
                    .send()
                    .await?,
            )
            .await?;
            let answer = result
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .context("No answer")?;
            history.push(json!({"role":"assistant","content":answer}));
            Ok(json!({"text":answer,"history":history}))
        }
        1 => {
            let (path, reference) = action
                .input
                .split_once('|')
                .map(|(p, r)| (p.trim(), Some(r.trim())))
                .unwrap_or((action.input.trim(), None));
            let metadata = tokio::fs::metadata(path)
                .await
                .context("Cannot read recording")?;
            if metadata.len() > 25 * 1024 * 1024 {
                bail!("Recording exceeds 25 MiB");
            }
            let form = reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(tokio::fs::read(path).await?)
                    .file_name("recording.wav"),
            );
            let result = json_response(
                client
                    .post(format!("{base}/v1/audio/transcriptions"))
                    .multipart(form)
                    .send()
                    .await?,
            )
            .await?;
            let text = result["text"].as_str().context("No transcript")?;
            if let Some(reference) =
                reference.filter(|_| action.history.first() == Some(&json!("read-aloud")))
            {
                let (wer, cer) = errors(reference, text)?;
                return Ok(
                    json!({"text":text,"read_aloud":{"heard":text,"wer":wer,"cer":cer,"audio":path}}),
                );
            }
            let output = if let Some(reference) = reference {
                let (wer, cer) = errors(reference, text)?;
                format!(
                    "{text}\n\nNormalized WER: {:.1}%    CER: {:.1}%\nReference: {reference}",
                    wer * 100.,
                    cer * 100.
                )
            } else {
                text.into()
            };
            Ok(json!({"text":output}))
        }
        2 => {
            let (text, path) = speak_destination(&action.input, Path::new("recordings"))?;
            let mut response = client
                .post(format!("{base}/v1/audio/speech"))
                .json(&json!({"input":text.trim(),"voice":"default","response_format":"wav"}))
                .send()
                .await?;
            if !response.status().is_success() {
                json_response(response).await?;
                unreachable!();
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > 25 * 1024 * 1024 {
                    bail!("Audio exceeds 25 MiB");
                }
                bytes.extend_from_slice(&chunk);
            }
            if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
                bail!("Invalid WAV response");
            }
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&path)?;
            file.write_all(&bytes)?;
            let phone = action.pair_id.is_some();
            if let Some(id) = action.pair_id {
                json_response(
                    client
                        .post(format!("{base}/v1/pair/{id}/playback"))
                        .body(bytes)
                        .send()
                        .await?,
                )
                .await?;
            }
            Ok(
                json!({"text":format!("{} • Play to replay\nSaved: {}",if phone{"Sent to phone"}else{"Playing on PC"},path.display()),"audio_path":path.canonicalize()?.to_string_lossy(),"play_on_pc":!phone}),
            )
        }
        6 => match action.input.as_str() {
            "record" => {
                let id = action.pair_id.context("Pair your phone first")?;
                json_response(
                    client
                        .post(format!("{base}/v1/pair/{id}/record"))
                        .send()
                        .await?,
                )
                .await
            }
            "disconnect" => {
                json_response(
                    client
                        .delete(format!("{base}/v1/pair"))
                        .json(&json!({"id":action.pair_id.context("No phone pairing")?}))
                        .send()
                        .await?,
                )
                .await
            }
            "play" => {
                let path = action.audio_path.context("Nothing to play yet")?;
                if tokio::fs::metadata(&path).await?.len() > 25 * 1024 * 1024 {
                    bail!("Audio exceeds 25 MiB");
                }
                let id = action.pair_id.context("Pair your phone first")?;
                let mut bytes = tokio::fs::read(&path).await?;
                // The phone plays WAV only. Decode natively; old WebM/Opus readings need ffmpeg.
                if !bytes.starts_with(b"RIFF") {
                    let native = bytes.clone();
                    bytes = match tokio::task::spawn_blocking(move || {
                        crate::asr::decode(native).and_then(|p| crate::asr::wav(&p))
                    })
                    .await?
                    {
                        Ok(wav) => wav,
                        Err(_) => {
                            let out = tokio::process::Command::new("ffmpeg")
                                .args(["-nostdin", "-v", "error", "-i"])
                                .arg(&path)
                                .args(["-ac", "1", "-ar", "16000", "-f", "wav", "pipe:1"])
                                .output()
                                .await
                                .context("This recording's format needs ffmpeg to convert")?;
                            anyhow::ensure!(out.status.success(), "Cannot convert this recording");
                            out.stdout
                        }
                    };
                }
                json_response(
                    client
                        .post(format!("{base}/v1/pair/{id}/playback"))
                        .body(bytes)
                        .send()
                        .await?,
                )
                .await?;
                Ok(json!({"text":"Audio sent to phone. Tap Play there if autoplay is blocked."}))
            }
            _ => {
                let pair =
                    json_response(client.post(format!("{base}/v1/pair")).send().await?).await?;
                Ok(
                    json!({"pair":pair,"pair_sync":action.input=="sync","text":"Scan the QR with your phone camera."}),
                )
            }
        },
        5 => {
            if action.aliases.len() < 2 {
                bail!(
                    "Configure at least two local reasoner aliases for a comparison. No paid models are used."
                );
            }
            let mut outputs = Vec::new();
            for alias in action.aliases.iter().take(2) {
                let start = Instant::now();
                let result=json_response(client.post(format!("{base}/v1/chat/completions")).json(&json!({"model":alias,"messages":[{"role":"user","content":action.input}]})).send().await?).await?;
                outputs.push(json!({"alias":alias,"text":result.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or_default(),"elapsed_ms":start.elapsed().as_millis()}));
            }
            if uuid::Uuid::new_v4().as_bytes()[0] & 1 == 1 {
                outputs.swap(0, 1);
            }
            Ok(
                json!({"text":"Choose A, B, Tie or Both bad. Names stay hidden until voting.","comparison":{"id":uuid::Uuid::new_v4().to_string(),"prompt":action.input,"outputs":outputs}}),
            )
        }
        _ => bail!("This tab updates automatically"),
    }
}
fn training(watches: &[(String, PathBuf)]) -> String {
    if watches.is_empty() {
        return "No training status configured.".into();
    }
    let mut text = String::new();
    for (name, path) in watches {
        text.push_str(&format!("{name}\n"));
        let result = (|| -> Result<Value> {
            let meta = std::fs::metadata(path)?;
            if meta.len() > 2 * 1024 * 1024 {
                bail!("Status file exceeds 2 MiB");
            }
            let data: Value = serde_json::from_slice(&std::fs::read(path)?)?;
            Ok(data)
        })();
        match result {
            Ok(value) => {
                let age = std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if age > 60 && value["phase"].as_str() != Some("completed") {
                    text.push_str(&format!("  Last update {age}s ago — may be stale\n"));
                }
                if let Some(jobs) = value["jobs"].as_object() {
                    for (job, status) in jobs {
                        job_line(&mut text, job, status);
                    }
                } else {
                    job_line(&mut text, name, &value);
                }
            }
            Err(error) => text.push_str(&format!("  Unavailable: {error}\n")),
        };
        text.push('\n');
    }
    clean(&text)
}
fn job_line(text: &mut String, name: &str, v: &Value) {
    let r = v.get("result").filter(|r| r.is_object()).unwrap_or(v);
    text.push_str(&format!(
        "  {name}: {}",
        r["phase"].as_str().unwrap_or("unknown")
    ));
    let conversation = r["phase"].as_str() == Some("conversation_tuning");
    let current = if conversation {
        r["stage_step"].as_u64()
    } else {
        r["total_training_steps"]
            .as_u64()
            .or_else(|| r["step"].as_u64())
            .or_else(|| r["steps"].as_u64())
    };
    if let Some(step) = current {
        text.push_str(&format!("   step {step}"));
        if let Some(total) = if conversation {
            r["sft_steps_target"].as_u64()
        } else if r["phase"].as_str() == Some("completed") {
            None
        } else {
            r["target_steps"]
                .as_u64()
                .or_else(|| r["cpt_steps_target"].as_u64())
                .or_else(|| r["max_steps"].as_u64())
        } {
            text.push_str(&format!(" / {total}"));
        }
    }
    for key in [
        "train_loss",
        "validation_loss",
        "validation_chrf",
        "validation_mel_loss",
        "candidate_test_wer",
    ] {
        if let Some(n) = r[key].as_f64() {
            text.push_str(&format!("   {key}: {n:.3}"));
        }
    }
    text.push('\n');
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Click {
    ReviewData,
    SkipReview,
    RejectReview,
    CorrectReply,
    CancelCorrection,
    AudioOutput,
    Tab(usize),
    Key(KeyCode),
    Pair,
    Record,
    Disconnect,
    Transcript,
    Eval,
    ReadAloud,
    Back,
    Restart,
    UpdateApp,
    UpdateModel,
    GetFont,
    DeleteTemp,
    ReadList,
    Pick(usize),
    PgToggle,
    PgPlay,
    PgStep(bool),
    PgChoose(&'static str),
    PlayPick,
    Compute,
    Toss,
    RedoPick,
    Attach,
    CopyAt(usize),
    VoiceToggle(usize),
    /// Jump to this many thousandths of the way through.
    VoiceSeek(usize, u16),
}
/// Which engine runs the speech model, and whether there is a graphics card to switch to:
/// (on the GPU, GPU available). None when the service does not say (an older one).
fn compute(state: &Screen) -> Option<(bool, bool)> {
    let told = state.status.as_ref().and_then(|status| {
        let compute = status.get("compute")?;
        Some((
            compute["mode"].as_str()? == "gpu",
            compute["gpu_available"].as_bool()?,
        ))
    });
    let (gpu, available) = told.or(state.demo_gpu.map(|gpu| (gpu, true)))?;
    Some((state.compute_switch.unwrap_or(gpu), available))
}
/// A click on the engine chip: the other engine shows at once, and is returned for the
/// service to switch to. Nothing happens while a switch is under way or without a GPU. Demo
/// mode's pretend engine just flips.
fn switch_compute(state: &mut Screen) -> Option<bool> {
    let (gpu, available) = compute(state)?;
    if !available || state.compute_switch.is_some() {
        return None;
    }
    let told = state
        .status
        .as_ref()
        .is_some_and(|status| status.get("compute").is_some());
    if !told {
        state.demo_gpu = Some(!gpu);
        return None;
    }
    state.compute_switch = Some(!gpu);
    Some(!gpu)
}
#[derive(Clone, Copy)]
struct Hit {
    area: Rect,
    action: Click,
}
fn hit_at(hits: &[Hit], x: u16, y: u16) -> Option<Click> {
    hits.iter()
        .find(|h| h.area.contains((x, y).into()))
        .map(|h| h.action)
}
fn button(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    area: Rect,
    label: &str,
    action: Click,
    enabled: bool,
    selected: bool,
) {
    let prominent =
        selected || (enabled && matches!(action, Click::Key(KeyCode::Enter) | Click::Record));
    // Record is the one button that matters; it turns red while recording.
    let hot = if label.starts_with('■') {
        RECORDING
    } else {
        ACCENT
    };
    let style = Style::default()
        .fg(if !enabled {
            MUTED
        } else if prominent {
            BACKGROUND
        } else {
            FOREGROUND
        })
        .bg(if prominent { hot } else { SURFACE });
    let widget = if area.height >= 3 {
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(style)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(if prominent { hot } else { OUTLINE })),
            )
    } else {
        Paragraph::new(format!("[{label}]")).style(style)
    };
    frame.render_widget(widget, area);
    if enabled {
        hits.push(Hit { area, action });
    }
}
type Control<'a> = (&'a str, Click, bool);

fn recording(state: &Screen) -> bool {
    state.recording.is_some()
        || state.demo_rec.is_some()
        || state
            .phone_status
            .as_ref()
            .is_some_and(|v| v["recording"] == true || v["record_requested"] == true)
}
fn controls(state: &Screen) -> Vec<Control<'static>> {
    let active = recording(state);
    let paired = state.pair.is_some();
    let chat_paused = PAUSED_TABS.contains(&0);
    let mut controls = Vec::new();
    // Transcribe only shows its file box (and button) once something is typed into it.
    if matches!(state.tab, 0 | 2 | 5) || (state.tab == 1 && !state.input.trim().is_empty()) {
        controls.push((
            match state.tab {
                0 if state.correction.is_some() => "Save fix",
                0 => "Send",
                1 => "Transcribe",
                2 => "Speak",
                _ => "Compare",
            },
            Click::Key(KeyCode::Enter),
            !state.busy && !state.input.trim().is_empty(),
        ));
    }
    if state.tab == 0 || (state.tab == 1 && !state.read_list && !state.pg_mode) {
        controls.push((
            if active { "■ Stop" } else { "● Record" },
            Click::Record,
            if paired {
                !state.phone_command_busy
            } else {
                active || !state.busy
            },
        ));
    }
    if state.tab == 1 && state.pg_mode {
        let any = !state.pg_items.is_empty();
        controls.extend([
            ("◀ Game", Click::PgToggle, true),
            ("▶ Play", Click::PgPlay, any && !state.busy),
            ("✔ Gold", Click::PgChoose("gold"), any),
            ("✔ Line", Click::PgChoose("line"), any),
            ("✔ Reference", Click::PgChoose("yl"), any),
            ("✔ Ours", Click::PgChoose("ours"), any),
            ("✗ Toss", Click::PgChoose("toss"), any),
            ("◀", Click::PgStep(false), any),
            ("▶", Click::PgStep(true), any),
            (
                "Refresh",
                Click::Key(KeyCode::Char('r')),
                !state.busy && !state.phone_command_busy && !active,
            ),
            ("Quit", Click::Key(KeyCode::Esc), true),
        ]);
        return controls;
    }
    if state.tab == 1 && state.read_list {
        let any = !state.read_items.is_empty();
        controls.extend([
            ("◀ Game", Click::ReadList, true),
            ("▶ Play", Click::PlayPick, any && !state.busy),
            ("✗ Toss", Click::Toss, any && !state.busy),
            ("↺ Redo", Click::RedoPick, any && !active),
            (
                "Refresh",
                Click::Key(KeyCode::Char('r')),
                !state.busy && !state.phone_command_busy && !active,
            ),
            ("Quit", Click::Key(KeyCode::Esc), true),
        ]);
        return controls;
    }
    if state.tab == 1 && state.read_line.is_some() {
        let settled = !state.busy && !active && !listening(state);
        controls.push(("Skip", Click::ReadAloud, settled));
        if can_go_back(state) {
            controls.push(("◀ Back", Click::Back, settled));
        }
        controls.push(("📋 Readings", Click::ReadList, !active));
        if Path::new(PG_FILE).is_file() {
            controls.push(("🧪 Playground", Click::PgToggle, !active));
        }
        // PC mic only: a phone recording restarts on the phone itself.
        if state.recording.is_some() {
            controls.push(("↺ Restart", Click::Restart, true));
        }
    }
    if matches!(state.tab, 0..=2) && (state.audio_path.is_some() || state.playback.is_some()) {
        controls.push((
            if state.playback.is_some() {
                "Stop play"
            } else {
                "Play"
            },
            Click::Key(KeyCode::F(9)),
            !state.busy,
        ));
    }
    if state.tab == 6 {
        if paired {
            controls.push((
                if state.phone_audio {
                    "Sound: Phone"
                } else {
                    "Sound: PC"
                },
                Click::AudioOutput,
                !state.busy && state.playback.is_none(),
            ));
            controls.push(("Unpair", Click::Disconnect, !state.busy));
        } else {
            controls.push(("Pair phone", Click::Pair, !state.busy));
        }
    }
    if state.tab == 0 {
        if state.correction.is_some() {
            controls.push(("Cancel", Click::CancelCorrection, true));
            if state
                .correction
                .as_ref()
                .is_some_and(|c| c.get("source_pair").is_some())
            {
                controls.push(("Skip", Click::SkipReview, true));
                controls.push(("Reject", Click::RejectReview, true));
            }
        } else if state
            .history
            .last()
            .is_some_and(|m| m["role"] == "assistant")
        {
            controls.push(("Fix reply", Click::CorrectReply, !state.busy));
        }
    }
    // These jump into the chat tab, so they go away while it is paused.
    if !chat_paused && matches!(state.tab, 3 | 4) && state.review_available {
        controls.push(("Review data", Click::ReviewData, !state.busy));
    }
    if !chat_paused && state.tab == 6 {
        controls.push((
            "Chat",
            Click::Transcript,
            state
                .phone_status
                .as_ref()
                .is_some_and(|v| v["transcript"].as_str().is_some_and(|t| !t.is_empty())),
        ));
    }
    if state.tab == 5 {
        controls.push(("Run eval", Click::Eval, !state.eval_running));
    }
    if state.tab == 5 && state.comparison.is_some() {
        controls.extend([
            ("Vote A", Click::Key(KeyCode::F(5)), true),
            ("Vote B", Click::Key(KeyCode::F(6)), true),
            ("Tie", Click::Key(KeyCode::F(7)), true),
            ("Both bad", Click::Key(KeyCode::F(8)), true),
        ]);
    }
    if matches!(state.tab, 0 | 2 | 4 | 5) {
        controls.push(("Clear", Click::Key(KeyCode::Char('l')), true));
    }
    if state.tab == 3 {
        controls.push((
            if state.delete_armed {
                "Sure? Delete"
            } else {
                "🗑 Delete temp recordings"
            },
            Click::DeleteTemp,
            !active,
        ));
    }
    if state.app_update.is_some() {
        controls.push((
            "⬆ Update app",
            Click::UpdateApp,
            !state.upgrading && !active,
        ));
    }
    if let Some(label) = &state.model_update {
        controls.push((
            if label == "first" {
                "⬇ Get model"
            } else {
                "⬇ New model"
            },
            Click::UpdateModel,
            !state.upgrading && !active,
        ));
    }
    controls.extend([
        (
            "Refresh",
            Click::Key(KeyCode::Char('r')),
            !state.busy && !state.phone_command_busy && !active && !state.eval_running,
        ),
        ("Quit", Click::Key(KeyCode::Esc), true),
    ]);
    controls
}
// Measure and render with the same rectangles, so wrapping cannot hide click targets.
fn flow(width: u16, labels: impl IntoIterator<Item = usize>) -> Vec<Rect> {
    control_flow(width, labels, 1)
}
fn control_flow(width: u16, labels: impl IntoIterator<Item = usize>, height: u16) -> Vec<Rect> {
    let (mut x, mut y) = (0, 0);
    labels
        .into_iter()
        .map(|length| {
            let length = (length as u16 + if height == 1 { 2 } else { 4 }).min(width);
            if x > 0 && x + length > width {
                x = 0;
                y += height;
            }
            let rect = Rect::new(x, y, length, height);
            x += length + 1;
            rect
        })
        .collect()
}
fn rows(rects: &[Rect]) -> u16 {
    rects.last().map_or(0, |rect| rect.bottom())
}
fn draw_flow(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    area: Rect,
    items: &[Control<'_>],
    rects: &[Rect],
    selected: Option<usize>,
) {
    for (index, ((label, action, enabled), rect)) in items.iter().zip(rects).enumerate() {
        if rect.bottom() <= area.height {
            button(
                frame,
                hits,
                Rect::new(area.x + rect.x, area.y + rect.y, rect.width, rect.height),
                label,
                *action,
                *enabled,
                selected == Some(index),
            );
        }
    }
}
fn model_status(state: &Screen) -> String {
    let Some(status) = state.status.as_ref() else {
        return "Connecting…".into();
    };
    let mut lines = Vec::new();
    for (name, key) in [
        ("Schmooze", "schmooze"),
        ("Transcribe", "transcribe"),
        ("Speak", "speak"),
    ] {
        let value = status["components"][key]["status"]
            .as_str()
            .unwrap_or("unknown");
        let label = match value {
            "configured-not-probed" => "configured · unprobed",
            "not-configured" => "not configured",
            other => other,
        };
        lines.push(format!("{name:<12} {label}"));
    }
    if let Some(reasoners) = status["reasoners"].as_array() {
        let names = reasoners
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("\nReasoners    {names}"));
    }
    lines.join("\n")
}
fn right_to_left(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| ('\u{0590}'..='\u{05ff}').contains(&c))
}
fn chat_view(state: &Screen) -> bool {
    state.tab == 0 && state.correction.is_none() && !state.history.is_empty()
}
fn scroll_view(state: &mut Screen, older: bool, amount: u16) {
    if state.tab == 1 && state.read_list {
        let step = usize::from(amount / 3).max(1);
        state.read_pick = if older {
            state.read_pick.saturating_sub(step)
        } else {
            (state.read_pick + step).min(state.read_items.len().saturating_sub(1))
        };
        return;
    }
    if older == chat_view(state) {
        state.scroll = state.scroll.saturating_add(amount);
    } else {
        state.scroll = state.scroll.saturating_sub(amount);
    }
}
fn draw_chat(frame: &mut Frame, state: &Screen, area: Rect) {
    let mut lines: Vec<(u16, u16, Line<'static>)> = Vec::new();
    for message in &state.history {
        let role = message["role"].as_str().unwrap_or_default();
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        let text = clean(message["content"].as_str().unwrap_or_default());
        let user = role == "user";
        let maximum = area
            .width
            .saturating_sub(if user { 4 } else { 2 })
            .clamp(6, 96)
            .min(area.width);
        if maximum < 6 {
            return;
        }
        let wrapped = visual(&text, usize::from(maximum - 4));
        let width = (wrapped
            .lines()
            .map(|s| Line::from(s).width())
            .max()
            .unwrap_or(0)
            + 4)
        .max(24)
        .min(maximum as usize) as u16;
        let inner = usize::from(width - 4);
        let x = if user { area.right() - width } else { area.x };
        let label = if user { "You" } else { "3Yiddish" };
        let border = Style::default()
            .fg(if user {
                Color::Rgb(141, 181, 235)
            } else {
                ACCENT
            })
            .bg(SURFACE);
        let title = format!("─ {label} ");
        let title_width = Line::from(title.as_str()).width();
        let top = format!(
            "╭{}{}╮",
            title,
            "─".repeat(usize::from(width - 2).saturating_sub(title_width))
        );
        lines.push((x, width, Line::styled(top, border)));
        for row in wrapped.lines() {
            let pad = " ".repeat(inner.saturating_sub(Line::from(row).width()));
            let row = if right_to_left(&text) {
                format!("{pad}{row}")
            } else {
                format!("{row}{pad}")
            };
            lines.push((
                x,
                width,
                Line::from(vec![
                    Span::styled("│ ", border),
                    Span::styled(row, Style::default().fg(FOREGROUND).bg(SURFACE)),
                    Span::styled(" │", border),
                ]),
            ));
        }
        lines.push((
            x,
            width,
            Line::styled(format!("╰{}╯", "─".repeat(usize::from(width - 2))), border),
        ));
        lines.push((area.x, 0, Line::default()));
    }
    if lines.last().is_some_and(|(_, w, _)| *w == 0) {
        lines.pop();
    }
    let end = lines
        .len()
        .saturating_sub(usize::from(state.scroll).min(lines.len().saturating_sub(1)));
    let start = end.saturating_sub(area.height as usize);
    let y = area.bottom().saturating_sub((end - start) as u16);
    for (i, (x, width, line)) in lines.into_iter().enumerate().take(end).skip(start) {
        if width > 0 {
            frame.render_widget(
                Paragraph::new(line).style(Style::default().bg(SURFACE)),
                Rect::new(x, y + (i - start) as u16, width, 1),
            );
        }
    }
    if state.scroll > 0 || start > 0 {
        frame.render_widget(
            Paragraph::new("↑ Older messages").style(Style::default().fg(MUTED)),
            Rect::new(area.x, area.y, area.width.min(18), 1),
        );
    }
}
fn draw(frame: &mut Frame, state: &Screen, watches: &[(String, PathBuf)]) -> Vec<Hit> {
    if plain(state) {
        return draw_plain(frame, state);
    }
    let mut hits = Vec::new();
    let screen = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(FOREGROUND).bg(Color::Reset)),
        screen,
    );
    let margin = if screen.width > 84 { 2 } else { 0 };
    let width = screen.width.saturating_sub(margin * 2).min(118);
    let bounds = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + u16::from(screen.height >= 32),
        width,
        screen
            .height
            .saturating_sub(if screen.height >= 32 { 2 } else { 0 }),
    );
    if bounds.width < 32 || bounds.height < 12 {
        frame.render_widget(
            Paragraph::new("3Yiddish\nResize to 32 × 12 or larger."),
            bounds,
        );
        if bounds.height > 2 && bounds.width >= 6 {
            button(
                frame,
                &mut hits,
                Rect::new(bounds.x, bounds.bottom() - 1, 6, 1),
                "Quit",
                Click::Key(KeyCode::Esc),
                true,
                false,
            );
        }
        return hits;
    }
    let tabs = TABS
        .iter()
        .enumerate()
        .filter(|(i, _)| !hidden(*i))
        .map(|(i, label)| (*label, Click::Tab(i), true))
        .collect::<Vec<_>>();
    let tab_rects = flow(bounds.width, tabs.iter().map(|(label, _, _)| label.len()));
    let all_actions = controls(state);
    let header_tools = bounds.width >= 80;
    let global = |action: Click| {
        matches!(
            action,
            Click::Key(KeyCode::Esc) | Click::Key(KeyCode::Char('r'))
        )
    };
    let actions = all_actions
        .iter()
        .copied()
        .filter(|(_, action, _)| !header_tools || !global(*action))
        .collect::<Vec<_>>();
    let tall_buttons = bounds.width >= 88 && bounds.height >= 24;
    let action_rects = control_flow(
        bounds.width,
        actions.iter().map(|(label, _, _)| label.len()),
        if tall_buttons { 3 } else { 1 },
    );
    let editable = matches!(state.tab, 0 | 2 | 5) || (state.tab == 1 && !state.input.is_empty());
    let input_height = if editable {
        if bounds.height >= 28 { 5 } else { 3 }
    } else {
        0
    };
    let minimum = rows(&tab_rects) + rows(&action_rects) + input_height + 7;
    if bounds.height < minimum {
        frame.render_widget(
            Paragraph::new(format!(
                "3Yiddish\nResize to {} × {minimum} or larger.",
                bounds.width
            )),
            bounds,
        );
        button(
            frame,
            &mut hits,
            Rect::new(bounds.x, bounds.bottom() - 1, 6, 1),
            "Quit",
            Click::Key(KeyCode::Esc),
            true,
            false,
        );
        return hits;
    }
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(rows(&tab_rects) + 1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(rows(&action_rects) + 1),
    ])
    .split(bounds);
    let status = if recording(state) {
        "● Recording"
    } else if state.phone_command_busy
        || state
            .phone_status
            .as_ref()
            .is_some_and(|p| p["busy"] == true)
    {
        "◌ Transcribing"
    } else if state.busy {
        if state.tab == 2 {
            "◌ Generating voice"
        } else {
            "◌ Thinking"
        }
    } else if state
        .phone_status
        .as_ref()
        .is_some_and(|p| p["mic_ready"] == true)
    {
        "● Phone ready"
    } else if state
        .phone_status
        .as_ref()
        .is_some_and(|p| p["connected"] == true)
    {
        "○ Phone setup"
    } else {
        "● PC mic"
    };
    let header = areas[0];
    let tools_width = if header_tools { 18 } else { 0 };
    let status_width = header.width.saturating_sub(tools_width + 10).min(25);
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "3Yiddish",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )])),
        Rect::new(
            header.x,
            header.y,
            header.width.saturating_sub(tools_width + status_width),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new(status)
            .alignment(Alignment::Right)
            .style(Style::default().fg(if recording(state) {
                Color::Rgb(255, 119, 143)
            } else {
                MUTED
            })),
        Rect::new(
            header.right() - tools_width - status_width,
            header.y,
            status_width,
            1,
        ),
    );
    if header_tools {
        let global_items = all_actions
            .iter()
            .copied()
            .filter(|(_, a, _)| global(*a))
            .collect::<Vec<_>>();
        let rects = flow(18, global_items.iter().map(|(label, _, _)| label.len()));
        draw_flow(
            frame,
            &mut hits,
            Rect::new(header.right() - 17, header.y, 17, 1),
            &global_items,
            &rects,
            None,
        );
    }
    draw_flow(
        frame,
        &mut hits,
        areas[1],
        &tabs,
        &tab_rects,
        // Position in the visible bar, not the tab's index (hidden tabs are skipped).
        tabs.iter()
            .position(|(_, a, _)| *a == Click::Tab(state.tab)),
    );
    let panel = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(OUTLINE));
    let body_area = panel.inner(areas[2]);
    frame.render_widget(panel, areas[2]);
    if state.tab == 6 {
        draw_pair(frame, state, body_area);
    } else if chat_view(state) {
        draw_chat(frame, state, body_area);
    } else if state.tab == 1 && state.pg_mode {
        draw_playground(frame, state, body_area);
    } else if state.tab == 1 && state.read_list {
        draw_list(frame, state, body_area, &mut hits);
    } else if state.tab == 1 && (state.read_line.is_some() || state.read_result.is_some()) {
        draw_reading(frame, state, body_area);
    } else {
        let body = if state.tab == 3 {
            format!("{}\n\n{}", readings_summary(), training(watches))
        } else if state.tab == 4 {
            model_status(state)
        } else if let Some(v) = state.comparison.as_ref().filter(|_| state.tab == 5) {
            format!(
                "A\n{}\n\nB\n{}\n\n{}",
                v["outputs"][0]["text"].as_str().unwrap_or_default(),
                v["outputs"][1]["text"].as_str().unwrap_or_default(),
                state.outputs[state.tab]
            )
        } else {
            state.outputs[state.tab].clone()
        };
        if body.trim().is_empty() {
            let caption = match state.tab {
                0 => "Type a message or record.",
                1 => "Press ● Record and speak Yiddish.",
                2 => "Type Yiddish, then Speak.",
                5 => "Compare local models.",
                _ => "",
            };
            let y = body_area.y + body_area.height / 2;
            frame.render_widget(
                Paragraph::new(TABS[state.tab])
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(FOREGROUND).add_modifier(Modifier::BOLD)),
                Rect::new(body_area.x, y.saturating_sub(1), body_area.width, 1),
            );
            frame.render_widget(
                Paragraph::new(caption)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(MUTED)),
                Rect::new(body_area.x, y, body_area.width, 1),
            );
        } else {
            frame.render_widget(
                Paragraph::new(visual(&clean(&body), body_area.width as usize))
                    .alignment(
                        if state.tab == 1 && (state.read_line.is_some() || right_to_left(&body)) {
                            Alignment::Right
                        } else {
                            Alignment::Left
                        },
                    )
                    .style(Style::default().fg(FOREGROUND))
                    .scroll((state.scroll, 0)),
                body_area,
            );
        }
    }
    if !state.notice.is_empty() {
        frame.render_widget(
            Paragraph::new(clean(&state.notice))
                .style(Style::default().fg(Color::Rgb(255, 184, 118))),
            areas[3],
        );
    }
    if editable {
        let hint = match state.tab {
            0 if state.correction.is_some() => " Edit reply ",
            0 => " Message ",
            1 => " Audio file · optional reference after | ",
            2 => " Text to speak ",
            _ => " Comparison prompt ",
        };
        let input = areas[4];
        let inner = Rect::new(
            input.x + 1,
            input.y + 1,
            input.width.saturating_sub(2),
            input.height.saturating_sub(2),
        );
        let wrapped = visual(&state.input, inner.width as usize);
        let count = wrapped.lines().count();
        let scroll = count.saturating_sub(inner.height as usize) as u16;
        frame.render_widget(
            Paragraph::new(wrapped.clone())
                .alignment(if right_to_left(&state.input) {
                    Alignment::Right
                } else {
                    Alignment::Left
                })
                .scroll((scroll, 0))
                .style(Style::default().fg(FOREGROUND).bg(INPUT_BG))
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title(hint)
                        .border_style(Style::default().fg(ACCENT)),
                ),
            input,
        );
        if inner.width > 0 && inner.height > 0 {
            let last = wrapped.lines().last().unwrap_or_default();
            let length = Line::from(last)
                .width()
                .min(inner.width.saturating_sub(1) as usize) as u16;
            let x = if right_to_left(&state.input) {
                inner.right().saturating_sub(length + 1)
            } else {
                inner.x + length
            };
            let y = inner.y
                + (count
                    .saturating_sub(1)
                    .min(inner.height.saturating_sub(1) as usize) as u16);
            frame.set_cursor_position((x, y));
        }
    }
    draw_flow(
        frame,
        &mut hits,
        Rect::new(
            areas[5].x,
            areas[5].y + 1,
            areas[5].width,
            areas[5].height.saturating_sub(1),
        ),
        &actions,
        &action_rects,
        None,
    );
    hits
}
fn readings_summary() -> String {
    let count = |log: &str| {
        std::fs::read_to_string(log)
            .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count())
    };
    format!(
        "Your readings\n  {} in the test set (frozen)  ·  {} for training",
        count(READ_BENCH),
        count(READ_LOG)
    )
}
/// One rounded card; Yiddish is shaped and right-aligned like the chat cards.
fn card(frame: &mut Frame, area: Rect, title: &str, text: &str, color: Color, bold: bool) {
    if area.height < 3 {
        return;
    }
    let inner = area.width.saturating_sub(4) as usize;
    let mut style = Style::default().fg(FOREGROUND).bg(SURFACE);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    frame.render_widget(
        Paragraph::new(visual(&clean(text), inner))
            .alignment(if right_to_left(text) {
                Alignment::Right
            } else {
                Alignment::Left
            })
            .style(style)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        format!(" {title} "),
                        Style::default().fg(color),
                    ))
                    .border_style(Style::default().fg(color))
                    .padding(Padding::horizontal(1)),
            ),
        area,
    );
}
fn card_height(text: &str, width: u16) -> u16 {
    visual(&clean(text), width.saturating_sub(4) as usize)
        .lines()
        .count()
        .max(1) as u16
        + 2
}
/// Load the playground, unsure-and-unreviewed readings first, then the rest by number.
fn load_playground(state: &mut Screen) {
    let mut items: Vec<Value> = std::fs::read_to_string(PG_FILE)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    state.pg_review = std::fs::read_to_string(PG_REVIEW)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|r| Some((r["n"].as_u64()?, r["choice"].as_str()?.to_owned())))
        .collect();
    let review = &state.pg_review;
    items.sort_by_key(|i| {
        let n = i["n"].as_u64().unwrap_or(0);
        (i["confident"] == true || review.contains_key(&n), n)
    });
    state.pg_items = items;
    state.pg_pick = 0;
}
fn pg_to_check(state: &Screen) -> usize {
    state
        .pg_items
        .iter()
        .filter(|i| {
            i["confident"] != true && !state.pg_review.contains_key(&i["n"].as_u64().unwrap_or(0))
        })
        .count()
}
/// Record the speaker's answer for one test reading, then move to the next one to check.
fn pg_decide(state: &mut Screen, n: u64, choice: &str) {
    if !PG_CHOICES.contains(&choice) {
        return;
    }
    let row = json!({"n": n, "choice": choice, "created_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)});
    let saved = std::fs::create_dir_all("state/playground").and_then(|_| {
        std::io::Write::write_all(
            &mut std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(PG_REVIEW)?,
            format!("{row}\n").as_bytes(),
        )
    });
    if let Err(e) = saved {
        state.notice = format!("Cannot save the answer: {e}");
        return;
    }
    state.pg_review.insert(n, choice.to_owned());
    let next = (1..=state.pg_items.len())
        .map(|k| (state.pg_pick + k) % state.pg_items.len())
        .find(|&i| {
            let item = &state.pg_items[i];
            item["confident"] != true
                && !state
                    .pg_review
                    .contains_key(&item["n"].as_u64().unwrap_or(0))
        });
    if let Some(i) = next {
        state.pg_pick = i;
    }
}
/// Playground review: the judge's guess of what was said against the line and both transcripts.
fn draw_playground(frame: &mut Frame, state: &Screen, area: Rect) {
    let width = area.width.min(100);
    let x = area.x + (area.width - width) / 2;
    let Some(item) = state.pg_items.get(state.pg_pick) else {
        frame.render_widget(
            Paragraph::new(format!("No playground yet ({PG_FILE} is missing)."))
                .style(Style::default().fg(MUTED)),
            Rect::new(x, area.y + 1, width, 1),
        );
        return;
    };
    let n = item["n"].as_u64().unwrap_or(0);
    let picked = state.pg_review.get(&n);
    let head = format!(
        "Playground · {} to check · {}/{} reviewed · #{n}{}",
        pg_to_check(state),
        state.pg_review.len(),
        state.pg_items.len(),
        match (picked, item["confident"] == true) {
            (Some(c), _) => format!("   ✔ you picked: {c}"),
            (None, false) => "   ⚠ judge unsure".to_owned(),
            (None, true) => String::new(),
        }
    );
    frame.render_widget(
        Paragraph::new(head).style(Style::default().fg(FOREGROUND).add_modifier(Modifier::BOLD)),
        Rect::new(x, area.y, width, 1),
    );
    let mut y = area.y + 2;
    for (title, key, color) in [
        ("Gold: judge's guess of what you said", "gold", ACCENT),
        ("Line on screen", "line", OUTLINE),
        ("Reference heard", "yl", OUTLINE),
        ("Ours heard", "ours", OUTLINE),
    ] {
        let text = item[key].as_str().unwrap_or_default();
        let color = if picked.is_some_and(|c| c == key) {
            RECORDING
        } else {
            color
        };
        let height = card_height(text, width).min(area.bottom().saturating_sub(y));
        if height < 3 {
            break;
        }
        card(
            frame,
            Rect::new(x, y, width, height),
            title,
            text,
            color,
            key == "gold",
        );
        y += height;
    }
    if let Some(note) = item["note"].as_str().filter(|n| !n.is_empty())
        && y < area.bottom()
    {
        frame.render_widget(
            Paragraph::new(format!("Judge: {note}")).style(Style::default().fg(MUTED)),
            Rect::new(x, y, width, 1),
        );
    }
}
/// "My readings": every current reading, newest first; click one, then Play / Toss / Redo.
fn draw_list(frame: &mut Frame, state: &Screen, area: Rect, hits: &mut Vec<Hit>) {
    let width = area.width.min(100);
    let x = area.x + (area.width - width) / 2;
    let items = &state.read_items;
    frame.render_widget(
        Paragraph::new(format!(
            "My readings · {}   ·   {}",
            items.len(),
            progress(state.read_done)
        ))
        .style(Style::default().fg(FOREGROUND).add_modifier(Modifier::BOLD)),
        Rect::new(x, area.y, width, 1),
    );
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new("Nothing read yet.").style(Style::default().fg(MUTED)),
            Rect::new(x, area.y + 2, width, 1),
        );
        return;
    }
    let pick = state.read_pick.min(items.len() - 1);
    let detail = 5;
    let visible = area.height.saturating_sub(2 + detail).max(1) as usize;
    let first = pick.saturating_sub(visible - 1);
    for (row, i) in (first..items.len()).take(visible).enumerate() {
        let r = &items[i];
        let y = area.y + 2 + row as u16;
        let line = r["reference"].as_str().unwrap_or_default();
        let score = r["cer"]
            .as_f64()
            .map_or(0, |c| ((1.0 - c).max(0.0) * 100.0).round() as u32);
        let selected = i == pick;
        let style = if selected {
            Style::default().fg(BACKGROUND).bg(ACCENT)
        } else {
            Style::default().fg(FOREGROUND)
        };
        let rect = Rect::new(x, y, width, 1);
        let text_width = width.saturating_sub(8) as usize;
        let clipped: String = line.chars().take(text_width).collect();
        frame.render_widget(Paragraph::new(format!("{score:>3}%")).style(style), rect);
        frame.render_widget(
            Paragraph::new(visual(&clean(&clipped), text_width))
                .alignment(Alignment::Right)
                .style(style),
            Rect::new(x + 6, y, width.saturating_sub(6), 1),
        );
        hits.push(Hit {
            area: rect,
            action: Click::Pick(i),
        });
    }
    let r = &items[pick];
    let heard = r["heard"].as_str().unwrap_or_default();
    card(
        frame,
        Rect::new(x, area.bottom().saturating_sub(detail), width, detail),
        "3Yiddish heard",
        heard,
        OUTLINE,
        false,
    );
}
/// The read-aloud game: last result on top, the line to read next below it.
fn draw_reading(frame: &mut Frame, state: &Screen, area: Rect) {
    let active = recording(state);
    let listening = listening(state);
    let width = area.width.min(100);
    let x = area.x + (area.width - width) / 2;
    let mut y = area.y + 1;
    let bottom = area.bottom();
    let mut place = |height: u16| {
        let height = height.min(bottom.saturating_sub(y));
        let rect = Rect::new(x, y, width, height);
        y += height + 1;
        rect
    };
    let result = state.read_result.as_ref();
    let headline = match result {
        Some(r) if r["missed_audio"] == true => {
            "🤷 Didn't catch that. Record and read it again.".to_owned()
        }
        Some(r) => format!(
            "{} {}%      {} · avg {}%",
            r["badge"].as_str().unwrap_or_default(),
            r["score"],
            progress(state.read_done),
            r["average"]
        ),
        None => format!(
            "{}      ● Record, read the line, ■ Stop.",
            progress(state.read_done)
        ),
    };
    frame.render_widget(
        Paragraph::new(headline)
            .style(Style::default().fg(FOREGROUND).add_modifier(Modifier::BOLD)),
        place(1),
    );
    if let Some(r) = result.filter(|r| r["missed_audio"] != true) {
        let line = r["line"].as_str().unwrap_or_default();
        card(
            frame,
            place(card_height(line, width)),
            "You read",
            line,
            OUTLINE,
            false,
        );
        let mut heard = r["heard"].as_str().unwrap_or_default().to_owned();
        if let Some(missed) = r["missed"].as_array().filter(|m| !m.is_empty()) {
            let words: Vec<&str> = missed.iter().filter_map(Value::as_str).collect();
            heard.push_str(&format!("\n✗ {}", words.join("  ")));
        }
        card(
            frame,
            place(card_height(&heard, width)),
            "3Yiddish heard",
            &heard,
            OUTLINE,
            false,
        );
    }
    if let Some(next) = &state.read_line {
        let (title, color) = if active {
            ("● Recording: read this", RECORDING)
        } else if listening {
            ("👂 Listening back…", MUTED)
        } else {
            ("Next", ACCENT)
        };
        let text = format!("\n{next}\n ");
        card(
            frame,
            place(card_height(&text, width)),
            title,
            &text,
            color,
            true,
        );
    } else {
        frame.render_widget(
            Paragraph::new(format!("No unread lines left in {READ_POOL}."))
                .style(Style::default().fg(MUTED)),
            place(1),
        );
    }
}
fn draw_pair(frame: &mut Frame, state: &Screen, area: Rect) {
    if state.pair_sync_busy {
        frame.render_widget(
            Paragraph::new("Syncing PC…").style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }
    let Some(pair) = state.pair.as_ref() else {
        frame.render_widget(
            Paragraph::new(format!("Phone not paired\n\n{}", state.outputs[6]))
                .style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    };
    if let Some(phone) = state
        .phone_status
        .as_ref()
        .filter(|v| v["connected"] == true)
    {
        let indicator = if phone["recording"] == true {
            "● Recording"
        } else if phone["mic_ready"] == true {
            "● Paired"
        } else {
            "○ Enable microphone on phone"
        };
        let error = phone["error"].as_str().unwrap_or_default();
        let transcript = phone["transcript"].as_str().unwrap_or_default();
        let action_error = state.outputs[6].as_str();
        let action_error = if ["Request failed:", "Phone microphone:", "Cannot "]
            .iter()
            .any(|prefix| action_error.starts_with(prefix))
        {
            action_error
        } else {
            ""
        };
        let text = [indicator, error, action_error, transcript]
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        frame.render_widget(
            Paragraph::new(visual(&clean(&text), area.width as usize))
                .style(Style::default().fg(Color::Cyan))
                .scroll((state.scroll, 0)),
            area,
        );
        return;
    }
    let qr = pair["qr"].as_str().unwrap_or_default();
    let width = qr
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let height = qr.lines().count() as u16;
    let link = pair["join_url"].as_str().unwrap_or_default();
    let description = format!("Scan to connect\nThis PC · reusable\n\n{link}");
    if area.width >= width + 22 && area.height >= height {
        let parts = Layout::horizontal([
            Constraint::Length(width),
            Constraint::Length(2),
            Constraint::Min(20),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(qr).style(Style::default().fg(Color::Gray).bg(Color::Reset)),
            Rect { height, ..parts[0] },
        );
        frame.render_widget(
            Paragraph::new(description).wrap(Wrap { trim: false }),
            parts[2],
        );
    } else if area.width >= width && area.height >= height + 4 {
        frame.render_widget(
            Paragraph::new(qr).style(Style::default().fg(Color::Gray).bg(Color::Reset)),
            Rect::new(area.x, area.y, width, height),
        );
        frame.render_widget(
            Paragraph::new(link).wrap(Wrap { trim: false }),
            Rect::new(
                area.x,
                area.y + height + 1,
                area.width,
                area.height - height - 1,
            ),
        );
    } else {
        frame.render_widget(
            Paragraph::new(format!(
                "Scan to pair\nEnlarge terminal for QR ({width} × {height}).\n\n{link}"
            ))
            .wrap(Wrap { trim: false }),
            area,
        );
    }
}
pub async fn run(
    client: reqwest::Client,
    base: String,
    watch: Vec<String>,
    config: PathBuf,
) -> Result<()> {
    let mut watches = Vec::new();
    for spec in watch {
        let (name, path) = spec
            .split_once('=')
            .context("--watch expects NAME=STATUS_JSON")?;
        watches.push((name.to_owned(), PathBuf::from(path)));
    }
    // Optional local monitor paths do not affect the service schema or shipped examples.
    if let Ok(text) = std::fs::read_to_string(config.with_file_name("monitor.toml")) {
        let value: toml::Value = toml::from_str(&text)?;
        if let Some(table) = value.get("status").and_then(toml::Value::as_table) {
            for (name, path) in table {
                if let Some(path) = path.as_str() {
                    watches.push((name.clone(), PathBuf::from(path)));
                }
            }
        }
    }
    let (tx, rx) = mpsc::unbounded_channel::<Update>();
    let (actions, mut pending) = mpsc::unbounded_channel::<Action>();
    let (mic_actions, mut mic_pending) = mpsc::unbounded_channel::<String>();
    let (game_actions, mut game_pending) = mpsc::unbounded_channel::<(String, Value)>();
    let (compute_actions, mut compute_pending) = mpsc::unbounded_channel::<bool>();
    let compute_client = client.clone();
    let compute_base = base.clone();
    let compute_tx = tx.clone();
    let compute_worker = tokio::spawn(async move {
        while let Some(gpu) = compute_pending.recv().await {
            let mode = if gpu { "gpu" } else { "cpu" };
            let result = async {
                json_response(
                    compute_client
                        .post(format!("{compute_base}/v1/compute"))
                        .json(&json!({ "mode": mode }))
                        .send()
                        .await?,
                )
                .await
            }
            .await
            .map_err(|e: anyhow::Error| e.to_string());
            if compute_tx.send(Update::Compute(result)).is_err() {
                break;
            }
        }
    });
    let game_client = client.clone();
    let game_base = base.clone();
    let game_worker = tokio::spawn(async move {
        while let Some((id, game)) = game_pending.recv().await {
            let _ = game_client
                .post(format!("{game_base}/v1/pair/{id}/game"))
                .json(&game)
                .send()
                .await;
        }
    });
    let mic_client = client.clone();
    let mic_base = base.clone();
    let mic_tx = tx.clone();
    let mic_worker = tokio::spawn(async move {
        while let Some(id) = mic_pending.recv().await {
            let result = async {
                json_response(
                    mic_client
                        .post(format!("{mic_base}/v1/pair/{id}/record"))
                        .send()
                        .await?,
                )
                .await
            }
            .await
            .map_err(|e: anyhow::Error| e.to_string());
            if mic_tx.send(Update::PhoneControl(result)).is_err() {
                break;
            }
        }
    });
    let status_tx = tx.clone();
    let status_client = client.clone();
    let status_base = base.clone();
    let refresh = tokio::spawn(async move {
        // Pair state every 0.5 s so phone taps feel instant; service status every 2 s.
        for tick in 0u64.. {
            if tick % 4 == 0
                && let Ok(r) = status_client
                    .get(format!("{status_base}/v1/status"))
                    .send()
                    .await
                && let Ok(v) = json_response(r).await
                && status_tx.send(Update::Status(v)).is_err()
            {
                break;
            }
            if let Ok(r) = status_client
                .get(format!("{status_base}/v1/pair"))
                .send()
                .await
                && let Ok(v) = json_response(r).await
            {
                let _ = status_tx.send(Update::Phone(v));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    let cleanup_client = client.clone();
    let cleanup_base = base.clone();
    let eval_tx = tx.clone();
    let worker = tokio::spawn(async move {
        while let Some(action) = pending.recv().await {
            let tab = action.tab;
            let result = execute(&client, &base, action)
                .await
                .map_err(|e| e.to_string());
            if tx.send(Update::Done(tab, result)).is_err() {
                break;
            }
        }
    });
    let result = tokio::task::spawn_blocking(move || {
        terminal(
            rx,
            actions,
            mic_actions,
            game_actions,
            compute_actions,
            watches,
            eval_tx,
            config,
        )
    })
    .await?;
    refresh.abort();
    worker.abort();
    mic_worker.abort();
    game_worker.abort();
    compute_worker.abort();
    let exit = result?;
    let refresh_error = if let Some(path) = exit.refresh {
        let error = reload(&path);
        let _ = std::fs::remove_file(path);
        error
    } else {
        Ok(())
    };
    if let Some(id) = exit.pair_id {
        let _ = cleanup_client
            .delete(format!("{cleanup_base}/v1/pair"))
            .json(&json!({"id":id}))
            .send()
            .await;
    }
    refresh_error
}
/// Runs the fixed suite in-process against the model worker, streaming progress lines.
fn start_eval(tx: mpsc::UnboundedSender<Update>, config: PathBuf) {
    tokio::runtime::Handle::current().spawn(async move {
        let progress = tx.clone();
        let result = async {
            let cases = crate::eval::load(Path::new("docs/evaluation/schmooze-suite.toml"))?;
            let report = crate::eval::run(
                crate::config::Config::load(&config)?,
                &cases,
                &[1.0],
                |line| {
                    let _ = progress.send(Update::Eval(line, false));
                },
            )
            .await?;
            let path = format!(
                "docs/evaluation/schmooze-eval-{}.json",
                report["created_unix"]
            );
            std::fs::write(&path, serde_json::to_string_pretty(&report)? + "\n")?;
            let run = &report["runs"][0];
            anyhow::Ok(format!(
                "{}/{} checked passed, {} for review. Report: {path}",
                run["passed"], run["checked"], run["review"]
            ))
        }
        .await;
        let _ = tx.send(Update::Eval(
            result.unwrap_or_else(|e| format!("Eval failed: {e}")),
            true,
        ));
    });
}
const READ_POOL: &str = "state/read-aloud.txt";
/// The user's first 84 readings: frozen test set, never trained on.
const READ_BENCH: &str = "state/asr-bench.jsonl";
/// New readings: training material (local only).
const READ_LOG: &str = "state/asr-train.jsonl";
/// The first 100 distinct new readings become the playground test set.
const READ_GOAL: usize = 100;
const PG_FILE: &str = "state/playground/playground.json";
const PG_REVIEW: &str = "state/playground/review.jsonl";
/// What the speaker can pick as the right answer for a test reading.
const PG_CHOICES: [&str; 5] = ["gold", "line", "yl", "ours", "toss"];
/// The pool is scraped forum text; posts that are not Yiddish would teach nothing.
fn is_yiddish(line: &str) -> bool {
    const COMMON: [&str; 22] = [
        "איז", "און", "די", "דער", "דאס", "א", "נישט", "ער", "זי", "מען", "מיט", "פון", "אויף",
        "צו", "ווי", "אז", "האט", "זיך", "ס", "כ", "איך", "מיר",
    ];
    crate::knowledge::normalize(line)
        .split_whitespace()
        .filter(|w| COMMON.contains(w))
        .count()
        >= 2
}
/// A random unread Yiddish line, skipping test and already-read lines.
fn pick_line(pool: &str, used: &std::collections::HashSet<String>, roll: u128) -> Option<String> {
    let lines: Vec<&str> = pool
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !used.contains(&crate::knowledge::normalize(l)))
        .filter(|l| is_yiddish(l))
        .collect();
    (!lines.is_empty()).then(|| lines[roll as usize % lines.len()].to_owned())
}
/// Current readings, newest first: the last take per line, minus tossed lines. The log is
/// append-only; Redo adds a take and Toss adds a `tossed` marker row.
// ponytail: O(n²) over the log, fine for a few thousand readings.
fn readings(log: &str) -> Vec<Value> {
    let mut current: Vec<Value> = Vec::new();
    for row in std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        current.retain(|r| r["reference"] != row["reference"]);
        if row["tossed"] != true {
            current.push(row);
        }
    }
    current.reverse();
    current
}
/// Stable id of a reading: its audio file name, else its timestamp.
fn reading_id(row: &Value) -> String {
    row["audio"]
        .as_str()
        .and_then(|a| Path::new(a).file_stem()?.to_str().map(str::to_owned))
        .unwrap_or_else(|| row["created_unix"].to_string())
}
fn toss_reading(log: &Path, id: &str) -> Result<()> {
    let row = readings(log.to_str().context("Log path is not UTF-8")?)
        .into_iter()
        .find(|r| reading_id(r) == id)
        .context("That reading is gone")?;
    let marker = json!({"created_unix":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d|d.as_secs()).unwrap_or(0),"reference":row["reference"],"tossed":true,"id":id});
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?,
        "{marker}"
    )?;
    Ok(())
}
fn refresh_readings(state: &mut Screen, log: &str) {
    state.read_items = readings(log);
    state.read_items.truncate(500);
    state.read_pick = state
        .read_pick
        .min(state.read_items.len().saturating_sub(1));
}
fn read_lines(log: &str) -> Vec<String> {
    readings(log)
        .iter()
        .filter_map(|r| r["reference"].as_str().map(crate::knowledge::normalize))
        .collect()
}
fn next_line(state: &mut Screen) {
    let pool = std::fs::read_to_string(READ_POOL).unwrap_or_default();
    let done = read_lines(READ_LOG);
    state.read_done = done.iter().collect::<std::collections::HashSet<_>>().len();
    let used = [read_lines(READ_BENCH), done]
        .concat()
        .into_iter()
        .collect();
    let Some(line) = pick_line(&pool, &used, uuid::Uuid::new_v4().as_u128()) else {
        state.outputs[1] = format!("No unread lines left in {READ_POOL}.");
        return;
    };
    state.outputs[1] = format!("{line}\n\n🎙 Record → read it → Stop rec");
    state.read_line = Some(line);
}
fn submit_reading(state: &mut Screen, actions: &mpsc::UnboundedSender<Action>) -> Result<()> {
    let Some(line) = state.read_pending.clone() else {
        // Plain transcription: no game line attached.
        actions.send(Action {
            tab: 1,
            input: std::mem::take(&mut state.input),
            history: vec![],
            aliases: vec![],
            pair_id: None,
            audio_path: None,
        })?;
        state.busy = true;
        state.outputs[1] = "Listening back…".into();
        return Ok(());
    };
    actions.send(Action {
        tab: 1,
        input: format!("{} | {line}", state.input),
        history: vec![json!("read-aloud")],
        aliases: vec![],
        pair_id: None,
        audio_path: None,
    })?;
    state.busy = true;
    state.input.clear();
    Ok(())
}
/// Scores one reading, logs it as a training row and shows a quick result card.
fn score_reading(state: &mut Screen, heard: &str, audio: Option<PathBuf>, log: &Path) {
    let Some(line) = state.read_pending.take() else {
        return;
    };
    // Near-silent clips make Whisper invent English; don't score or save those.
    if !heard
        .chars()
        .any(|c| ('\u{05d0}'..='\u{05ea}').contains(&c))
    {
        state.read_result = Some(json!({"missed_audio": true, "line": line}));
        state.read_line = Some(line);
        return;
    }
    let Ok((wer, cer)) = errors(&line, heard) else {
        return;
    };
    let score = ((1.0 - cer).max(0.0) * 100.0).round() as u32;
    // Keep a stable copy: the worker's recent-input folder rotates after 20 files.
    let audio = audio.and_then(|path| {
        let folder = log.with_extension("");
        let kept = folder.join(path.file_name()?);
        std::fs::create_dir_all(&folder).ok()?;
        std::fs::copy(&path, &kept).ok()?;
        Some(kept)
    });
    let heard_words: std::collections::HashSet<String> = crate::knowledge::normalize(heard)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let missed: Vec<String> = crate::knowledge::normalize(&line)
        .split_whitespace()
        .filter(|w| !heard_words.contains(*w))
        .take(8)
        .map(str::to_owned)
        .collect();
    let badge = match score {
        95.. => "🔥",
        85.. => "👍",
        70.. => "😐",
        _ => "💥",
    };
    // A Redo appends a new row for the same line; readers keep the last take per line.
    let row = json!({"created_unix":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d|d.as_secs()).unwrap_or(0),"source":"read-aloud-train","reference":line,"heard":heard,"wer":wer,"cer":cer,"audio":audio});
    let logged = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .and_then(|mut f| std::io::Write::write_all(&mut f, format!("{row}\n").as_bytes()));
    // Recent average from the log itself, so it survives Refresh.
    let rows: Vec<f64> = readings(log.to_str().unwrap_or_default())
        .iter()
        .filter_map(|r| r["cer"].as_f64())
        .collect();
    let recent = &rows[..rows.len().min(20)];
    let average = if recent.is_empty() {
        score
    } else {
        (recent.iter().map(|c| (1.0 - c).max(0.0)).sum::<f64>() / recent.len() as f64 * 100.0)
            .round() as u32
    };
    state.read_scores.push(score);
    state.read_result = Some(json!({
        "score": score, "badge": badge, "line": line, "heard": heard, "missed": missed,
        "count": rows.len(), "average": average, "saved": logged.is_ok(),
    }));
    if logged.is_err() {
        state.notice = "Could not save this reading.".into();
    }
    if let Some(log) = log.to_str() {
        refresh_readings(state, log);
        state.read_done = state.read_items.len();
    }
    if state.read_history.last() != Some(&line) {
        state.read_history.push(line.clone());
        let extra = state.read_history.len().saturating_sub(100);
        state.read_history.drain(..extra);
    }
    // Auto-advance unless Skip already moved on while this one was being transcribed.
    if state.read_line.as_deref() == Some(line.as_str()) {
        next_line(state);
    }
}
/// A reading was recorded and its score has not come back yet.
// The phone takes a moment between Stop and starting its upload, so this holds from Stop
// until score_reading takes read_pending, or a failed transcription clears it.
fn listening(state: &Screen) -> bool {
    state.read_pending.is_some() && !recording(state)
}
/// What the phone shows: the same line and result cards as the Transcribe tab.
fn game_view(state: &Screen) -> Value {
    let listening = listening(state);
    json!({
        "lab": state.lab,
        // Plain mode: the phone shows the last transcript instead of a game line.
        "transcript": if state.lab { "" } else { state.outputs[1].as_str() },
        "busy": state.busy
            || state.phone_command_busy
            || state.phone_status.as_ref().is_some_and(|p| p["busy"] == true),

        "line": state.read_line,
        "result": state.read_result,
        "recording": recording(state),
        "listening": listening,
        "back": can_go_back(state),
        // Phone pages loaded before the Back button show their Redo button on this key.
        "redo": can_go_back(state),
        "progress": progress(state.read_done),
        "playground": state.pg_items.iter().map(|i| {
            let n = i["n"].as_u64().unwrap_or(0);
            json!({
                "n": n,
                "id": i["audio"].as_str().and_then(|a| Path::new(a).file_stem()?.to_str()),
                "gold": i["gold"], "line": i["line"], "yl": i["yl"], "ours": i["ours"],
                "unsure": i["confident"] != true,
                "choice": state.pg_review.get(&n),
            })
        }).collect::<Vec<_>>(),
        "readings": state.read_items.iter().take(300).map(|r| json!({
            "id": reading_id(r),
            "line": r["reference"],
            "heard": r["heard"],
            "score": r["cer"].as_f64().map(|c| ((1.0 - c).max(0.0) * 100.0).round()),
        })).collect::<Vec<_>>(),
    })
}
fn progress(done: usize) -> String {
    if done < READ_GOAL {
        format!("{done} / {READ_GOAL} done")
    } else {
        format!(
            "✓ {READ_GOAL} / {READ_GOAL} · {} extra for training",
            done - READ_GOAL
        )
    }
}
fn toss_line(state: &mut Screen, id: &str) {
    match toss_reading(Path::new(READ_LOG), id) {
        Ok(()) => {
            refresh_readings(state, READ_LOG);
            state.read_done = state.read_items.len();
            state.notice = "Tossed. It no longer counts; the line can come up again.".into();
        }
        Err(e) => state.notice = format!("Cannot toss: {e}"),
    }
}
fn can_go_back(state: &Screen) -> bool {
    state
        .read_history
        .first()
        .is_some_and(|first| state.read_line.as_ref() != Some(first))
}
/// One reading further back: from a new line to the last one read, then earlier ones.
fn step_back(state: &mut Screen) {
    let h = &state.read_history;
    let at = state
        .read_line
        .as_ref()
        .and_then(|l| h.iter().rposition(|x| x == l));
    let target = match at {
        Some(0) => None,
        Some(i) => h.get(i - 1),
        None => h.last(),
    };
    if let Some(line) = target.cloned() {
        state.read_line = Some(line);
    }
}
/// A game button tapped on the phone page; same effect as the TUI button.
fn phone_tap(state: &mut Screen, action: &str) {
    if state.read_line.is_none() && state.read_result.is_none() {
        next_line(state);
    }
    match action {
        "record" => state.pending_key = Some(KeyCode::F(10)),
        // Until the score is back, Back would skip past the reading being transcribed.
        "skip" if !recording(state) && !listening(state) => next_line(state),
        _ if action.contains(':') => {
            let (verb, id) = action.split_once(':').unwrap_or_default();
            let item = state
                .read_items
                .iter()
                .chain(&state.pg_items)
                .find(|r| reading_id(r) == id)
                .cloned();
            if verb == "pick"
                && let Some((n, choice)) = id.split_once('-')
                && let Ok(n) = n.parse::<u64>()
            {
                pg_decide(state, n, choice);
                return;
            }
            match (verb, item) {
                ("play", Some(r)) => state.phone_play = r["audio"].as_str().map(PathBuf::from),
                ("toss", Some(_)) => toss_line(state, id),
                ("redo", Some(r)) if !recording(state) => {
                    state.read_line = r["reference"].as_str().map(str::to_owned)
                }
                _ => {}
            }
        }
        // "redo" is what phone pages from before the Back button send.
        "back" | "redo" if !recording(state) && !listening(state) => step_back(state),
        _ => {}
    }
}
fn pair_needs_sync(state: &Screen, status: &Value) -> bool {
    !state.busy
        && !state.phone_command_busy
        && state
            .pair
            .as_ref()
            .is_some_and(|p| p["id"] != status["id"] || p.get("qr").is_none())
}
fn sync_pair(state: &mut Screen, actions: &mpsc::UnboundedSender<Action>) -> Result<()> {
    actions.send(Action {
        tab: 6,
        input: "sync".into(),
        history: vec![],
        aliases: vec![],
        pair_id: None,
        audio_path: None,
    })?;
    state.busy = true;
    state.pair_sync_busy = true;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn terminal(
    mut updates: mpsc::UnboundedReceiver<Update>,
    actions: mpsc::UnboundedSender<Action>,
    mic_actions: mpsc::UnboundedSender<String>,
    game_actions: mpsc::UnboundedSender<(String, Value)>,
    compute_actions: mpsc::UnboundedSender<bool>,
    watches: Vec<(String, PathBuf)>,
    eval_tx: mpsc::UnboundedSender<Update>,
    config: PathBuf,
) -> Result<Exit> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("The TUI needs an interactive terminal; use CLI commands in scripts");
    }
    quiet_stderr();
    let mut terminal = ratatui::init();
    let _restore = Restore;
    // Whatever was on the screen before (a terminal left in a half state) goes.
    terminal.clear()?;
    // Before mouse reporting starts, so nothing else arrives in the terminal's answers.
    init_graphics();
    crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    // Background check for a newer app and model; never blocks the screen, silent offline.
    let tx = eval_tx.clone();
    std::thread::spawn(move || {
        let app = crate::update::latest()
            .ok()
            .filter(|v| crate::update::newer(env!("CARGO_PKG_VERSION"), v));
        let have = crate::model::current(&crate::paths::models()).map(|(v, ..)| v);
        // Without a model the button to get one shows even offline (it then says so).
        let model = if have.is_none() {
            Some("first".to_owned())
        } else {
            crate::model::latest_version()
                .ok()
                .filter(|v| have.as_ref() != Some(v))
        };
        let _ = tx.send(Update::Available(app, model));
    });
    LAB.store(
        Path::new(READ_POOL).is_file(),
        std::sync::atomic::Ordering::Relaxed,
    );
    let mut state = Screen::default();
    if hidden(state.tab) {
        state.tab = next_tab(state.tab, 1);
    }
    state.review_available = Path::new("state/training-review.jsonl").is_file();
    state.outputs[0] = String::new();
    if let Some(path) = std::env::var_os("THREE_YIDDISH_TUI_RESUME") {
        let path = PathBuf::from(path);
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.is_file()
            && meta.len() <= 8 * 1024 * 1024
        {
            let bytes = std::fs::read(&path)?;
            std::fs::remove_file(&path)?;
            let resume: Resume =
                serde_json::from_slice(&bytes).context("Invalid refresh workspace")?;
            resume.apply(&mut state);
        }
    }
    state.lab = lab();
    state.font_offer = crate::font::wanted();
    if hidden(state.tab) {
        state.tab = next_tab(state.tab, 1);
    }
    let demo = std::env::var_os("THREE_YIDDISH_DEMO").is_some();
    state.demo_gpu = demo.then_some(true);
    if !state.lab {
        state.pair = None;
        // Demo mode starts with a couple of messages so the screen can be judged at a glance.
        if demo && state.plain_log.is_empty() {
            state.plain_log = vec![
                (true, String::new()),
                (false, "א גוטן טאג, וואס מאכט איר?".into()),
                (true, String::new()),
                (
                    false,
                    "ווען מען רעדט אריין אין מיקראפאן, שרייבט עס זיך אויס דא, שורה נאך שורה, און מען קען קליקן אויף דעם ענטפער כדי עס צו קאפירן.".into(),
                ),
            ];
            let wave: Vec<f32> = (0..96)
                .map(|i| (noise(i, 9) % 100) as f32 / 400.0 * ((i as f32 / 9.0).sin().abs() + 0.2))
                .collect();
            state.voices.insert(
                0,
                Voice {
                    path: PathBuf::new(),
                    seconds: 7.0,
                    peaks: peaks(&wave),
                },
            );
            state.voices.insert(
                2,
                Voice {
                    path: PathBuf::new(),
                    seconds: 42.0,
                    peaks: Vec::new(),
                },
            );
        }
    }
    refresh_readings(&mut state, READ_LOG);
    load_playground(&mut state);
    // Back can reach readings from earlier sessions: seed history from the log.
    if state.read_history.is_empty() {
        let mut lines: Vec<String> = readings(READ_LOG)
            .iter()
            .rev()
            .filter_map(|r| r["reference"].as_str().map(str::to_owned))
            .collect();
        state.read_history = lines.split_off(lines.len().saturating_sub(100));
    }
    if state.tab == 1 && state.lab && state.read_line.is_none() {
        next_line(&mut state);
    }
    if state.pair.is_some() {
        sync_pair(&mut state, &actions)?;
    }
    loop {
        while let Ok(update) = updates.try_recv() {
            match update {
                Update::PhoneControl(result) => {
                    state.phone_command_busy = false;
                    state.outputs[6] = match result {
                        Ok(_) => {
                            state.notice.clear();
                            String::new()
                        }
                        Err(e) => {
                            state.notice = format!("Microphone: {e}");
                            format!("Phone microphone: {e}")
                        }
                    };
                }
                Update::Status(v) => state.status = Some(v),
                Update::Compute(result) => {
                    state.compute_switch = None;
                    match result {
                        // Kept until the next status refresh brings the same news.
                        Ok(v) => {
                            if v.get("mode").is_some()
                                && let Some(status) =
                                    state.status.as_mut().and_then(Value::as_object_mut)
                            {
                                status.insert("compute".into(), v);
                            }
                        }
                        Err(e) => state.notice = format!("Engine: {e}"),
                    }
                }
                Update::Available(app, model) => {
                    state.app_update = app;
                    state.model_update = model;
                }
                Update::Upgrade(text, finished) => {
                    state.notice = text;
                    // The same words again (a second try) stay up for their full time.
                    state.notice_since = None;
                    if finished {
                        state.upgrading = false;
                    }
                }
                Update::Eval(line, done) => {
                    state.eval_running = !done;
                    state.outputs[5].push_str(&line);
                    state.outputs[5].push('\n');
                }
                Update::Phone(v) => {
                    if v["connected"] == true
                        && state.pair.as_ref().is_none_or(|p| p["id"] != v["id"])
                    {
                        state.pair = Some(json!({"id":v["id"]}));
                        state.phone_audio = true;
                        state.phone_transcript_revision = 0;
                        state.phone_command_seen = None;
                        state.published_game = Value::Null;
                    }
                    if pair_needs_sync(&state, &v) {
                        sync_pair(&mut state, &actions)?;
                    }
                    if state.pair.as_ref().is_some_and(|p| p["id"] == v["id"]) {
                        let revision = v["transcript_revision"].as_u64().unwrap_or(0);
                        if revision > state.phone_transcript_revision {
                            state.notice.clear();
                            state.phone_transcript_revision = revision;
                            state.outputs[1] = v["transcript"].as_str().unwrap_or_default().into();
                            if state.read_pending.is_some() {
                                let heard = v["transcript"].as_str().unwrap_or_default().to_owned();
                                // The speech worker keeps the phone upload in state/asr-recent.
                                let audio =
                                    std::fs::read_dir("state/asr-recent").ok().and_then(|d| {
                                        d.flatten()
                                            .map(|e| e.path())
                                            .filter(|p| p.extension().is_some_and(|x| x == "audio"))
                                            .max()
                                    });
                                score_reading(&mut state, &heard, audio, Path::new(READ_LOG));
                            }
                            if state.tab == 0 && state.input.is_empty() {
                                state.input = v["transcript"].as_str().unwrap_or_default().into();
                            }
                        }
                        if v["error"].as_str().is_some_and(|e| !e.is_empty())
                            && v["recording"] != true
                            && v["record_requested"] != true
                        {
                            state.read_pending = None;
                        }
                        let taps = v["command_revision"].as_u64().unwrap_or(0);
                        if state.phone_command_seen.is_some_and(|seen| taps > seen) {
                            phone_tap(&mut state, v["command"].as_str().unwrap_or_default());
                        }
                        state.phone_command_seen = Some(taps);
                        state.phone_status = Some(v);
                    }
                }
                Update::Recorded(path, result) => {
                    state.busy = false;
                    match result {
                        Ok((seconds, levels)) => {
                            if let Some(voice) = state.voices.values_mut().find(|v| v.path == path)
                            {
                                voice.seconds = seconds;
                                // No live level came from the microphone (a phone): the
                                // wave is drawn from the finished take.
                                if voice.peaks.iter().all(|p| *p == 0) {
                                    voice.peaks = peaks(&levels);
                                }
                            }
                            state.input = path.to_string_lossy().into_owned();
                            state.audio_path = Some(path);
                            submit_reading(&mut state, &actions)?;
                        }
                        Err(error) => {
                            // The take never became a voice note: take its empty bubble away.
                            if let Some(index) = state
                                .voices
                                .iter()
                                .find(|(_, v)| v.path == path)
                                .map(|(index, _)| *index)
                                && index + 1 == state.plain_log.len()
                            {
                                state.voices.remove(&index);
                                state.plain_log.pop();
                            }
                            state.notice = if termux() {
                                error
                            } else {
                                format!("Microphone recording failed: {error}")
                            };
                            state.read_pending = None;
                        }
                    }
                }
                Update::Notice(text) => {
                    state.notice = text;
                    state.notice_since = None;
                }
                Update::Again(Some(label)) => state.model_update = Some(label),
                Update::Again(None) => state.font_offer = crate::font::wanted(),
                Update::Picked(path) => {
                    if let Some(path) = path {
                        state.input = path;
                        state.pending_key = Some(KeyCode::Enter);
                    }
                }
                Update::Done(tab, result) => {
                    state.busy = false;
                    if tab == 6 {
                        state.pair_sync_busy = false;
                    }
                    match result {
                        Ok(value) => {
                            state.notice.clear();
                            if let Some(r) = value.get("read_aloud") {
                                let heard = r["heard"].as_str().unwrap_or_default().to_owned();
                                let audio = r["audio"].as_str().map(PathBuf::from);
                                score_reading(&mut state, &heard, audio, Path::new(READ_LOG));
                                continue;
                            }
                            if value["pair_sync"] == true {
                                if let Some(pair) = value.get("pair") {
                                    if state.pair.as_ref().is_none_or(|p| p["id"] != pair["id"]) {
                                        state.phone_status = None;
                                        state.phone_transcript_revision = 0;
                                    }
                                    state.pair = Some(pair.clone());
                                }
                                continue;
                            }
                            if let Some(pair) = value.get("pair") {
                                state.pair = Some(pair.clone());
                                state.phone_status = None;
                                state.phone_transcript_revision = 0;
                                state.phone_audio = true;
                            }
                            if value["disconnected"] == true {
                                state.pair = None;
                                state.phone_status = None;
                                state.phone_audio = false;
                            }
                            let answer = value["text"].as_str().unwrap_or_default();
                            if tab == 1 && !state.lab && !answer.trim().is_empty() {
                                state.plain_log.push((false, answer.trim().to_owned()));
                            }
                            state.outputs[tab] = if value.get("history").is_some() {
                                format!(
                                    "{}\n\nyou> {}\n\n3Yiddish> {}",
                                    state.outputs[tab], state.prompt, answer
                                )
                            } else {
                                answer.into()
                            };
                            if let Some(history) = value["history"].as_array() {
                                state.history = history.clone();
                            }
                            state.comparison = value.get("comparison").cloned();
                            if let Some(path) = value["audio_path"].as_str() {
                                state.audio_path = Some(PathBuf::from(path));
                                if value["play_on_pc"] == true {
                                    state.playback = None;
                                    match crate::audio::Playback::start(Path::new(path)) {
                                        Ok(player) => state.playback = Some(player),
                                        Err(error) => state.outputs[tab]
                                            .push_str(&format!("\nPlayback failed: {error}")),
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            if tab == 1 {
                                state.read_pending = None;
                            }
                            state.notice = if plain(&state) {
                                plain_alert(&error)
                            } else {
                                format!("Request failed: {error}")
                            };
                            if tab != 0 {
                                state.outputs[tab] = state.notice.clone();
                            }
                            state.comparison = None;
                        }
                    }
                }
            }
        }
        // The microphone could not be opened (found out in the background).
        if let Some(error) = state.recording.as_mut().and_then(|r| r.failed()) {
            state.recording = None;
            state.recording_path = None;
            state.read_pending = None;
            state.notice = error;
            state.notice_since = None;
        }
        // The recording hit its time limit by itself.
        if state.recording.as_ref().is_some_and(|r| r.finished()) {
            finish_recording(&mut state, &eval_tx);
        }
        if state.playback.as_mut().is_some_and(|p| p.done()) {
            state.playback = None;
        }
        if let Some(path) = state.phone_play.take()
            && !state.busy
        {
            actions.send(Action {
                tab: 6,
                input: "play".into(),
                history: vec![],
                aliases: vec![],
                pair_id: state
                    .pair
                    .as_ref()
                    .and_then(|v| v["id"].as_str())
                    .map(str::to_owned),
                audio_path: Some(path),
            })?;
            state.busy = true;
        }
        if let Some(id) = state.pair.as_ref().and_then(|p| p["id"].as_str()) {
            let game = game_view(&state);
            if game != state.published_game {
                game_actions.send((id.to_owned(), game.clone()))?;
                state.published_game = game;
            }
        }
        // Space to talk: the hold ended (its repeats stopped), or a lone tap asked to stop.
        if recording(&state) && state.pending_key.is_none() {
            let since = |at: Option<std::time::Instant>| at.map(|t| t.elapsed().as_millis());
            let released =
                state.space_hold && since(state.space_last).is_some_and(|ms| ms > SPACE_RELEASE_MS);
            let tapped =
                !state.space_hold && since(state.space_tap).is_some_and(|ms| ms >= SPACE_REPEAT_MS);
            if released || tapped {
                state.space_hold = false;
                state.space_tap = None;
                state.pending_key = Some(KeyCode::F(10));
            }
        } else if !recording(&state) {
            state.space_hold = false;
            state.space_tap = None;
        }
        // Follow the microphone while recording (demo mode makes up a voice).
        let live = state.recording.as_ref().map(|r| r.level()).or_else(|| {
            state.demo_rec.map(|at| {
                let t = at.elapsed().as_secs_f32();
                ((t * 7.0).sin() * (t * 2.3).sin()).abs() * 0.22
                    + (noise((t * 20.0) as u64, 3) % 100) as f32 / 1500.0
            })
        });
        match live {
            Some(level) => {
                if state.rec_started.is_none() {
                    state.rec_started = Some(std::time::Instant::now());
                    state.levels.clear();
                }
                state.levels.push(level);
            }
            None => state.rec_started = None,
        }
        // A voice note stops at its end.
        if let Some(play) = &state.voice_play {
            let seconds = state.voices.get(&play.index).map_or(0.0, |v| v.seconds);
            let finished = state.playback.as_mut().is_some_and(|p| p.done());
            if finished || (seconds > 0.0 && play.position() >= seconds) {
                state.voice_play = None;
                state.playback = None;
            }
        }
        // Alerts in the plain app go away by themselves after a few seconds.
        if state.notice.is_empty() {
            state.notice_since = None;
        } else if state
            .notice_since
            .as_ref()
            .is_none_or(|(text, _)| *text != state.notice)
        {
            state.notice_since = Some((state.notice.clone(), std::time::Instant::now()));
        } else if plain(&state)
            && state
                .notice_since
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed().as_secs() >= 4)
        {
            state.notice.clear();
        }
        let mut hits = Vec::new();
        terminal.draw(|f| {
            hits = draw(f, &state, &watches);
        })?;
        let key = if let Some(code) = state.pending_key.take() {
            KeyEvent::new(code, KeyModifiers::NONE)
        } else {
            // The plain app animates, so it redraws twice as often.
            if !event::poll(Duration::from_millis(if plain(&state) { 50 } else { 100 }))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) => key,
                Event::Paste(text) => {
                    attach_text(&mut state, &text);
                    continue;
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        scroll_view(&mut state, true, 3);
                        continue;
                    }
                    MouseEventKind::ScrollDown => {
                        scroll_view(&mut state, false, 3);
                        continue;
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        let Some(action) = hit_at(&hits, mouse.column, mouse.row) else {
                            continue;
                        };
                        let code = match action {
                            Click::ReviewData => {
                                select_tab(&mut state, 0);
                                if let Err(e) = begin_data_review(&mut state) {
                                    state.outputs[0] = format!("Review unavailable: {e}");
                                }
                                continue;
                            }
                            Click::SkipReview => {
                                state.review_cursor += 1;
                                cancel_correction(&mut state);
                                if let Err(e) = begin_data_review(&mut state) {
                                    state.outputs[0] = format!("Review unavailable: {e}");
                                }
                                continue;
                            }
                            Click::RejectReview => {
                                if let Some(c) = state.correction.as_mut() {
                                    c["decision"] = json!("reject");
                                }
                                if let Err(e) = save_correction(
                                    &mut state,
                                    Path::new("state/corrections.jsonl"),
                                ) {
                                    if let Some(c) = state.correction.as_mut() {
                                        c["decision"] = Value::Null;
                                    }
                                    state.outputs[0]
                                        .push_str(&format!("\nCannot save rejection: {e}"));
                                }
                                continue;
                            }
                            Click::CorrectReply => {
                                begin_correction(&mut state);
                                continue;
                            }
                            Click::CancelCorrection => {
                                cancel_correction(&mut state);
                                continue;
                            }
                            Click::AudioOutput => {
                                state.phone_audio = !state.phone_audio;
                                continue;
                            }
                            Click::Compute => {
                                if let Some(gpu) = switch_compute(&mut state) {
                                    compute_actions.send(gpu)?;
                                }
                                continue;
                            }
                            Click::Tab(tab) => {
                                select_tab(&mut state, tab);
                                continue;
                            }
                            Click::Attach => {
                                pick_file(eval_tx.clone());
                                continue;
                            }
                            Click::VoiceToggle(index) => {
                                voice_toggle(&mut state, index);
                                continue;
                            }
                            Click::VoiceSeek(index, permille) => {
                                voice_seek(&mut state, index, permille);
                                continue;
                            }
                            Click::CopyAt(index) => {
                                if let Some((_, text)) = state.plain_log.get(index).cloned() {
                                    copy_text(&mut state, &text, &eval_tx);
                                    state.copied = Some((index, std::time::Instant::now()));
                                }
                                continue;
                            }
                            Click::Key(code) => code,
                            Click::Pair => {
                                select_tab(&mut state, 6);
                                KeyCode::Enter
                            }
                            Click::Disconnect => {
                                select_tab(&mut state, 6);
                                KeyCode::F(8)
                            }
                            Click::ReadAloud => {
                                select_tab(&mut state, 1);
                                next_line(&mut state);
                                continue;
                            }
                            Click::DeleteTemp => {
                                if !state.delete_armed {
                                    state.delete_armed = true;
                                    continue;
                                }
                                state.delete_armed = false;
                                // Phone-upload cache and PC mic takes only; game readings and the
                                // test set are the user's deliberate data and stay.
                                let mut n = 0;
                                for dir in ["state/asr-recent", "state/recordings"] {
                                    for entry in
                                        std::fs::read_dir(dir).into_iter().flatten().flatten()
                                    {
                                        if entry.path().is_file()
                                            && std::fs::remove_file(entry.path()).is_ok()
                                        {
                                            n += 1;
                                        }
                                    }
                                }
                                state.notice = format!("Deleted {n} temporary recordings.");
                                continue;
                            }
                            Click::UpdateApp => {
                                state.upgrading = true;
                                state.notice = "Downloading the new version…".into();
                                let tx = eval_tx.clone();
                                std::thread::spawn(move || {
                                    let text = match crate::update::update() {
                                        Ok(Some(v)) => {
                                            format!("Updated to {v}. Click Refresh to start it.")
                                        }
                                        Ok(None) => "Already up to date.".into(),
                                        Err(e) => crate::model::plain_error(&e),
                                    };
                                    let _ = tx.send(Update::Upgrade(text, true));
                                });
                                state.app_update = None;
                                continue;
                            }
                            Click::UpdateModel => {
                                state.upgrading = true;
                                let tx = eval_tx.clone();
                                let label = state.model_update.take();
                                std::thread::spawn(move || {
                                    let mut last = 101;
                                    let result = crate::model::pull(
                                        &crate::paths::models(),
                                        &mut |name, done, total| {
                                            let pct = done * 100 / total.max(1);
                                            if pct != last {
                                                last = pct;
                                                let _ = tx.send(Update::Upgrade(
                                                    format!("Downloading {name}: {pct}%"),
                                                    false,
                                                ));
                                            }
                                        },
                                    );
                                    let text = match result {
                                        Ok(v) => format!("Model {v} is installed and active."),
                                        Err(e) => {
                                            let _ = tx.send(Update::Again(label));
                                            crate::model::plain_error(&e)
                                        }
                                    };
                                    let _ = tx.send(Update::Upgrade(text, true));
                                });
                                continue;
                            }
                            Click::GetFont => {
                                state.upgrading = true;
                                state.font_offer = false;
                                let tx = eval_tx.clone();
                                std::thread::spawn(move || {
                                    let mut last = 101;
                                    let result = crate::font::install(&mut |done, total| {
                                        let pct = done * 100 / total.max(1);
                                        if pct != last {
                                            last = pct;
                                            let _ = tx.send(Update::Upgrade(
                                                format!("Getting the font: {pct}%"),
                                                false,
                                            ));
                                        }
                                    });
                                    // The buttons follow the new font from the next frame.
                                    NERD.store(0, std::sync::atomic::Ordering::Relaxed);
                                    let text = match result {
                                        Ok(()) => "The font is in place.".to_owned(),
                                        Err(e) => {
                                            let _ = tx.send(Update::Again(None));
                                            crate::model::plain_error(&e)
                                        }
                                    };
                                    let _ = tx.send(Update::Upgrade(text, true));
                                });
                                continue;
                            }
                            Click::Back => {
                                step_back(&mut state);
                                continue;
                            }
                            Click::PgToggle => {
                                state.pg_mode = !state.pg_mode;
                                state.read_list = false;
                                if state.pg_mode {
                                    load_playground(&mut state);
                                }
                                continue;
                            }
                            Click::PgPlay => {
                                let item = state.pg_items.get(state.pg_pick);
                                state.audio_path =
                                    item.and_then(|r| r["audio"].as_str()).map(PathBuf::from);
                                KeyCode::F(9)
                            }
                            Click::PgStep(forward) => {
                                let len = state.pg_items.len().max(1);
                                state.pg_pick = if forward {
                                    (state.pg_pick + 1) % len
                                } else {
                                    (state.pg_pick + len - 1) % len
                                };
                                continue;
                            }
                            Click::PgChoose(choice) => {
                                if let Some(n) = state
                                    .pg_items
                                    .get(state.pg_pick)
                                    .and_then(|i| i["n"].as_u64())
                                {
                                    pg_decide(&mut state, n, choice);
                                }
                                continue;
                            }
                            Click::ReadList => {
                                state.read_list = !state.read_list;
                                if state.read_list {
                                    refresh_readings(&mut state, READ_LOG);
                                    state.read_pick = 0;
                                }
                                continue;
                            }
                            Click::Pick(i) => {
                                state.read_pick = i;
                                continue;
                            }
                            Click::PlayPick => {
                                let item = state.read_items.get(state.read_pick);
                                state.audio_path =
                                    item.and_then(|r| r["audio"].as_str()).map(PathBuf::from);
                                KeyCode::F(9)
                            }
                            Click::Toss => {
                                if let Some(id) =
                                    state.read_items.get(state.read_pick).map(reading_id)
                                {
                                    toss_line(&mut state, &id);
                                }
                                continue;
                            }
                            Click::RedoPick => {
                                if let Some(line) = state
                                    .read_items
                                    .get(state.read_pick)
                                    .and_then(|r| r["reference"].as_str())
                                {
                                    state.read_line = Some(line.to_owned());
                                    state.read_list = false;
                                }
                                continue;
                            }
                            Click::Restart => {
                                // Throw this take away (never transcribed) and record the line again.
                                state.recording = None;
                                if let Some(path) = state.recording_path.take() {
                                    let _ = std::fs::remove_file(path);
                                }
                                state.pending_key = Some(KeyCode::F(10));
                                continue;
                            }
                            Click::Eval => {
                                select_tab(&mut state, 5);
                                state.eval_running = true;
                                state.outputs[5] =
                                    "Running the conversation suite at the current LoRA scale…\n"
                                        .into();
                                start_eval(eval_tx.clone(), config.clone());
                                continue;
                            }
                            Click::Transcript => {
                                select_tab(&mut state, 6);
                                KeyCode::F(6)
                            }
                            Click::Record => {
                                if state.pair.is_none() && state.recording.is_none() {
                                    select_tab(&mut state, 1);
                                }
                                KeyCode::F(10)
                            }
                        };
                        KeyEvent::new(
                            code,
                            if matches!(code, KeyCode::Char('l' | 'r')) {
                                KeyModifiers::CONTROL
                            } else {
                                KeyModifiers::NONE
                            },
                        )
                    }
                    _ => continue,
                },
                _ => continue,
            }
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Space, with nothing typed: hold to talk, let go to send; a single tap starts and
        // another tap stops. Terminals do not say when a key is released, but a held key
        // repeats, so a run of Spaces is a hold and its end is the release.
        if plain(&state)
            && key.code == KeyCode::Char(' ')
            && key.modifiers.is_empty()
            && state.input.is_empty()
        {
            let now = std::time::Instant::now();
            if !recording(&state) {
                if !state.busy {
                    state.space_last = Some(now);
                    state.space_hold = false;
                    state.space_tap = None;
                    state.pending_key = Some(KeyCode::F(10));
                }
            } else if state.space_hold {
                state.space_last = Some(now);
            } else if state
                .space_tap
                .is_some_and(|at| now.duration_since(at).as_millis() < SPACE_REPEAT_MS)
            {
                // A second Space right behind the first: the key is being held.
                state.space_hold = true;
                state.space_tap = None;
                state.space_last = Some(now);
            } else {
                state.space_tap = Some(now);
                state.space_last = Some(now);
            }
            continue;
        }
        if plain(&state) {
            // THREE_YIDDISH_DEMO: canned replies, for trying the screen without a microphone.
            let demo = std::env::var_os("THREE_YIDDISH_DEMO").is_some();
            match key.code {
                KeyCode::Enter if !state.busy && !state.input.trim().is_empty() => {
                    let text = state.input.trim().to_owned();
                    if let Some(name) = Path::new(&text)
                        .is_file()
                        .then(|| Path::new(&text).file_name())
                        .flatten()
                    {
                        let name = name.to_string_lossy().into_owned();
                        state.voices.insert(
                            state.plain_log.len(),
                            Voice {
                                path: PathBuf::from(&text),
                                seconds: 0.0,
                                peaks: Vec::new(),
                            },
                        );
                        let _ = name;
                        state.plain_log.push((true, String::new()));
                    } else if demo {
                        state.plain_log.push((true, text));
                        state.input.clear();
                        demo_reply(&mut state, eval_tx.clone());
                        continue;
                    } else {
                        state.notice = "That is not an audio file.".into();
                        continue;
                    }
                }
                KeyCode::F(10) if demo && !state.busy && state.recording.is_none() => {
                    // First click starts a pretend recording, the second sends it.
                    match state.demo_rec.take() {
                        None => state.demo_rec = Some(std::time::Instant::now()),
                        Some(at) => {
                            let seconds = at.elapsed().as_secs_f32();
                            state.voices.insert(
                                state.plain_log.len(),
                                Voice {
                                    path: PathBuf::new(),
                                    seconds,
                                    peaks: peaks(&state.levels),
                                },
                            );
                            state.plain_log.push((true, String::new()));
                            demo_reply(&mut state, eval_tx.clone());
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let recording = state.recording.is_some()
                    || state
                        .phone_status
                        .as_ref()
                        .is_some_and(|v| v["recording"] == true || v["record_requested"] == true);
                if state.busy || state.phone_command_busy || recording {
                    state.outputs[state.tab] =
                        "Finish the current request or recording before refreshing.".into();
                    continue;
                }
                match Resume::capture(&state).save() {
                    Ok(path) => {
                        return Ok(Exit {
                            pair_id: state
                                .pair
                                .as_ref()
                                .and_then(|v| v["id"].as_str())
                                .map(str::to_owned),
                            refresh: Some(path),
                        });
                    }
                    Err(error) => state.outputs[state.tab] = format!("Cannot refresh: {error}"),
                }
            }
            KeyCode::Esc => break,
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                paste_clipboard(&mut state);
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Tab => {
                let tab = next_tab(state.tab, 1);
                select_tab(&mut state, tab);
            }
            KeyCode::BackTab => {
                let tab = next_tab(state.tab, TABS.len() - 1);
                select_tab(&mut state, tab);
            }
            KeyCode::F(6) if state.tab == 6 => {
                if let Some(text) = state
                    .phone_status
                    .as_ref()
                    .and_then(|v| v["transcript"].as_str())
                    .map(str::to_owned)
                {
                    select_tab(&mut state, 0);
                    state.input = text;
                }
            }
            KeyCode::Enter if state.tab == 6 && !state.busy => {
                actions.send(Action {
                    tab: 6,
                    input: String::new(),
                    history: vec![],
                    aliases: vec![],
                    pair_id: None,
                    audio_path: None,
                })?;
                state.busy = true;
            }
            KeyCode::F(8) if state.tab == 6 && !state.busy && state.pair.is_some() => {
                actions.send(Action {
                    tab: 6,
                    input: "disconnect".into(),
                    history: vec![],
                    aliases: vec![],
                    pair_id: state
                        .pair
                        .as_ref()
                        .and_then(|v| v["id"].as_str())
                        .map(str::to_owned),
                    audio_path: None,
                })?;
                state.busy = true;
            }
            KeyCode::F(9) if state.pair.is_some() && state.phone_audio && !state.busy => {
                actions.send(Action {
                    tab: 6,
                    input: "play".into(),
                    history: vec![],
                    aliases: vec![],
                    pair_id: state
                        .pair
                        .as_ref()
                        .and_then(|v| v["id"].as_str())
                        .map(str::to_owned),
                    audio_path: state.audio_path.clone(),
                })?;
                state.busy = true;
            }
            KeyCode::F(9) => {
                if state.playback.take().is_some() {
                    // dropping it stopped playback
                } else if let Some(path) = &state.audio_path {
                    match crate::audio::Playback::start(path) {
                        Ok(player) => state.playback = Some(player),
                        Err(error) => state.notice = format!("Cannot play audio: {error}"),
                    }
                } else {
                    state.outputs[state.tab] = "Nothing to play yet.".into();
                }
            }
            KeyCode::F(10) if state.pair.is_some() && !state.phone_command_busy => {
                if !recording(&state) {
                    state.read_pending = state.read_line.clone();
                }
                if let Some(id) = state.pair.as_ref().and_then(|v| v["id"].as_str()) {
                    mic_actions.send(id.to_owned())?;
                    state.phone_command_busy = true;
                }
            }
            KeyCode::F(10) if state.recording.is_some() || (state.tab == 1 && !state.busy) => {
                if state.recording.is_some() {
                    finish_recording(&mut state, &eval_tx);
                } else {
                    let folder = Path::new("state/recordings");
                    std::fs::create_dir_all(folder)?;
                    let path = folder
                        .canonicalize()?
                        .join(format!("{}.wav", uuid::Uuid::new_v4()));
                    // Never waits for the device: the screen flips to recording at once.
                    state.recording = Some(crate::audio::Recorder::start_now());
                    state.recording_path = Some(path);
                    state.read_pending = state.read_line.clone();
                }
            }
            KeyCode::PageUp => scroll_view(&mut state, true, 8),
            KeyCode::PageDown => scroll_view(&mut state, false, 8),
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                cancel_correction(&mut state);
                state.history.clear();
                state.outputs[state.tab].clear();
                state.comparison = None;
                state.scroll = 0;
            }
            KeyCode::F(n)
                if (5..=8).contains(&n) && state.tab == 5 && state.comparison.is_some() =>
            {
                let mut comparison = state.comparison.take().unwrap();
                comparison["vote"] = json!(match n {
                    5 => "A",
                    6 => "B",
                    7 => "tie",
                    _ => "both_bad",
                });
                let path = Path::new("state");
                std::fs::create_dir_all(path)?;
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path.join("benchmark-votes.jsonl"))?;
                writeln!(file, "{}", comparison)?;
                state.outputs[state.tab] = format!(
                    "Vote saved: {}\n\nA: {} ({} ms)\n{}\n\nB: {} ({} ms)\n{}\n\nExploratory local backend comparison; not a formal leaderboard.",
                    comparison["vote"],
                    comparison["outputs"][0]["alias"],
                    comparison["outputs"][0]["elapsed_ms"],
                    comparison["outputs"][0]["text"]
                        .as_str()
                        .unwrap_or_default(),
                    comparison["outputs"][1]["alias"],
                    comparison["outputs"][1]["elapsed_ms"],
                    comparison["outputs"][1]["text"]
                        .as_str()
                        .unwrap_or_default()
                );
            }
            KeyCode::Enter if state.tab == 0 && state.correction.is_some() && !state.busy => {
                match save_correction(&mut state, Path::new("state/corrections.jsonl")) {
                    Ok(()) => {}
                    Err(error) => state.outputs[0] = format!("Cannot save correction: {error}"),
                }
            }
            KeyCode::Enter
                if !state.busy
                    && !state.input.trim().is_empty()
                    && matches!(state.tab, 0 | 1 | 2 | 5) =>
            {
                let aliases = state
                    .status
                    .as_ref()
                    .and_then(|v| v["reasoners"].as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                state.prompt = state.input.clone();
                actions.send(Action {
                    tab: state.tab,
                    input: std::mem::take(&mut state.input),
                    history: state.history.clone(),
                    aliases,
                    pair_id: state
                        .pair
                        .as_ref()
                        .filter(|_| state.tab != 2 || state.phone_audio)
                        .and_then(|v| v["id"].as_str())
                        .map(str::to_owned),
                    audio_path: state.audio_path.clone(),
                })?;
                state.busy = true;
                state.comparison = None;
                state.scroll = 0;
            }
            KeyCode::Backspace => {
                state.input.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && state.input.chars().count() < 16000 =>
            {
                state.input.push(c);
            }
            _ => {}
        }
    }
    Ok(Exit {
        pair_id: state
            .pair
            .as_ref()
            .and_then(|v| v["id"].as_str())
            .map(str::to_owned),
        refresh: None,
    })
}
/// While the screen is ours, anything a library prints to stderr would be drawn over it.
/// Send it to state/app.log instead (Unix; on Windows the known sources are silenced).
fn quiet_stderr() {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if let Ok(log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("state/app.log")
        {
            // SAFETY: dup2 onto stderr's descriptor; `log` stays open for the process's life.
            unsafe {
                libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO);
            }
            std::mem::forget(log);
        }
    }
}
/// A failed request as the regular app says it: what went wrong in a few words, without
/// the status code in front, and short enough for a phone's width.
fn plain_alert(error: &str) -> String {
    let said = match error.strip_prefix("HTTP ") {
        Some(rest) => rest.split_once(": ").map_or(rest, |(_, said)| said),
        None => error,
    };
    if said.starts_with("No Transcribe model yet") {
        "No model yet: use Get model first.".into()
    } else {
        said.to_owned()
    }
}
/// The regular app: one centered column. Logo on top, transcripts as message cards, and a
/// typing box at the bottom with square buttons on both sides (attach left, record/send right).
fn plain(state: &Screen) -> bool {
    !state.lab && state.tab == 1
}
/// The plain app takes its colors from the terminal theme (ANSI), so it matches the desktop.
const THEME_ACCENT: Color = Color::Blue;
const THEME_RECORD: Color = Color::Red;
fn started() -> std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *START.get_or_init(std::time::Instant::now)
}
fn noise(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}
/// Live background: Yiddish letters raining in two layers, a dim slow one far away and a
/// slightly brighter faster one up close, all very faint. The wallpaper shows between.
fn draw_rain(frame: &mut Frame, area: Rect) {
    const LETTERS: [char; 22] = [
        'א', 'ב', 'ג', 'ד', 'ה', 'ח', 'ט', 'כ', 'ל', 'מ', 'נ', 'ס', 'ע', 'פ', 'צ', 'ק', 'ר', 'ש',
        'ת', 'ו', 'ז', 'י',
    ];
    // Termux stretches a letter narrower than its cell to fill it, which turns the three
    // slim ones (at the end of the list) into blobs with the phone's own font.
    let letters = if outline() { 19 } else { 22 };
    let t = started().elapsed().as_secs_f32();
    let (accent, _) = theme_rgb();
    let height = area.height as f32;
    if area.height == 0 {
        return;
    }
    let buffer = frame.buffer_mut();
    // (layer, one column in N, slowest speed, speed range, shortest trail, brightness)
    for (layer, every, slow, range, short, bright) in [
        (1u64, 5u64, 0.8f32, 2.0f32, 6u64, 0.30f32),
        (2, 12, 4.0, 5.0, 4, 0.38),
    ] {
        // Every other column, and each column in at most one layer, so drops never touch.
        for column in (0..area.width).step_by(2) {
            let seed = noise(column as u64, 101);
            let mine = if layer == 1 {
                seed.is_multiple_of(every)
            } else {
                !seed.is_multiple_of(5) && (seed / 7).is_multiple_of(every)
            };
            if !mine {
                continue;
            }
            let speed = slow + ((seed >> 8) % 100) as f32 / 100.0 * range;
            let trail = short + (seed >> 16) % 6;
            let span = height + trail as f32 + ((seed >> 24) % 40) as f32;
            let head = (t * speed + ((seed >> 32) % 1000) as f32) % span;
            for k in 0..trail {
                let row = head - k as f32;
                if row < 0.0 || row >= height {
                    continue;
                }
                // Letters flicker now and then, slower further up the trail.
                let tick = (t * 2.5) as u64 / (1 + k);
                let letter = LETTERS
                    [(noise(column as u64 * 977 + row as u64, tick + layer) % letters) as usize];
                // A narrow band of mid tones: never bright, never black, so the letters sit
                // quietly on light and dark wallpaper alike.
                let fade = bright * (0.8 + 0.2 * (1.0 - k as f32 / trail as f32));
                let color = if k == 0 {
                    // The head is only a little brighter than its trail.
                    let glow = |v: u8| (v as f32 * (bright + 0.06)) as u8;
                    Color::Rgb(glow(accent[0]), glow(accent[1]), glow(accent[2]))
                } else {
                    let dim = |v: u8| (v as f32 * fade) as u8;
                    Color::Rgb(dim(accent[0]), dim(accent[1]), dim(accent[2]))
                };
                buffer[(area.x + column, area.y + row as u16)]
                    .set_char(letter)
                    .set_fg(color);
            }
        }
    }
}
struct Graphics {
    picker: ratatui_image::picker::Picker,
    cache: std::collections::HashMap<(String, u16, u16), ratatui_image::protocol::Protocol>,
    dark: [u8; 3],
    exact: bool,
}
thread_local! {
    static GRAPHICS: std::cell::RefCell<Option<Graphics>> = const { std::cell::RefCell::new(None) };
}
thread_local! {
    /// The theme's accent, red and surface color; known even where images are not.
    static THEME: std::cell::Cell<([u8; 3], [u8; 3], [u8; 3])> =
        const { std::cell::Cell::new(([107, 224, 211], [255, 119, 143], [29, 39, 52])) };
}
/// Ask the terminal for the theme's accent, red and background (so everything matches the
/// desktop), and whether it can draw images (Kitty, iTerm2 or Sixel). Without images the app
/// draws the same shapes from characters.
fn init_graphics() {
    use ratatui_image::picker::{Picker, ProtocolType};
    // Termux tells its background but not its palette: its own blue and red stand in, so the
    // falling letters match the buttons.
    let (accent, record, dark) = theme_colors().unwrap_or(if termux() {
        ([92, 92, 255], [255, 0, 0], [29, 39, 52])
    } else {
        ([107, 224, 211], [255, 119, 143], [29, 39, 52])
    });
    THEME.with(|t| t.set((accent, record, dark)));
    PHONE_LOOK.with(|o| o.set(termux()));
    if termux() {
        // Termux draws no pictures, and it answers none of the picture questions.
        return;
    }
    let Ok(picker) = Picker::from_query_stdio() else {
        return;
    };
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return;
    }
    // Sixel rounds every picture to bands of six pixels, so one-row pictures (bubble corners)
    // do not line up; those terminals get character corners instead.
    // Kitty pictures are placed with ordinary characters. Sixel and iTerm2 pictures are raw
    // escape codes that leave the text cursor somewhere else (see `settle_cursor`).
    let exact = picker.protocol_type() == ProtocolType::Kitty;
    GRAPHICS.with(|g| {
        *g.borrow_mut() = Some(Graphics {
            picker,
            cache: std::collections::HashMap::new(),
            dark,
            exact,
        })
    });
}
fn theme_rgb() -> ([u8; 3], [u8; 3]) {
    let (accent, record, _) = THEME.with(|t| t.get());
    (accent, record)
}
/// The surface color for bubbles, buttons and the typing bar.
fn surface() -> Color {
    let (_, _, s) = THEME.with(|t| t.get());
    Color::Rgb(s[0], s[1], s[2])
}
/// The terminal theme's blue and red (ANSI 4 and 1), and a surface color one step off its
/// background, asked over the terminal itself. The device-attributes request at the end is
/// answered by every terminal that ignores the color question, and the wait has a deadline
/// for one that answers nothing.
#[cfg(unix)]
fn theme_colors() -> Option<([u8; 3], [u8; 3], [u8; 3])> {
    use std::io::Write;
    let mut out = std::io::stdout();
    out.write_all(b"\x1b]4;4;?\x07\x1b]4;1;?\x07\x1b]11;?\x07\x1b[c")
        .ok()?;
    out.flush().ok()?;
    parse_theme(&read_reply(libc::STDIN_FILENO, Duration::from_secs(1))?)
}
/// The terminal's answer up to the end of its device-attributes reply (ESC [ ? ... c), read
/// one byte at a time straight from the descriptor, so nothing after it is taken (the next
/// bytes are the user's keys). None when the answer does not come within `limit`: a terminal
/// that answers nothing must not hold up the start.
#[cfg(unix)]
fn read_reply(fd: std::os::fd::RawFd, limit: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + limit;
    let mut reply = Vec::new();
    while reply.len() < 512 {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let mut waiting = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd, for the time that is left.
        if unsafe { libc::poll(&mut waiting, 1, left.as_millis() as libc::c_int) } <= 0 {
            return None;
        }
        let mut byte = 0u8;
        // SAFETY: reads at most one byte into `byte`.
        if unsafe { libc::read(fd, (&raw mut byte).cast(), 1) } != 1 {
            return None;
        }
        reply.push(byte);
        if byte == b'c' {
            let text = String::from_utf8_lossy(&reply);
            if text.rfind("\x1b[?").is_some_and(|at| {
                !text[at + 3..text.len() - 1].contains(|c: char| c.is_alphabetic())
            }) {
                break;
            }
        }
    }
    Some(String::from_utf8_lossy(&reply).into_owned())
}
/// Accent, red and surface out of the terminal's answer. None unless it told all three.
#[cfg(unix)]
fn parse_theme(text: &str) -> Option<([u8; 3], [u8; 3], [u8; 3])> {
    let color = |index: u8| -> Option<[u8; 3]> {
        let start = text.find(&format!("]4;{index};rgb:"))? + format!("]4;{index};rgb:").len();
        let mut parts = text[start..].split(|c: char| !c.is_ascii_hexdigit());
        let mut channel = || u8::from_str_radix(parts.next()?.get(..2)?, 16).ok();
        Some([channel()?, channel()?, channel()?])
    };
    // The window's own background (OSC 11), nudged a step toward the middle so bubbles and
    // buttons stand out from it on dark and light themes alike. The theme's "black" (ANSI 0)
    // is often the background itself, so it cannot be used.
    let start = text.find("]11;rgb:")? + "]11;rgb:".len();
    let mut parts = text[start..].split(|c: char| !c.is_ascii_hexdigit());
    let mut channel = || u8::from_str_radix(parts.next()?.get(..2)?, 16).ok();
    let background = [channel()?, channel()?, channel()?];
    let light = background.iter().map(|&c| c as u32).sum::<u32>() > 3 * 128;
    let surface = background.map(|c| {
        if light {
            (c as f32 * 0.9) as u8
        } else {
            (c as f32 + (255.0 - c as f32) * 0.09) as u8
        }
    });
    Some((color(4)?, color(1)?, surface))
}
#[cfg(not(unix))]
fn theme_colors() -> Option<([u8; 3], [u8; 3], [u8; 3])> {
    None
}
/// How many columns a piece of art needs when it fills `rows` rows, scaled by a whole number.
fn art_columns(width: u32, height: u32, rows: u16) -> Option<u16> {
    GRAPHICS.with(|g| {
        let g = g.borrow();
        let font = g.as_ref()?.picker.font_size();
        let scale = (rows as u32 * font.height as u32) / height;
        (scale > 0).then(|| (width * scale).div_ceil(font.width as u32) as u16)
    })
}
/// Draw pixel art in `area`, scaled by a whole number so every pixel stays square and crisp.
/// False when the terminal cannot draw images or the art does not fit.
fn pixel_art(
    frame: &mut Frame,
    area: Rect,
    key: &str,
    centered: bool,
    art: impl FnOnce() -> image::RgbaImage,
) -> bool {
    GRAPHICS.with(|g| {
        let mut g = g.borrow_mut();
        let Some(g) = g.as_mut() else { return false };
        let id = (key.to_owned(), area.width, area.height);
        if !g.cache.contains_key(&id) {
            let font = g.picker.font_size();
            let (w, h) = (
                area.width as u32 * font.width as u32,
                area.height as u32 * font.height as u32,
            );
            let art = art();
            let scale = (w / art.width()).min(h / art.height());
            if scale == 0 {
                return false;
            }
            let scaled = image::imageops::resize(
                &art,
                art.width() * scale,
                art.height() * scale,
                image::imageops::FilterType::Nearest,
            );
            let mut canvas = image::RgbaImage::new(w, h);
            let x = if centered {
                (w - scaled.width()) / 2
            } else {
                0
            };
            image::imageops::overlay(
                &mut canvas,
                &scaled,
                x as i64,
                ((h - scaled.height()) / 2) as i64,
            );
            let Ok(protocol) = g.picker.new_protocol(
                image::DynamicImage::ImageRgba8(canvas),
                area.as_size(),
                ratatui_image::Resize::Fit(None),
            ) else {
                return false;
            };
            g.cache.insert(id.clone(), protocol);
        }
        frame.render_widget(ratatui_image::Image::new(&g.cache[&id]), area);
        if !g.exact {
            settle_cursor(frame, area);
        }
        true
    })
}
/// How wide a round button is, in columns: as wide as two rows are tall.
fn button_columns() -> u16 {
    GRAPHICS.with(|g| {
        g.borrow().as_ref().map_or(5, |g| {
            let font = g.picker.font_size();
            (font.height * 2).div_ceil(font.width.max(1))
        })
    })
}
/// After a Sixel/iTerm2 picture the terminal's cursor is wherever the picture left it, but the
/// screen writer assumes it moved one cell right, so the next neighbouring cell would be
/// printed in the wrong place. Put the cursor where it is expected.
fn settle_cursor(frame: &mut Frame, area: Rect) {
    let cell = &mut frame.buffer_mut()[(area.x, area.y)];
    let symbol = format!("{}\x1b[{};{}H", cell.symbol(), area.y + 1, area.x + 2);
    cell.set_symbol(&symbol);
}
/// One corner cell of a bubble or bar: drawn as a smooth picture where the terminal places
/// small pictures exactly, otherwise as a quarter/half block character in the same color.
fn corner(frame: &mut Frame, cell: Rect, rgb: [u8; 3], left: bool, kind: Corner) {
    if picture_fits(1) {
        let key = format!("corner-{left}-{kind:?}-{rgb:?}");
        let drawn = pixel_cell(frame, cell, &key, |cw, ch| match kind {
            Corner::Top => crate::art::half_cap(cw, ch, rgb, left, true),
            Corner::Bottom => crate::art::half_cap(cw, ch, rgb, left, false),
            Corner::Side { top, bottom } => crate::art::cap(cw, ch, rgb, left, top, bottom),
        });
        if drawn {
            return;
        }
    }
    let glyph = match (kind, left) {
        (Corner::Top, true) => "▗",
        (Corner::Top, false) => "▖",
        (Corner::Bottom, true) => "▝",
        (Corner::Bottom, false) => "▘",
        (Corner::Side { .. }, true) => "▐",
        (Corner::Side { .. }, false) => "▌",
    };
    frame.render_widget(
        Paragraph::new(glyph).style(Style::default().fg(Color::Rgb(rgb[0], rgb[1], rgb[2]))),
        cell,
    );
}
/// Can a picture `rows` tall be placed exactly? Kitty and iTerm2: always. Sixel draws in
/// bands of six pixels, so only heights that are a multiple of six line up.
fn picture_fits(rows: u16) -> bool {
    GRAPHICS.with(|g| {
        g.borrow().as_ref().is_some_and(|g| {
            // One-row pictures on Sixel spill a couple of pixels into the next row (it draws
            // in bands of six); taller ones are fine.
            g.exact
                || rows >= 3
                || (rows as u32 * g.picker.font_size().height as u32).is_multiple_of(6)
        })
    })
}
/// Both ends of a fat bar (three rows: half a row of padding, the text row, half a row):
/// truly round where a three-row picture fits, block-character corners otherwise.
fn bar_ends(frame: &mut Frame, x: u16, right: u16, text_row: u16, rgb: [u8; 3]) {
    for left in [true, false] {
        let column = if left { x } else { right };
        // Three-row pictures place fine everywhere (the round buttons are the same size).
        if pixel_cell(
            frame,
            Rect::new(column, text_row - 1, 1, 3),
            &format!("bar-end-{left}-{rgb:?}"),
            |cw, ch| crate::art::bar_end(cw, ch, rgb, left),
        ) {
            continue;
        }
        corner(
            frame,
            Rect::new(column, text_row - 1, 1, 1),
            rgb,
            left,
            Corner::Top,
        );
        corner(
            frame,
            Rect::new(column, text_row + 1, 1, 1),
            rgb,
            left,
            Corner::Bottom,
        );
    }
}
#[derive(Clone, Copy, Debug)]
enum Corner {
    /// The half row above a bar (filled in its lower half).
    Top,
    /// The half row below a bar.
    Bottom,
    /// The end of a text row; `top`/`bottom` round that end of the bubble.
    Side { top: bool, bottom: bool },
}
fn surface_rgb() -> [u8; 3] {
    THEME.with(|t| t.get()).2
}
/// A block of screen cells drawn as one image of exactly its size. False without image support.
fn pixel_cell(
    frame: &mut Frame,
    cell: Rect,
    key: &str,
    make: impl FnOnce(u32, u32) -> image::RgbaImage,
) -> bool {
    GRAPHICS.with(|g| {
        let mut g = g.borrow_mut();
        let Some(g) = g.as_mut() else { return false };
        let id = (key.to_owned(), cell.width, cell.height);
        let font = g.picker.font_size();
        // Sixel draws in bands of six pixels: a picture whose height is not a multiple of six
        // runs a few (clear) pixels into the row below, so it is given that row as well.
        let spill = !g.exact && !(font.height as u32 * cell.height as u32).is_multiple_of(6);
        let area = Rect::new(cell.x, cell.y, cell.width, cell.height + u16::from(spill));
        if area.bottom() > frame.area().bottom() {
            return false;
        }
        if !g.cache.contains_key(&id) {
            let image = make(
                font.width as u32 * cell.width as u32,
                font.height as u32 * cell.height as u32,
            );
            let Ok(protocol) = g.picker.new_protocol(
                image::DynamicImage::ImageRgba8(image),
                area.as_size(),
                ratatui_image::Resize::Fit(None),
            ) else {
                return false;
            };
            g.cache.insert(id.clone(), protocol);
        }
        frame.render_widget(ratatui_image::Image::new(&g.cache[&id]), area);
        if spill {
            // The extra row still belongs to whatever is drawn there.
            for x in cell.x..cell.right() {
                frame.buffer_mut()[(x, cell.bottom())]
                    .set_diff_option(ratatui::buffer::CellDiffOption::None);
            }
        }
        if !g.exact {
            settle_cursor(frame, cell);
        }
        true
    })
}
/// The theme's dark tone, when the terminal draws images (so bubbles get truly round ends).
fn round_theme() -> Option<[u8; 3]> {
    GRAPHICS.with(|g| g.borrow().as_ref().map(|g| g.dark))
}
/// Text-only terminal with a Nerd Font: shapes are one line tall with true half-circle ends
/// (the font's round caps are exactly one line high), instead of fat stepped ones.
fn slim_round() -> bool {
    round_theme().is_none() && nerd_font()
}
/// A one-line pill: half-circle, filled middle, half-circle.
fn pill_caps(frame: &mut Frame, x: u16, right: u16, row: u16, fill: Color) {
    let cap = Style::default().fg(fill).bg(Color::Reset);
    frame.render_widget(
        Paragraph::new("\u{e0b6}").style(cap),
        Rect::new(x, row, 1, 1),
    );
    frame.render_widget(
        Paragraph::new("\u{e0b4}").style(cap),
        Rect::new(right, row, 1, 1),
    );
}
/// Is the system's monospace font a Nerd Font (so its icon glyphs can be used)? Asked once
/// from fontconfig; anywhere that cannot be told, plain Unicode symbols are used.
fn nerd_font() -> bool {
    use std::sync::atomic::Ordering;
    if let Some(told) = NERD_FOR_TEST.with(|n| n.get()) {
        return told;
    }
    match NERD.load(Ordering::Relaxed) {
        0 => {
            let nerd = if termux() {
                // Termux has no fontconfig: its font is the file in ~/.termux (the one "Get
                // font" puts there is a Nerd Font), or the phone's own when there is none.
                std::env::var_os("HOME")
                    .and_then(|home| std::fs::read(Path::new(&home).join(".termux/font.ttf")).ok())
                    .is_some_and(|font| names_nerd_font(&font))
            } else {
                std::process::Command::new("fc-match")
                    .arg("monospace")
                    .output()
                    .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Nerd"))
            };
            NERD.store(if nerd { 2 } else { 1 }, Ordering::Relaxed);
            nerd
        }
        known => known == 2,
    }
}
/// What `nerd_font` found: 0 not asked yet, 1 no, 2 yes. Set back to 0 when the font changes
/// under the app ("Get font"), so the next frame asks again.
static NERD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
thread_local! {
    /// Tests draw each look whatever font this machine has.
    static NERD_FOR_TEST: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}
/// Does this font file call itself a Nerd Font? Its name is stored as plain bytes or as
/// two-byte characters, depending on the font.
fn names_nerd_font(font: &[u8]) -> bool {
    let wide: Vec<u8> = "Nerd Font".bytes().flat_map(|b| [0, b]).collect();
    font.windows(9).any(|w| w == b"Nerd Font") || font.windows(wide.len()).any(|w| w == wide)
}
use crate::font::termux;
thread_local! {
    /// In Termux, shapes are drawn as round outlines (see `outline`).
    static PHONE_LOOK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// The phone look. Termux draws block characters from the font, and they do not fill their
/// cells there, so filled shapes built from half blocks come out as bricks. Line characters
/// with round corners join cleanly, so every shape is a round outline.
fn outline() -> bool {
    PHONE_LOOK.with(|o| o.get())
}
/// Buttons and the typing bar as outlines too: with the phone's own font. With a Nerd Font
/// they are one-line pills with true half-circle ends, which Termux draws cleanly.
fn outline_controls() -> bool {
    outline() && !nerd_font()
}
/// A round outline around `area` (at least 2 by 2).
fn outline_box(frame: &mut Frame, area: Rect, color: Color) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let style = Style::default().fg(color).bg(Color::Reset);
    let middle = "─".repeat(area.width as usize - 2);
    frame.render_widget(
        Paragraph::new(format!("╭{middle}╮")).style(style),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(format!("╰{middle}╯")).style(style),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    );
    for row in area.y + 1..area.bottom() - 1 {
        for column in [area.x, area.right() - 1] {
            frame.render_widget(
                Paragraph::new("│").style(style),
                Rect::new(column, row, 1, 1),
            );
        }
    }
}
/// A square button with a drawn icon.
#[allow(clippy::too_many_arguments)]
fn square(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    area: Rect,
    (art, icon): (&str, &[&str]),
    color: Color,
    action: Click,
    enabled: bool,
) {
    let color = if enabled { color } else { Color::DarkGray };
    if outline_controls() && area.height >= 3 {
        // The phone look: a round outline with the icon inside. Only characters the phone's
        // own font has, and none that Android would swap for a color picture.
        let symbol = match art {
            "plus" => "✚",
            "close" => "✕",
            "refresh" => "↻",
            "mic" => "●",
            "stop" => "■",
            "send" => "➤",
            _ => icon[0],
        };
        // The record button is the one that matters: its outline is in its own color.
        let edge = if matches!(art, "mic" | "stop") {
            color
        } else {
            Color::DarkGray
        };
        outline_box(frame, area, edge);
        frame.render_widget(
            Paragraph::new(symbol)
                .alignment(Alignment::Center)
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Rect::new(area.x + 1, area.y + 1, area.width - 2, 1),
        );
        if enabled {
            hits.push(Hit { area, action });
        }
        return;
    }
    // Real pixel art when the terminal can draw images; characters otherwise.
    let (accent, record) = theme_rgb();
    let rgb = match color {
        THEME_ACCENT => accent,
        THEME_RECORD => record,
        _ => [110, 118, 140],
    };
    if area.height >= 3
        && let Some(dark) = round_theme()
    {
        let key = format!("button-{art}-{rgb:?}");
        if pixel_cell(frame, area, &key, |w, h| {
            crate::art::button(w, h, art, rgb, dark)
        }) {
            if enabled {
                hits.push(Hit { area, action });
            }
            return;
        }
    }
    let key = format!("{art}-{rgb:?}");
    // As much air between box and icon as the row height allows.
    let size = GRAPHICS.with(|g| {
        g.borrow().as_ref().map_or(16, |g| {
            u32::from(g.picker.font_size().height).clamp(16, 20) / 2 * 2
        })
    });
    if pixel_art(frame, area, &key, true, || crate::art::tile(art, rgb, size)) {
        if enabled {
            hits.push(Hit { area, action });
        }
        return;
    }
    if area.height < 3 {
        frame.render_widget(
            Paragraph::new(format!("[{}]", icon[0]))
                .alignment(Alignment::Center)
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            area,
        );
        if enabled {
            hits.push(Hit { area, action });
        }
        return;
    }
    // The largest clean single characters: the box-drawing cross and diagonals span the
    // whole cell.
    let symbol = if nerd_font() {
        // Real icons from the Nerd Font the terminal is using.
        match art {
            "plus" => "\u{f067}",
            "close" => "\u{f00d}",
            "refresh" => "\u{f021}",
            "mic" => "\u{f130}",
            "stop" => "\u{f04d}",
            "send" => "\u{f1d8}",
            _ => icon[0],
        }
    } else {
        match art {
            "plus" => "╋",
            "close" => "╳",
            "refresh" => "↻",
            "mic" => "⬤",
            "stop" => "■",
            "send" => "➤",
            _ => icon[0],
        }
    };
    text_plate(frame, area, symbol, color);
    if enabled {
        hits.push(Hit { area, action });
    }
}
/// A filled shape out of characters, three rows tall with `text` on its middle row: a
/// one-line pill with a Nerd Font, otherwise half blocks above and below (half a row of
/// padding either side), the same shape as the typing bar.
fn text_plate(frame: &mut Frame, area: Rect, text: &str, color: Color) {
    let fill = surface();
    let inner = area.width.saturating_sub(2) as usize;
    let slim = slim_round();
    if !slim {
        frame.render_widget(
            Paragraph::new(format!("▗{}▖", "▄".repeat(inner))).style(Style::default().fg(fill)),
            Rect::new(area.x, area.y, area.width, 1),
        );
    }
    frame.render_widget(
        Paragraph::new(text).alignment(Alignment::Center).style(
            Style::default()
                .fg(color)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
    if slim {
        pill_caps(frame, area.x, area.right() - 1, area.y + 1, fill);
    } else {
        frame.render_widget(
            Paragraph::new(format!("▝{}▘", "▀".repeat(inner))).style(Style::default().fg(fill)),
            Rect::new(area.x, area.y + 2, area.width, 1),
        );
    }
}
/// A gray that reads on the surface color: `strength` of the way from it to white (to black
/// on a light theme).
fn neutral(strength: f32) -> [u8; 3] {
    let surface = surface_rgb();
    let light = surface.iter().map(|&c| c as u32).sum::<u32>() > 3 * 128;
    let end = if light { 0.0 } else { 255.0 };
    surface.map(|c| (c as f32 + (end - c as f32) * strength) as u8)
}
/// The engine chip's width in columns, and how far its plate sits in from the sides of its
/// picture: as far as a round button's does, so the gaps between all three are the same.
fn chip_columns() -> (u16, u32) {
    let side = button_columns() as u32;
    GRAPHICS
        .with(|g| {
            g.borrow().as_ref().map(|g| {
                let font = g.picker.font_size();
                let (cell, height) = (font.width.max(1) as u32, font.height as u32 * 3);
                let inset = (side * cell).saturating_sub(height * 2 / 3) / 2;
                let columns = (crate::art::chip_width(height) + 2 * inset).div_ceil(cell);
                (columns as u16, inset)
            })
        })
        .unwrap_or((if slim_round() { 7 } else { 5 }, 0))
}
/// The engine chip: "GPU" in the accent color or "CPU" in gray (dimmer when there is no GPU
/// to switch to), as tall as the round buttons. A click switches engines.
fn engine_chip(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    area: Rect,
    (gpu, available): (bool, bool),
    enabled: bool,
) {
    let word = if gpu { "GPU" } else { "CPU" };
    if enabled {
        hits.push(Hit {
            area,
            action: Click::Compute,
        });
    }
    if let Some(dark) = round_theme() {
        let rgb = if gpu {
            theme_rgb().0
        } else {
            neutral(if available { 0.6 } else { 0.3 })
        };
        let inset = chip_columns().1;
        if pixel_cell(frame, area, &format!("chip-{word}-{rgb:?}"), |w, h| {
            crate::art::chip(w, h, inset, word, rgb, dark)
        }) {
            return;
        }
    }
    let color = if gpu {
        THEME_ACCENT
    } else if available {
        Color::Reset
    } else {
        Color::DarkGray
    };
    if outline_controls() {
        outline_box(frame, area, Color::DarkGray);
        frame.render_widget(
            Paragraph::new(word)
                .alignment(Alignment::Center)
                .style(Style::default().fg(color)),
            Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), 1),
        );
        return;
    }
    text_plate(frame, area, word, color);
}
fn draw_plain(frame: &mut Frame, state: &Screen) -> Vec<Hit> {
    let mut hits = Vec::new();
    let screen = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(Color::Reset).bg(Color::Reset)),
        screen,
    );
    if screen.width < 40 || screen.height < 16 {
        let how = if outline() {
            // On a phone the keyboard takes most of a sideways screen.
            "3Yiddish\nNot enough room: hide the keyboard (Back)."
        } else {
            "3Yiddish\nMake the window bigger."
        };
        frame.render_widget(
            Paragraph::new(how).wrap(ratatui::widgets::Wrap { trim: true }),
            screen,
        );
        return hits;
    }
    let margin = if screen.width >= 80 { 2 } else { 1 };
    let area = Rect::new(
        screen.x + margin,
        screen.y + 1,
        screen.width - margin * 2,
        // One spare row at the bottom: Sixel terminals round images up to whole pixel bands,
        // and a picture spilling past the last row scrolls the screen.
        screen.height - 3,
    );
    // The top bar is three rows (its content sits on the middle one), like the typing bar.
    let head = area.y + 1;
    let active = recording(state);
    let side_w = button_columns();
    // The typing bar: its text row, with half a row of padding above and below.
    let bar_y = area.bottom() - 2;
    // The rain stays between the top bar and the typing bar.
    draw_rain(
        frame,
        Rect::new(
            screen.x,
            area.y + 5,
            screen.width,
            // A clear row above the bar's line: some terminals let a letter's glyph spill
            // into the row below it and nick the line.
            bar_y.saturating_sub(area.y + 8),
        ),
    );
    // One slim row: logo on the left; updates, refresh and quit on the right.
    let (accent, record) = theme_rgb();
    let big = art_columns(86, 14, 2).filter(|w| area.width >= w + 2 * side_w + 6);
    if let Some(w) = big {
        // A bit smaller than the buttons, centered on the bar's three rows.
        pixel_cell(
            frame,
            Rect::new(area.x, area.y, w, 3),
            "logo-big",
            |cw, ch| {
                let art = crate::art::logo(accent, record);
                let scale = ((ch * 4 / 9) / art.height()).max(1);
                let scaled = image::imageops::resize(
                    &art,
                    art.width() * scale,
                    art.height() * scale,
                    image::imageops::FilterType::Nearest,
                );
                let mut canvas = image::RgbaImage::new(cw, ch);
                image::imageops::overlay(
                    &mut canvas,
                    &scaled,
                    0,
                    (ch.saturating_sub(scaled.height()) / 2) as i64,
                );
                canvas
            },
        );
    }
    match art_columns(86, 14, 1).filter(|w| big.is_none() && area.width >= w + 16) {
        _ if big.is_some() => {}
        Some(w) => {
            pixel_art(frame, Rect::new(area.x, head, w, 1), "logo", false, || {
                crate::art::logo(accent, record)
            });
        }
        None => frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("3", Style::default().fg(THEME_RECORD)),
                Span::styled("YIDDISH", Style::default().fg(THEME_ACCENT)),
            ]))
            .style(Style::default().add_modifier(Modifier::BOLD)),
            Rect::new(area.x, head, 10, 1),
        ),
    }
    // Where the logo ends, so alerts never write over it.
    // (The big logo is drawn at two thirds of the room kept for it.)
    let logo_end = area.x
        + big
            .map(|w| w * 2 / 3 + 1)
            .or_else(|| art_columns(86, 14, 1))
            .unwrap_or(8)
        + 2;
    let mut tools: Vec<Control> = Vec::new();
    if state.app_update.is_some() {
        tools.push(("Update app", Click::UpdateApp, !state.upgrading && !active));
    }
    if let Some(label) = &state.model_update {
        tools.push((
            if label == "first" {
                "Get model"
            } else {
                "New model"
            },
            Click::UpdateModel,
            !state.upgrading && !active,
        ));
    }
    if state.font_offer {
        // Before the model in the list: the model's chip keeps its place nearest the buttons.
        let at = tools
            .len()
            .saturating_sub(usize::from(state.model_update.is_some()));
        tools.insert(
            at,
            ("Get font", Click::GetFont, !state.upgrading && !active),
        );
    }
    let mut x = area.right();
    for (icon, color, action, enabled) in [
        (("close", "✕"), THEME_RECORD, Click::Key(KeyCode::Esc), true),
        (
            ("refresh", "↻"),
            THEME_ACCENT,
            Click::Key(KeyCode::Char('r')),
            !state.busy && !active,
        ),
    ] {
        x -= side_w;
        square(
            frame,
            &mut hits,
            Rect::new(x, area.y, side_w, 3),
            (icon.0, &[icon.1]),
            color,
            action,
            enabled,
        );
        x -= 1;
    }
    // The engine chip is the first to go when the bar is crowded: the update chips, the
    // status (room is kept for it, so the chip does not come and go with every recording)
    // and an alert all come before it.
    if let Some(engine) = compute(state) {
        let w = chip_columns().0;
        let others = tools
            .iter()
            .map(|(label, ..)| label.chars().count() as u16 + 3)
            .sum::<u16>()
            + 16
            // On the phone look an alert has its own row under the bar.
            + if state.notice.is_empty() || outline() { 0 } else { 9 };
        if x >= logo_end + w + 1 + others {
            x -= w;
            engine_chip(
                frame,
                &mut hits,
                Rect::new(x, area.y, w, 3),
                engine,
                engine.1 && state.compute_switch.is_none() && !state.busy && !active,
            );
            x -= 1;
        }
    }
    for (label, action, enabled) in tools.iter().rev() {
        let text = format!(" {label} ");
        let w = text.chars().count() as u16;
        // On a narrow screen the chips furthest from the buttons give way to the logo.
        if x < logo_end + w {
            break;
        }
        x -= w;
        let chip = Rect::new(x, head, w, 1);
        if outline_controls() {
            // The phone look: the same round outline as the buttons, in the accent color.
            let color = if *enabled {
                THEME_ACCENT
            } else {
                Color::DarkGray
            };
            let chip = Rect::new(x, area.y, w, 3);
            outline_box(frame, chip, color);
            frame.render_widget(
                Paragraph::new(*label).style(Style::default().fg(color)),
                Rect::new(x + 1, head, w - 2, 1),
            );
            if *enabled {
                hits.push(Hit {
                    area: chip,
                    action: *action,
                });
            }
            x = x.saturating_sub(1);
            continue;
        }
        frame.render_widget(
            Paragraph::new(text).style(
                Style::default()
                    .fg(if *enabled {
                        Color::Black
                    } else {
                        Color::DarkGray
                    })
                    .bg(THEME_ACCENT),
            ),
            chip,
        );
        if slim_round() {
            // A pill like the buttons: half-circle ends in place of the two side spaces.
            pill_caps(frame, x, x + w - 1, head, THEME_ACCENT);
        }
        if *enabled {
            hits.push(Hit {
                area: chip,
                action: *action,
            });
        }
        x = x.saturating_sub(1);
    }
    let status = if active {
        "● Recording"
    } else if state.busy {
        "◌ Transcribing"
    } else {
        ""
    };
    // The lines around the two bars. The phone look has none: eighth-of-a-cell lines come out
    // dashed there, and the shapes frame the bars by themselves.
    let lines = !outline();
    let mut status_w = status.chars().count() as u16;
    // On a narrow screen the status would run into the logo; the typing bar and the
    // thinking bubble say the same there.
    if x.saturating_sub(status_w + 1) + 2 < logo_end {
        status_w = 0;
    } else {
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(if active {
                THEME_RECORD
            } else {
                Color::DarkGray
            })),
            Rect::new(x.saturating_sub(status_w + 1), head, status_w, 1),
        );
    }
    // Alerts live in the top bar, between the logo and the buttons. One that does not fit
    // there whole (a narrow window, a phone) gets the row under the bar to itself.
    if !state.notice.is_empty() {
        let left = logo_end;
        let right = x.saturating_sub(status_w + 2);
        let notice = clean(&state.notice).replace('\n', " ");
        let length = notice.chars().count() as u16;
        if right > left && length <= right - left {
            // Centered on the window itself; nudged only if it would touch the logo or buttons.
            let start = (area.x + area.width.saturating_sub(length) / 2)
                .min(right - length)
                .max(left);
            frame.render_widget(
                Paragraph::new(notice).style(Style::default().fg(THEME_ACCENT)),
                Rect::new(start, head, length, 1),
            );
        } else {
            let mut text: String = notice.chars().take(area.width as usize).collect();
            if length > area.width {
                text.pop();
                text.push('…');
            }
            let width = text.chars().count() as u16;
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(THEME_ACCENT)),
                Rect::new(
                    area.x + (area.width - width) / 2,
                    area.y + if lines { 4 } else { 3 },
                    width,
                    1,
                ),
            );
        }
    }
    if lines {
        frame.render_widget(
            Paragraph::new("▔".repeat(area.width as usize))
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x, area.y + 3, area.width, 1),
        );
        // And one above the bar; both hug it (drawn on the near edge of their rows).
        frame.render_widget(
            Paragraph::new("▁".repeat(area.width as usize))
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x, area.y - 1, area.width, 1),
        );
    }
    // Bottom: one slim row, edge to edge: plus, the typing line, mic/send.
    let typed = !state.input.trim().is_empty();
    square(
        frame,
        &mut hits,
        Rect::new(area.x, bar_y - 1, side_w, 3),
        ("plus", &["+"]),
        THEME_ACCENT,
        Click::Attach,
        !active && !state.busy,
    );
    let (icon, color, action, enabled): ((&str, &[&str]), Color, Click, bool) = if active {
        (("stop", &["■"]), THEME_RECORD, Click::Record, true)
    } else if typed {
        (
            ("send", &["➤"]),
            THEME_ACCENT,
            Click::Key(KeyCode::Enter),
            !state.busy,
        )
    } else {
        (("mic", &["●"]), THEME_RECORD, Click::Record, !state.busy)
    };
    square(
        frame,
        &mut hits,
        Rect::new(area.right() - side_w, bar_y - 1, side_w, 3),
        icon,
        color,
        action,
        enabled,
    );
    let box_area = Rect::new(
        area.x + side_w + 1,
        bar_y,
        area.width.saturating_sub(2 * side_w + 2),
        1,
    );
    let inner = box_area.width.saturating_sub(2) as usize;
    // The longest of these that fits the bar.
    let fitting = |choices: &[&'static str]| -> &'static str {
        choices
            .iter()
            .find(|text| text.chars().count() <= inner)
            .or(choices.last())
            .copied()
            .unwrap_or_default()
    };
    let line = if active && !crate::audio::LIVE_LEVEL {
        // A phone's recorder gives no live level, so there is no wave to draw: the timer
        // and what to do next.
        let timer = format!(
            " {} ",
            clock(
                state
                    .rec_started
                    .map_or(0.0, |at| at.elapsed().as_secs_f32())
            )
        );
        Line::from(vec![
            Span::styled(timer, Style::default().fg(THEME_RECORD)),
            Span::raw(if nerd_font() {
                fitting(&[
                    " Recording. Tap \u{f04d} to send it.",
                    " Tap \u{f04d} to send",
                    "",
                ])
            } else {
                fitting(&[" Recording. Tap ■ to send it.", " Tap ■ to send", ""])
            }),
        ])
    } else if active {
        // Recording: the bar is the live voice, newest on the right, with a running timer.
        const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let timer = format!(
            " {} ",
            clock(
                state
                    .rec_started
                    .map_or(0.0, |at| at.elapsed().as_secs_f32())
            )
        );
        let room = inner.saturating_sub(timer.chars().count() + 1);
        let recent = &state.levels[state.levels.len().saturating_sub(room)..];
        let wave: String = std::iter::repeat_n('▁', room - recent.len())
            .chain(recent.iter().map(|l| LEVELS[bar(*l) as usize]))
            .collect();
        Line::from(vec![
            Span::styled(timer, Style::default().fg(THEME_RECORD)),
            Span::styled(wave, Style::default().fg(THEME_RECORD)),
        ])
    } else if typed {
        let chars: Vec<char> = state.input.chars().collect();
        Line::from(format!(
            " {}▏",
            chars[chars.len().saturating_sub(inner.saturating_sub(1))..]
                .iter()
                .collect::<String>()
        ))
    } else {
        let hint = if outline() {
            // The same symbol as on the record button (an icon with a Nerd Font).
            if nerd_font() {
                fitting(&[
                    " Tap \u{f130} to talk, or + for a recording",
                    " Tap \u{f130} to talk",
                    "",
                ])
            } else {
                fitting(&[" Tap ● to talk, or + for a recording", " Tap ● to talk", ""])
            }
        } else {
            fitting(&[
                " Hold Space to talk, or drop / Ctrl+V a recording",
                " Hold Space to talk, or drop a recording",
                " Hold Space to talk",
                "",
            ])
        };
        Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)))
    };
    // The typing bar is a fat rounded bubble: the text row plus half a row above and below,
    // with corners drawn as real pixels when the terminal can.
    let fill = surface();
    if outline_controls() {
        // The phone look: a round outline, red while recording.
        outline_box(
            frame,
            Rect::new(box_area.x, bar_y - 1, box_area.width, 3),
            if active {
                THEME_RECORD
            } else {
                Color::DarkGray
            },
        );
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(box_area.x + 1, bar_y, box_area.width.saturating_sub(2), 1),
        );
    } else {
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(fill)),
            box_area,
        );
    }
    if outline_controls() {
        // Drawn above.
    } else if slim_round() && box_area.width > 4 {
        pill_caps(frame, box_area.x, box_area.right() - 1, bar_y, fill);
    } else if box_area.width > 4 {
        let edge = (box_area.width - 2) as usize;
        for (row, half, top) in [(bar_y - 1, "▄", true), (bar_y + 1, "▀", false)] {
            frame.render_widget(
                Paragraph::new(half.repeat(edge)).style(Style::default().fg(fill)),
                Rect::new(box_area.x + 1, row, box_area.width - 2, 1),
            );
            let _ = top;
        }
        bar_ends(
            frame,
            box_area.x,
            box_area.right() - 1,
            bar_y,
            surface_rgb(),
        );
    }
    if lines {
        frame.render_widget(
            Paragraph::new("▁".repeat(area.width as usize))
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x, bar_y - 2, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new("▔".repeat(area.width as usize))
                .style(Style::default().fg(Color::DarkGray)),
            Rect::new(area.x, bar_y + 2, area.width, 1),
        );
    }
    // Messages, newest at the bottom, sized to their text. What you sent is on the right in
    // the accent color; transcripts come in on the left. Clicking a transcript copies it.
    let top = area.y + 5;
    let mut y = bar_y.saturating_sub(4);
    let top = top + 1;
    let text_width = (area.width as usize * 3 / 4).clamp(20, 76);
    // A bubble. With image support its ends are truly round (drawn pixel by pixel) and it is
    // exactly as tall as its text; otherwise half-blocks soften the edges.
    let round = round_theme();
    let (accent, _) = theme_rgb();
    let bubble = |frame: &mut Frame,
                  y: u16,
                  lines: Vec<Line<'static>>,
                  inner: u16,
                  (fg, bg): (Color, Color),
                  right: bool| {
        let h = lines.len() as u16;
        let w = (inner + 4).min(area.width);
        let x = if right { area.right() - w } else { area.x };
        // Wipe whatever the background drew here first, so no letters show through.
        let wipe = if round.is_some() {
            Rect::new(x, y, w, h)
        } else {
            Rect::new(x, y - 1, w, h + 2)
        };
        frame.render_widget(ratatui::widgets::Clear, wipe);
        if let Some(dark) = round {
            let rgb = if bg == THEME_ACCENT { accent } else { dark };
            let fill = Color::Rgb(rgb[0], rgb[1], rgb[2]);
            frame.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(fg).bg(fill))
                    .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1))),
                Rect::new(x + 1, y, w - 2, h),
            );
            // Each end is one picture the height of the bubble, round at top and bottom.
            for left in [true, false] {
                let column = if left { x } else { x + w - 1 };
                let drawn = pixel_cell(
                    frame,
                    Rect::new(column, y, 1, h),
                    &format!("bubble-end-{left}-{rgb:?}"),
                    |cw, ch| crate::art::cap(cw, ch, rgb, left, true, true),
                );
                if !drawn {
                    for row in 0..h {
                        let kind = Corner::Side {
                            top: row == 0,
                            bottom: row + 1 == h,
                        };
                        corner(frame, Rect::new(column, y + row, 1, 1), rgb, left, kind);
                    }
                }
            }
            return Rect::new(x, y, w, h);
        }
        if outline() {
            // The phone look: a round outline, in the accent color for what you sent (and
            // for a reply that was just copied).
            let edge = if bg == THEME_ACCENT || fg == THEME_ACCENT {
                THEME_ACCENT
            } else {
                Color::DarkGray
            };
            outline_box(frame, Rect::new(x, y - 1, w, h + 2), edge);
            frame.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(Color::Reset))
                    .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1))),
                Rect::new(x + 1, y, w - 2, h),
            );
            return Rect::new(x, y - 1, w, h + 2);
        }
        if h == 1 && nerd_font() {
            // One line: a true pill.
            frame.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(fg).bg(bg))
                    .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1))),
                Rect::new(x + 1, y, w - 2, 1),
            );
            pill_caps(frame, x, x + w - 1, y, bg);
            return Rect::new(x, y - 1, w, 3);
        }
        let cap = Style::default().fg(bg);
        for row in y..y + h {
            frame.render_widget(Paragraph::new("▐").style(cap), Rect::new(x, row, 1, 1));
            frame.render_widget(
                Paragraph::new("▌").style(cap),
                Rect::new(x + w - 1, row, 1, 1),
            );
        }
        // Half a row of padding above and below.
        let edge = (w - 2) as usize;
        frame.render_widget(
            Paragraph::new("▄".repeat(edge)).style(cap),
            Rect::new(x + 1, y - 1, w - 2, 1),
        );
        frame.render_widget(
            Paragraph::new("▀".repeat(edge)).style(cap),
            Rect::new(x + 1, y + h, w - 2, 1),
        );
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Right)
                .style(Style::default().fg(fg).bg(bg))
                .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1))),
            Rect::new(x + 1, y, w - 2, h),
        );
        Rect::new(x, y - 1, w, h + 2)
    };
    // Round bubbles sit one row apart; the half-block kind needs its padding rows too.
    let gap = if round.is_some() { 1 } else { 2 };
    if round.is_some() {
        y += 1;
    }
    if state.busy {
        // Thinking: a little sound wave, rolling, with a bright crest sweeping through it.
        const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let t = started().elapsed().as_secs_f32();
        let (accent, _) = theme_rgb();
        let bars: Vec<Span> = (0..14)
            .map(|i| {
                let x = i as f32;
                let wave = (t * 5.0 + x * 0.9).sin() * 0.5 + (t * 3.1 - x * 0.5).sin() * 0.5;
                let level = (((wave + 1.0) / 2.0) * 7.0).round() as usize;
                let crest = ((t * 9.0) % 20.0 - x).abs();
                let glow = (1.0 - crest / 4.0).clamp(0.35, 1.0);
                let c = |v: u8| (v as f32 * glow) as u8;
                Span::styled(
                    if outline() {
                        // The phone look: dots like the voice note's wave (blocks leave gaps there).
                        ['⠂', '⠆', '⠇', '⡇'][level.min(7) / 2].to_string()
                    } else {
                        LEVELS[level.min(7)].to_string()
                    },
                    Style::default().fg(Color::Rgb(c(accent[0]), c(accent[1]), c(accent[2]))),
                )
            })
            .collect();
        y = y.saturating_sub(1);
        bubble(
            frame,
            y,
            vec![Line::from(bars)],
            14,
            (Color::Reset, surface()),
            false,
        );
        y = y.saturating_sub(gap);
    }
    for (index, (mine, text)) in state.plain_log.iter().enumerate().rev() {
        let body = visual(&clean(text), text_width);
        let mut lines: Vec<Line> = body.lines().map(|l| Line::from(l.to_owned())).collect();
        let mut inner = body.lines().map(|l| l.chars().count()).max().unwrap_or(1) as u16;
        let voice = state.voices.get(&index);
        if let (Some(dark), Some(voice)) = (round, voice) {
            // Drawn with pixels: a fat, short bubble (see `draw_voice`).
            if y < top + 3 {
                break;
            }
            y -= 3;
            let w = VOICE_WIDTH.min(area.width);
            draw_voice(
                frame,
                &mut hits,
                state,
                (index, voice),
                Rect::new(area.right() - w, y, w, 3),
                (accent, dark),
            );
            y = y.saturating_sub(gap);
            continue;
        }
        // Without image support: play/pause, the wave in characters, and the time.
        if let Some(voice) = voice {
            let play = state.voice_play.as_ref().filter(|p| p.index == index);
            let position = play.map_or(0.0, |p| p.position());
            let done = if voice.seconds > 0.0 {
                ((position / voice.seconds) * PEAKS as f32) as usize
            } else {
                0
            };
            let rest = Style::default().fg(match round {
                Some(_) => Color::Rgb(
                    (accent[0] as f32 * 0.55) as u8,
                    (accent[1] as f32 * 0.55) as u8,
                    (accent[2] as f32 * 0.55) as u8,
                ),
                None => Color::DarkGray,
            });
            let mut spans = vec![Span::raw(if play.is_some_and(|p| !p.paused) {
                "▮▮ "
            } else if outline() {
                // Not the usual play triangle: phones may draw that one as a color picture.
                "►  "
            } else {
                "▶  "
            })];
            for i in 0..PEAKS {
                let glyph = match voice.peaks.get(i * voice.peaks.len() / PEAKS) {
                    // A thin dotted bar per column (braille), centered, with a gap beside it.
                    Some(level) => ['⠂', '⠆', '⠇', '⡇'][(*level / 64) as usize],
                    None if i < done => '━',
                    None => '─',
                };
                spans.push(if i < done {
                    Span::raw(glyph.to_string())
                } else {
                    Span::styled(glyph.to_string(), rest)
                });
            }
            let shown = if play.is_some() {
                position
            } else {
                voice.seconds
            };
            spans.push(Span::raw(format!(" {}", clock(shown))));
            if text.is_empty() {
                lines.clear();
            }
            lines.push(Line::from(spans));
            inner = inner.max(PEAKS as u16 + 8);
        }
        let h = lines.len().max(1) as u16;
        if y < top + h {
            break;
        }
        y -= h;
        let copied = state
            .copied
            .is_some_and(|(i, at)| i == index && at.elapsed().as_millis() < 1200);
        let colors = if *mine {
            (Color::Black, THEME_ACCENT)
        } else if copied {
            (THEME_ACCENT, surface())
        } else {
            (Color::Reset, surface())
        };
        let rect = bubble(frame, y, lines, inner, colors, *mine);
        if voice.is_some() {
            // The player is the bubble's last row: text starts two columns in.
            let row = rect.bottom() - if round.is_some() { 1 } else { 2 };
            let start = rect.right().saturating_sub(2 + PEAKS as u16 + 8);
            hits.push(Hit {
                area: Rect::new(start, row, 3, 1),
                action: Click::VoiceToggle(index),
            });
            for i in 0..PEAKS as u16 {
                hits.push(Hit {
                    area: Rect::new(start + 3 + i, row, 1, 1),
                    action: Click::VoiceSeek(index, i * 1000 / PEAKS as u16),
                });
            }
        }
        if !*mine {
            hits.push(Hit {
                area: rect,
                action: Click::CopyAt(index),
            });
        }
        y = y.saturating_sub(gap);
    }
    hits
}
/// Columns of a voice note: end, button, wave, time, end.
const VOICE_WAVE: u16 = 16;
const VOICE_WIDTH: u16 = 1 + 4 + VOICE_WAVE + 6 + 1;
/// A voice note as a fat bubble three rows tall (the middle row plus half a row either
/// side): a play/pause button, the wave as a slider with the played part dark, the time.
fn draw_voice(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    state: &Screen,
    (index, voice): (usize, &Voice),
    rect: Rect,
    (accent, dark): ([u8; 3], [u8; 3]),
) {
    let fill = Color::Rgb(accent[0], accent[1], accent[2]);
    let ink = Color::Rgb(dark[0], dark[1], dark[2]);
    let (x, y, w) = (rect.x, rect.y + 1, rect.width);
    frame.render_widget(ratatui::widgets::Clear, rect);
    frame.render_widget(
        Block::default().style(Style::default().bg(fill)),
        Rect::new(x, y, w, 1),
    );
    for (row, half, top) in [(y - 1, "▄", true), (y + 1, "▀", false)] {
        frame.render_widget(
            Paragraph::new(half.repeat(w.saturating_sub(2) as usize))
                .style(Style::default().fg(fill)),
            Rect::new(x + 1, row, w.saturating_sub(2), 1),
        );
        let _ = top;
    }
    bar_ends(frame, x, x + w - 1, y, accent);
    let play = state.voice_play.as_ref().filter(|p| p.index == index);
    let done = match play {
        Some(p) if voice.seconds > 0.0 => (p.position() / voice.seconds).min(1.0),
        _ => 0.0,
    };
    let name = if play.is_some_and(|p| !p.paused) {
        "pause"
    } else {
        "play"
    };
    let button = Rect::new(x + 1, rect.y, 4, 3);
    pixel_cell(
        frame,
        button,
        &format!("voice-{name}-{accent:?}"),
        |cw, ch| crate::art::icon_on(cw, ch, name, dark, accent),
    );
    let faded = [0, 1, 2].map(|c| (accent[c] as f32 * 0.6 + dark[c] as f32 * 0.4) as u8);
    let wave = Rect::new(x + 5, rect.y, VOICE_WAVE.min(w.saturating_sub(12)), 3);
    pixel_cell(
        frame,
        wave,
        &format!(
            "wave-{index}-{}-{}",
            (done * 96.0) as u32,
            voice.peaks.len()
        ),
        |cw, ch| crate::art::wave(cw, ch, &voice.peaks, done, (dark, faded, accent)),
    );
    let shown = if play.is_some() {
        play.map_or(0.0, |p| p.position())
    } else {
        voice.seconds
    };
    frame.render_widget(
        Paragraph::new(format!(" {}", clock(shown))).style(Style::default().fg(ink).bg(fill)),
        Rect::new(wave.right(), y, 6.min(x + w - wave.right()), 1),
    );
    hits.push(Hit {
        area: button,
        action: Click::VoiceToggle(index),
    });
    for i in 0..wave.width {
        hits.push(Hit {
            area: Rect::new(wave.x + i, rect.y, 1, 3),
            action: Click::VoiceSeek(index, i * 1000 / wave.width.max(1)),
        });
    }
}
/// Demo mode: answer with a canned line after a short think.
fn demo_reply(state: &mut Screen, tx: mpsc::UnboundedSender<Update>) {
    const LINES: [&str; 4] = [
        "א גוטן טאג, וואס מאכט איר?",
        "דאס איז א פרובע פון דעם עקראן, נישט קיין אמת'ע איבערשרייבונג.",
        "מען קען קליקן אויף דעם ענטפער כדי עס צו קאפירן.",
        "ווען מען רעדט אריין אין מיקראפאן, שרייבט עס זיך אויס דא אויף דער רעכטער זייט, שורה נאך שורה.",
    ];
    state.busy = true;
    state.notice.clear();
    let line = LINES[state.plain_log.len() / 2 % LINES.len()];
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2200));
        let _ = tx.send(Update::Done(1, Ok(json!({ "text": line }))));
    });
}
/// Pasted or dropped text: a path to an audio file is attached and sent; anything else is
/// typed into the box.
fn attach_text(state: &mut Screen, text: &str) {
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let mut path = first
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .to_owned();
    if let Some(rest) = path.strip_prefix("file://") {
        path = percent_decode(rest);
    }
    if plain(state) && Path::new(&path).is_file() {
        state.input = path;
        state.pending_key = Some(KeyCode::Enter);
    } else {
        state.input.push_str(&clean(text).replace('\n', " "));
    }
}
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = text
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
/// Ctrl+V: a copied file (from a file manager) or copied text.
fn paste_clipboard(state: &mut Screen) {
    let run = |args: &[&str]| {
        std::process::Command::new("wl-paste")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let text = if std::env::var_os("WAYLAND_DISPLAY").is_some() && run(&["--version"]).is_some() {
        if run(&["--list-types"]).is_some_and(|t| t.contains("text/uri-list")) {
            run(&["-n", "-t", "text/uri-list"])
        } else {
            run(&["-n"])
        }
    } else {
        clipboard_get()
    };
    match text {
        Some(text) if !text.trim().is_empty() => attach_text(state, &text),
        _ => state.notice = "Nothing to paste.".into(),
    }
}
#[cfg(not(target_os = "android"))]
fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok().and_then(|mut c| {
        c.get()
            .file_list()
            .ok()
            .and_then(|f| f.first().map(|p| p.display().to_string()))
            .or_else(|| c.get_text().ok())
    })
}
/// Android: the phone's clipboard through Termux:API.
#[cfg(target_os = "android")]
fn clipboard_get() -> Option<String> {
    crate::audio::termux("termux-clipboard-get", &[], None, 3).ok()
}
#[cfg(not(target_os = "android"))]
fn copy_text(state: &mut Screen, text: &str, _tx: &mpsc::UnboundedSender<Update>) {
    if state.clipboard.is_none() {
        state.clipboard = arboard::Clipboard::new().ok();
    }
    state.notice = match state
        .clipboard
        .as_mut()
        .map(|c| c.set_text(text.to_owned()))
    {
        Some(Ok(())) => "Copied.".into(),
        _ => "Cannot reach the clipboard.".into(),
    };
}
/// Android: the phone's clipboard through Termux:API, asked in the background so the screen
/// never waits for the phone (it answers in a fraction of a second). The bubble lights up
/// at once; "Copied." is said when the phone has taken it, or what is missing.
#[cfg(target_os = "android")]
fn copy_text(_state: &mut Screen, text: &str, tx: &mpsc::UnboundedSender<Update>) {
    let (text, tx) = (text.to_owned(), tx.clone());
    std::thread::spawn(move || {
        let said = match crate::audio::termux("termux-clipboard-set", &[], Some(&text), 4) {
            Ok(_) => "Copied.".to_owned(),
            Err(error) => error.to_string(),
        };
        let _ = tx.send(Update::Notice(said));
    });
}
/// Open the system file picker in the background; the choice comes back as `Update::Picked`.
fn pick_file(tx: mpsc::UnboundedSender<Update>) {
    std::thread::spawn(move || {
        // Android: the phone's own file picker, through Termux:API. The command returns at
        // once and the chosen file is copied here when the user picks one (never, if they
        // back out), so the file is waited for.
        #[cfg(target_os = "android")]
        {
            let folder = Path::new("state/picked");
            let _ = std::fs::create_dir_all(folder);
            let Ok(file) = std::path::absolute(folder.join(uuid::Uuid::new_v4().to_string()))
            else {
                return;
            };
            let name = file.to_string_lossy().into_owned();
            if let Err(error) = crate::audio::termux("termux-storage-get", &[&name], None, 5) {
                let _ = tx.send(Update::Notice(error.to_string()));
                return;
            }
            let size = || std::fs::metadata(&file).map_or(0, |m| m.len());
            for _ in 0..1500 {
                std::thread::sleep(Duration::from_millis(200));
                let before = size();
                if before > 0 {
                    // Still being copied while it grows.
                    std::thread::sleep(Duration::from_millis(300));
                    if size() == before {
                        let _ = tx.send(Update::Picked(Some(name)));
                        return;
                    }
                }
            }
        }
        // Linux: the desktop's own picker (works on GNOME, KDE, Hyprland, ...).
        #[cfg(target_os = "linux")]
        {
            let picked = rfd::FileDialog::new()
                .set_title("Pick a recording")
                .add_filter(
                    "Audio",
                    &["wav", "mp3", "m4a", "flac", "ogg", "opus", "aac", "mp4"],
                )
                .add_filter("All files", &["*"])
                .pick_file();
            if let Some(path) = picked {
                let _ = tx.send(Update::Picked(Some(path.display().to_string())));
                return;
            }
        }
        #[cfg(not(target_os = "android"))]
        let attempts: Vec<(&str, Vec<&str>)> = if cfg!(target_os = "macos") {
            vec![(
                "osascript",
                vec![
                    "-e",
                    "POSIX path of (choose file with prompt \"Pick a recording\")",
                ],
            )]
        } else if cfg!(windows) {
            vec![(
                "powershell",
                vec![
                    "-NoProfile",
                    "-Command",
                    "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Filter = 'Audio|*.wav;*.mp3;*.m4a;*.flac;*.ogg|All files|*.*'; if ($d.ShowDialog() -eq 'OK') { $d.FileName }",
                ],
            )]
        } else {
            vec![
                (
                    "zenity",
                    vec!["--file-selection", "--title=Pick a recording"],
                ),
                ("kdialog", vec!["--getopenfilename", "."]),
            ]
        };
        #[cfg(not(target_os = "android"))]
        {
            let mut picked = None;
            for (program, args) in attempts {
                if let Ok(out) = std::process::Command::new(program).args(&args).output() {
                    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                    if out.status.success() && !path.is_empty() {
                        picked = Some(path);
                    }
                    break;
                }
            }
            let _ = tx.send(Update::Picked(picked));
        }
    });
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    #[test]
    fn tab_drafts_survive_navigation_and_refresh() {
        let mut state = Screen::default();
        state.input = "chat draft".into();
        select_tab(&mut state, 2);
        state.input = "א גוטן".into();
        select_tab(&mut state, 4);
        select_tab(&mut state, 0);
        assert_eq!(state.input, "chat draft");
        let mut refreshed = Screen::default();
        Resume::capture(&state).apply(&mut refreshed);
        select_tab(&mut refreshed, 2);
        assert_eq!(refreshed.input, "א גוטן");
    }
    #[test]
    fn wide_chat_has_readable_cards_and_keeps_device_notices_out_of_history() {
        let mut state = Screen::default();
        state.review_available = true;
        state.pair = Some(json!({"id":"preview"}));
        state.phone_audio = true;
        state.phone_status = Some(json!({"connected":true,"mic_ready":true}));
        state.history = vec![
            json!({"role":"user","content":"א גוטן!"}),
            json!({"role":"assistant","content":"א גוטן! ווי גייט עס אייך היינט?"}),
            json!({"role":"user","content":"לאמיר אביסל שמועסן אויף אידיש."}),
            json!({"role":"assistant","content":"מיט פארגעניגן. וואס ווילט איר היינט באשמועסן?"}),
        ];
        state.outputs[0] = "Phone mic: stopping and transcribing…".into();
        for (width, height, tab) in [(160, 45, 0), (80, 24, 0), (40, 24, 0), (118, 34, 2)] {
            state.tab = tab;
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut hits = vec![];
            term.draw(|f| {
                hits = draw(f, &state, &[]);
            })
            .unwrap();
            let text = rendered_text(&term);
            if tab == 0 {
                assert!(!text.contains("stopping and transcribing"));
                assert!(text.contains("3Yiddish"));
                assert!(text.contains("You"));
                assert!(!text.contains("Review data"));
            }
            if width == 160 {
                assert!(hits.iter().all(|h| h.area.x >= 21 && h.area.right() <= 139));
            }
            if let Ok(folder) = std::env::var("TUI_LAYOUT_JSON_DIR") {
                std::fs::create_dir_all(&folder).unwrap();
                let buffer = term.backend().buffer();
                let cells=buffer.content.iter().map(|c|json!({"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg),"bold":c.modifier.contains(Modifier::BOLD)})).collect::<Vec<_>>();
                std::fs::write(
                    Path::new(&folder).join(format!("{width}x{height}-tab{tab}.json")),
                    serde_json::to_vec(&json!({"width":width,"height":height,"cells":cells}))
                        .unwrap(),
                )
                .unwrap();
            }
        }
        state.tab = 0;
        state.history=(0..30).map(|i|json!({"role":if i%2==0{"user"}else{"assistant"},"content":format!("Message number {i}")})).collect();
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        assert!(rendered_text(&term).contains("Message number 29"));
        scroll_view(&mut state, true, 20);
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        assert!(!rendered_text(&term).contains("Message number 29"));
        scroll_view(&mut state, false, 20);
        assert_eq!(state.scroll, 0);
    }
    #[test]
    fn speak_accepts_plain_text_and_keeps_explicit_files_safe() {
        let dir =
            std::env::temp_dir().join(format!("3yiddish-speak-test-{}", uuid::Uuid::new_v4()));
        let (text, first) = speak_destination("א גוטן!", &dir).unwrap();
        let (_, second) = speak_destination("א גוטן!", &dir).unwrap();
        assert_eq!(text, "א גוטן!");
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(dir.as_path()));
        std::fs::write(&first, b"existing audio").unwrap();
        assert!(speak_destination(&format!("א גוטן | {}", first.display()), &dir).is_err());
        assert!(speak_destination(" ", &dir).is_err());
        assert_eq!(std::fs::read(&first).unwrap(), b"existing audio");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn speak_plain_text_selects_pc_or_phone_without_filename() {
        use axum::{Json, Router, routing::post};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let phone_calls = Arc::new(AtomicUsize::new(0));
        let observed = phone_calls.clone();
        let app = Router::new()
            .route(
                "/v1/audio/speech",
                post(|Json(body): Json<Value>| async move {
                    assert_eq!(body["input"], "א גוטן!");
                    let mut wav = b"RIFF".to_vec();
                    wav.extend_from_slice(&356u32.to_le_bytes());
                    wav.extend_from_slice(b"WAVEfmt ");
                    wav.extend_from_slice(&16u32.to_le_bytes());
                    wav.extend_from_slice(&1u16.to_le_bytes());
                    wav.extend_from_slice(&1u16.to_le_bytes());
                    wav.extend_from_slice(&16000u32.to_le_bytes());
                    wav.extend_from_slice(&32000u32.to_le_bytes());
                    wav.extend_from_slice(&2u16.to_le_bytes());
                    wav.extend_from_slice(&16u16.to_le_bytes());
                    wav.extend_from_slice(b"data");
                    wav.extend_from_slice(&320u32.to_le_bytes());
                    wav.extend_from_slice(&[0; 320]);
                    ([("content-type", "audio/wav")], wav)
                }),
            )
            .route(
                "/v1/pair/fixture/playback",
                post(move |bytes: axum::body::Bytes| {
                    let count = observed.clone();
                    async move {
                        assert!(bytes.starts_with(b"RIFF"));
                        count.fetch_add(1, Ordering::SeqCst);
                        Json(json!({"queued":true}))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        for phone in [false, true] {
            let result = execute(
                &reqwest::Client::new(),
                &base,
                Action {
                    tab: 2,
                    input: "א גוטן!".into(),
                    history: vec![],
                    aliases: vec![],
                    pair_id: phone.then(|| "fixture".into()),
                    audio_path: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(result["play_on_pc"], !phone);
            let file = Path::new(result["audio_path"].as_str().unwrap());
            assert_eq!(std::fs::read(file).unwrap().len(), 364);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            std::fs::remove_file(file).unwrap();
        }
        assert_eq!(phone_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }
    #[test]
    fn explicit_corrections_are_private_and_survive_tab_changes() {
        let folder =
            std::env::temp_dir().join(format!("3yiddish-correction-test-{}", uuid::Uuid::new_v4()));
        let path = folder.join("corrections.jsonl");
        let mut state = Screen::default();
        state.history = vec![
            json!({"role":"user","content":"hello"}),
            json!({"role":"assistant","content":"bad draft"}),
        ];
        state.input = "pending question".into();
        state.outputs[0] = "3Yiddish> bad draft".into();
        begin_correction(&mut state);
        assert!(!path.exists());
        state.input = "א גוטן!".into();
        select_tab(&mut state, 4);
        select_tab(&mut state, 0);
        assert_eq!(state.input, "א גוטן!");
        let snapshot = Resume::capture(&state);
        let mut restored = Screen::default();
        snapshot.apply(&mut restored);
        assert!(restored.correction.is_some());
        save_correction(&mut restored, &path).unwrap();
        let record: Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(record["corrected_response"], "א גוטן!");
        assert_eq!(record["original_response"], "bad draft");
        assert_eq!(record["review_kind"], "user_corrected");
        assert_eq!(restored.input, "pending question");
        assert_eq!(restored.history.last().unwrap()["content"], "א גוטן!");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn stale_pairing_metadata_is_refetched_without_showing_old_qr() {
        let mut state = Screen::default();
        state.tab = 6;
        state.pair = Some(json!({"id":"old","qr":"STALE-CODE","join_url":"https://old/#stale"}));
        assert!(pair_needs_sync(&state, &json!({"id":"new"})));
        let (tx, mut rx) = mpsc::unbounded_channel();
        sync_pair(&mut state, &tx).unwrap();
        let action = rx.try_recv().unwrap();
        assert_eq!(action.input, "sync");
        assert!(!pair_needs_sync(&state, &json!({"id":"new"})));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        let text = rendered_text(&term);
        assert!(text.contains("Syncing PC"));
        assert!(!text.contains("STALE-CODE"));
    }
    fn rendered_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn responsive_controls_remain_visible_and_hit_targets_never_overlap() {
        LAB.store(true, std::sync::atomic::Ordering::Relaxed); // developer layout
        for width in [32, 40, 60, 80, 120] {
            for tab in 0..TABS.len() {
                let mut state = Screen::default();
                state.lab = true;
                state.tab = tab;
                state.input = "א גוטן".into();
                state.pair = Some(json!({"id":"fixture"}));
                state.phone_audio = true;
                state.audio_path = Some(PathBuf::from("fixture.wav"));
                state.phone_status =
                    Some(json!({"connected":true,"mic_ready":true,"transcript":"א גוטן"}));
                if tab == 5 {
                    state.comparison =
                        Some(json!({"outputs":[{"text":"א גוטן"},{"text":"שלום עליכם"}]}));
                }
                let mut term =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
                let mut hits = vec![];
                term.draw(|f| {
                    hits = draw(f, &state, &[]);
                })
                .unwrap();
                for (_, action, enabled) in controls(&state) {
                    if enabled {
                        assert!(
                            hits.iter().any(|hit| hit.action == action),
                            "{action:?} disappeared at {width} columns / tab {tab}"
                        );
                    }
                }
                for tab in (0..TABS.len()).filter(|t| !hidden(*t)) {
                    assert!(hits.iter().any(|hit| hit.action == Click::Tab(tab)));
                }
                for (index, hit) in hits.iter().enumerate() {
                    assert!(hit.area.right() <= width && hit.area.bottom() <= 24);
                    assert_eq!(hit_at(&hits, hit.area.x, hit.area.y), Some(hit.action));
                    for other in &hits[index + 1..] {
                        assert!(
                            hit.area.intersection(other.area).is_empty(),
                            "overlapping mouse targets"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn paired_view_keeps_action_errors_without_pairing_boilerplate() {
        let mut state = Screen::default();
        state.tab = 6;
        state.pair = Some(json!({"id":"fixture"}));
        state.phone_status = Some(json!({"connected":true,"mic_ready":true}));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        state.outputs[6] = "Request failed: fixture playback failure".into();
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        assert!(rendered_text(&term).contains("fixture playback failure"));
        state.outputs[6] = "Scan the QR with your phone camera.".into();
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        assert!(!rendered_text(&term).contains("Scan the QR"));
    }
    #[test]
    fn compact_layout_snapshots_and_undersized_notice() {
        for width in [40, 80] {
            for tab in [0, 6] {
                let mut state = Screen::default();
                state.tab = tab;
                state.input = "א גוטן".into();
                state.pair = Some(json!({"id":"fixture"}));
                state.phone_audio = true;
                state.phone_status =
                    Some(json!({"connected":true,"mic_ready":true,"transcript":"א גוטן"}));
                state.outputs[0] = "you> א גוטן\n\n3Yiddish> שלום עליכם!".into();
                let mut term =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
                term.draw(|f| {
                    draw(f, &state, &[]);
                })
                .unwrap();
                let text = rendered_text(&term);
                assert!(text.contains("[Refresh]") && text.contains("[Quit]"));
                assert!(!text.contains("click") && !text.contains("F10"));
                if tab == 6 {
                    assert!(text.contains("● Paired"));
                    assert!(!text.contains("┌"));
                }
                if let Ok(folder) = std::env::var("TUI_LAYOUT_DUMP_DIR") {
                    std::fs::create_dir_all(&folder).unwrap();
                    std::fs::write(
                        Path::new(&folder).join(format!("{width}x24-tab-{tab}.txt")),
                        text,
                    )
                    .unwrap();
                }
            }
        }
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 8)).unwrap();
        term.draw(|f| {
            draw(f, &Screen::default(), &[]);
        })
        .unwrap();
        assert!(rendered_text(&term).contains("Resize"));
        let mut cramped = Screen::default();
        cramped.tab = 5;
        cramped.input = "fixture".into();
        cramped.pair = Some(json!({"id":"fixture"}));
        cramped.comparison = Some(json!({"outputs":[]}));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(32, 10)).unwrap();
        let mut hits = vec![];
        term.draw(|f| {
            hits = draw(f, &cramped, &[]);
        })
        .unwrap();
        assert!(rendered_text(&term).contains("Resize"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, Click::Key(KeyCode::Esc));
    }
    #[test]
    fn refresh_preserves_workspace_and_pairing_in_private_handoff() {
        LAB.store(true, std::sync::atomic::Ordering::Relaxed); // developer layout
        let mut state = Screen::default();
        state.tab = 6;
        state.input = "א גוטן".into();
        state.pair = Some(json!({"id":"fixture-pair","join_url":"https://192.168.1.20/#fixture"}));
        state.history = vec![json!({"role":"user","content":"א גוטן"})];
        state.phone_audio = true;
        // The regular app's chat and its voice notes come along too.
        state.plain_log = vec![(true, String::new()), (false, "א גוטן טאג".into())];
        state.voices.insert(
            0,
            Voice {
                path: PathBuf::from("/tmp/take.wav"),
                seconds: 3.0,
                peaks: vec![1, 2, 3],
            },
        );
        let path = Resume::capture(&state).save().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let resume: Resume = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();
        let mut restored = Screen::default();
        resume.apply(&mut restored);
        assert_eq!(restored.tab, 6);
        assert_eq!(restored.input, state.input);
        assert_eq!(restored.history, state.history);
        assert_eq!(restored.pair, state.pair);
        assert!(restored.phone_audio);
        assert_eq!(restored.plain_log, state.plain_log);
        assert_eq!(restored.voices[&0].seconds, 3.0);
        assert_eq!(restored.voices[&0].peaks, vec![1, 2, 3]);
    }
    #[test]
    fn paired_audio_output_is_clickable_and_independent_of_phone_mic() {
        let mut state = Screen::default();
        state.pair = Some(json!({"id":"fixture"}));
        state.phone_status = Some(json!({"connected":true,"mic_ready":true}));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        for phone_audio in [false, true] {
            state.phone_audio = phone_audio;
            let mut hits = vec![];
            state.tab = 1;
            term.draw(|f| {
                hits = draw(f, &state, &[]);
            })
            .unwrap();
            assert!(hits.iter().any(|h| h.action == Click::Record));
            // The sound switch lives on the Phone tab only.
            assert!(!hits.iter().any(|h| h.action == Click::AudioOutput));
            state.tab = 6;
            term.draw(|f| {
                hits = draw(f, &state, &[]);
            })
            .unwrap();
            assert!(hits.iter().any(|h| h.action == Click::AudioOutput));
            let text = term
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains(if phone_audio {
                "Sound: Phone"
            } else {
                "Sound: PC"
            }));
            assert_eq!(state.phone_status.as_ref().unwrap()["mic_ready"], true);
        }
    }
    #[test]
    fn mouse_targets_match_visible_controls_and_disabled_buttons_do_not_fire() {
        LAB.store(true, std::sync::atomic::Ordering::Relaxed); // developer layout
        for (width, height) in [(80, 24), (120, 40)] {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut state = Screen::default();
            state.input = "א גוטן".into();
            let mut hits = vec![];
            term.draw(|f| {
                hits = draw(f, &state, &[]);
            })
            .unwrap();
            for action in [
                Click::Tab(6),
                Click::Record,
                Click::Key(KeyCode::Enter),
                Click::Key(KeyCode::Esc),
            ] {
                let target = hits
                    .iter()
                    .find(|h| h.action == action)
                    .expect("visible control must be clickable");
                assert_eq!(hit_at(&hits, target.area.x, target.area.y), Some(action));
                assert_eq!(
                    hit_at(&hits, target.area.right() - 1, target.area.y),
                    Some(action)
                );
            }
            assert!(!hits.iter().any(|h| h.action == Click::Key(KeyCode::F(9))));
            state.busy = true;
            state.pair = Some(json!({"id":"paired"}));
            term.draw(|f| {
                hits = draw(f, &state, &[]);
            })
            .unwrap();
            assert!(hits.iter().any(|h| h.action == Click::Record));
            assert!(!hits.iter().any(|h| h.action == Click::Key(KeyCode::Enter)));
        }
    }
    #[test]
    fn error_rates_handle_yiddish_and_empty_references() {
        let (w, c) = errors("א גוטן שבת", "א גוטן").unwrap();
        assert!((w - 1.0 / 3.0).abs() < 1e-8);
        assert!(c > 0.);
        assert!(errors("", "א גוטן").is_err());
        assert_eq!(errors("שַׁבָּת!", "שבת").unwrap(), (0., 0.));
    }
    #[test]
    fn training_screen_shows_completed_and_active_jobs() {
        let mut output = String::new();
        job_line(
            &mut output,
            "Speak",
            &json!({"phase":"speech_synthesis_training","step":250,"target_steps":4000}),
        );
        assert!(output.contains("250 / 4000"));
    }
    #[test]
    fn pair_qr_is_never_clipped_in_small_terminals() {
        let qr = qrcode::QrCode::new(b"http://192.168.1.20:8796/#01234567890123456789012345678901")
            .unwrap()
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .build();
        let mut state = Screen::default();
        state.tab = 6;
        state.pair = Some(json!({"id":"test","qr":qr,"join_url":"http://192.168.1.20:8796/#test"}));
        for (width, height) in [(80, 24), (120, 40)] {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            term.draw(|f| {
                draw(f, &state, &[]);
            })
            .unwrap();
            let buffer = term.backend().buffer();
            let text = buffer
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            if height == 24 {
                assert!(text.contains("Enlarge terminal"));
            } else {
                assert!(buffer.content.iter().any(|c| c.fg == Color::Gray));
                assert!(!buffer.content.iter().any(|c| c.bg == Color::White));
                assert!(text.contains("http://192.168.1.20:8796/#test"));
            }
        }
    }
    #[test]
    fn all_tabs_render_in_a_small_terminal() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut term = ratatui::Terminal::new(backend).unwrap();
        for tab in 0..TABS.len() {
            let mut state = Screen::default();
            state.tab = tab;
            term.draw(|frame| {
                draw(frame, &state, &[]);
            })
            .unwrap();
        }
    }
    #[test]
    fn plain_app_has_attach_record_send_and_copy() {
        for (width, height) in [(40, 16), (80, 24), (160, 50)] {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut state = Screen::default();
            state.tab = 1;
            let mut hits = vec![];
            term.draw(|f| hits = draw(f, &state, &[])).unwrap();
            for action in [Click::Attach, Click::Record, Click::Key(KeyCode::Esc)] {
                assert!(
                    hits.iter().any(|h| h.action == action),
                    "{action:?} at {width}"
                );
            }
            state.input = "shiur.mp3".into();
            state.plain_log = vec![(false, "א גוטן טאג".into())];
            term.draw(|f| hits = draw(f, &state, &[])).unwrap();
            assert!(hits.iter().any(|h| h.action == Click::Key(KeyCode::Enter)));
            assert!(hits.iter().any(|h| h.action == Click::CopyAt(0)));
            assert!(!hits.iter().any(|h| matches!(h.action, Click::Tab(_))));
            for (i, a) in hits.iter().enumerate() {
                for b in &hits[i + 1..] {
                    assert!(a.area.intersection(b.area).is_empty(), "{width}: overlap");
                }
            }
        }
    }
    #[test]
    fn engine_chip_shows_the_engine_and_switches_once() {
        let show = |state: &Screen, width: u16| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw(f, state, &[])).unwrap();
            for (i, a) in hits.iter().enumerate() {
                for b in &hits[i + 1..] {
                    assert!(a.area.intersection(b.area).is_empty(), "{width}: overlap");
                }
            }
            let clickable = hits.iter().any(|h| h.action == Click::Compute);
            (rendered_text(&term), clickable)
        };
        let engine = |mode: &str, available: bool| {
            Some(json!({"compute": {"mode": mode, "gpu_available": available}}))
        };
        let mut state = Screen::default();
        state.tab = 1;
        // A service that says nothing about engines: no chip.
        state.status = Some(json!({"name": "3Yiddish"}));
        let (text, clickable) = show(&state, 80);
        assert!(!clickable && !text.contains("GPU") && !text.contains("CPU"));
        for width in [62, 80, 160] {
            state.status = engine("gpu", true);
            let (text, clickable) = show(&state, width);
            assert!(clickable && text.contains("GPU"), "{width}");
            state.status = engine("cpu", true);
            let (text, clickable) = show(&state, width);
            assert!(clickable && text.contains("CPU"), "{width}");
            // No graphics card: the chip is there, and is not a button.
            state.status = engine("cpu", false);
            let (text, clickable) = show(&state, width);
            assert!(!clickable && text.contains("CPU"), "{width}");
        }
        // Crowded: everything else keeps its place, with or without the chip.
        state.status = engine("gpu", true);
        assert!(!show(&state, 40).0.contains("GPU"));
        state.app_update = Some("9.9.9".into());
        state.model_update = Some("first".into());
        state.notice = "Could not hear anything.".into();
        state.busy = true;
        for width in [62, 70, 80, 100] {
            let (text, clickable) = show(&state, width);
            assert!(!clickable, "{width}: no switching while transcribing");
            for kept in ["Update app", "Get model", "◌ Transcribing"] {
                assert!(text.contains(kept), "{width}: {kept}");
            }
        }
        assert!(show(&state, 100).0.contains("Could not hear anything."));
        state.busy = false;
        // A click shows the other engine at once and asks the service once.
        assert_eq!(switch_compute(&mut state), Some(false));
        assert_eq!(compute(&state), Some((false, true)));
        assert_eq!(switch_compute(&mut state), None);
        let (text, clickable) = show(&state, 100);
        assert!(!clickable && text.contains("CPU"));
        // Without a graphics card a click does nothing.
        state.compute_switch = None;
        state.status = engine("cpu", false);
        assert_eq!(switch_compute(&mut state), None);
        assert_eq!(compute(&state), Some((false, false)));
        // Demo mode pretends by itself when the service says nothing.
        state.status = None;
        assert_eq!(compute(&state), None);
        state.demo_gpu = Some(true);
        assert!(show(&state, 100).1);
        assert_eq!(switch_compute(&mut state), None);
        assert_eq!(compute(&state), Some((false, true)));
    }
    #[test]
    fn read_aloud_button_and_score_card() {
        let mut state = Screen::default();
        state.lab = true;
        state.tab = 1;
        // No file box or Transcribe button until something is typed; Record is always there.
        assert!(!controls(&state).iter().any(|c| c.0 == "Transcribe"));
        assert!(controls(&state).iter().any(|c| c.1 == Click::Record));
        state.read_line = Some("גוט מארגן".into());
        assert!(controls(&state).iter().any(|c| c.0 == "Skip"));
        // A reading is scored against the line it was recorded for, not the one now on screen.
        state.read_pending = Some("גוט מארגן".into());
        state.read_line = Some("א גוטן".into());
        let log = std::env::temp_dir().join(format!("read-aloud-{}.jsonl", uuid::Uuid::new_v4()));
        score_reading(&mut state, "גוט מארגן", None, &log);
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
        std::fs::remove_file(&log).unwrap();
        let r = state.read_result.clone().unwrap();
        assert_eq!(
            (r["score"].as_u64(), r["line"].as_str()),
            (Some(100), Some("גוט מארגן"))
        );
        // Skip had already moved on, so the next line stays; Back brings the scored line back.
        assert_eq!(state.read_line.as_deref(), Some("א גוטן"));
        assert!(state.read_pending.is_none());
        assert!(controls(&state).iter().any(|c| c.0 == "◀ Back"));
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        let screen = rendered_text(&term);
        assert!(
            screen.contains("You read")
                && screen.contains("3Yiddish heard")
                && screen.contains("Next")
        );
        assert!(!screen.contains("Audio file"));
        state.read_pending = Some("גוט מארגן".into());
        score_reading(&mut state, "I'll see you next time.", None, &log);
        assert!(state.read_result.as_ref().unwrap()["missed_audio"] == true && !log.exists());
        // Phone taps do what the TUI buttons do; Back walks through earlier readings.
        state.read_history = vec!["אחד".into(), "צוויי".into()];
        state.read_line = Some("דריי".into());
        assert_eq!(game_view(&state)["back"], true);
        // While a reading is still being transcribed, Back and Skip do nothing.
        state.read_pending = Some("דריי".into());
        assert!(controls(&state).iter().any(|c| c.0 == "◀ Back" && !c.2));
        phone_tap(&mut state, "back");
        assert_eq!(state.read_line.as_deref(), Some("דריי"));
        state.read_pending = None;
        phone_tap(&mut state, "back");
        assert_eq!(state.read_line.as_deref(), Some("צוויי"));
        phone_tap(&mut state, "back");
        assert_eq!(state.read_line.as_deref(), Some("אחד"));
        assert_eq!(game_view(&state)["back"], false);
        phone_tap(&mut state, "record");
        assert_eq!(state.pending_key, Some(KeyCode::F(10)));
        // My readings: last take per line wins, tossed lines drop out, newest first.
        let log = std::env::temp_dir().join(format!("readings-{}.jsonl", uuid::Uuid::new_v4()));
        std::fs::write(
            &log,
            concat!(
                r#"{"reference":"אחד","cer":0.5,"audio":"state/asr-train/11.audio"}"#,
                "\n",
                r#"{"reference":"צוויי","cer":0.1,"audio":"state/asr-train/22.audio"}"#,
                "\n",
                r#"{"reference":"אחד","cer":0.0,"audio":"state/asr-train/33.audio"}"#,
                "\n",
            ),
        )
        .unwrap();
        let items = readings(log.to_str().unwrap());
        assert_eq!(items.len(), 2);
        assert_eq!(
            (
                items[0]["reference"].as_str(),
                reading_id(&items[0]).as_str()
            ),
            (Some("אחד"), "33")
        );
        toss_reading(&log, "33").unwrap();
        let items = readings(log.to_str().unwrap());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["reference"], "צוויי");
        std::fs::remove_file(&log).unwrap();
        state.read_items = items;
        state.read_list = true;
        let labels: Vec<_> = controls(&state).iter().map(|c| c.0).collect();
        assert!(
            labels.contains(&"▶ Play")
                && labels.contains(&"✗ Toss")
                && !labels.contains(&"● Record")
        );
        // Playground: the four versions are shown and each is a choice.
        state.read_list = false;
        state.pg_mode = true;
        state.pg_items = vec![
            json!({"n":7,"gold":"א גוטן","line":"א גוטען","yl":"א גוטן","ours":"א גוט","confident":false,"audio":"state/asr-train/7.audio"}),
        ];
        let labels: Vec<_> = controls(&state).iter().map(|c| c.0).collect();
        assert!(
            ["✔ Gold", "✔ Line", "✔ Reference", "✔ Ours", "✗ Toss"]
                .iter()
                .all(|l| labels.contains(l))
        );
        assert_eq!(pg_to_check(&state), 1);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        term.draw(|f| {
            draw(f, &state, &[]);
        })
        .unwrap();
        let screen = rendered_text(&term);
        assert!(
            screen.contains("Gold")
                && screen.contains("Reference heard")
                && screen.contains("judge unsure")
        );
        state.pg_mode = false;
        // Plain mode (no game line): a recording is still sent to be transcribed.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut plain = Screen::default();
        plain.input = "state/recordings/x.wav".into();
        submit_reading(&mut plain, &tx).unwrap();
        let sent = rx.try_recv().unwrap();
        assert_eq!(
            (sent.tab, sent.input.as_str()),
            (1, "state/recordings/x.wav")
        );
        assert!(sent.history.is_empty() && plain.busy);
        let (_, cer) = errors("גוט מארגן", "גוט מארגן").unwrap();
        assert_eq!(
            errors("כ'בין זעהענדיג יאהרן", "איך בין זעענדיג יארן").unwrap(),
            (0.0, 0.0)
        );
        assert!(errors("מקפיד", "מאכטערט").unwrap().1 > 0.5);
        assert_eq!(cer, 0.0);
        // Already-read lines are never served again.
        let pool: String = (0..30)
            .map(|i| format!("ער איז גוט {i}\n"))
            .collect::<String>()
            + "די אשה השונמית איז תלוי\nא גוטן\n";
        let used = [crate::knowledge::normalize("די אשה השונמית איז תלוי")].into();
        for roll in 0..40 {
            let line = pick_line(&pool, &used, roll).unwrap();
            assert!(!line.contains("השונמית"));
        }
        assert_eq!(pick_line("א גוטן\n", &["א גוטן".into()].into(), 0), None);
        assert!(!is_yiddish(
            "זכתה בעלזא, ורבים מהניגונים החסידיים יצאו מבית מדרשה."
        ));
    }
    /// The phone look (Termux): every state, on screens from the narrowest allowed up to a
    /// phone held upright, with the phone's own font and with a Nerd Font. Nothing overlaps,
    /// no shape is built from half blocks (Termux draws those as bricks), and nothing runs
    /// off the screen.
    #[test]
    fn phone_look_fits_narrow_screens_without_blocks_or_overlap() {
        PHONE_LOOK.with(|o| o.set(true));
        for nerd in [false, true] {
            NERD_FOR_TEST.with(|n| n.set(Some(nerd)));
            for width in 40..=64u16 {
                for height in [16u16, 22, 31, 52] {
                    let mut plain = Screen::default();
                    plain.tab = 1;
                    let mut first = Screen::default();
                    first.tab = 1;
                    first.model_update = Some("first".into());
                    first.font_offer = true;
                    let mut talking = Screen::default();
                    talking.tab = 1;
                    talking.demo_rec = Some(std::time::Instant::now());
                    talking.rec_started = Some(std::time::Instant::now());
                    let mut alert = Screen::default();
                    alert.tab = 1;
                    alert.notice = "Install the Termux:API app first.".into();
                    alert.plain_log = vec![
                        (true, String::new()),
                        (false, "א גוטן טאג, וואס מאכט איר? ".repeat(4)),
                    ];
                    alert.voices.insert(
                        0,
                        Voice {
                            path: PathBuf::new(),
                            seconds: 7.0,
                            peaks: vec![40, 200, 90, 255],
                        },
                    );
                    let mut busy = Screen::default();
                    busy.tab = 1;
                    busy.busy = true;
                    busy.input = "/sdcard/Download/a.m4a".into();
                    for (name, state) in [
                        ("plain", &plain),
                        ("first", &first),
                        ("talking", &talking),
                        ("alert", &alert),
                        ("busy", &busy),
                    ] {
                        let at = format!("{name} {width}x{height} nerd={nerd}");
                        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
                            width, height,
                        ))
                        .unwrap();
                        let mut hits = vec![];
                        term.draw(|f| hits = draw(f, state, &[])).unwrap();
                        let text = rendered_text(&term);
                        assert!(
                            !text.contains(['▄', '▀', '▐', '▌', '▗', '▖', '▝', '▘', '▔']),
                            "{at}: block shapes\n{text}"
                        );
                        assert!(text.contains("3YIDDISH"), "{at}\n{text}");
                        // Controls with the phone's font are outlines; pills with a Nerd Font.
                        assert_eq!(text.contains("╭───╮"), !nerd, "{at}\n{text}");
                        for (i, a) in hits.iter().enumerate() {
                            assert!(
                                a.area.right() <= width && a.area.bottom() <= height,
                                "{at}: {:?} off the screen",
                                a.action
                            );
                            for b in &hits[i + 1..] {
                                assert!(
                                    a.area.intersection(b.area).is_empty(),
                                    "{at}: {:?} over {:?}\n{text}",
                                    a.action,
                                    b.action
                                );
                            }
                        }
                        for action in [Click::Attach, Click::Key(KeyCode::Esc)] {
                            assert!(
                                hits.iter().any(|h| h.area.width > 0 && h.action == action)
                                    || (action == Click::Attach
                                        && (name == "talking" || name == "busy")),
                                "{at}: {action:?}"
                            );
                        }
                        if name == "alert" {
                            // The alert is shown whole, on its own row.
                            assert!(
                                text.contains("Install the Termux:API app first."),
                                "{at}\n{text}"
                            );
                        }
                        if name == "first" {
                            // The font chip gives way first where both do not fit.
                            assert!(width < 50 || text.contains("Get font"), "{at}\n{text}");
                            assert!(text.contains("Get model"), "{at}\n{text}");
                        }
                        if let Ok(folder) = std::env::var("TUI_LAYOUT_DUMP_DIR") {
                            std::fs::create_dir_all(&folder).unwrap();
                            std::fs::write(
                                Path::new(&folder)
                                    .join(format!("phone-{name}-{width}x{height}-nerd-{nerd}.txt")),
                                &text,
                            )
                            .unwrap();
                        }
                    }
                }
            }
        }
        NERD_FOR_TEST.with(|n| n.set(None));
        PHONE_LOOK.with(|o| o.set(false));
    }
    #[test]
    fn nerd_font_is_told_from_the_font_file_itself() {
        assert!(names_nerd_font(b"\0\0Cousine Nerd Font Mono\0"));
        let wide: Vec<u8> = "JetBrainsMono Nerd Font"
            .bytes()
            .flat_map(|b| [0, b])
            .collect();
        assert!(names_nerd_font(&wide));
        assert!(!names_nerd_font(b"Droid Sans Mono"));
    }
    /// The terminal's answers arrive in one piece: all of it is read (not just what one
    /// buffered read happened to take), nothing after it is taken, and a terminal that
    /// answers nothing does not hold up the start.
    #[cfg(unix)]
    #[test]
    fn terminal_reply_is_read_whole_and_never_waited_for_long() {
        let pipe = || {
            let mut fds = [0; 2];
            // SAFETY: a fresh pipe; both ends are closed below.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            (fds[0], fds[1])
        };
        let (read, write) = pipe();
        // Termux: no palette answer, the background, then the attributes; a key follows.
        let reply = b"\x1b]11;rgb:0000/0000/0000\x07\x1b[?64;1;2;6;9;15;18;21;22cq";
        // SAFETY: writes the bytes above to the pipe just made.
        unsafe { libc::write(write, reply.as_ptr().cast(), reply.len()) };
        let begun = std::time::Instant::now();
        let text = read_reply(read, Duration::from_secs(5)).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert!(text.ends_with("22c") && text.contains("]11;rgb:0000"));
        // No palette: no theme, and the defaults are used.
        assert_eq!(parse_theme(&text), None);
        // The key after the answer is still there for the app.
        let mut key = 0u8;
        // SAFETY: reads one byte from the pipe.
        assert_eq!(unsafe { libc::read(read, (&raw mut key).cast(), 1) }, 1);
        assert_eq!(key, b'q');
        // A desktop terminal answers everything.
        let full = b"\x1b]4;4;rgb:8080/a0a0/ffff\x07\x1b]4;1;rgb:ffff/4040/4040\x07\x1b]11;rgb:1010/1010/2020\x07\x1b[?62;4c";
        // SAFETY: as above.
        unsafe { libc::write(write, full.as_ptr().cast(), full.len()) };
        let (accent, record, _) =
            parse_theme(&read_reply(read, Duration::from_secs(5)).unwrap()).unwrap();
        assert_eq!((accent, record), ([0x80, 0xa0, 0xff], [0xff, 0x40, 0x40]));
        // Silence: given up on after the limit.
        let begun = std::time::Instant::now();
        assert_eq!(read_reply(read, Duration::from_millis(150)), None);
        assert!(begun.elapsed() < Duration::from_secs(2));
        // SAFETY: the two ends of the pipe made above.
        unsafe {
            libc::close(read);
            libc::close(write);
        }
    }
}
