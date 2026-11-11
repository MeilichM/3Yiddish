//! Terminal-only playground. Models stay behind the same local middleware API.
use anyhow::{Context, Result, bail};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
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
fn next_tab(from: usize, step: usize) -> usize {
    let mut tab = from;
    loop {
        tab = (tab + step) % TABS.len();
        if !PAUSED_TABS.contains(&tab) {
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
}
#[derive(Default)]
struct Screen {
    tab: usize,
    input: String,
    drafts: [String; 7],
    outputs: [String; 7],
    status: Option<Value>,
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
        }
    }
    fn apply(self, s: &mut Screen) {
        s.review_cursor = self.review_cursor;
        s.correction = self.correction;
        s.phone_audio = self.phone_audio;
        s.drafts = self.drafts;
        s.tab = self.tab.min(TABS.len() - 1);
        if PAUSED_TABS.contains(&s.tab) {
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
/// Stop the PC microphone, save the take as a WAV and send it to be transcribed.
fn finish_recording(state: &mut Screen, actions: &mpsc::UnboundedSender<Action>) -> Result<()> {
    let Some(recorder) = state.recording.take() else {
        return Ok(());
    };
    let path = state.recording_path.take().context("No recording file")?;
    let saved = recorder.stop().and_then(|pcm| {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        std::io::Write::write_all(&mut options.open(&path)?, &crate::asr::wav(&pcm)?)?;
        Ok(())
    });
    if let Err(error) = saved {
        state.notice = format!("Microphone recording failed: {error}");
        state.read_pending = None;
        return Ok(());
    }
    state.input = path.to_string_lossy().into_owned();
    state.audio_path = Some(path);
    submit_reading(state, actions)
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
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
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
    DeleteTemp,
    ReadList,
    Pick(usize),
    PgToggle,
    PgPlay,
    PgStep(bool),
    PgChoose(&'static str),
    PlayPick,
    Toss,
    RedoPick,
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
        .filter(|(i, _)| !PAUSED_TABS.contains(i))
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
        Some(state.tab),
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
fn terminal(
    mut updates: mpsc::UnboundedReceiver<Update>,
    actions: mpsc::UnboundedSender<Action>,
    mic_actions: mpsc::UnboundedSender<String>,
    game_actions: mpsc::UnboundedSender<(String, Value)>,
    watches: Vec<(String, PathBuf)>,
    eval_tx: mpsc::UnboundedSender<Update>,
    config: PathBuf,
) -> Result<Exit> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("The TUI needs an interactive terminal; use CLI commands in scripts");
    }
    let mut terminal = ratatui::init();
    let _restore = Restore;
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    // Background check for a newer app and model; never blocks the screen, silent offline.
    let tx = eval_tx.clone();
    std::thread::spawn(move || {
        let app = crate::update::latest()
            .ok()
            .filter(|v| crate::update::newer(env!("CARGO_PKG_VERSION"), v));
        let have = crate::model::current(&crate::paths::models()).map(|(v, ..)| v);
        let model = crate::model::latest_version()
            .ok()
            .filter(|v| have.as_ref() != Some(v))
            .map(|v| {
                if have.is_none() {
                    "first".to_owned()
                } else {
                    v
                }
            });
        let _ = tx.send(Update::Available(app, model));
    });
    let mut state = Screen::default();
    if PAUSED_TABS.contains(&state.tab) {
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
    state.lab = Path::new(READ_POOL).is_file();
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
                Update::Available(app, model) => {
                    state.app_update = app;
                    state.model_update = model;
                }
                Update::Upgrade(text, finished) => {
                    state.notice = text;
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
                            state.notice = format!("Request failed: {error}");
                            if tab != 0 {
                                state.outputs[tab] = state.notice.clone();
                            }
                            state.comparison = None;
                        }
                    }
                }
            }
        }
        // The recording hit its time limit by itself.
        if state.recording.as_ref().is_some_and(|r| r.finished()) {
            finish_recording(&mut state, &actions)?;
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
        let mut hits = Vec::new();
        terminal.draw(|f| {
            hits = draw(f, &state, &watches);
        })?;
        let key = if let Some(code) = state.pending_key.take() {
            KeyEvent::new(code, KeyModifiers::NONE)
        } else {
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) => key,
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
                            Click::Tab(tab) => {
                                select_tab(&mut state, tab);
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
                                        Err(e) => format!("Update failed: {e}"),
                                    };
                                    let _ = tx.send(Update::Upgrade(text, true));
                                });
                                state.app_update = None;
                                continue;
                            }
                            Click::UpdateModel => {
                                state.upgrading = true;
                                let tx = eval_tx.clone();
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
                                        Err(e) => format!("Model download failed: {e}"),
                                    };
                                    let _ = tx.send(Update::Upgrade(text, true));
                                });
                                state.model_update = None;
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
                    finish_recording(&mut state, &actions)?;
                } else {
                    let folder = Path::new("state/recordings");
                    std::fs::create_dir_all(folder)?;
                    let path = folder
                        .canonicalize()?
                        .join(format!("{}.wav", uuid::Uuid::new_v4()));
                    match crate::audio::Recorder::start() {
                        Ok(recorder) => {
                            state.recording = Some(recorder);
                            state.recording_path = Some(path);
                            state.read_pending = state.read_line.clone();
                        }
                        Err(error) => state.notice = format!("{error}"),
                    }
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
        for width in [32, 40, 60, 80, 120] {
            for tab in 0..TABS.len() {
                let mut state = Screen::default();
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
                for tab in (0..TABS.len()).filter(|t| !PAUSED_TABS.contains(t)) {
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
        let mut state = Screen::default();
        state.tab = 6;
        state.input = "א גוטן".into();
        state.pair = Some(json!({"id":"fixture-pair","join_url":"https://192.168.1.20/#fixture"}));
        state.history = vec![json!({"role":"user","content":"א גוטן"})];
        state.phone_audio = true;
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
    fn read_aloud_button_and_score_card() {
        let mut state = Screen::default();
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
}
