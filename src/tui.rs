//! The app's screen: one chat of recordings and their transcripts. The speech model stays
//! behind the local HTTP service.
use anyhow::{Context, Result, bail};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::mpsc;
mod playground;
use playground::{Page, PlayClick, ShareCard};
enum Update {
    Status(Value),
    /// A transcript (or, in chat, the reply) from the service, or why there is none.
    Done(Result<Value, String>),
    /// Chat: what the service heard in the recording just sent, shown as the user's words.
    Heard(String),
    /// Newer app release / newer (or first) model found by the background check.
    Available(Option<String>, Option<String>),
    /// Progress or result of an app/model update, shown as the notice line.
    Upgrade(String, bool),
    /// The models that can be picked (published or on disk), found in the background.
    Catalog(Vec<crate::model::Listing>),
    /// A download under way: what it is, bytes done and bytes in all.
    Progress(String, u64, u64),
    /// How much chat takes to download here, found in the background.
    ChatSize(Option<u64>),
    /// Chat finished downloading: the app moves to it.
    ChatReady,
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
    /// The phone pairing as the service tells it; `true` when it answers a Pair or Unpair
    /// click (the rest are the regular look every second).
    Pair(Result<Value, String>, bool),
    /// The service answered a playground request.
    Play(playground::Answer),
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
    /// This session's messages, newest last; `true` marks what the user sent (a file name,
    /// a recording), `false` a transcript.
    plain_log: Vec<(bool, String)>,
    /// Sent recordings and files, by their place in `plain_log`: shown as voice notes.
    voices: std::collections::HashMap<usize, Voice>,
    /// Chatting (true) or writing down what is said (false). The messages, voice notes and
    /// draft of the one not showing wait in `other_*`.
    chat: bool,
    other_log: Vec<(bool, String)>,
    other_voices: std::collections::HashMap<usize, Voice>,
    other_input: String,
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
    /// The notice on screen and when it appeared, so it can clear by itself.
    notice_since: Option<(String, std::time::Instant)>,
    /// Kept alive so copied text stays on the clipboard (Linux drops it with the owner).
    #[cfg_attr(target_os = "android", allow(dead_code))]
    clipboard: Option<Clip>,
    input: String,
    status: Option<Value>,
    /// The engine (true: GPU) a click asked for, shown at once while the service switches.
    compute_switch: Option<bool>,
    /// Demo mode's pretend engine, for a service that does not report one.
    demo_gpu: Option<bool>,
    /// Updates found at startup (app version, model label) and whether one is running.
    app_update: Option<String>,
    model_update: Option<String>,
    /// The active model's name, when there is more than one to pick and a click can switch.
    model_switch: Option<String>,
    /// The models that can be picked: published, or on disk.
    catalog: Vec<crate::model::Listing>,
    /// The question to download the model at this place in `catalog`.
    model_card: Option<usize>,
    /// A download under way: what it is, bytes done and bytes in all.
    progress: Option<(String, u64, u64)>,
    /// How much chat takes to download here (None: not known, or chat does not run here).
    chat_size: Option<u64>,
    /// The question to download chat.
    chat_card: bool,
    /// Termux with the phone's own font: offer the font in which Yiddish letters read well.
    font_offer: bool,
    upgrading: bool,
    /// A new version was just put in place: the refresh button is for starting it.
    restart_due: bool,
    /// The playground is switched on here (`3yiddish playground on`): Pair phone shows.
    playground: bool,
    /// The phone pairing as the service last told it.
    pair: Option<Value>,
    /// The pairing view (QR code, address) is open in place of the chat.
    pair_view: bool,
    /// A Pair or Unpair click is on its way to the service.
    pair_busy: bool,
    /// A key press asked for by a click, handled like the key itself.
    pending_key: Option<KeyCode>,
    busy: bool,
    notice: String,
    playback: Option<crate::audio::Playback>,
    recording: Option<crate::audio::Recorder>,
    recording_path: Option<PathBuf>,
    /// The playground's screens (where it is switched on).
    play: playground::Play,
    /// Sharing is on here: the heart in the top bar is full, and Contribute can stop it.
    sharing: bool,
    /// The sharing card is showing: the first-run question, or the chip's card.
    share_card: Option<ShareCard>,
}
/// What Refresh carries over to the new program: the chat and what is being typed. Every
/// field is optional, and unknown ones are ignored, so a file written by an older version
/// (which had more to carry) still opens.
#[derive(Default, Serialize, Deserialize)]
struct Resume {
    #[serde(default)]
    input: String,
    /// The chat: (sent by the user, text) for each message.
    #[serde(default)]
    plain_log: Vec<(bool, String)>,
    /// Its voice notes: (message number, file, seconds, wave).
    #[serde(default)]
    voices: Vec<(usize, PathBuf, f32, Vec<u8>)>,
    /// The pairing view was open.
    #[serde(default)]
    pair_view: bool,
    /// The playground was showing (true) or the chat (false); none from a version without
    /// it, or where the playground is off: the playground then opens first.
    #[serde(default)]
    playground_view: Option<bool>,
    /// Chat was showing, and the messages and draft of the side that was not.
    #[serde(default)]
    chat: bool,
    #[serde(default)]
    other_log: Vec<(bool, String)>,
    #[serde(default)]
    other_input: String,
}
impl Resume {
    fn capture(s: &Screen) -> Self {
        Self {
            pair_view: s.pair_view,
            playground_view: s.playground.then_some(s.play.view),
            input: s.input.clone(),
            plain_log: s.plain_log.clone(),
            chat: s.chat,
            // Without its voice notes: a recording that was sent shows as its text only.
            other_log: s
                .other_log
                .iter()
                .filter(|(_, text)| !text.is_empty())
                .cloned()
                .collect(),
            other_input: s.other_input.clone(),
            // The chat's voice notes only (a playground take is fetched again).
            voices: s
                .voices
                .iter()
                .filter(|(index, _)| **index < s.plain_log.len())
                .map(|(index, v)| (*index, v.path.clone(), v.seconds, v.peaks.clone()))
                .collect(),
        }
    }
    fn apply(self, s: &mut Screen) {
        s.pair_view = self.pair_view;
        s.input = self.input;
        s.plain_log = self.plain_log;
        s.chat = self.chat;
        s.other_log = self.other_log;
        s.other_input = self.other_input;
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
    // A playground take goes to the free bench, not into the chat.
    if state.play.recording {
        return playground::finish(state, tx);
    }
    let Some(recorder) = state.recording.take() else {
        return;
    };
    let Some(path) = state.recording_path.take() else {
        return;
    };
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
    state.busy = true;
    save_take(recorder, path, tx);
}
/// Finish a recording in a helper thread: its last moments, then a private WAV file at
/// `path`; reported with `Update::Recorded`.
fn save_take(recorder: crate::audio::Recorder, path: PathBuf, tx: &mpsc::UnboundedSender<Update>) {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Click {
    Key(KeyCode),
    /// Open the pairing view (and pair, when no phone is paired yet).
    Pair,
    /// Leave the pairing view.
    PairBack,
    Unpair,
    Record,
    UpdateApp,
    UpdateModel,
    /// Use the next model (asking to download it first when it is not here).
    SwitchModel,
    /// Download the model at this place in the list, from its question.
    GetModel(usize),
    /// Leave the download question.
    ModelNotNow,
    /// Download chat, from its question.
    GetChat,
    /// Chat, or write down what is said.
    Mode,
    GetFont,
    Compute,
    Attach,
    CopyAt(usize),
    VoiceToggle(usize),
    /// Jump to this many thousandths of the way through.
    VoiceSeek(usize, u16),
    /// Something on the playground's screens.
    Play(PlayClick),
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
/// The active model's name for the switch chip; None with fewer than two models installed.
/// The name of the model in use, when there is more than one to pick from (on disk or
/// published): then the model button shows it.
fn model_switch_label(catalog: &[crate::model::Listing]) -> Option<String> {
    let root = crate::paths::models();
    let all = crate::model::installed(&root);
    let now = crate::model::current(&root)?.0;
    let choices = catalog.len().max(all.len());
    (choices > 1).then(|| {
        all.into_iter()
            .find(|(v, _)| *v == now)
            .map(|(_, label)| label)
    })?
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
type Control<'a> = (&'a str, Click, bool);
/// Work for the service: a recording or file to write down, or a chat turn.
enum Job {
    Transcribe(PathBuf),
    /// The chat so far (sent by the user, text), ending in what the user just typed; or, with
    /// `audio`, in what that recording says once it is written down.
    Chat {
        log: Vec<(bool, String)>,
        audio: Option<PathBuf>,
    },
}
/// One chat turn: the recording (if any) is written down first, then the whole chat goes to
/// the service's chat model. Returns the reply as `{"text": ...}`.
async fn chat(
    client: &reqwest::Client,
    base: &str,
    mut log: Vec<(bool, String)>,
    audio: Option<&Path>,
    heard: &mpsc::UnboundedSender<Update>,
) -> Result<Value> {
    if let Some(path) = audio {
        let said = transcribe(client, base, path).await?["text"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if said.is_empty() {
            bail!("Could not hear anything.");
        }
        let _ = heard.send(Update::Heard(said.clone()));
        log.push((true, said));
    }
    // The last turns only: what a small model can keep in mind.
    let from = log.len().saturating_sub(12);
    let from = (from..log.len()).find(|&i| log[i].0).unwrap_or(from);
    let messages: Vec<Value> = log[from..]
        .iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(user, text)| json!({"role": if *user { "user" } else { "assistant" }, "content": text}))
        .collect();
    let answer = json_response(
        client
            .post(format!("{base}/v1/chat/completions"))
            .json(&json!({"messages": messages}))
            .send()
            .await?,
    )
    .await?;
    let reply = answer
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .context("No reply")?;
    Ok(json!({"text": reply}))
}
/// "Downloading our Whisper-based model ▕██████░░░░░░░░░░▏ 37%".
fn progress_bar(what: &str, done: u64, total: u64) -> String {
    let share = (done as f64 / total.max(1) as f64).clamp(0.0, 1.0);
    let filled = (share * 16.0).round() as usize;
    format!(
        "Downloading {what} ▕{}{}▏ {}%",
        "█".repeat(filled),
        "░".repeat(16 - filled),
        (share * 100.0) as u32
    )
}
/// What there is to get, most needed first, with the words the bar shows for it: the first
/// model (nothing works without it), a new version of the app, a newer model, the font.
/// Nothing while one is on its way, or once a new version waits for Refresh to start it.
fn offer(state: &Screen) -> Option<(Click, &'static str)> {
    if state.upgrading || state.restart_due {
        return None;
    }
    let model = state.model_update.as_deref();
    if model == Some("first") {
        Some((Click::UpdateModel, "Click the arrow to get the model."))
    } else if state.app_update.is_some() {
        Some((Click::UpdateApp, "New version: click the arrow."))
    } else if model.is_some() {
        Some((Click::UpdateModel, "New model: click the arrow."))
    } else if state.font_offer {
        Some((Click::GetFont, "Click the arrow for a better font."))
    } else {
        None
    }
}
/// Chat runs here but is not downloaded yet: the Chat button asks to download it.
fn chat_missing(state: &Screen) -> bool {
    state.status.as_ref().is_some_and(|s| {
        s.pointer("/components/schmooze/status")
            .and_then(Value::as_str)
            == Some("not-installed")
    })
}
/// Does the service have a chat model set up, or one to download? Then there is a Chat
/// button.
fn chat_offered(state: &Screen) -> bool {
    state.status.as_ref().is_some_and(|s| {
        s.pointer("/components/schmooze/status")
            .and_then(Value::as_str)
            .is_some_and(|status| status != "placeholder")
    })
}
/// Switch between writing down and chatting: each keeps its own messages and its own draft.
fn switch_mode(state: &mut Screen) {
    state.chat = !state.chat;
    std::mem::swap(&mut state.plain_log, &mut state.other_log);
    std::mem::swap(&mut state.voices, &mut state.other_voices);
    std::mem::swap(&mut state.input, &mut state.other_input);
    state.voice_play = None;
    state.copied = None;
}

/// Is this playground button on the screen just drawn?
fn offered(hits: &[Hit], action: PlayClick) -> bool {
    hits.iter().any(|h| h.action == Click::Play(action))
}
/// How long the microphone stays open after a take, so the next one starts at once.
const MIC_REST: Duration = Duration::from_secs(120);
fn recording(state: &Screen) -> bool {
    state.recording.is_some() || state.demo_rec.is_some()
}
/// Send one audio file to the service and return its answer (`text` holds the transcript).
async fn transcribe(client: &reqwest::Client, base: &str, path: &Path) -> Result<Value> {
    let metadata = tokio::fs::metadata(path)
        .await
        .context("Cannot read recording")?;
    if metadata.len() > 25 * 1024 * 1024 {
        bail!("Recording exceeds 25 MiB");
    }
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(tokio::fs::read(path).await?).file_name("recording.wav"),
    );
    let result = json_response(
        client
            .post(format!("{base}/v1/audio/transcriptions"))
            .multipart(form)
            .send()
            .await?,
    )
    .await?;
    result["text"].as_str().context("No transcript")?;
    Ok(result)
}
pub async fn run(client: reqwest::Client, base: String) -> Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<Update>();
    let (jobs, mut pending) = mpsc::unbounded_channel::<Job>();
    let (compute_actions, mut compute_pending) = mpsc::unbounded_channel::<bool>();
    // Pair (true) and Unpair (false) clicks, for where the playground is switched on.
    let (pair_actions, mut pair_pending) = mpsc::unbounded_channel::<bool>();
    // The playground's own requests. Someone may accept Contribute while the app runs, so
    // nothing here depends on whether it was open at the start.
    let (play_jobs, play_pending) = mpsc::unbounded_channel::<playground::Job>();
    let play_worker = Some(tokio::spawn(playground::worker(
        client.clone(),
        base.clone(),
        play_pending,
        tx.clone(),
    )));
    let pair_client = client.clone();
    let pair_base = base.clone();
    let pair_tx = tx.clone();
    let pair_worker = tokio::spawn(async move {
        while let Some(open) = pair_pending.recv().await {
            let request = if open {
                pair_client.post(format!("{pair_base}/v1/pair"))
            } else {
                pair_client.delete(format!("{pair_base}/v1/pair"))
            };
            let result = async { json_response(request.send().await?).await }
                .await
                .map_err(|e: anyhow::Error| e.to_string());
            if pair_tx.send(Update::Pair(result, true)).is_err() {
                break;
            }
        }
    });
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
    let status_tx = tx.clone();
    let status_client = client.clone();
    let status_base = base.clone();
    let refresh = tokio::spawn(async move {
        // The service's status every two seconds; the phone pairing every second, where
        // there is a playground.
        for tick in 0u64.. {
            if tick % 2 == 0
                && let Ok(r) = status_client
                    .get(format!("{status_base}/v1/status"))
                    .send()
                    .await
                && let Ok(v) = json_response(r).await
                && status_tx.send(Update::Status(v)).is_err()
            {
                break;
            }
            if crate::phone::playground()
                && let Ok(r) = status_client
                    .get(format!("{status_base}/v1/pair"))
                    .send()
                    .await
                && let Ok(v) = json_response(r).await
            {
                let _ = status_tx.send(Update::Pair(Ok(v), false));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let done_tx = tx.clone();
    let (close_client, close_base) = (client.clone(), base.clone());
    let worker = tokio::spawn(async move {
        while let Some(job) = pending.recv().await {
            let path = match job {
                Job::Transcribe(path) => path,
                Job::Chat { log, audio } => {
                    let result = chat(&client, &base, log, audio.as_deref(), &done_tx)
                        .await
                        .map_err(|e| e.to_string());
                    if done_tx.send(Update::Done(result)).is_err() {
                        break;
                    }
                    continue;
                }
            };
            let started = std::time::Instant::now();
            let result = transcribe(&client, &base, &path)
                .await
                .map_err(|e| e.to_string());
            if crate::share::active() {
                let took = started.elapsed();
                // Which engine ran it: the service's own word, read just after.
                let engine = async {
                    let status =
                        json_response(client.get(format!("{base}/v1/status")).send().await?)
                            .await?;
                    anyhow::Ok(status["compute"]["mode"].as_str().map(str::to_owned))
                }
                .await
                .ok()
                .flatten();
                let (path, result) = (path.clone(), result.clone());
                tokio::task::spawn_blocking(move || {
                    // Made in the app (state/recordings), or a file someone attached.
                    let kind = if path
                        .parent()
                        .is_some_and(|p| p.ends_with("state/recordings"))
                    {
                        "recording"
                    } else {
                        "file"
                    };
                    let seconds = crate::share::wav_seconds(&path);
                    let facts =
                        crate::share::facts(kind, seconds, took, engine.as_deref(), &result);
                    crate::share::record(&path, facts);
                });
            }
            if done_tx.send(Update::Done(result)).is_err() {
                break;
            }
        }
    });
    // The window was closed, or the program was told to stop: the phone's page still comes
    // off Tailscale before the program goes.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (client, base) = (close_client.clone(), close_base.clone());
        if let (Ok(mut hangup), Ok(mut terminate)) = (
            signal(SignalKind::hangup()),
            signal(SignalKind::terminate()),
        ) {
            tokio::spawn(async move {
                tokio::select! {
                    _ = hangup.recv() => {}
                    _ = terminate.recv() => {}
                }
                let _ = client
                    .delete(format!("{base}/v1/pair?keep=true"))
                    .send()
                    .await;
                // Ending without the screen's own tidying: its copies of takes go here.
                playground::sweep(true);
                drop(Restore);
                std::process::exit(0);
            });
        }
    }
    let result = tokio::task::spawn_blocking(move || {
        terminal(rx, jobs, compute_actions, pair_actions, play_jobs, tx)
    })
    .await?;
    if let Some(worker) = play_worker {
        worker.abort();
    }
    refresh.abort();
    worker.abort();
    compute_worker.abort();
    pair_worker.abort();
    let exit = result?;
    if exit.refresh.is_none() && crate::share::active() {
        // Quitting sends what sharing has waiting, for a few seconds at most; the rest goes
        // next time.
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            crate::share::send(
                &crate::share::Queue::here(),
                &crate::share::relay(),
                true,
                crate::share::active,
            ),
        )
        .await;
    }
    if crate::phone::playground() && exit.refresh.is_none() {
        // Quitting closes the phone's page and takes its address off Tailscale. The link
        // itself is kept: the next start pairs again with the same one. (Refresh leaves
        // everything as it is: the new program carries on.)
        let _ = close_client
            .delete(format!("{close_base}/v1/pair?keep=true"))
            .send()
            .await;
    }
    // Chat's helpers end with the app (the refreshed app starts them again when needed).
    crate::chat::stop();
    if let Some(path) = exit.refresh {
        let error = reload(&path);
        let _ = std::fs::remove_file(path);
        return error;
    }
    Ok(())
}
fn terminal(
    mut updates: mpsc::UnboundedReceiver<Update>,
    jobs: mpsc::UnboundedSender<Job>,
    compute_actions: mpsc::UnboundedSender<bool>,
    pair_actions: mpsc::UnboundedSender<bool>,
    play_jobs: mpsc::UnboundedSender<playground::Job>,
    tx: mpsc::UnboundedSender<Update>,
) -> Result<Exit> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("The app needs an interactive terminal; use CLI commands in scripts");
    }
    quiet_stderr();
    let mut terminal = ratatui::init();
    let _restore = Restore;
    // Whatever was on the screen before (a terminal left in a half state) goes.
    terminal.clear()?;
    // Before mouse reporting starts, so nothing else arrives in the terminal's answers.
    init_graphics();
    // The microphone opens with the first take and closes again after a quiet while
    // (MIC_REST), so an idle app holds nothing open.
    crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    // Background check for a newer app and model; never blocks the screen, silent offline.
    let check_tx = tx.clone();
    std::thread::spawn(move || {
        let app = crate::update::latest()
            .ok()
            .filter(|v| crate::update::newer(env!("CARGO_PKG_VERSION"), v));
        let have = crate::model::current(&crate::paths::models()).map(|(v, ..)| v);
        // Without a model the button to get one shows even offline (it then says so).
        let model = if have.is_none() {
            Some("first".to_owned())
        } else if crate::model::current_is_local(&crate::paths::models()) {
            // A model put here by hand is not replaced by the published one.
            None
        } else {
            // Any published model that is not on disk yet, whichever one is in use.
            crate::model::wanted(&crate::paths::models())
                .ok()
                .filter(|wanted| *wanted)
                .map(|_| "new".to_owned())
        };
        let _ = check_tx.send(Update::Available(app, model));
        let _ = check_tx.send(Update::Catalog(crate::model::catalog(
            &crate::paths::models(),
        )));
        if crate::chat::installed().is_none() {
            let _ = check_tx.send(Update::ChatSize(crate::chat::size().ok().flatten()));
        }
    });
    let mut state = Screen {
        model_switch: model_switch_label(&[]),
        ..Screen::default()
    };
    let mut playground_view = None;
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
            playground_view = resume.playground_view;
            resume.apply(&mut state);
        }
    }
    state.font_offer = crate::font::wanted();
    state.playground = crate::phone::playground();
    // Sharing: asked once (a fresh install, or one from before the question), never on the
    // owner's own computer; what has waited an hour goes now.
    state.sharing = crate::share::on();
    if crate::share::ask() {
        state.share_card = Some(ShareCard::Ask);
    }
    crate::share::nudge(false);
    let mut share_seen = std::time::Instant::now();
    // Takes' temporary files a closed window left behind go first.
    if state.playground {
        playground::sweep(false);
    }
    // With the playground on, the app opens in it (or where Refresh left it).
    state.play.view = state.playground && playground_view.unwrap_or_else(crate::phone::owner);
    if !state.playground {
        state.pair_view = false;
    } else if crate::paths::pair_code().exists() {
        // A phone was paired when the app last ran (or before this Refresh): its link works
        // again without a click.
        pair_actions.send(true)?;
        state.pair_busy = true;
    }
    // THREE_YIDDISH_DEMO: canned replies, for trying the screen without a microphone.
    let demo = std::env::var_os("THREE_YIDDISH_DEMO").is_some();
    state.demo_gpu = demo.then_some(true);
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
    // When something last happened: for a short while after, the screen keeps its full pace.
    let mut stirred = std::time::Instant::now();
    // The microphone was last used then (None: it is closed).
    let mut mic_used: Option<std::time::Instant> = None;
    loop {
        if recording(&state) {
            mic_used = Some(std::time::Instant::now());
        } else if mic_used.is_some_and(|at| at.elapsed() >= MIC_REST) {
            crate::audio::rest();
            mic_used = None;
        }
        while let Ok(update) = updates.try_recv() {
            // The regular status look every second is not news.
            if !matches!(update, Update::Status(_) | Update::Pair(_, false)) {
                stirred = std::time::Instant::now();
            }
            match update {
                Update::Status(v) => state.status = Some(v),
                Update::Pair(result, answer) => {
                    if answer {
                        state.pair_busy = false;
                    }
                    match result {
                        Ok(v) => state.pair = Some(v),
                        Err(e) => state.notice = plain_alert(&e),
                    }
                }
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
                Update::Catalog(list) => {
                    // Nothing to transcribe with yet: the first thing asked is whether to
                    // download the model (after the sharing question, if that is up).
                    let none = crate::model::current(&crate::paths::models()).is_none();
                    if none && list.first().is_some_and(|m| !m.installed) {
                        state.model_card = Some(0);
                    }
                    state.catalog = list;
                    state.model_switch = model_switch_label(&state.catalog);
                }
                Update::Progress(label, done, total) => {
                    state.progress = Some((label, done, total));
                }
                Update::ChatSize(size) => state.chat_size = size,
                Update::ChatReady => {
                    if !state.chat {
                        switch_mode(&mut state);
                    }
                }
                Update::Upgrade(text, finished) => {
                    state.notice = text;
                    // The same words again (a second try) stay up for their full time.
                    state.notice_since = None;
                    if finished {
                        state.upgrading = false;
                        state.restart_due |= state.notice.starts_with("Updated to");
                        state.progress = None;
                        // A second model may have arrived: its switch button shows at once.
                        state.model_switch = model_switch_label(&state.catalog);
                    }
                }
                Update::Play(answer) => playground::answer(&mut state, answer, &play_jobs),
                Update::Recorded(path, result) if state.play.pending.as_ref() == Some(&path) => {
                    playground::recorded(&mut state, path, result, &play_jobs);
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
                            jobs.send(if state.chat {
                                Job::Chat {
                                    log: state.plain_log.clone(),
                                    audio: Some(path),
                                }
                            } else {
                                Job::Transcribe(path)
                            })?;
                            state.busy = true;
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
                Update::Heard(said) => state.plain_log.push((true, said)),
                Update::Done(result) => {
                    state.busy = false;
                    match result {
                        Ok(value) => {
                            state.notice.clear();
                            let answer = value["text"].as_str().unwrap_or_default().trim();
                            if answer.is_empty() {
                                state.notice = "Could not hear anything.".into();
                            } else {
                                state.plain_log.push((false, answer.to_owned()));
                            }
                        }
                        Err(error) => state.notice = plain_alert(&error),
                    }
                }
            }
        }
        // The microphone could not be opened (found out in the background).
        if let Some(error) = state.recording.as_mut().and_then(|r| r.failed()) {
            state.recording = None;
            state.recording_path = None;
            state.play.recording = false;
            state.notice = error;
            state.notice_since = None;
        }
        // The recording hit its time limit by itself.
        if state.recording.as_ref().is_some_and(|r| r.finished()) {
            finish_recording(&mut state, &tx);
        }
        if state.playback.as_mut().is_some_and(|p| p.done()) {
            state.playback = None;
        }
        // Space to talk: the hold ended (its repeats stopped), or a lone tap asked to stop.
        // Only with no key waiting to be read: after a slow moment (a big redraw, a busy
        // computer) the repeats are still in the queue, and the hold has not ended.
        let waiting = event::poll(Duration::ZERO).unwrap_or(false);
        if recording(&state) && state.pending_key.is_none() && !waiting {
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
        // Alerts go away by themselves after a few seconds.
        if state.notice.is_empty() {
            state.notice_since = None;
        } else if state
            .notice_since
            .as_ref()
            .is_none_or(|(text, _)| *text != state.notice)
        {
            state.notice_since = Some((state.notice.clone(), std::time::Instant::now()));
        } else if state
            .notice_since
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed().as_secs() >= 4)
        {
            state.notice.clear();
        }
        playground::tick(&mut state, &play_jobs);
        // `3yiddish share on|off` in a shell shows here within a second.
        if share_seen.elapsed() >= Duration::from_secs(1) {
            share_seen = std::time::Instant::now();
            state.sharing = crate::share::on();
            if state.share_card == Some(ShareCard::Ask) && !crate::share::ask() {
                state.share_card = None;
            }
        }
        let mut hits = Vec::new();
        terminal.draw(|f| {
            hits = draw_plain(f, &state);
        })?;
        // A key asked for by a click (or by Space to talk) is not one the user typed.
        let mut typed = false;
        let key = if let Some(code) = state.pending_key.take() {
            KeyEvent::new(code, KeyModifiers::NONE)
        } else {
            // While something moves (a take, a transcription, playback, a download, the
            // moments after a click) the screen redraws twenty times a second; at rest, once
            // a second, so an idle app costs next to nothing.
            let lively = recording(&state)
                || state.busy
                || state.upgrading
                || state.progress.is_some()
                || state.voice_play.is_some()
                || state.playback.is_some()
                || state.play.thinking
                || state.play.recording
                || stirred.elapsed() < Duration::from_secs(3);
            let pace = if lively { 50 } else { 1000 };
            if !event::poll(Duration::from_millis(pace))? {
                continue;
            }
            stirred = std::time::Instant::now();
            match event::read()? {
                Event::Key(key) => {
                    typed = true;
                    key
                }
                Event::Paste(_) if state.share_card.is_some() => continue,
                Event::Paste(text) if state.play.view && !state.pair_view => {
                    playground::paste(&mut state, &text);
                    continue;
                }
                Event::Paste(text) => {
                    attach_text(&mut state, &text);
                    continue;
                }
                Event::Mouse(mouse)
                    if state.play.view
                        && matches!(
                            mouse.kind,
                            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                        ) =>
                {
                    playground::scroll(&mut state, mouse.kind == MouseEventKind::ScrollUp);
                    continue;
                }
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                    let Some(action) = hit_at(&hits, mouse.column, mouse.row) else {
                        continue;
                    };
                    match click(
                        &mut state,
                        action,
                        &tx,
                        &compute_actions,
                        &pair_actions,
                        &play_jobs,
                    )? {
                        Some(key) => key,
                        None => continue,
                    }
                }
                _ => continue,
            }
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // The sharing card: Esc and Enter are No thanks (the question) or Keep sharing and
        // Stop sharing (the chip's card); nothing is typed.
        if let Some(card) = state.share_card
            && !recording(&state)
        {
            let control = key.modifiers.contains(KeyModifiers::CONTROL);
            let answer = match (key.code, card) {
                (KeyCode::Esc, ShareCard::On) => Some(PlayClick::ShareClose),
                (KeyCode::Esc | KeyCode::Enter, _) => Some(PlayClick::Share(false)),
                (KeyCode::Char('r' | 'c'), _) if control => None,
                _ => continue,
            };
            if let Some(answer) = answer {
                playground::click(&mut state, answer, &play_jobs)?;
                continue;
            }
        }
        // The download question: Enter downloads, Esc is Not now; nothing is typed.
        let asking = if state.chat_card {
            Some(Click::GetChat)
        } else {
            state.model_card.map(Click::GetModel)
        };
        if let Some(get) = asking
            && state.share_card.is_none()
            && !recording(&state)
        {
            let control = key.modifiers.contains(KeyModifiers::CONTROL);
            let answer = match key.code {
                KeyCode::Enter => get,
                KeyCode::Esc => Click::ModelNotNow,
                KeyCode::Char('r' | 'c') if control => Click::Key(KeyCode::Null),
                _ => continue,
            };
            if answer != Click::Key(KeyCode::Null) {
                click(
                    &mut state,
                    answer,
                    &tx,
                    &compute_actions,
                    &pair_actions,
                    &play_jobs,
                )?;
                continue;
            }
        }
        // The playground: its fields and screens have the keys first (Esc steps back there,
        // and Space types a space in a field); recording, refresh and quit go on below.
        if typed
            && state.play.view
            && !state.pair_view
            && !recording(&state)
            && playground::key(&mut state, key, &play_jobs)
        {
            continue;
        }
        // The Contribute explanation: Esc is Never mind, Enter accepts; nothing is typed.
        if state.play.consent && !state.pair_view && !recording(&state) {
            let control = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc => {
                    state.play.consent = false;
                    continue;
                }
                // Only where I accept is offered: a window too small for all of the terms
                // has no way to agree.
                KeyCode::Enter => {
                    if offered(&hits, PlayClick::Accept) {
                        playground::click(&mut state, PlayClick::Accept, &play_jobs)?;
                    }
                    continue;
                }
                KeyCode::Char('r' | 'c') if control => {}
                _ => continue,
            }
        }
        // The pairing view: Esc goes back to the chat; nothing is typed or recorded there.
        if state.pair_view && !recording(&state) {
            let control = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc => {
                    state.pair_view = false;
                    continue;
                }
                KeyCode::Char('r' | 'c') if control => {}
                _ => continue,
            }
        }
        // Space, with nothing typed: hold to talk, let go to send; a single tap starts and
        // another tap stops. Terminals do not say when a key is released, but a held key
        // repeats, so a run of Spaces is a hold and its end is the release.
        // (On the playground the chat's typing is not on screen, so it does not count.)
        if key.code == KeyCode::Char(' ')
            && key.modifiers.is_empty()
            && (state.input.is_empty() || state.play.view)
        {
            let now = std::time::Instant::now();
            let busy = if state.play.view {
                state.play.thinking
            } else {
                state.busy
            };
            if !recording(&state) {
                if !busy {
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
        match key.code {
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if state.busy || recording(&state) || state.play.thinking || state.play.saving {
                    state.notice = "Finish the recording first, then refresh.".into();
                    continue;
                }
                match Resume::capture(&state).save() {
                    Ok(path) => {
                        return Ok(Exit {
                            refresh: Some(path),
                        });
                    }
                    Err(error) => state.notice = format!("Cannot refresh: {error}"),
                }
            }
            KeyCode::Esc => break,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                paste_clipboard(&mut state);
            }
            KeyCode::Enter if !state.busy && !state.input.trim().is_empty() => {
                let text = state.input.trim().to_owned();
                if Path::new(&text).is_file() {
                    // An audio file: shown as a voice note, and sent.
                    state.voices.insert(
                        state.plain_log.len(),
                        Voice {
                            path: PathBuf::from(&text),
                            seconds: 0.0,
                            peaks: Vec::new(),
                        },
                    );
                    state.plain_log.push((true, String::new()));
                    state.input.clear();
                    jobs.send(if state.chat {
                        Job::Chat {
                            log: state.plain_log.clone(),
                            audio: Some(PathBuf::from(text)),
                        }
                    } else {
                        Job::Transcribe(PathBuf::from(text))
                    })?;
                    state.busy = true;
                } else if state.chat && !demo {
                    state.plain_log.push((true, text));
                    state.input.clear();
                    jobs.send(Job::Chat {
                        log: state.plain_log.clone(),
                        audio: None,
                    })?;
                    state.busy = true;
                } else if demo {
                    state.plain_log.push((true, text));
                    state.input.clear();
                    demo_reply(&mut state, tx.clone());
                } else {
                    state.notice = "That is not an audio file.".into();
                }
            }
            KeyCode::F(10) if state.play.view && !state.pair_view => {
                playground::record_key(&mut state, demo, &tx)?;
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
                        demo_reply(&mut state, tx.clone());
                    }
                }
            }
            KeyCode::F(10) if state.recording.is_some() => finish_recording(&mut state, &tx),
            KeyCode::F(10) if !state.busy => {
                let folder = Path::new("state/recordings");
                std::fs::create_dir_all(folder)?;
                let path = folder
                    .canonicalize()?
                    .join(format!("{}.wav", uuid::Uuid::new_v4()));
                // Never waits for the device: the screen flips to recording at once.
                state.recording = Some(crate::audio::Recorder::start_now());
                state.recording_path = Some(path);
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
    Ok(Exit { refresh: None })
}
/// What a click does. Most act at once; the rest come back as the key that does the same.
fn click(
    state: &mut Screen,
    action: Click,
    tx: &mpsc::UnboundedSender<Update>,
    compute_actions: &mpsc::UnboundedSender<bool>,
    pair_actions: &mpsc::UnboundedSender<bool>,
    play_jobs: &mpsc::UnboundedSender<playground::Job>,
) -> Result<Option<KeyEvent>> {
    let code = match action {
        Click::Key(code) => code,
        Click::Play(action) => {
            let was = state.play.view;
            let done = playground::click(state, action, play_jobs);
            // The Contribute button says where it took you.
            if action == PlayClick::Toggle && state.play.view != was && state.notice.is_empty() {
                state.notice = if state.play.view {
                    "Contribute: say a few things and fix the words."
                } else if state.chat {
                    "Back to the chat."
                } else {
                    "Back to transcribing."
                }
                .into();
            }
            return done;
        }
        Click::Pair => {
            // The chip opens the view, and closes it again.
            state.pair_view = !state.pair_view;
            if state.pair_view && !paired(state) && !state.pair_busy {
                pair_actions.send(true)?;
                state.pair_busy = true;
            }
            return Ok(None);
        }
        Click::PairBack => {
            state.pair_view = false;
            return Ok(None);
        }
        Click::Unpair => {
            if !state.pair_busy {
                pair_actions.send(false)?;
                state.pair_busy = true;
            }
            state.pair_view = false;
            return Ok(None);
        }
        Click::Record => KeyCode::F(10),
        Click::Compute => {
            if let Some(gpu) = switch_compute(state) {
                compute_actions.send(gpu)?;
                state.notice = if gpu {
                    "Now running on the graphics card."
                } else {
                    "Now running on the processor."
                }
                .into();
            }
            return Ok(None);
        }
        Click::Attach => {
            pick_file(tx.clone());
            return Ok(None);
        }
        Click::VoiceToggle(index) => {
            voice_toggle(state, index);
            return Ok(None);
        }
        Click::VoiceSeek(index, permille) => {
            voice_seek(state, index, permille);
            return Ok(None);
        }
        Click::CopyAt(index) => {
            if let Some((_, text)) = state.plain_log.get(index).cloned() {
                copy_text(state, &text, tx);
                state.copied = Some((index, std::time::Instant::now()));
            }
            return Ok(None);
        }
        Click::UpdateApp => {
            state.upgrading = true;
            state.notice = "Downloading the new version…".into();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let text = match crate::update::update() {
                    Ok(Some(v)) => format!("Updated to {v}. Click Refresh to start it."),
                    Ok(None) => "Already up to date.".into(),
                    Err(e) => crate::model::plain_error(&e),
                };
                let _ = tx.send(Update::Upgrade(text, true));
            });
            state.app_update = None;
            return Ok(None);
        }
        Click::UpdateModel
            if state.model_update.as_deref() == Some("first")
                && state.catalog.first().is_some_and(|m| !m.installed) =>
        {
            state.model_card = Some(0);
            return Ok(None);
        }
        Click::UpdateModel => {
            state.upgrading = true;
            let tx = tx.clone();
            let label = state.model_update.take();
            std::thread::spawn(move || {
                let mut last = 101;
                let result = crate::model::pull_updates(
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
            return Ok(None);
        }
        Click::Mode if !state.chat && chat_missing(state) => {
            // Chat is not here yet: ask before downloading it.
            match state.chat_size {
                Some(_) => state.chat_card = true,
                None => {
                    state.notice =
                        "Chat cannot be downloaded right now; check the connection.".into()
                }
            }
            return Ok(None);
        }
        Click::GetChat => {
            state.chat_card = false;
            state.upgrading = true;
            state.progress = Some(("chat".into(), 0, state.chat_size.unwrap_or(0)));
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = crate::chat::pull(&mut |done, total| {
                    let _ = tx.send(Update::Progress("chat".into(), done, total));
                });
                let text = match result {
                    Ok(()) => {
                        let _ = tx.send(Update::ChatReady);
                        "Chat is ready: talk or type, and it answers in Yiddish.".to_owned()
                    }
                    Err(e) => crate::model::plain_error(&e),
                };
                let _ = tx.send(Update::Upgrade(text, true));
            });
            return Ok(None);
        }
        Click::Mode => {
            switch_mode(state);
            state.notice = if state.chat {
                "Chat: talk or type, and it answers."
            } else {
                "Transcribe: it writes down what you say."
            }
            .into();
            return Ok(None);
        }
        Click::SwitchModel if !state.catalog.is_empty() => {
            let root = crate::paths::models();
            let now = crate::model::current(&root).map(|(v, ..)| v);
            let at = state
                .catalog
                .iter()
                .position(|m| Some(&m.version) == now.as_ref());
            let next = at.map_or(0, |i| (i + 1) % state.catalog.len());
            let pick = state.catalog[next].clone();
            if pick.installed {
                match crate::model::use_version(&root, &pick.version) {
                    Ok(()) => {
                        state.notice = format!("Now using our {}-based model.", pick.label);
                        state.model_switch = model_switch_label(&state.catalog);
                    }
                    Err(e) => state.notice = crate::model::plain_error(&e),
                }
            } else {
                state.model_card = Some(next);
            }
            return Ok(None);
        }
        Click::ModelNotNow => {
            state.model_card = None;
            state.chat_card = false;
            return Ok(None);
        }
        Click::GetModel(index) => {
            state.model_card = None;
            let Some(pick) = state.catalog.get(index).cloned() else {
                return Ok(None);
            };
            state.upgrading = true;
            let what = format!("our {}-based model", pick.label);
            state.progress = Some((what.clone(), 0, pick.size));
            let tx = tx.clone();
            std::thread::spawn(move || {
                let root = crate::paths::models();
                // The download goes file by file; the bar shows the whole model.
                let (mut base, mut file, mut last) = (0u64, String::new(), (0u64, 0u64));
                let result =
                    crate::model::pull_one(&root, &pick.version, &mut |name, done, total| {
                        if name != file {
                            base += last.1;
                            file = name.to_owned();
                        }
                        last = (done, total);
                        let _ = tx.send(Update::Progress(what.clone(), base + done, pick.size));
                    });
                let text = match result {
                    Ok(_) => format!("Now using our {}-based model.", pick.label),
                    Err(e) => crate::model::plain_error(&e),
                };
                let _ = tx.send(Update::Catalog(crate::model::catalog(&root)));
                let _ = tx.send(Update::Upgrade(text, true));
            });
            return Ok(None);
        }
        Click::SwitchModel => {
            match crate::model::switch_next(&crate::paths::models()) {
                Ok((_, label)) => {
                    state.notice = format!("Now using our {label}-based model.");
                    state.model_switch = model_switch_label(&state.catalog);
                }
                Err(e) => state.notice = crate::model::plain_error(&e),
            }
            return Ok(None);
        }
        Click::GetFont => {
            state.upgrading = true;
            state.font_offer = false;
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut last = 101;
                let result = crate::font::install(&mut |done, total| {
                    let pct = done * 100 / total.max(1);
                    if pct != last {
                        last = pct;
                        let _ =
                            tx.send(Update::Upgrade(format!("Getting the font: {pct}%"), false));
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
            return Ok(None);
        }
    };
    Ok(Some(KeyEvent::new(
        code,
        if matches!(code, KeyCode::Char('r')) {
            KeyModifiers::CONTROL
        } else {
            KeyModifiers::NONE
        },
    )))
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
        "No model yet: click the arrow at the top first.".into()
    } else if said.starts_with("Model endpoint could not be reached")
        || said.starts_with("Translation worker is unreachable")
    {
        "The chat model is not running.".into()
    } else {
        said.to_owned()
    }
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
/// under the app (the download arrow), so the next frame asks again.
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
            "phone" => "▯",
            "check" => "✓",
            "heart" | "heart-open" => "♡",
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
            "download" => "\u{f019}",
            "phone" => "\u{f10b}",
            "check" => "\u{f00c}",
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
            "phone" => "▯",
            "check" => "✓",
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
/// The picture and the character for what the app is doing: a speech bubble for chat, a
/// written page for transcribing.
fn mode_icon(chat: bool) -> (&'static str, &'static [&'static str]) {
    if chat {
        ("chat", &["❝"])
    } else {
        ("page", &["¶"])
    }
}
/// The picture and the character that stand for a model on its button.
fn model_icon(label: &str) -> (&'static str, &'static [&'static str]) {
    match label {
        "Whisper" => ("whisper", &["W"]),
        "Omnilingual" => ("omni", &["Ω"]),
        _ => ("model", &["≡"]),
    }
}
/// Pair phone is offered on computers; on a phone the app is already on the phone.
fn phone_button() -> bool {
    cfg!(not(target_os = "android"))
}
/// Is a phone's page being served right now?
fn paired(state: &Screen) -> bool {
    state.pair.as_ref().is_some_and(|p| p["active"] == true)
}
/// The modules of the QR code for `text`, light border included: `true` is a dark module.
/// The text is sent in capitals: a code of capitals and digits is small enough for a
/// terminal (web addresses do not mind).
fn qr_modules(text: &str) -> Option<Vec<Vec<bool>>> {
    use qrcode::{Color as Module, EcLevel, QrCode};
    let text = text.to_uppercase();
    let small = QrCode::with_error_correction_level(&text, EcLevel::L).ok()?;
    // The sturdier code when it is no bigger.
    let code = QrCode::with_error_correction_level(&text, EcLevel::M)
        .ok()
        .filter(|code| code.width() == small.width())
        .unwrap_or(small);
    const QUIET: usize = 2;
    let (n, modules) = (code.width(), code.to_colors());
    let size = n + 2 * QUIET;
    Some(
        (0..size)
            .map(|y| {
                (0..size)
                    .map(|x| {
                        (QUIET..n + QUIET).contains(&x)
                            && (QUIET..n + QUIET).contains(&y)
                            && modules[(y - QUIET) * n + (x - QUIET)] == Module::Dark
                    })
                    .collect()
            })
            .collect(),
    )
}
/// A QR code as rows of half blocks (two modules per row), see-through: only the light
/// modules are drawn, in gray, and the dark ones are left empty, so the terminal's own
/// background (the wallpaper) is the code's dark. No box is filled in behind it.
fn qr_lines(text: &str) -> Option<Vec<Line<'static>>> {
    let modules = qr_modules(text)?;
    let light = |x: usize, y: usize| !modules.get(y).is_some_and(|row| row[x]);
    let style = Style::default().fg(Color::Gray).bg(Color::Reset);
    Some(
        (0..modules.len())
            .step_by(2)
            .map(|y| {
                let row: String = (0..modules.len())
                    .map(|x| match (light(x, y), light(x, y + 1)) {
                        (true, true) => '█',
                        (true, false) => '▀',
                        (false, true) => '▄',
                        (false, false) => ' ',
                    })
                    .collect();
                Line::styled(row, style)
            })
            .collect(),
    )
}
/// The pairing view, in place of the chat: the QR code to scan, the address in words, whether
/// the phone is there, and Back / Unpair. Beside each other on a wide window, stacked on a
/// tall one; where the code cannot be shown whole it is left out (a cut code cannot be
/// scanned) and the address stays.
fn draw_pair(frame: &mut Frame, state: &Screen, area: Rect, hits: &mut Vec<Hit>) {
    let pair = state.pair.as_ref();
    let field = |name: &str| pair.and_then(|p| p[name].as_str()).unwrap_or_default();
    let active = paired(state);
    let url = field("url");
    let qr = if active { qr_lines(url) } else { None };
    let (qr_w, qr_h) = qr.as_ref().map_or((0, 0), |lines| {
        (lines[0].width() as u16, lines.len() as u16)
    });
    // Beside the code when both fit; else under it; else the words alone.
    let beside = qr_w > 0 && area.width >= qr_w + 3 + 30 && area.height >= qr_h;
    let text_w = if beside {
        (area.width - qr_w - 3).min(46)
    } else {
        area.width.min(56)
    } as usize;
    // Gray, not the darker one: these words sit on the bare wallpaper.
    let dim = Style::default().fg(Color::Gray);
    let accent = Style::default().fg(THEME_ACCENT);
    // The words, each with how much it matters (0: always shown). On a small window the
    // least needed go first: blank rows, the count, the kind of link, advice, the note.
    let mut text: Vec<(u8, Line)> = vec![(
        0,
        Line::styled("Pair phone", Style::default().add_modifier(Modifier::BOLD)),
    )];
    let wrapped = |text: &mut Vec<(u8, Line)>, rank: u8, words: &str, style: Style| {
        for line in textwrap::wrap(words, text_w.max(8)) {
            text.push((rank, Line::styled(line.into_owned(), style)));
        }
    };
    if !active {
        wrapped(
            &mut text,
            0,
            if state.pair_busy || pair.is_none() {
                "Opening a link for the phone…"
            } else {
                "No phone is paired."
            },
            dim,
        );
    } else {
        if pair.is_some_and(|p| p["connected"] == true) {
            wrapped(&mut text, 0, "● Phone connected", accent);
        } else {
            wrapped(
                &mut text,
                0,
                "○ Waiting for the phone: scan the code",
                Style::default(),
            );
        }
        let (takes, goal) = (
            pair.and_then(|p| p["takes"].as_u64()).unwrap_or(0),
            pair.and_then(|p| p["goal"].as_u64()).unwrap_or(100),
        );
        // A contributor's count has no goal.
        let count = match (goal, takes) {
            (0, 1) => "1 take done".to_owned(),
            (0, _) => format!("{takes} takes done"),
            _ => format!("{takes} of {goal} takes done"),
        };
        wrapped(&mut text, 5, &count, dim);
        text.push((6, Line::raw("")));
        wrapped(
            &mut text,
            4,
            match field("transport") {
                "tailscale-https" => "Secure link over Tailscale.",
                "tailscale-http" => {
                    "Plain link over Tailscale: the phone records with its own recorder."
                }
                _ => "Plain link on this network: the phone records with its own recorder.",
            },
            dim,
        );
        for line in field("hint").lines() {
            wrapped(&mut text, 3, line, Style::default().fg(THEME_RECORD));
        }
        text.push((6, Line::raw("")));
        // The address in words; too long for one row, it breaks before the code.
        match url.trim_end_matches('/').rsplit_once('/') {
            Some((origin, code)) if url.chars().count() > text_w => {
                wrapped(&mut text, 0, &format!("{origin}/"), Style::default());
                wrapped(&mut text, 0, &format!("{code}/"), Style::default());
            }
            _ => wrapped(&mut text, 0, url, Style::default()),
        }
    }
    let stacked = !beside && qr_w > 0 && area.width >= qr_w && {
        let needed = qr_h + 1 + text.len() as u16 + 2;
        area.height >= needed
    };
    if !beside && !stacked && qr_w > 0 {
        text.push((6, Line::raw("")));
        wrapped(
            &mut text,
            2,
            "Make the window bigger to see the QR code.",
            dim,
        );
    }
    // Two rows are kept for the buttons.
    let room = area.height.saturating_sub(2) as usize;
    while text.len() > room {
        let Some(least) = text.iter().map(|(rank, _)| *rank).max().filter(|r| *r > 0) else {
            break;
        };
        text.retain(|(rank, _)| *rank != least);
    }
    let text: Vec<Line> = text.into_iter().map(|(_, line)| line).collect();
    // The buttons sit under the words, one row.
    let buttons: &[(&str, Click)] = if active {
        &[("Back", Click::PairBack), ("Unpair", Click::Unpair)]
    } else {
        &[("Back", Click::PairBack)]
    };
    let text_h = (text.len() as u16 + 2).min(area.height);
    let text_block_w = text.iter().map(|l| l.width()).max().unwrap_or(0).max(16) as u16;
    let (text_x, text_y) = if beside {
        let total = qr_w + 3 + text_block_w;
        let x = area.x + area.width.saturating_sub(total) / 2;
        let qr_y = area.y + (area.height - qr_h) / 2;
        frame.render_widget(
            Paragraph::new(qr.clone().unwrap_or_default()),
            Rect::new(x, qr_y, qr_w, qr_h),
        );
        (
            x + qr_w + 3,
            area.y + area.height.saturating_sub(text_h) / 2,
        )
    } else if stacked {
        let total = qr_h + 1 + text_h;
        let y = area.y + (area.height - total) / 2;
        frame.render_widget(
            Paragraph::new(qr.clone().unwrap_or_default()),
            Rect::new(area.x + (area.width - qr_w) / 2, y, qr_w, qr_h),
        );
        (
            area.x + area.width.saturating_sub(text_block_w) / 2,
            y + qr_h + 1,
        )
    } else {
        (
            area.x + area.width.saturating_sub(text_block_w) / 2,
            area.y + area.height.saturating_sub(text_h) / 2,
        )
    };
    let rows = text_h.saturating_sub(2);
    frame.render_widget(
        Paragraph::new(text.into_iter().take(rows as usize).collect::<Vec<_>>()),
        Rect::new(
            text_x,
            text_y,
            text_block_w.min(area.right().saturating_sub(text_x)),
            rows,
        ),
    );
    let mut x = text_x;
    let y = text_y + rows + 1;
    if y >= area.bottom() {
        return;
    }
    for (label, action) in buttons {
        let chip = format!(" {label} ");
        let w = chip.chars().count() as u16;
        if x + w > area.right() {
            break;
        }
        let rect = Rect::new(x, y, w, 1);
        let fill = if *action == Click::Unpair {
            THEME_RECORD
        } else {
            THEME_ACCENT
        };
        if outline_controls() {
            // The phone look has no filled shapes: the label in its color, in brackets.
            frame.render_widget(
                Paragraph::new(format!("[{label}]")).style(Style::default().fg(fill)),
                rect,
            );
        } else {
            frame.render_widget(
                Paragraph::new(chip).style(Style::default().fg(Color::Black).bg(fill)),
                rect,
            );
            if slim_round() {
                pill_caps(frame, x, x + w - 1, y, fill);
            }
        }
        hits.push(Hit {
            area: rect,
            action: *action,
        });
        x += w + 2;
    }
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
    // The playground's Review and List have no typing bar: they reach to the bottom.
    let play = state.play.view && !state.pair_view;
    let typing_bar = !play || state.play.page == Page::Record;
    // The rain stays between the top bar and the typing bar (the pairing view has none: a
    // QR code wants a quiet background).
    if !state.pair_view {
        draw_rain(
            frame,
            Rect::new(
                screen.x,
                area.y + 5,
                screen.width,
                if typing_bar {
                    // A clear row above the bar's line: some terminals let a letter's glyph
                    // spill into the row below it and nick the line.
                    bar_y.saturating_sub(area.y + 8)
                } else {
                    area.bottom().saturating_sub(area.y + 5)
                },
            ),
        );
    }
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
    if let Some(label) = &state.model_switch {
        tools.insert(
            0,
            (
                label.as_str(),
                Click::SwitchModel,
                !state.upgrading && !state.busy && !active,
            ),
        );
    }
    if chat_offered(state) {
        tools.insert(0, ("mode", Click::Mode, !state.busy && !active));
    }
    let mut x = area.right();
    for (icon, color, action, enabled) in [
        (("close", "✕"), THEME_RECORD, Click::Key(KeyCode::Esc), true),
        // Something to get (a new version, a model, the font): the refresh button becomes
        // the way to get it, and the bar says what a click will do.
        match offer(state) {
            Some((action, _)) => (("download", "↓"), THEME_ACCENT, action, !active),
            None => (
                ("refresh", "↻"),
                THEME_ACCENT,
                Click::Key(KeyCode::Char('r')),
                !state.busy && !active,
            ),
        },
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
    // The engine button is the first to go when the bar is crowded: the
    // status (room is kept for it, so the button does not come and go with every recording)
    // and an alert all come before it.
    // A chip's columns and the gap after it.
    let tool_columns = |(label, action, _): &Control| {
        if matches!(action, Click::SwitchModel | Click::Mode) {
            side_w + 1
        } else {
            label.chars().count() as u16 + 3
        }
    };
    let chips: u16 = tools.iter().map(tool_columns).sum();
    let others = chips
        + 16
        // On the phone look an alert has its own row under the bar.
        + if state.notice.is_empty() || outline() { 0 } else { 9 };
    if let Some((gpu, available)) = compute(state)
        && x >= logo_end + side_w + 1 + others
    {
        // A bolt for the graphics card, a chip for the processor; a click switches.
        x -= side_w;
        square(
            frame,
            &mut hits,
            Rect::new(x, area.y, side_w, 3),
            if gpu {
                ("gpu", &["▦"])
            } else {
                ("cpu", &["▣"])
            },
            if gpu { THEME_ACCENT } else { Color::Gray },
            Click::Compute,
            available && state.compute_switch.is_none() && !state.busy && !active,
        );
        x -= 1;
    }
    // Contribute: in the accent color while the playground (or its explanation) shows.
    let lit = play || state.play.consent;
    if x >= logo_end + side_w && (lit || x >= logo_end + side_w + 1 + chips + 16) {
        // The buttons to its left come first on a narrow screen, but never the way back to the chat.
        x -= side_w;
        square(
            frame,
            &mut hits,
            Rect::new(x, area.y, side_w, 3),
            // A full heart while sharing is on, an open one otherwise; the way to stop
            // sharing is inside Contribute.
            if state.sharing {
                ("heart", &["♥"])
            } else {
                ("heart-open", &["♡"])
            },
            if lit { THEME_ACCENT } else { Color::Gray },
            Click::Play(PlayClick::Toggle),
            !active && !state.play.thinking,
        );
        x = x.saturating_sub(1);
    }
    for (label, action, enabled) in tools.iter().rev() {
        if matches!(action, Click::SwitchModel | Click::Mode) {
            // The model in use, and chat or transcribe, as buttons like the ones beside
            // them; a click moves to the next one and the bar says its name.
            if x < logo_end + side_w {
                break;
            }
            x -= side_w;
            square(
                frame,
                &mut hits,
                Rect::new(x, area.y, side_w, 3),
                if *action == Click::Mode {
                    mode_icon(state.chat)
                } else {
                    model_icon(label)
                },
                THEME_ACCENT,
                *action,
                *enabled,
            );
            x = x.saturating_sub(1);
            continue;
        }
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
        if state.chat {
            "◌ Thinking"
        } else {
            "◌ Transcribing"
        }
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
    // With nothing else to say, the bar says what the download button is offering.
    let hint = offer(state)
        .filter(|_| state.notice.is_empty() && !active && !state.busy)
        .map(|(_, words)| words.to_owned());
    // A model on its way: a bar, filling as it comes, in place of any other words.
    let hint = state
        .progress
        .as_ref()
        .map(|(label, done, total)| progress_bar(label, *done, *total))
        .or(hint);
    if !state.notice.is_empty() || hint.is_some() {
        let left = logo_end;
        let right = x.saturating_sub(status_w + 2);
        let notice = match &hint {
            Some(words) => words.clone(),
            None => clean(&state.notice).replace('\n', " "),
        };
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
        } else if hint.is_none() {
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
    if let Some(card) = state.share_card {
        let top = area.y + 4;
        playground::draw_share(
            frame,
            card,
            Rect::new(
                area.x,
                top,
                area.width,
                screen.height.saturating_sub(top + 1),
            ),
            &mut hits,
        );
        return hits;
    }
    if state.chat_card {
        let top = area.y + 4;
        playground::draw_chat_card(
            frame,
            state.chat_size.unwrap_or(0),
            Rect::new(
                area.x,
                top,
                area.width,
                screen.height.saturating_sub(top + 1),
            ),
            &mut hits,
        );
        return hits;
    }
    if let Some(model) = state
        .model_card
        .and_then(|i| state.catalog.get(i).map(|m| (i, m)))
    {
        let top = area.y + 4;
        let first = !state.catalog.iter().any(|m| m.installed);
        playground::draw_model_card(
            frame,
            model,
            first,
            Rect::new(
                area.x,
                top,
                area.width,
                screen.height.saturating_sub(top + 1),
            ),
            &mut hits,
        );
        return hits;
    }
    if state.play.consent && !state.pair_view {
        let top = area.y + 4;
        playground::draw_consent(
            frame,
            Rect::new(
                area.x,
                top,
                area.width,
                screen.height.saturating_sub(top + 1),
            ),
            &mut hits,
        );
        return hits;
    }
    if state.pair_view {
        // Everything under the top bar: a QR code needs the rows the typing bar would take.
        let top = area.y + 4;
        draw_pair(
            frame,
            state,
            Rect::new(
                area.x,
                top,
                area.width,
                screen.height.saturating_sub(top + 1),
            ),
            &mut hits,
        );
        return hits;
    }
    if !typing_bar {
        let top = area.y + 5;
        playground::draw(
            frame,
            state,
            Rect::new(area.x, top, area.width, area.bottom().saturating_sub(top)),
            &mut hits,
        );
        return hits;
    }
    // Bottom: one slim row, edge to edge: plus, the typing line, mic/send. The playground's
    // Record screen has the line and the record button only (nothing is typed or attached).
    let typed = !play && !state.input.trim().is_empty();
    let busy = if play {
        state.play.thinking
    } else {
        state.busy
    };
    if !play {
        square(
            frame,
            &mut hits,
            Rect::new(area.x, bar_y - 1, side_w, 3),
            ("plus", &["+"]),
            THEME_ACCENT,
            Click::Attach,
            !active && !state.busy,
        );
    } else if phone_button() {
        // Pair a phone to record and review there: in the accent color while one is paired.
        square(
            frame,
            &mut hits,
            Rect::new(area.x, bar_y - 1, side_w, 3),
            ("phone", &["▯"]),
            if paired(state) {
                THEME_ACCENT
            } else {
                Color::Gray
            },
            Click::Pair,
            !active && !busy,
        );
    }
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
        (("mic", &["●"]), THEME_RECORD, Click::Record, !busy)
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
    let box_area = if play && !phone_button() {
        Rect::new(area.x, bar_y, area.width.saturating_sub(side_w + 1), 1)
    } else {
        Rect::new(
            area.x + side_w + 1,
            bar_y,
            area.width.saturating_sub(2 * side_w + 2),
            1,
        )
    };
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
            .chain(recent.iter().map(|l| {
                // The phone look: dots like the voice note's wave (blocks leave gaps there).
                if outline() {
                    ['⠂', '⠆', '⠇', '⡇'][bar(*l) as usize / 2]
                } else {
                    LEVELS[bar(*l) as usize]
                }
            }))
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
    } else if play {
        let hint = match (outline(), nerd_font()) {
            (true, true) => fitting(&[
                " Tap \u{f130}, and say it in your own words",
                " Tap \u{f130} to talk",
                "",
            ]),
            (true, false) => {
                fitting(&[" Tap ●, and say it in your own words", " Tap ● to talk", ""])
            }
            _ => fitting(&[
                " Hold Space, and say it in your own words",
                " Hold Space to talk",
                "",
            ]),
        };
        Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray)))
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
    if play {
        let top = area.y + 5;
        playground::draw(
            frame,
            state,
            Rect::new(
                area.x,
                top,
                area.width,
                bar_y.saturating_sub(2).saturating_sub(top),
            ),
            &mut hits,
        );
        return hits;
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
                  colors: (Color, Color),
                  right: bool| {
        let h = lines.len() as u16;
        let w = (inner + 4).min(area.width);
        let x = if right { area.right() - w } else { area.x };
        let (text, style, outer) = plate(frame, Rect::new(x, y, w, h), colors);
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Right)
                .style(style)
                .block(Block::default().padding(ratatui::widgets::Padding::horizontal(1))),
            text,
        );
        outer
    };
    // Round bubbles sit one row apart; the half-block kind needs its padding rows too.
    let gap = if round.is_some() { 1 } else { 2 };
    if round.is_some() {
        y += 1;
    }
    if state.busy {
        y = y.saturating_sub(1);
        bubble(
            frame,
            y,
            vec![Line::from(thinking())],
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
            if text.is_empty() {
                lines.clear();
            }
            lines.push(Line::from(voice_spans(state, index, voice)));
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
/// The shape behind a bubble or card of `rect.height` text rows: truly round ends where the
/// terminal draws pictures, a round outline in the phone look, a pill with a Nerd Font for one
/// row, half blocks otherwise. All but the first need half a row above and below. Whatever
/// the background drew there is wiped first, so no letters show through. Returns where the
/// text goes (one column in from each end), the style to write it in, and all it covers.
fn plate(frame: &mut Frame, rect: Rect, colors: (Color, Color)) -> (Rect, Style, Rect) {
    plate_ends(frame, rect, colors, 1)
}
/// `plate` whose round ends are `ends` columns wide where the terminal draws pictures: two
/// columns give a tall card or button corners as round as the typing bar's. The text then
/// starts that many columns in.
fn plate_ends(
    frame: &mut Frame,
    rect: Rect,
    (fg, bg): (Color, Color),
    ends: u16,
) -> (Rect, Style, Rect) {
    let Rect {
        x,
        y,
        width: w,
        height: h,
    } = rect;
    let text = Rect::new(x + 1, y, w.saturating_sub(2), h);
    if let Some(dark) = round_theme() {
        let ends = ends.clamp(1, (w / 2).max(1));
        let text = Rect::new(x + ends, y, w.saturating_sub(2 * ends), h);
        frame.render_widget(ratatui::widgets::Clear, rect);
        let rgb = match bg {
            THEME_ACCENT => theme_rgb().0,
            THEME_RECORD => theme_rgb().1,
            _ => dark,
        };
        let fill = Color::Rgb(rgb[0], rgb[1], rgb[2]);
        let style = Style::default().fg(fg).bg(fill);
        frame.render_widget(Block::default().style(style), text);
        // Each end is one picture the height of the bubble, round at top and bottom.
        for left in [true, false] {
            let column = if left { x } else { x + w - ends };
            let drawn = pixel_cell(
                frame,
                Rect::new(column, y, ends, h),
                &format!("bubble-end-{left}-{rgb:?}"),
                |cw, ch| crate::art::cap(cw, ch, rgb, left, true, true),
            );
            if !drawn {
                let column = if left { x } else { x + w - 1 };
                if ends > 1 {
                    let inner = if left { x + 1 } else { x + w - ends };
                    frame.render_widget(
                        Block::default().style(style),
                        Rect::new(inner, y, ends - 1, h),
                    );
                }
                for row in 0..h {
                    let kind = Corner::Side {
                        top: row == 0,
                        bottom: row + 1 == h,
                    };
                    corner(frame, Rect::new(column, y + row, 1, 1), rgb, left, kind);
                }
            }
        }
        return (text, style, rect);
    }
    let outer = Rect::new(x, y - 1, w, h + 2);
    frame.render_widget(ratatui::widgets::Clear, outer);
    if outline() {
        // The phone look: a round outline, in the accent color for what you sent (and for a
        // reply that was just copied).
        let edge = if bg == THEME_ACCENT || fg == THEME_ACCENT {
            THEME_ACCENT
        } else {
            Color::DarkGray
        };
        outline_box(frame, outer, edge);
        return (text, Style::default().fg(Color::Reset), outer);
    }
    let style = Style::default().fg(fg).bg(bg);
    frame.render_widget(Block::default().style(style), text);
    if h == 1 && nerd_font() {
        // One line: a true pill.
        pill_caps(frame, x, x + w - 1, y, bg);
        return (text, style, outer);
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
    let edge = w.saturating_sub(2) as usize;
    frame.render_widget(
        Paragraph::new("▄".repeat(edge)).style(cap),
        Rect::new(x + 1, y - 1, w.saturating_sub(2), 1),
    );
    frame.render_widget(
        Paragraph::new("▀".repeat(edge)).style(cap),
        Rect::new(x + 1, y + h, w.saturating_sub(2), 1),
    );
    (text, style, outer)
}
/// Thinking: a little sound wave, rolling, with a bright crest sweeping through it.
fn thinking() -> Vec<Span<'static>> {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let t = started().elapsed().as_secs_f32();
    let (accent, _) = theme_rgb();
    (0..14)
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
        .collect()
}
/// A voice note in characters, for terminals without pictures: play/pause (3 columns), the
/// wave (`PEAKS` columns, a slider), and the time.
fn voice_spans(state: &Screen, index: usize, voice: &Voice) -> Vec<Span<'static>> {
    let play = state.voice_play.as_ref().filter(|p| p.index == index);
    let position = play.map_or(0.0, |p| p.position());
    let done = if voice.seconds > 0.0 {
        ((position / voice.seconds) * PEAKS as f32) as usize
    } else {
        0
    };
    let accent = theme_rgb().0;
    let rest = Style::default().fg(match round_theme() {
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
    spans
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
        let _ = tx.send(Update::Done(Ok(json!({ "text": line }))));
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
    if Path::new(&path).is_file() {
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
    match clipboard_text() {
        Some(text) if !text.trim().is_empty() => attach_text(state, &text),
        _ => state.notice = "Nothing to paste.".into(),
    }
}
/// What is on the clipboard: a copied file (its address) or text.
fn clipboard_text() -> Option<String> {
    let run = |args: &[&str]| {
        std::process::Command::new("wl-paste")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && run(&["--version"]).is_some() {
        if run(&["--list-types"]).is_some_and(|t| t.contains("text/uri-list")) {
            run(&["-n", "-t", "text/uri-list"])
        } else {
            run(&["-n"])
        }
    } else {
        clipboard_get()
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
    pub(super) fn rendered_text(
        terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
    ) -> String {
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
    fn refresh_keeps_the_chat_in_a_private_handoff() {
        let mut state = Screen::default();
        state.input = "א גוטן".into();
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
        assert_eq!(restored.input, state.input);
        assert_eq!(restored.plain_log, state.plain_log);
        assert_eq!(restored.voices[&0].seconds, 3.0);
        assert_eq!(restored.voices[&0].peaks, vec![1, 2, 3]);
    }
    /// Refresh hands over from the version that is running to the one just installed: a
    /// workspace written by an older version (tabs, drafts, a phone pairing) still opens, and
    /// only the chat is taken from it.
    #[test]
    fn a_workspace_from_an_older_version_still_opens() {
        let old = json!({
            "drafts": ["", "", "", "", "", "", ""], "review_cursor": 3, "correction": null,
            "phone_audio": true, "tab": 6, "input": "", "outputs": ["", "x", "", "", "", "", ""],
            "history": [{"role": "user", "content": "א גוטן"}],
            "pair": {"id": "old-pair", "join_url": "https://192.168.1.20:8796/#code"},
            "phone_status": {"connected": true}, "phone_transcript_revision": 4,
            "prompt": "", "scroll": 2, "comparison": null, "audio_path": "/tmp/a.wav",
            "read_line": "א גוטן", "read_pending": null, "read_result": null,
            "read_history": ["א גוטן"],
            "plain_log": [[true, ""], [false, "א גוטן טאג"]],
            "voices": [[0, "/tmp/take.wav", 3.0, [1, 2, 3]]]
        });
        let resume: Resume = serde_json::from_value(old).unwrap();
        let mut restored = Screen::default();
        resume.apply(&mut restored);
        assert_eq!(restored.plain_log.len(), 2);
        assert_eq!(restored.voices[&0].peaks, vec![1, 2, 3]);
        // The oldest kind, from before the chat was carried over.
        let resume: Resume = serde_json::from_value(json!({"tab": 1, "input": "x"})).unwrap();
        assert!(resume.plain_log.is_empty());
    }
    #[test]
    fn scores_fold_heimish_spelling_choices() {
        assert_eq!(
            errors("כ'בין זעהענדיג יאהרן", "איך בין זעענדיג יארן").unwrap(),
            (0.0, 0.0)
        );
        assert!(errors("מקפיד", "מאכטערט").unwrap().1 > 0.5);
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
    fn plain_app_has_attach_record_send_and_copy() {
        for (width, height) in [(40, 16), (80, 24), (160, 50)] {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut state = Screen::default();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, &state)).unwrap();
            for action in [Click::Attach, Click::Record, Click::Key(KeyCode::Esc)] {
                assert!(
                    hits.iter().any(|h| h.action == action),
                    "{action:?} at {width}"
                );
            }
            state.input = "shiur.mp3".into();
            state.plain_log = vec![(false, "א גוטן טאג".into())];
            term.draw(|f| hits = draw_plain(f, &state)).unwrap();
            assert!(hits.iter().any(|h| h.action == Click::Key(KeyCode::Enter)));
            assert!(hits.iter().any(|h| h.action == Click::CopyAt(0)));
            for (i, a) in hits.iter().enumerate() {
                for b in &hits[i + 1..] {
                    assert!(a.area.intersection(b.area).is_empty(), "{width}: overlap");
                }
            }
        }
    }
    #[test]
    fn model_chip_shows_only_when_there_is_a_model_to_switch_to() {
        let show = |state: &Screen| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, state)).unwrap();
            for (i, a) in hits.iter().enumerate() {
                for b in &hits[i + 1..] {
                    assert!(a.area.intersection(b.area).is_empty(), "overlap");
                }
            }
            let clickable = hits.iter().any(|h| h.action == Click::SwitchModel);
            (rendered_text(&term), clickable)
        };
        let mut state = Screen::default();
        let (text, clickable) = show(&state);
        assert!(!clickable && !text.contains("Ω"));
        state.model_switch = Some("Omnilingual".into());
        let (text, clickable) = show(&state);
        assert!(clickable && text.contains("Ω"));
    }
    /// The model, engine and Contribute chips are buttons of the same shape as close and
    /// refresh, in every look: three rows from the bar's top, the same plate or outline
    /// above and below the text, and clickable over all of it. On a narrow pane the model's
    /// chip stays, with close and refresh.
    /// Chat and Transcribe each keep their own messages and draft, and the Chat button is
    /// there only where the service has a chat model set up.
    #[test]
    fn chat_and_transcribe_keep_their_own_messages() {
        let mut state = Screen::default();
        assert!(!chat_offered(&state));
        state.status = Some(json!({"components": {"schmooze": {"status": "placeholder"}}}));
        assert!(!chat_offered(&state));
        state.status =
            Some(json!({"components": {"schmooze": {"status": "configured-not-probed"}}}));
        assert!(chat_offered(&state));
        state.plain_log.push((false, "written down".into()));
        state.input = "a draft".into();
        switch_mode(&mut state);
        assert!(state.chat && state.plain_log.is_empty() && state.input.is_empty());
        state.plain_log.push((true, "a question".into()));
        // Refresh carries both sides and the side that was showing.
        let kept = Resume::capture(&state);
        let mut again = Screen::default();
        serde_json::from_slice::<Resume>(&serde_json::to_vec(&kept).unwrap())
            .unwrap()
            .apply(&mut again);
        assert!(again.chat);
        assert_eq!(again.plain_log, vec![(true, "a question".to_owned())]);
        switch_mode(&mut again);
        assert!(!again.chat);
        assert_eq!(again.plain_log, vec![(false, "written down".to_owned())]);
        assert_eq!(again.input, "a draft");
    }
    #[test]
    fn chat_is_offered_for_download_and_asks_first() {
        let mut state = Screen::default();
        // A fresh install: the service says chat runs here but is not downloaded.
        state.status = Some(json!({"components": {"schmooze": {"status": "not-installed"}}}));
        assert!(chat_offered(&state) && chat_missing(&state));
        let draw = |state: &Screen| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, state)).unwrap();
            (rendered_text(&term), hits)
        };
        // The chat button is in the top bar.
        assert!(draw(&state).1.iter().any(|h| h.action == Click::Mode));
        state.chat_size = Some(6_398_000_000);
        state.chat_card = true;
        let (text, hits) = draw(&state);
        assert!(text.contains("Download chat?"), "{text}");
        assert!(text.contains("about 6.4 GB"), "{text}");
        assert!(hits.iter().any(|h| h.action == Click::GetChat));
        assert!(hits.iter().any(|h| h.action == Click::ModelNotNow));
        // Once downloaded, the same button switches to chat.
        state.chat_card = false;
        state.status = Some(json!({"components": {"schmooze": {"status": "local"}}}));
        assert!(chat_offered(&state) && !chat_missing(&state));
    }
    #[test]
    fn a_model_is_downloaded_only_after_saying_yes() {
        let listing = |version: &str, label: &str, size: u64, installed| crate::model::Listing {
            version: version.into(),
            label: label.into(),
            size,
            installed,
        };
        let draw = |state: &Screen| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, state)).unwrap();
            (rendered_text(&term), hits)
        };
        // First open, nothing here: the question names the model and its size.
        let mut state = Screen::default();
        state.catalog = vec![
            listing("w", "Whisper", 575_000_000, false),
            listing("o", "Omnilingual", 1_032_000_000, false),
        ];
        state.model_card = Some(0);
        let (text, hits) = draw(&state);
        assert!(text.contains("Download the speech model?"), "{text}");
        assert!(text.contains("about 575 MB"), "{text}");
        assert!(hits.iter().any(|h| h.action == Click::GetModel(0)));
        assert!(hits.iter().any(|h| h.action == Click::ModelNotNow));
        // The other model, asked for with the model button.
        state.catalog[0].installed = true;
        state.model_card = Some(1);
        let (text, hits) = draw(&state);
        assert!(
            text.contains("Download our Omnilingual-based model?"),
            "{text}"
        );
        assert!(text.contains("about 1032 MB"), "{text}");
        assert!(hits.iter().any(|h| h.action == Click::GetModel(1)));
        // While it downloads, the bar fills in the middle of the top bar.
        state.model_card = None;
        state.progress = Some((
            "our Omnilingual-based model".into(),
            516_000_000,
            1_032_000_000,
        ));
        let (text, _) = draw(&state);
        assert!(
            text.contains("Downloading our Omnilingual-based model"),
            "{text}"
        );
        assert!(text.contains("50%"), "{text}");
        assert_eq!(
            progress_bar("chat", 1, 4),
            "Downloading chat ▕████░░░░░░░░░░░░▏ 25%"
        );
    }
    #[test]
    fn the_refresh_button_offers_what_there_is_to_get() {
        let mut state = Screen::default();
        assert!(offer(&state).is_none());
        state.font_offer = true;
        assert_eq!(offer(&state).unwrap().0, Click::GetFont);
        state.model_update = Some("2027.1.1".into());
        assert_eq!(offer(&state).unwrap().0, Click::UpdateModel);
        state.app_update = Some("9.9.9".into());
        assert_eq!(offer(&state).unwrap().0, Click::UpdateApp);
        state.model_update = Some("first".into());
        assert_eq!(offer(&state).unwrap().0, Click::UpdateModel);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
        let mut hits = vec![];
        term.draw(|f| hits = draw_plain(f, &state)).unwrap();
        assert!(rendered_text(&term).contains("Click the arrow to get the model."));
        assert!(hits.iter().any(|h| h.action == Click::UpdateModel));
        // While it downloads, and once a new version waits to be started, it is Refresh again.
        state.upgrading = true;
        assert!(offer(&state).is_none());
        state.upgrading = false;
        state.restart_due = true;
        assert!(offer(&state).is_none());
    }
    #[test]
    fn top_bar_chips_are_the_shape_of_the_buttons() {
        let buttons = [
            Click::Key(KeyCode::Esc),
            Click::Key(KeyCode::Char('r')),
            Click::Compute,
            Click::Play(PlayClick::Toggle),
            Click::SwitchModel,
            Click::Mode,
        ];
        for (look, nerd) in [(false, false), (false, true), (true, false)] {
            PHONE_LOOK.with(|o| o.set(look));
            NERD_FOR_TEST.with(|n| n.set(Some(nerd)));
            for (width, crowded) in [(100, false), (160, false), (100, true)] {
                let mut state = Screen::default();
                state.status = Some(json!({
                    "compute": {"mode": "gpu", "gpu_available": true},
                    "components": {"schmooze": {"status": "configured-not-probed"}},
                }));
                state.model_switch = Some("2026.11.11".into());
                if crowded {
                    state.sharing = true;
                    state.app_update = Some("9.9.9".into());
                    state.model_update = Some("first".into());
                    state.font_offer = true;
                    state.notice = "Could not hear anything.".into();
                }
                let mut term =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
                let mut hits = vec![];
                term.draw(|f| hits = draw_plain(f, &state)).unwrap();
                let at = format!("look {look}, nerd {nerd}, {width} columns, crowded {crowded}");
                for (i, a) in hits.iter().enumerate() {
                    for b in &hits[i + 1..] {
                        assert!(a.area.intersection(b.area).is_empty(), "{at}: overlap");
                    }
                }
                let buffer = term.backend().buffer();
                // Each button's top and bottom rows: the left corner, the middle and the
                // right corner, with their colors.
                let edges = |area: Rect| {
                    [area.y, area.bottom() - 1].map(|y| {
                        [area.x, area.x + 1, area.right() - 1].map(|x| {
                            let cell = &buffer[(x, y)];
                            (cell.symbol().to_owned(), cell.fg, cell.bg)
                        })
                    })
                };
                let close = hits
                    .iter()
                    .find(|h| h.action == buttons[0])
                    .unwrap_or_else(|| panic!("{at}: close"))
                    .area;
                assert_eq!(close.height, 3, "{at}");
                for action in buttons {
                    // With something to get, the refresh button is the way to get it.
                    let action = if crowded && action == Click::Key(KeyCode::Char('r')) {
                        Click::UpdateModel
                    } else {
                        action
                    };
                    let Some(hit) = hits.iter().find(|h| h.action == action) else {
                        // Crowded: the engine and Contribute chips give way first.
                        assert!(
                            crowded && matches!(action, Click::Compute | Click::Play(_)),
                            "{at}: {action:?} is missing"
                        );
                        continue;
                    };
                    assert_eq!(
                        (hit.area.y, hit.area.width, hit.area.height),
                        (close.y, close.width, close.height),
                        "{at}: {action:?}"
                    );
                    assert_eq!(edges(hit.area), edges(close), "{at}: {action:?}");
                    // The text row's two end cells: a pill's caps, the plate's fill or the
                    // outline's sides (the text between them has its own color).
                    let ends = |area: Rect| {
                        [area.x, area.right() - 1].map(|x| {
                            let cell = &buffer[(x, area.y + 1)];
                            (cell.symbol().to_owned(), cell.bg)
                        })
                    };
                    assert_eq!(ends(hit.area), ends(close), "{at}: {action:?}");
                }
                assert!(rendered_text(&term).contains("≡"), "{at}");
            }
        }
        NERD_FOR_TEST.with(|n| n.set(None));
        PHONE_LOOK.with(|o| o.set(false));
        // Where the terminal draws pictures: every one of them three rows from the bar's top.
        let mut picker = ratatui_image::picker::Picker::halfblocks();
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        GRAPHICS.with(|g| {
            *g.borrow_mut() = Some(Graphics {
                picker,
                cache: std::collections::HashMap::new(),
                dark: [29, 39, 52],
                exact: true,
            })
        });
        for width in [100, 160] {
            let mut state = Screen::default();
            state.status = Some(json!({
                "compute": {"mode": "gpu", "gpu_available": true},
                "components": {"schmooze": {"status": "configured-not-probed"}},
            }));
            state.model_switch = Some("Omnilingual".into());
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, &state)).unwrap();
            for (i, a) in hits.iter().enumerate() {
                for b in &hits[i + 1..] {
                    assert!(a.area.intersection(b.area).is_empty(), "{width}: overlap");
                }
            }
            for action in buttons {
                let hit = hits.iter().find(|h| h.action == action);
                let hit = hit.unwrap_or_else(|| panic!("{width}: {action:?} is missing"));
                assert_eq!((hit.area.y, hit.area.height), (1, 3), "{width}: {action:?}");
            }
        }
        GRAPHICS.with(|g| *g.borrow_mut() = None);
    }
    #[test]
    fn engine_chip_shows_the_engine_and_switches_once() {
        let show = |state: &Screen, width: u16| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, state)).unwrap();
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
        // A service that says nothing about engines: no chip.
        state.status = Some(json!({"name": "3Yiddish"}));
        let (text, clickable) = show(&state, 80);
        assert!(!clickable && !text.contains("▦") && !text.contains("▣"));
        for width in [62, 80, 160] {
            state.status = engine("gpu", true);
            let (text, clickable) = show(&state, width);
            assert!(clickable && text.contains("▦"), "{width}");
            state.status = engine("cpu", true);
            let (text, clickable) = show(&state, width);
            assert!(clickable && text.contains("▣"), "{width}");
            // No graphics card: the chip is there, and is not a button.
            state.status = engine("cpu", false);
            let (text, clickable) = show(&state, width);
            assert!(!clickable && text.contains("▣"), "{width}");
        }
        // Crowded: everything else keeps its place, with or without the chip.
        state.status = engine("gpu", true);
        assert!(!show(&state, 40).0.contains("▦"));
        state.app_update = Some("9.9.9".into());
        state.model_update = Some("first".into());
        state.notice = "Could not hear anything.".into();
        state.busy = true;
        for width in [62, 70, 80, 100] {
            let (text, clickable) = show(&state, width);
            assert!(!clickable, "{width}: no switching while transcribing");
            assert!(text.contains("◌ Transcribing"), "{width}");
        }
        assert!(show(&state, 100).0.contains("Could not hear anything."));
        state.busy = false;
        // A click shows the other engine at once and asks the service once.
        assert_eq!(switch_compute(&mut state), Some(false));
        assert_eq!(compute(&state), Some((false, true)));
        assert_eq!(switch_compute(&mut state), None);
        let (text, clickable) = show(&state, 100);
        assert!(!clickable && text.contains("▣"));
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
                    let plain = Screen::default();
                    let mut first = Screen::default();
                    first.model_update = Some("first".into());
                    first.font_offer = true;
                    let mut talking = Screen::default();
                    talking.demo_rec = Some(std::time::Instant::now());
                    talking.rec_started = Some(std::time::Instant::now());
                    let mut alert = Screen::default();
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
                        term.draw(|f| hits = draw_plain(f, state)).unwrap();
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
                            // The refresh button is the way to get the model.
                            assert!(
                                hits.iter().any(|h| h.action == Click::UpdateModel)
                                    && !hits
                                        .iter()
                                        .any(|h| h.action == Click::Key(KeyCode::Char('r'))),
                                "{at}\n{text}"
                            );
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
    fn paired_state(url: &str, connected: bool) -> Screen {
        let mut state = Screen::default();
        state.playground = true;
        state.pair = Some(json!({
            "playground": true, "active": true, "connected": connected, "takes": 12, "all": 14,
            "goal": 100, "url": url, "transport": "tailscale-https", "secure": true, "hint": "",
        }));
        state
    }
    const PAIR_URL: &str = "https://desk.tail0a1b2c.ts.net:8443/0123456789ABCDEFGHIJ/";
    /// Pair phone lives inside the playground, at the left of its Record bar (where the chat
    /// has Attach): as big as the record button, gray until a phone is paired.
    #[test]
    fn phone_button_sits_in_the_playgrounds_record_bar() {
        let draw = |state: &Screen| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 24)).unwrap();
            let mut hits = vec![];
            term.draw(|f| hits = draw_plain(f, state)).unwrap();
            for (i, a) in hits.iter().enumerate() {
                for b in &hits[i + 1..] {
                    assert!(a.area.intersection(b.area).is_empty(), "overlap");
                }
            }
            (term, hits)
        };
        let spot =
            |hits: &[Hit], action: Click| hits.iter().find(|h| h.action == action).map(|h| h.area);
        // The chat has no phone button, with or without the playground.
        let mut chat = Screen::default();
        assert!(spot(&draw(&chat).1, Click::Pair).is_none());
        chat.playground = true;
        assert!(spot(&draw(&chat).1, Click::Pair).is_none());
        assert!(spot(&draw(&chat).1, Click::Attach).is_some());
        for (nerd, symbol) in [(false, "▯"), (true, "\u{f10b}")] {
            NERD_FOR_TEST.with(|n| n.set(Some(nerd)));
            let mut state = Screen::default();
            state.playground = true;
            state.play.view = true;
            let (term, hits) = draw(&state);
            assert!(
                rendered_text(&term).contains(symbol),
                "symbol for nerd={nerd}"
            );
            let (phone, record) = (
                spot(&hits, Click::Pair).unwrap(),
                spot(&hits, Click::Record).unwrap(),
            );
            assert_eq!(
                (phone.y, phone.height, phone.width),
                (record.y, record.height, record.width)
            );
            assert!(phone.x < record.x && spot(&hits, Click::Attach).is_none());
            let color = |state: &Screen| {
                let (term, _) = draw(state);
                let buffer = term.backend().buffer();
                buffer
                    .content
                    .iter()
                    .find(|c| c.symbol() == symbol)
                    .unwrap()
                    .fg
            };
            assert_eq!(color(&state), Color::Gray);
            let mut paired = paired_state(PAIR_URL, false);
            paired.play.view = true;
            assert_eq!(color(&paired), THEME_ACCENT);
        }
        NERD_FOR_TEST.with(|n| n.set(None));
    }
    #[test]
    fn pair_clicks_open_the_view_pair_once_and_unpair() {
        let (tx, _updates) = mpsc::unbounded_channel();
        let (compute, _c) = mpsc::unbounded_channel();
        let (pair, mut asked) = mpsc::unbounded_channel();
        let (play, _p) = mpsc::unbounded_channel();
        let mut state = Screen::default();
        state.playground = true;
        // First click: the view opens and a pairing is asked for, once.
        assert!(
            click(&mut state, Click::Pair, &tx, &compute, &pair, &play)
                .unwrap()
                .is_none()
        );
        assert!(state.pair_view && state.pair_busy);
        assert_eq!(asked.try_recv().ok(), Some(true));
        // The chip closes the view again; opening it once more does not pair twice.
        click(&mut state, Click::Pair, &tx, &compute, &pair, &play).unwrap();
        assert!(!state.pair_view);
        click(&mut state, Click::Pair, &tx, &compute, &pair, &play).unwrap();
        assert!(state.pair_view && asked.try_recv().is_err());
        // Paired: the chip only shows the view. Back leaves it; Unpair asks and leaves.
        state = paired_state(PAIR_URL, true);
        click(&mut state, Click::Pair, &tx, &compute, &pair, &play).unwrap();
        assert!(state.pair_view && asked.try_recv().is_err());
        click(&mut state, Click::PairBack, &tx, &compute, &pair, &play).unwrap();
        assert!(!state.pair_view);
        state.pair_view = true;
        click(&mut state, Click::Unpair, &tx, &compute, &pair, &play).unwrap();
        assert!(!state.pair_view && state.pair_busy);
        assert_eq!(asked.try_recv().ok(), Some(false));
    }
    /// The pairing view: the QR code is whole or not there at all (a cut code cannot be
    /// scanned), the address is always readable, and Back / Unpair are always clickable.
    #[test]
    fn pairing_view_never_clips_the_qr_code() {
        let code = qr_lines(PAIR_URL).unwrap();
        let (qr_w, qr_h) = (code[0].width(), code.len());
        // Small enough for a plain 80 by 24 window.
        assert!(qr_w <= 33 && qr_h <= 17, "{qr_w} by {qr_h}");
        assert!(code.iter().all(|line| line.width() == qr_w));
        let modules = qr_modules(PAIR_URL).unwrap();
        // What is on the screen, read back as modules: the cells inside the code's outline
        // (its light border is drawn all round), two modules to a cell.
        let read_back = |buffer: &ratatui::buffer::Buffer| -> Option<Vec<Vec<bool>>> {
            let drawn = |c: &ratatui::buffer::Cell| {
                ["█", "▀", "▄"].contains(&c.symbol()) && c.fg == Color::Gray
            };
            let width = buffer.area.width as usize;
            let cells: Vec<(usize, usize)> = buffer
                .content
                .iter()
                .enumerate()
                .filter(|(_, c)| drawn(c))
                .map(|(i, _)| (i % width, i / width))
                .collect();
            let (x0, y0) = (
                cells.iter().map(|c| c.0).min()?,
                cells.iter().map(|c| c.1).min()?,
            );
            let (x1, y1) = (
                cells.iter().map(|c| c.0).max()?,
                cells.iter().map(|c| c.1).max()?,
            );
            let mut rows = Vec::new();
            for y in y0..=y1 {
                let (mut top, mut bottom) = (Vec::new(), Vec::new());
                for x in x0..=x1 {
                    let cell = &buffer.content[y * width + x];
                    // Nothing is filled in behind the code.
                    assert_eq!(cell.bg, Color::Reset);
                    top.push(!["█", "▀"].contains(&cell.symbol()));
                    bottom.push(!["█", "▄"].contains(&cell.symbol()));
                }
                rows.push(top);
                rows.push(bottom);
            }
            Some(rows)
        };
        for look in [false, true] {
            PHONE_LOOK.with(|o| o.set(look));
            for (width, height, shown) in [
                (80u16, 24u16, true),
                (100, 30, true),
                (160, 50, true),
                (60, 40, true),
                (44, 34, true),
                (40, 16, false),
                (50, 20, false),
                (70, 22, false),
                (200, 23, true),
            ] {
                let mut state = paired_state(PAIR_URL, false);
                state.pair_view = true;
                let mut term =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .unwrap();
                let mut hits = vec![];
                term.draw(|f| hits = draw_plain(f, &state)).unwrap();
                let buffer = term.backend().buffer();
                let size = format!("{width} by {height}");
                match read_back(buffer) {
                    // Whole, and module for module the code the address makes.
                    Some(rows) => {
                        assert!(shown, "{size}: a code where none fits");
                        assert_eq!(rows.len(), qr_h * 2, "{size}");
                        assert_eq!(&rows[..modules.len()], &modules[..], "{size}");
                    }
                    None => assert!(!shown, "{size}: no code"),
                }
                let screen = rendered_text(&term);
                let flat: String = screen
                    .replace(['█', '▀', '▄'], " ")
                    .split_whitespace()
                    .collect();
                assert!(flat.contains("0123456789ABCDEFGHIJ"), "{size}: address");
                assert_eq!(flat.contains("Makethewindowbigger"), !shown, "{size}");
                assert!(screen.contains("Waiting for the phone"), "{size}");
                for action in [Click::PairBack, Click::Unpair] {
                    let hit = hits.iter().find(|h| h.action == action);
                    let hit = hit.unwrap_or_else(|| panic!("{size}: {action:?} missing"));
                    assert!(hit.area.right() <= width && hit.area.bottom() <= height);
                }
                // Nothing of the chat's typing bar is left to click.
                assert!(!hits.iter().any(|h| h.action == Click::Record));
                for (i, a) in hits.iter().enumerate() {
                    for b in &hits[i + 1..] {
                        assert!(a.area.intersection(b.area).is_empty(), "{size}: overlap");
                    }
                }
            }
        }
        PHONE_LOOK.with(|o| o.set(false));
        // A plain link says so, and shows what to do about it.
        let mut state = paired_state("http://100.116.154.30:8796/0123456789ABCDEFGHIJ/", true);
        state.pair_view = true;
        if let Some(pair) = state.pair.as_mut() {
            pair["transport"] = json!("tailscale-http");
            pair["hint"] = json!(
                "For a secure link, allow Tailscale once, then pair again:\nsudo tailscale set --operator=me"
            );
        }
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 30)).unwrap();
        term.draw(|f| {
            draw_plain(f, &state);
        })
        .unwrap();
        let screen = rendered_text(&term);
        assert!(screen.contains("sudo tailscale set --operator=me"));
        assert!(screen.contains("Phone connected") && screen.contains("12 of 100 takes done"));
        // Not paired yet (the click is on its way), and Refresh brings the view back.
        let mut state = Screen::default();
        state.playground = true;
        state.pair_view = true;
        state.pair_busy = true;
        term.draw(|f| {
            draw_plain(f, &state);
        })
        .unwrap();
        assert!(rendered_text(&term).contains("Opening a link for the phone"));
        let resume: Resume =
            serde_json::from_slice(&serde_json::to_vec(&Resume::capture(&state)).unwrap()).unwrap();
        let mut restored = Screen::default();
        resume.apply(&mut restored);
        assert!(restored.pair_view);
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
