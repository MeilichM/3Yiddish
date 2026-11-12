//! The playground in the app: the phone page's Record, Review and List screens, drawn in the
//! terminal and recorded with this computer's microphone. It is a second view of the same
//! takes: everything goes through the service's playground requests (the phone page's own
//! handlers), so the takes keep one writer, and the idea on the Record screen is the one the
//! phone shows. Takes are a held-out exam: they go to the local service and nowhere else.
use super::*;
use crate::freebench::{self, Mark, Take};
use std::time::Instant;

/// The reasons offered for a mark, with their labels (the phone page's list, in its order).
pub(super) const REASONS: [(&str, &str); 8] = [
    ("wrong", "Wrong word"),
    ("weird", "Weird"),
    ("fluency", "Not fluent / my fault"),
    ("accent", "Wrong accent"),
    ("loshn-koydesh", "Loshn koydesh"),
    ("name", "Name"),
    ("english", "English word"),
    ("custom", "Custom"),
];
/// Each reason's tint: the terminal theme's own colors, as near as they come to the page's.
fn tint(reason: &str) -> Color {
    match reason {
        "wrong" => Color::Red,
        "weird" => Color::Magenta,
        "fluency" => Color::Yellow,
        "accent" => Color::Cyan,
        "loshn-koydesh" => Color::Blue,
        "name" => Color::Green,
        "english" => Color::LightRed,
        "dropped" => Color::LightMagenta,
        _ => Color::Gray,
    }
}
fn label(reason: &str) -> &'static str {
    match reason {
        "dropped" => "Dropped word",
        _ => REASONS
            .iter()
            .find(|(key, _)| *key == reason)
            .map_or("Custom", |(_, label)| label),
    }
}
/// A take's status as the page words and colors it.
fn status(status: &str) -> (&'static str, Color) {
    match status {
        "pending" => ("To review", Color::Yellow),
        "correct" => ("Correct", Color::Green),
        "fixed" => ("Fixed", THEME_ACCENT),
        _ => ("Tossed", Color::DarkGray),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Page {
    #[default]
    Record,
    Review,
    List,
}
/// A text field that can have the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    /// What the marked words should be.
    Right,
    /// The speaker's own words for a custom reason.
    Note,
    /// The whole text typed out.
    Typed,
    /// The name takes are sent under.
    Name,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlayClick {
    /// The top bar's button: the playground or the chat (or, the first time, what
    /// contributing means).
    Toggle,
    /// The explanation's answers.
    Accept,
    Decline,
    /// Sharing: the top bar's chip opens its card; the answer (yes or no / stop), or close.
    ShareOpen,
    Share(bool),
    ShareClose,
    /// Send: ask (name, what it means), go, or leave it.
    SendOpen,
    SendNow,
    SendCancel,
    Idea,
    List,
    Record,
    Open(u32),
    /// A word of the draft, by its number in `freebench::words`.
    Word(usize),
    /// A saved mark, by its place in the marks.
    Mark(usize),
    Reason(usize),
    Focus(Field),
    Before,
    After,
    Remove,
    Cancel,
    SaveMark,
    TypeOut,
    Toss,
    Save,
    Redo,
    AskYes,
    AskNo,
    Retry,
    /// Scroll the draft (Review) or the takes (List): up or down.
    Scroll(bool),
}

/// A request for the service, done in the background.
pub(super) enum Job {
    State,
    Upload(PathBuf),
    Review(u32, Value),
    Remove(u32),
    Audio(u32),
    Idea,
    /// Send the reviewed takes under this name.
    Send(String),
}
/// The service's answer to a `Job`.
pub(super) enum Answer {
    State(Result<Value, String>),
    Uploaded(PathBuf, Result<Value, String>),
    Saved(Result<Value, String>),
    Removed(u32, Result<(), String>),
    /// A take's recording, fetched to a private file: (file, seconds, wave).
    Audio(u32, Result<(PathBuf, f32, Vec<u8>), String>),
    Idea(Result<Value, String>),
    Sent(Result<Value, String>),
}

/// The sharing card: the first-run question, or (from the top bar's Sharing chip) what is
/// being shared, with Stop sharing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShareCard {
    Ask,
    On,
}

#[derive(Default)]
pub(super) struct Play {
    /// The playground is showing instead of the chat.
    pub view: bool,
    /// The Contribute explanation is showing (nobody has accepted it here yet).
    pub consent: bool,
    /// These takes can be sent (a contributor's, never the owner's own set); Send is being
    /// asked; the takes are on their way; the name they go under.
    pub can_send: bool,
    pub send: bool,
    pub sending: bool,
    pub name: String,
    pub page: Page,
    pub items: Vec<Take>,
    pub goal: usize,
    pub idea: String,
    /// The service said it keeps demo takes (THREE_YIDDISH_DEMO), not the real set.
    pub demo_service: bool,
    /// The takes have been heard from the service at least once.
    pub loaded: bool,
    /// A state request is on its way, and when the last one was asked.
    pub asking: bool,
    pub asked: Option<Instant>,
    /// The take in Review, with the marks and typed text being made.
    pub take: Option<Take>,
    pub marks: Vec<Mark>,
    pub typed: Option<String>,
    pub from_list: bool,
    /// The words chosen for a mark (`start == end`: a place between two words), and the saved
    /// mark being changed.
    pub sel: Option<(usize, usize)>,
    pub editing: Option<usize>,
    pub reason: &'static str,
    /// The correction was typed in by hand (a new reason no longer replaces it).
    pub touched: bool,
    pub right: String,
    pub note: String,
    pub focus: Option<Field>,
    /// The line under the correction: help, or what is missing (`true`).
    pub help: (String, bool),
    /// The microphone's recording under way is a take.
    pub recording: bool,
    /// A take on its way to the service (its file), and the draft being made.
    pub pending: Option<PathBuf>,
    pub thinking: bool,
    /// A take the service did not keep, to send again; the takes known before it was sent.
    pub retry: Option<PathBuf>,
    pub known: Option<Vec<u32>>,
    pub saving: bool,
    /// "Record take N again?" is asked.
    pub ask: bool,
    /// The recording of the take in Review, fetched to a private file.
    pub audio: Option<(u32, PathBuf)>,
    /// Rows scrolled: the takes in List, the draft in Review.
    pub list_scroll: usize,
    pub words_scroll: usize,
}
impl Drop for Play {
    fn drop(&mut self) {
        // Copies of exam takes are kept nowhere but the free bench.
        for path in [
            self.audio.take().map(|(_, path)| path),
            self.pending.take(),
            self.retry.take(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Play {
    /// How many takes make the set; none for a contributor, who records as many as he likes.
    fn goal(&self) -> Option<usize> {
        (self.goal > 0).then_some(self.goal)
    }
    fn done(&self) -> usize {
        self.items
            .iter()
            .filter(|t| matches!(t.status.as_str(), "correct" | "fixed"))
            .count()
    }
    fn words(&self) -> Vec<&str> {
        self.take
            .as_ref()
            .map_or_else(Vec::new, |t| freebench::words(&t.draft))
    }
    fn heard(&self) -> String {
        let words = self.words();
        self.sel
            .map(|(start, end)| words[start.min(words.len())..end.min(words.len())].join(" "))
            .unwrap_or_default()
    }
    /// The text the take would be saved as.
    fn final_text(&self) -> String {
        match (&self.typed, &self.take) {
            (Some(typed), _) => typed.split_whitespace().collect::<Vec<_>>().join(" "),
            (None, Some(take)) => freebench::apply(&take.draft, &self.marks),
            _ => String::new(),
        }
    }
    fn changed(&self) -> bool {
        !self.marks.is_empty() || self.typed.is_some()
    }
    fn open_review(&mut self, take: Take, from_list: bool, jobs: &mpsc::UnboundedSender<Job>) {
        self.marks = take.marks.clone();
        self.typed = take.typed.then(|| take.final_text.clone());
        self.from_list = from_list;
        self.sel = None;
        self.editing = None;
        self.focus = None;
        self.ask = false;
        self.words_scroll = 0;
        self.page = Page::Review;
        if self.audio.as_ref().is_none_or(|(n, _)| *n != take.n) {
            let _ = jobs.send(Job::Audio(take.n));
        }
        self.take = Some(take);
    }
    /// A word was clicked: open its mark, grow the selection to it, or select it.
    fn tap_word(&mut self, i: usize) {
        if let Some(k) = self.marks.iter().position(|m| m.start <= i && i < m.end) {
            let m = &self.marks[k];
            return self.open_sheet((m.start, m.end), Some(k), false);
        }
        if let (Some((start, end)), None) = (self.sel, self.editing)
            && end > start
        {
            if end - start == 1 && start == i {
                return self.close_sheet();
            }
            let next = (start.min(i), end.max(i + 1));
            if clashes(&self.marks, next.0, next.1, None) {
                self.help = (
                    "That would run into another mark. Mark up to it, or remove it first.".into(),
                    true,
                );
                return;
            }
            return self.open_sheet(next, None, true);
        }
        self.open_sheet((i, i + 1), None, false);
    }
    /// The panel for one mark. `editing` is the saved mark being changed; `keep` keeps what is
    /// already chosen in it (the selection only grew).
    fn open_sheet(&mut self, span: (usize, usize), editing: Option<usize>, keep: bool) {
        self.sel = Some(span);
        self.editing = editing;
        let place = span.0 == span.1;
        let mark = editing.and_then(|k| self.marks.get(k)).cloned();
        if !keep {
            self.reason = match &mark {
                Some(m) => REASONS
                    .iter()
                    .map(|(key, _)| *key)
                    .chain(["dropped"])
                    .find(|key| *key == m.reason)
                    .unwrap_or("wrong"),
                None if place => "dropped",
                None => "wrong",
            };
            self.touched = mark.is_some();
            self.right = match &mark {
                Some(m) => m.right.clone().unwrap_or_default(),
                None if place => String::new(),
                None => self.heard(),
            };
            self.note = mark.map(|m| m.note).unwrap_or_default();
        } else if !self.touched {
            self.right = if self.reason == "weird" {
                String::new()
            } else {
                self.heard()
            };
        }
        self.focus = Some(Field::Right);
        self.help = (self.default_help(), false);
    }
    fn default_help(&self) -> String {
        match self.sel {
            Some((start, end)) if start == end => String::new(),
            _ => match self.reason {
                "weird" => "Leave it empty if nothing was said there.".into(),
                "fluency" => "Optional: leave it empty to only flag the spot.".into(),
                _ => String::new(),
            },
        }
    }
    fn pick_reason(&mut self, reason: &'static str) {
        self.reason = reason;
        if !self.touched {
            self.right = if reason == "weird" {
                String::new()
            } else {
                self.heard()
            };
        }
        self.focus = Some(if reason == "custom" {
            Field::Note
        } else {
            Field::Right
        });
        self.help = (self.default_help(), false);
    }
    fn close_sheet(&mut self) {
        self.sel = None;
        self.editing = None;
        if self.focus != Some(Field::Typed) {
            self.focus = None;
        }
        self.help = (String::new(), false);
    }
    fn save_mark(&mut self) {
        let Some((start, end)) = self.sel else {
            return;
        };
        let place = start == end;
        let heard = self.heard();
        let typed = self.right.split_whitespace().collect::<Vec<_>>().join(" ");
        let note = self.note.trim().to_owned();
        let mut bad = |text: &str| self.help = (text.into(), true);
        let right = if place {
            if typed.is_empty() {
                return bad("Type the missing word.");
            }
            Some(typed)
        } else if self.reason == "fluency" {
            (!typed.is_empty()).then_some(typed)
        } else if self.reason != "weird" {
            if typed.is_empty() {
                return bad("Type what it should be.");
            }
            if self.reason == "wrong" && typed == heard {
                return bad("That is what was written. Change it, or pick another reason.");
            }
            Some(typed)
        } else {
            Some(typed)
        };
        let custom = !place && self.reason == "custom";
        if custom && note.is_empty() {
            return bad("Say the reason in a few words.");
        }
        let mark = Mark {
            start,
            end,
            heard,
            right,
            reason: if place { "dropped" } else { self.reason }.into(),
            note: if custom { note } else { String::new() },
        };
        match self.editing {
            Some(k) if k < self.marks.len() => self.marks[k] = mark,
            _ => self.marks.push(mark),
        }
        self.marks.sort_by_key(|m| (m.start, m.end));
        self.close_sheet();
    }
    /// "Add word before/after": a place between two words, for a word the model dropped.
    fn add_place(&mut self, p: usize) {
        if let Some(k) = self.marks.iter().position(|m| m.start == p && m.end == p) {
            return self.open_sheet((p, p), Some(k), false);
        }
        if clashes(&self.marks, p, p, None) {
            self.help = ("That spot is inside another mark.".into(), true);
            return;
        }
        self.open_sheet((p, p), None, false);
    }
    fn type_out(&mut self) {
        self.close_sheet();
        if self.typed.is_some() {
            self.typed = None;
            self.focus = None;
        } else {
            self.typed = Some(self.final_text());
            self.focus = Some(Field::Typed);
        }
    }
    /// How many reviewed takes have not been sent; none where takes are not sent at all.
    fn unsent(&self) -> usize {
        if !self.can_send {
            return 0;
        }
        self.items
            .iter()
            .filter(|t| t.sent.is_none() && matches!(t.status.as_str(), "correct" | "fixed"))
            .count()
    }
    /// "Send N takes", where there is something to send.
    fn send_button(&self) -> Option<(Line<'static>, Color, Click, bool)> {
        let n = self.unsent();
        (n > 0).then(|| {
            (
                Line::from(if n == 1 {
                    "Send 1 take".to_owned()
                } else {
                    format!("Send {n} takes")
                }),
                THEME_ACCENT,
                Click::Play(PlayClick::SendOpen),
                true,
            )
        })
    }
    /// What the service is told about the take in Review.
    fn review(&self, toss: bool) -> Value {
        json!({"marks": self.marks, "typed": self.typed, "toss": toss})
    }
    fn field(&mut self, field: Field) -> &mut String {
        match field {
            Field::Right => &mut self.right,
            Field::Note => &mut self.note,
            Field::Typed => self.typed.get_or_insert_with(String::new),
            Field::Name => &mut self.name,
        }
    }
    /// Leave Review: back to the list it was opened from, else to Record.
    fn leave(&mut self, state_playback: &mut Option<crate::audio::Playback>) {
        *state_playback = None;
        self.take = None;
        self.sel = None;
        self.editing = None;
        self.focus = None;
        self.ask = false;
        self.page = if self.from_list {
            Page::List
        } else {
            Page::Record
        };
    }
}

/// Would a mark over words `start..end` (`start == end`: a place between two words) run into
/// another mark? `skip` is the mark being changed. The phone page's rule (app.js `clashes`).
pub(super) fn clashes(marks: &[Mark], start: usize, end: usize, skip: Option<usize>) -> bool {
    marks.iter().enumerate().any(|(k, m)| {
        if Some(k) == skip {
            return false;
        }
        let (place, other) = (start == end, m.start == m.end);
        match (place, other) {
            (true, true) => m.start == start,
            (true, false) => m.start < start && start < m.end,
            (false, true) => start < m.start && m.start < end,
            (false, false) => m.start < end && start < m.end,
        }
    })
}

/// The voice note number a take's recording is kept under (well clear of the chat's).
pub(super) fn voice_index(n: u32) -> usize {
    usize::MAX - n as usize
}
/// A new private file for a take on its way, outside the app's folders. Its name carries this
/// program's number, so what a program left behind (a closed window ends it at once) can be
/// told and removed by the next one (`sweep`).
pub(super) fn temp_take() -> PathBuf {
    std::env::temp_dir().join(format!(
        "3yiddish-take-{}-{}.wav",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}
/// Remove takes' temporary files left by programs that are gone; with `own`, this program's
/// too (it is about to end without tidying up: the window was closed).
pub(super) fn sweep(own: bool) {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("3yiddish-take-"))
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if (own && pid == std::process::id()) || !alive(pid) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
use crate::share::alive;
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

// ---- The service ----

pub(super) async fn get_state(client: &reqwest::Client, base: &str) -> Result<Value> {
    json_response(
        client
            .get(format!("{base}/v1/playground/state"))
            .send()
            .await?,
    )
    .await
}
pub(super) async fn upload(client: &reqwest::Client, base: &str, path: &Path) -> Result<Value> {
    let bytes = tokio::fs::read(path)
        .await
        .context("Cannot read the recording")?;
    json_response(
        client
            .post(format!("{base}/v1/playground/takes"))
            .header(reqwest::header::CONTENT_TYPE, "audio/wav")
            .body(bytes)
            .send()
            .await?,
    )
    .await
}
pub(super) async fn send_review(
    client: &reqwest::Client,
    base: &str,
    n: u32,
    review: &Value,
) -> Result<Value> {
    json_response(
        client
            .post(format!("{base}/v1/playground/takes/{n}"))
            .json(review)
            .send()
            .await?,
    )
    .await
}
pub(super) async fn remove(client: &reqwest::Client, base: &str, n: u32) -> Result<()> {
    json_response(
        client
            .delete(format!("{base}/v1/playground/takes/{n}"))
            .send()
            .await?,
    )
    .await
    .map(|_| ())
}
pub(super) async fn audio(client: &reqwest::Client, base: &str, n: u32) -> Result<Vec<u8>> {
    let response = client
        .get(format!("{base}/v1/playground/audio/{n}"))
        .send()
        .await?;
    if !response.status().is_success() {
        return json_response(response).await.map(|_| Vec::new());
    }
    Ok(response.bytes().await?.to_vec())
}
pub(super) async fn another_idea(client: &reqwest::Client, base: &str) -> Result<Value> {
    json_response(
        client
            .post(format!("{base}/v1/playground/idea"))
            .send()
            .await?,
    )
    .await
}
/// Send: the service gives the reviewed takes to the project. Uploading takes a while.
pub(super) async fn send_takes(client: &reqwest::Client, base: &str, name: &str) -> Result<Value> {
    json_response(
        client
            .post(format!("{base}/v1/playground/send"))
            .timeout(Duration::from_secs(660))
            .json(&json!({"name": name}))
            .send()
            .await?,
    )
    .await
}
/// Does the service's jobs one at a time and sends the answers to the screen.
pub(super) async fn worker(
    client: reqwest::Client,
    base: String,
    mut jobs: mpsc::UnboundedReceiver<Job>,
    tx: mpsc::UnboundedSender<Update>,
) {
    let said = |e: anyhow::Error| e.to_string();
    while let Some(job) = jobs.recv().await {
        let answer = match job {
            Job::State => Answer::State(get_state(&client, &base).await.map_err(said)),
            Job::Upload(path) => {
                let result = upload(&client, &base, &path).await.map_err(said);
                Answer::Uploaded(path, result)
            }
            Job::Review(n, review) => {
                Answer::Saved(send_review(&client, &base, n, &review).await.map_err(said))
            }
            Job::Remove(n) => Answer::Removed(n, remove(&client, &base, n).await.map_err(said)),
            Job::Idea => Answer::Idea(another_idea(&client, &base).await.map_err(said)),
            Job::Send(name) => Answer::Sent(send_takes(&client, &base, &name).await.map_err(said)),
            Job::Audio(n) => {
                let fetched = match audio(&client, &base, n).await {
                    Ok(bytes) => tokio::task::spawn_blocking(move || {
                        let path = temp_take();
                        write_private(&path, &bytes)?;
                        let pcm = crate::asr::decode(bytes)?;
                        let levels: Vec<f32> = pcm
                            .chunks(800)
                            .map(|c| c.iter().fold(0.0f32, |p, v| p.max(v.abs())))
                            .collect();
                        Ok((path, pcm.len() as f32 / 16_000.0, peaks(&levels)))
                    })
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r)
                    .map_err(said),
                    Err(e) => Err(said(e)),
                };
                Answer::Audio(n, fetched)
            }
        };
        if tx.send(Update::Play(answer)).is_err() {
            break;
        }
    }
}

// ---- What happens ----

/// The service answered.
pub(super) fn answer(state: &mut Screen, answer: Answer, jobs: &mpsc::UnboundedSender<Job>) {
    let play = &mut state.play;
    match answer {
        Answer::State(Ok(value)) => {
            play.asking = false;
            if let Ok(items) = serde_json::from_value::<Vec<Take>>(value["items"].clone()) {
                play.items = items;
            }
            play.goal = value["goal"].as_u64().unwrap_or(0) as usize;
            play.can_send = value["can_send"] == true;
            if !play.send
                && play.name.is_empty()
                && let Some(name) = value["name"].as_str()
            {
                play.name = name.to_owned();
            }
            if let Some(idea) = value["idea"].as_str() {
                play.idea = idea.to_owned();
            }
            play.demo_service = value["demo"] == true;
            let first = !play.loaded;
            play.loaded = true;
            // A take sent while the answer was lost may still have been kept: look for it
            // before offering to send it again.
            if let Some(known) = play.known.take() {
                if let Some(kept) = play
                    .items
                    .iter()
                    .find(|t| !known.contains(&t.n) && t.status == "pending")
                    .cloned()
                {
                    if let Some(path) = play.retry.take() {
                        let _ = std::fs::remove_file(path);
                    }
                    play.open_review(kept, false, jobs);
                }
            } else if first && play.page == Page::Record && !play.thinking && !play.recording {
                // A take that was recorded but not reviewed comes back.
                if let Some(waiting) = play.items.iter().rev().find(|t| t.status == "pending") {
                    let waiting = waiting.clone();
                    play.open_review(waiting, false, jobs);
                }
            }
        }
        Answer::State(Err(error)) => {
            play.asking = false;
            if play.known.take().is_some() || !play.loaded {
                state.notice = plain_alert(&error);
            }
        }
        Answer::Uploaded(path, Ok(value)) => {
            play.thinking = false;
            play.pending = None;
            let _ = std::fs::remove_file(&path);
            match serde_json::from_value::<Take>(value) {
                Ok(take) => {
                    play.items.retain(|t| t.n != take.n);
                    play.items.push(take.clone());
                    play.open_review(take, false, jobs);
                }
                Err(_) => {
                    let _ = jobs.send(Job::State);
                }
            }
        }
        Answer::Uploaded(path, Err(error)) => {
            play.thinking = false;
            play.pending = None;
            state.notice = plain_alert(&error);
            play.retry = Some(path);
            play.known = Some(play.items.iter().map(|t| t.n).collect());
            play.asking = true;
            let _ = jobs.send(Job::State);
        }
        Answer::Saved(Ok(value)) => {
            play.saving = false;
            if let Ok(take) = serde_json::from_value::<Take>(value)
                && let Some(item) = play.items.iter_mut().find(|t| t.n == take.n)
            {
                *item = take;
            }
            if !play.from_list {
                let _ = jobs.send(Job::Idea);
            }
            state.voice_play = None;
            play.leave(&mut state.playback);
        }
        Answer::Saved(Err(error)) => {
            play.saving = false;
            state.notice = plain_alert(&error);
        }
        Answer::Removed(n, result) => {
            play.saving = false;
            match result {
                Ok(()) => {
                    play.items.retain(|t| t.n != n);
                    state.voice_play = None;
                    play.from_list = false;
                    play.leave(&mut state.playback);
                }
                Err(error) => state.notice = plain_alert(&error),
            }
        }
        Answer::Audio(n, Ok((path, seconds, wave))) => {
            if play.take.as_ref().is_some_and(|t| t.n == n) {
                if let Some((old, old_path)) = play.audio.replace((n, path.clone())) {
                    let _ = std::fs::remove_file(old_path);
                    state.voices.remove(&voice_index(old));
                }
                state.voices.insert(
                    voice_index(n),
                    Voice {
                        path,
                        seconds,
                        peaks: wave,
                    },
                );
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
        Answer::Audio(_, Err(_)) => state.notice = "The recording could not be loaded.".into(),
        Answer::Idea(Ok(value)) => {
            if let Some(idea) = value["idea"].as_str() {
                play.idea = idea.to_owned();
            }
        }
        Answer::Idea(Err(error)) => state.notice = plain_alert(&error),
        Answer::Sent(Ok(value)) => {
            play.sending = false;
            play.send = false;
            play.focus = None;
            let sent = value["sent"].as_u64().unwrap_or(0);
            let left = value["left"].as_u64().unwrap_or(0);
            state.notice = if left > 0 {
                format!("Sent {sent} takes. Thank you! Send again for the other {left}.")
            } else if sent == 1 {
                "Sent 1 take. Thank you!".into()
            } else {
                format!("Sent {sent} takes. Thank you!")
            };
            play.asked = None;
        }
        Answer::Sent(Err(error)) => {
            play.sending = false;
            state.notice = plain_alert(&error);
        }
    }
}

/// Every frame: on Record and List, look at the service's takes now and then, so takes made on
/// the phone show here. Never in the middle of a review.
pub(super) fn tick(state: &mut Screen, jobs: &mpsc::UnboundedSender<Job>) {
    let play = &mut state.play;
    let due = play
        .asked
        .is_none_or(|at| at.elapsed() >= Duration::from_secs(3));
    if play.view
        && !play.asking
        && due
        && play.page != Page::Review
        && !play.thinking
        && !play.recording
    {
        play.asking = true;
        play.asked = Some(Instant::now());
        let _ = jobs.send(Job::State);
    }
}

/// The microphone stopped on a take: the file is finished in the background, then sent.
pub(super) fn finish(state: &mut Screen, tx: &mpsc::UnboundedSender<Update>) {
    state.play.recording = false;
    let (Some(recorder), Some(path)) = (state.recording.take(), state.recording_path.take()) else {
        return;
    };
    state.play.thinking = true;
    state.play.pending = Some(path.clone());
    save_take(recorder, path, tx);
}
/// The finished take's file (or why there is none).
pub(super) fn recorded(
    state: &mut Screen,
    path: PathBuf,
    result: Result<(f32, Vec<f32>), String>,
    jobs: &mpsc::UnboundedSender<Job>,
) {
    match result {
        // As on the phone: under half a second is not a take (the service refuses it too).
        Ok((seconds, _)) if seconds < 0.5 => {
            state.play.thinking = false;
            state.play.pending = None;
            let _ = std::fs::remove_file(path);
            state.notice = "That was too short. Speak, then stop.".into();
        }
        Ok(_) => {
            let _ = jobs.send(Job::Upload(path));
        }
        Err(error) => {
            state.play.thinking = false;
            state.play.pending = None;
            let _ = std::fs::remove_file(path);
            state.notice = format!("Microphone recording failed: {error}");
        }
    }
}
/// Record (the round button, Space): start, or stop and send. Demo mode pretends, and sends
/// made-up sound only to a service that keeps demo takes.
pub(super) fn record_key(
    state: &mut Screen,
    demo: bool,
    tx: &mpsc::UnboundedSender<Update>,
) -> Result<()> {
    if state.recording.is_some() {
        finish_recording(state, tx);
        return Ok(());
    }
    if let Some(at) = state.demo_rec {
        state.demo_rec = None;
        state.play.recording = false;
        if !state.play.demo_service {
            state.notice = "Demo mode: this service keeps real takes, so nothing was sent.".into();
            return Ok(());
        }
        let seconds = at.elapsed().as_secs_f32().max(1.0);
        let levels = state.levels.clone();
        let pcm: Vec<f32> = (0..(seconds * 16_000.0) as usize)
            .map(|i| {
                let level = levels
                    .get(i * levels.len() / (seconds * 16_000.0) as usize)
                    .copied()
                    .unwrap_or(0.0);
                (i as f32 / 11.0).sin() * level * 0.5
            })
            .collect();
        let path = temp_take();
        write_private(&path, &crate::asr::wav(&pcm)?)?;
        state.play.thinking = true;
        state.play.pending = Some(path.clone());
        let _ = tx.send(Update::Recorded(path, Ok((seconds, levels))));
        return Ok(());
    }
    if state.play.page != Page::Record || state.play.thinking || state.pair_view {
        return Ok(());
    }
    // A new take: the one that did not get through is let go, as on the phone.
    if let Some(path) = state.play.retry.take() {
        let _ = std::fs::remove_file(path);
    }
    state.play.known = None;
    state.play.recording = true;
    if demo {
        state.demo_rec = Some(Instant::now());
        return Ok(());
    }
    // Never waits for the device: the screen flips to recording at once.
    state.recording = Some(crate::audio::Recorder::start_now());
    state.recording_path = Some(temp_take());
    Ok(())
}

/// A click on the playground.
pub(super) fn click(
    state: &mut Screen,
    action: PlayClick,
    jobs: &mpsc::UnboundedSender<Job>,
) -> Result<Option<KeyEvent>> {
    // Under the first-run question the top bar's Contribute does nothing (its explanation
    // would wait unseen behind the card); elsewhere it closes the Sharing card.
    if action == PlayClick::Toggle {
        if state.share_card == Some(ShareCard::Ask) {
            return Ok(None);
        }
        state.share_card = None;
    }
    let play = &mut state.play;
    match action {
        PlayClick::Toggle if !state.playground => {
            play.consent = !play.consent;
            state.pair_view = false;
        }
        PlayClick::Decline => play.consent = false,
        PlayClick::ShareOpen => {
            state.share_card = Some(ShareCard::On);
            play.consent = false;
            state.pair_view = false;
        }
        PlayClick::ShareClose => state.share_card = None,
        PlayClick::Share(yes) => {
            // The owner's own computer never shares (it is never asked either).
            let yes = yes && !crate::phone::owner();
            if let Err(error) = crate::share::set(yes) {
                state.notice = format!("Cannot save your answer: {error}");
                return Ok(None);
            }
            if state.sharing && !yes {
                state.notice = "Sharing is off.".into();
            }
            state.sharing = yes;
            state.share_card = None;
        }
        PlayClick::SendOpen => {
            play.send = true;
            play.focus = Some(Field::Name);
        }
        PlayClick::SendCancel => {
            play.send = false;
            play.focus = None;
        }
        PlayClick::SendNow => {
            let name = play.name.split_whitespace().collect::<Vec<_>>().join(" ");
            if name.is_empty() {
                state.notice = "Type a name first (any name will do).".into();
            } else if !play.sending {
                play.name = name.clone();
                play.sending = true;
                let _ = jobs.send(Job::Send(name));
            }
        }
        PlayClick::Accept => {
            play.consent = false;
            match crate::share::accept_contribute() {
                Ok(sharing) => state.sharing = sharing,
                Err(error) => {
                    state.notice = format!("Cannot save your answer: {error}");
                    return Ok(None);
                }
            }
            state.playground = true;
            play.view = true;
            play.page = Page::Record;
            sweep(false);
            let _ = jobs.send(Job::State);
        }
        PlayClick::Toggle => {
            play.view = !play.view || state.pair_view;
            state.pair_view = false;
            play.asked = None;
        }
        PlayClick::Idea => {
            let _ = jobs.send(Job::Idea);
        }
        PlayClick::List => {
            play.page = Page::List;
            play.list_scroll = 0;
            play.asked = None;
        }
        PlayClick::Record => {
            play.page = Page::Record;
            play.asked = None;
        }
        PlayClick::Open(n) => {
            if let Some(take) = play.items.iter().find(|t| t.n == n).cloned() {
                play.open_review(take, true, jobs);
            }
        }
        PlayClick::Word(i) => play.tap_word(i),
        PlayClick::Mark(k) => {
            if let Some(m) = play.marks.get(k) {
                let span = (m.start, m.end);
                play.open_sheet(span, Some(k), false);
            }
        }
        PlayClick::Reason(i) => {
            if let Some((key, _)) = REASONS.get(i) {
                play.pick_reason(key);
            }
        }
        PlayClick::Focus(field) => play.focus = Some(field),
        PlayClick::Before => {
            if let Some((start, _)) = play.sel {
                play.add_place(start);
            }
        }
        PlayClick::After => {
            if let Some((_, end)) = play.sel {
                play.add_place(end);
            }
        }
        PlayClick::Remove => {
            if let Some(k) = play.editing.filter(|k| *k < play.marks.len()) {
                play.marks.remove(k);
            }
            play.close_sheet();
        }
        PlayClick::Cancel => play.close_sheet(),
        PlayClick::SaveMark => play.save_mark(),
        PlayClick::TypeOut => play.type_out(),
        PlayClick::Toss | PlayClick::Save => save(state, action == PlayClick::Toss, jobs),
        PlayClick::Redo => {
            if let Some(take) = &play.take
                && !play.saving
            {
                if take.status == "pending" {
                    play.saving = true;
                    let _ = jobs.send(Job::Remove(take.n));
                } else {
                    play.ask = true;
                }
            }
        }
        PlayClick::AskYes => {
            play.ask = false;
            if let Some(take) = &play.take
                && !play.saving
            {
                play.saving = true;
                let _ = jobs.send(Job::Remove(take.n));
            }
        }
        PlayClick::AskNo => play.ask = false,
        PlayClick::Retry => {
            if let Some(path) = play.retry.take()
                && !play.thinking
            {
                play.thinking = true;
                play.pending = Some(path.clone());
                let _ = jobs.send(Job::Upload(path));
            }
        }
        PlayClick::Scroll(up) => scroll(state, up),
    }
    Ok(None)
}
fn save(state: &mut Screen, toss: bool, jobs: &mpsc::UnboundedSender<Job>) {
    let play = &mut state.play;
    let Some(n) = play.take.as_ref().map(|t| t.n) else {
        return;
    };
    if play.saving {
        return;
    }
    if !toss && play.typed.as_deref().is_some_and(|t| t.trim().is_empty()) {
        state.notice = "Type what you said, or toss the take.".into();
        return;
    }
    play.close_sheet();
    play.saving = true;
    let _ = jobs.send(Job::Review(n, play.review(toss)));
}
/// The mouse wheel, or a click on "more": the takes in List, the draft in Review.
pub(super) fn scroll(state: &mut Screen, up: bool) {
    let play = &mut state.play;
    let step = |at: &mut usize, by: usize| {
        *at = if up {
            at.saturating_sub(by)
        } else {
            at.saturating_add(by)
        }
    };
    match play.page {
        Page::List => step(&mut play.list_scroll, 3),
        Page::Review => step(&mut play.words_scroll, 1),
        Page::Record => {}
    }
}

/// A key on the playground; false when the app's usual handling should have it (refresh,
/// quit, a click's own key, Space to record on the Record screen).
pub(super) fn key(state: &mut Screen, key: KeyEvent, jobs: &mpsc::UnboundedSender<Job>) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    if control && matches!(key.code, KeyCode::Char('r' | 'c')) {
        return false;
    }
    if state.play.ask {
        if key.code == KeyCode::Esc {
            state.play.ask = false;
        }
        return true;
    }
    if state.play.send {
        // The name has the keyboard; nothing changes while the takes are on their way.
        let action = match key.code {
            _ if state.play.sending => None,
            KeyCode::Esc => Some(PlayClick::SendCancel),
            KeyCode::Enter => Some(PlayClick::SendNow),
            KeyCode::Backspace => {
                state.play.name.pop();
                None
            }
            KeyCode::Char('v') if control => {
                if let Some(text) = clipboard_text() {
                    paste(state, &text);
                }
                None
            }
            KeyCode::Char(c) if !control => {
                paste(state, &c.to_string());
                None
            }
            _ => None,
        };
        if let Some(action) = action {
            let _ = click(state, action, jobs);
        }
        return true;
    }
    if let Some(field) = state.play.focus {
        let play = &mut state.play;
        match key.code {
            KeyCode::Esc if play.sel.is_some() => play.close_sheet(),
            KeyCode::Esc => play.focus = None,
            KeyCode::Enter if play.sel.is_some() => play.save_mark(),
            // Enter in the typed-out text saves the take, like the main button.
            KeyCode::Enter if field == Field::Typed => save(state, false, jobs),
            KeyCode::Tab if play.sel.is_some() && play.reason == "custom" => {
                play.focus = Some(if field == Field::Note {
                    Field::Right
                } else {
                    Field::Note
                });
            }
            KeyCode::Backspace => {
                play.field(field).pop();
            }
            KeyCode::Char('v') if control => {
                if let Some(text) = clipboard_text() {
                    paste(state, &text);
                }
            }
            KeyCode::Char(c) if !control => paste(state, &c.to_string()),
            _ => {}
        }
        return true;
    }
    let play = &mut state.play;
    match key.code {
        KeyCode::Esc if play.sel.is_some() => play.close_sheet(),
        KeyCode::Esc if play.page == Page::Review => {
            state.voice_play = None;
            play.leave(&mut state.playback);
        }
        KeyCode::Esc if play.page == Page::List => play.page = Page::Record,
        KeyCode::Enter if play.sel.is_some() => play.save_mark(),
        // Hold Space to talk: on the Record screen only.
        KeyCode::Char(' ') if play.page == Page::Record && key.modifiers.is_empty() => {
            return false;
        }
        KeyCode::F(10) => return false,
        KeyCode::Char(' ') if play.page == Page::Review => {
            if let Some(n) = play.take.as_ref().map(|t| t.n)
                && state.voices.contains_key(&voice_index(n))
            {
                voice_toggle(state, voice_index(n));
            }
        }
        KeyCode::Up | KeyCode::PageUp => scroll(state, true),
        KeyCode::Down | KeyCode::PageDown => scroll(state, false),
        _ => {}
    }
    true
}
/// Pasted text goes into the field that has the keyboard (on one line).
pub(super) fn paste(state: &mut Screen, text: &str) {
    let play = &mut state.play;
    let Some(field) = play.focus else {
        return;
    };
    if field == Field::Right {
        play.touched = true;
        play.help = (play.default_help(), false);
    }
    let line = clean(text).replace(['\n', '\t'], " ");
    let limit: usize = match field {
        Field::Note => 200,
        Field::Name => 40,
        _ => 4000,
    };
    let target = play.field(field);
    let room = limit.saturating_sub(target.chars().count());
    target.extend(line.chars().take(room));
}

// ---- Drawing ----

/// One piece of the draft as drawn: its column and row (from the draft's top left), what it
/// shows (already in display order), its look, and what a click on it does.
#[derive(Debug)]
pub(super) struct Piece {
    pub x: u16,
    pub row: u16,
    pub text: String,
    pub style: Style,
    pub click: Option<PlayClick>,
}
struct Bit {
    text: String,
    style: Style,
    click: Option<PlayClick>,
}
/// Words that stay together: one word, or a marked span with its correction, and the tag
/// shown under it.
struct Group {
    bits: Vec<Bit>,
    tag: Option<(String, Color, PlayClick)>,
}
fn width_of(text: &str) -> u16 {
    Span::raw(text).width() as u16
}
/// A text no wider than `width`: the end of it gives way, with "…".
fn shorten(text: &str, width: u16) -> String {
    if width_of(text) <= width {
        return text.to_owned();
    }
    let mut out: String = text
        .chars()
        .take(width.saturating_sub(1) as usize)
        .collect();
    while width_of(&out) + 1 > width && out.pop().is_some() {}
    out.push('…');
    out
}
/// One right-to-left line, as the terminal is to show it.
fn shown(text: &str) -> String {
    visual(text, 10_000)
}
/// The draft laid out right to left in `width` columns, the way the phone shows it: every
/// word on its own, a marked span's words with its correction and its tag under them, places
/// where a word is missing. Each piece keeps its word's number, also across wrapped lines.
/// Returns the pieces and the rows they take.
pub(super) fn lay_out(play: &Play, width: u16) -> (Vec<Piece>, u16) {
    let words = play.words();
    let width = width.max(4);
    let selected = Style::default().fg(Color::Black).bg(THEME_ACCENT);
    let mut groups: Vec<Group> = Vec::new();
    let mut p = 0;
    loop {
        // A place between two words: a missing word, saved or being added.
        let place = play.marks.iter().position(|m| m.start == p && m.end == p);
        let adding = play.sel == Some((p, p));
        if place.is_some() || adding {
            let color = tint("dropped");
            groups.push(Group {
                bits: vec![Bit {
                    text: match place {
                        Some(k) => shown(&format!(
                            "+ {}",
                            play.marks[k].right.as_deref().unwrap_or_default()
                        )),
                        None => "[ + ]".into(),
                    },
                    style: if adding {
                        Style::default().fg(Color::Black).bg(color)
                    } else {
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    },
                    click: place.map(PlayClick::Mark),
                }],
                tag: place.map(|k| ("missing".to_owned(), color, PlayClick::Mark(k))),
            });
        }
        if p >= words.len() {
            break;
        }
        if let Some(k) = play.marks.iter().position(|m| m.start == p && m.end > p) {
            let m = &play.marks[k];
            let color = tint(&m.reason);
            let heard = words[m.start..m.end.min(words.len())].join(" ");
            let changed = m.right.as_ref().is_some_and(|r| *r != heard);
            let editing = play.editing == Some(k);
            let mut style = if changed {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::CROSSED_OUT)
            } else {
                Style::default().fg(color)
            };
            if editing {
                style = Style::default().fg(Color::Black).bg(color);
            }
            let mut bits: Vec<Bit> = (m.start..m.end.min(words.len()))
                .map(|i| Bit {
                    text: shown(words[i]),
                    style,
                    click: Some(PlayClick::Word(i)),
                })
                .collect();
            if let Some(right) = m.right.as_ref().filter(|r| changed && !r.is_empty()) {
                bits.push(Bit {
                    text: shown(right),
                    style: Style::default().fg(color).add_modifier(Modifier::BOLD),
                    click: Some(PlayClick::Mark(k)),
                });
            }
            let tag = if changed && m.right.as_deref() == Some("") {
                "not said".to_owned()
            } else if m.reason == "custom" && !m.note.is_empty() {
                m.note.clone()
            } else {
                label(&m.reason).to_owned()
            };
            groups.push(Group {
                bits,
                tag: Some((shorten(&tag, 16), color, PlayClick::Mark(k))),
            });
            p = m.end;
            continue;
        }
        let chosen = play.editing.is_none() && play.sel.is_some_and(|(s, e)| s <= p && p < e);
        groups.push(Group {
            bits: vec![Bit {
                text: shown(words[p]),
                style: if chosen { selected } else { Style::default() },
                click: Some(PlayClick::Word(p)),
            }],
            tag: None,
        });
        p += 1;
    }
    // A group wider than the line is cut into pieces that fit (the tag stays on the first),
    // and a single word wider than the line keeps its beginning (its right-hand end).
    let mut fitted: Vec<Group> = Vec::new();
    for group in groups {
        let mut tag = group.tag;
        let mut part: Vec<Bit> = Vec::new();
        let mut used = 0u16;
        for mut bit in group.bits {
            if width_of(&bit.text) > width {
                let chars: Vec<char> = bit.text.chars().collect();
                bit.text = chars[chars.len().saturating_sub(width as usize)..]
                    .iter()
                    .collect();
            }
            let w = width_of(&bit.text);
            if !part.is_empty() && used + 1 + w > width {
                fitted.push(Group {
                    bits: std::mem::take(&mut part),
                    tag: tag.take(),
                });
                used = 0;
            }
            used += if part.is_empty() { w } else { w + 1 };
            part.push(bit);
        }
        fitted.push(Group { bits: part, tag });
    }
    // Lines, filled from the right.
    let mut placed: Vec<(usize, u16, u16, Group)> = Vec::new();
    let (mut line, mut edge) = (0usize, width);
    for group in fitted {
        let content = group.bits.iter().map(|b| width_of(&b.text)).sum::<u16>()
            + group.bits.len().saturating_sub(1) as u16;
        let tag = group.tag.as_ref().map_or(0, |(t, ..)| width_of(t));
        let w = content.max(tag).min(width);
        let gap = u16::from(edge < width);
        if edge < width && w + gap > edge {
            line += 1;
            edge = width;
        }
        let gap = u16::from(edge < width);
        let left = edge - gap - w;
        placed.push((line, left, w, group));
        edge = left;
    }
    let lines = placed.last().map_or(0, |p| p.0 + 1);
    let mut row_of = vec![0u16; lines + 1];
    for l in 0..lines {
        let tagged = placed.iter().any(|p| p.0 == l && p.3.tag.is_some());
        row_of[l + 1] = row_of[l] + 1 + u16::from(tagged);
    }
    let mut pieces = Vec::new();
    for (line, left, w, group) in placed {
        let row = row_of[line];
        let content = group.bits.iter().map(|b| width_of(&b.text)).sum::<u16>()
            + group.bits.len().saturating_sub(1) as u16;
        // Right to left, centered in the group's room.
        let mut right = left + w - (w - content.min(w)) / 2;
        for bit in group.bits {
            let bw = width_of(&bit.text);
            let x = right.saturating_sub(bw).max(left);
            right = x.saturating_sub(1);
            pieces.push(Piece {
                x,
                row,
                text: bit.text,
                style: bit.style,
                click: bit.click,
            });
        }
        if let Some((tag, color, click)) = group.tag {
            let tag = shorten(&tag, w);
            pieces.push(Piece {
                x: left + (w - width_of(&tag)) / 2,
                row: row + 1,
                text: tag,
                style: Style::default().fg(color),
                click: Some(click),
            });
        }
    }
    (pieces, row_of[lines])
}

/// The column the playground's screens sit in: centered, at most `most` wide.
fn column(region: Rect, most: u16) -> Rect {
    let w = region.width.min(most);
    Rect::new(
        region.x + (region.width - w) / 2,
        region.y,
        w,
        region.height,
    )
}
/// A line on the bare background, with the background wiped behind it first (so no falling
/// letters run through the words).
fn bare(frame: &mut Frame, rect: Rect, line: Line<'_>, alignment: Alignment) {
    frame.render_widget(ratatui::widgets::Clear, rect);
    frame.render_widget(Paragraph::new(line).alignment(alignment), rect);
}
/// Which parts fit `room` rows, top to bottom. Each part is (rank, rows, fewest rows, what):
/// while they do not fit, the parts of the highest rank go (rank 0 always stays); rows left
/// over go to parts that can grow, in order.
fn fit<T: Copy>(room: u16, mut parts: Vec<(u8, u16, u16, T)>) -> Vec<(u16, T)> {
    while parts.iter().map(|p| p.2).sum::<u16>() > room {
        let Some(worst) = parts.iter().map(|p| p.0).max().filter(|r| *r > 0) else {
            break;
        };
        let at = parts.iter().rposition(|p| p.0 == worst).unwrap_or(0);
        parts.remove(at);
    }
    let mut spare = room.saturating_sub(parts.iter().map(|p| p.2).sum());
    parts
        .into_iter()
        .map(|(_, rows, least, what)| {
            let more = rows.saturating_sub(least).min(spare);
            spare -= more;
            (least + more, what)
        })
        .collect()
}
/// Draw `parts` (as `fit` left them) down the middle of `region`; `draw` gets each part's top
/// row and height. Parts that would run past the region are left out.
fn stack<T: Copy>(region: Rect, parts: Vec<(u16, T)>, mut draw: impl FnMut(T, u16, u16)) {
    let total: u16 = parts.iter().map(|p| p.0).sum();
    let mut y = region.y + region.height.saturating_sub(total) / 2;
    for (rows, what) in parts {
        if y + rows > region.bottom() {
            break;
        }
        draw(what, y, rows);
        y += rows;
    }
}
/// The rows a card or button keeps above and below its text, in every look: a card is never
/// a bare line of text on a strip.
fn pad() -> u16 {
    1
}
/// A card around `rect.height` rows of text: the shared plate, with a row of its own colour
/// above and below the text where the terminal draws pictures (elsewhere the plate brings
/// half rows itself). Needs `pad()` free rows above and below `rect`.
fn card(frame: &mut Frame, rect: Rect, colors: (Color, Color)) -> (Rect, Style, Rect) {
    if round_theme().is_none() {
        return plate(frame, rect, colors);
    }
    let all = Rect::new(
        rect.x,
        rect.y.saturating_sub(1),
        rect.width,
        rect.height + 2,
    );
    let (text, style, _) = plate_ends(frame, all, colors, 2);
    (
        Rect::new(text.x, rect.y, text.width, rect.height),
        style,
        all,
    )
}
/// A one-row button for a window too small for the fat ones: a label, as wide as the label and a column either side: filled in
/// `fill` (the accent, the red or the surface), with round ends where the terminal can draw
/// them; in the phone look the label in brackets. Returns its width.
#[allow(clippy::too_many_arguments)]
fn chip(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    (x, y): (u16, u16),
    label: Line<'_>,
    (fill, ground): (Color, Color),
    action: Click,
    enabled: bool,
) -> u16 {
    let w = label.width() as u16 + 2;
    let rect = Rect::new(x, y, w, 1);
    if enabled {
        hits.push(Hit { area: rect, action });
    }
    if outline_controls() {
        let color = if enabled && fill != surface() {
            fill
        } else if enabled {
            Color::Reset
        } else {
            Color::DarkGray
        };
        let mut spans = vec![Span::styled("[", Style::default().fg(color))];
        spans.extend(label.spans);
        spans.push(Span::styled("]", Style::default().fg(color)));
        frame.render_widget(ratatui::widgets::Clear, rect);
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().fg(color)),
            rect,
        );
        return w;
    }
    let ink = if !enabled {
        Color::DarkGray
    } else if fill == surface() {
        Color::Reset
    } else {
        Color::Black
    };
    let style = Style::default().fg(ink).bg(fill);
    if slim_round() {
        frame.render_widget(
            Paragraph::new(label).style(style),
            Rect::new(x + 1, y, w - 2, 1),
        );
        pill_caps(frame, x, x + w - 1, y, fill);
    } else if let Some(dark) = round_theme() {
        // Only the middle is filled: the ends are pictures whose corners show what the chip
        // sits on (the bare background, or a card).
        frame.render_widget(ratatui::widgets::Clear, rect);
        if ground != Color::Reset {
            frame.render_widget(Block::default().style(Style::default().bg(ground)), rect);
        }
        frame.render_widget(
            Paragraph::new(label).style(style),
            Rect::new(x + 1, y, w - 2, 1),
        );
        let rgb = match fill {
            THEME_ACCENT => theme_rgb().0,
            THEME_RECORD => theme_rgb().1,
            _ => dark,
        };
        for (left, column) in [(true, x), (false, x + w - 1)] {
            corner(
                frame,
                Rect::new(column, y, 1, 1),
                rgb,
                left,
                Corner::Side {
                    top: true,
                    bottom: true,
                },
            );
        }
    } else {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(label.spans);
        spans.push(Span::raw(" "));
        frame.render_widget(Paragraph::new(Line::from(spans)).style(style), rect);
    }
    w
}

thread_local! {
    /// The window is too small for fat buttons: they are one row each (see `chip`).
    static SLIM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// How tall a button is.
fn button_rows() -> u16 {
    if SLIM.get() { 1 } else { 3 }
}
/// Columns a card's round ends take beyond one, each side, where pictures are drawn.
fn extra() -> u16 {
    u16::from(round_theme().is_some())
}
/// Buttons on the bare background, centered in `area`, wrapping onto more rows when they do
/// not fit: each as tall and as round as the typing bar, its label in the middle. Returns the
/// rows they take; without a frame, only counts.
fn chips(
    frame: Option<&mut Frame>,
    hits: &mut Vec<Hit>,
    area: Rect,
    items: &[(Line<'static>, Color, Click, bool)],
) -> u16 {
    let slim = SLIM.get();
    let width =
        |item: &(Line, Color, Click, bool)| item.0.width() as u16 + if slim { 2 } else { 4 };
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0;
    for (i, item) in items.iter().enumerate() {
        let w = width(item);
        let last = rows.last_mut().unwrap();
        if !last.is_empty() && used + 2 + w > area.width {
            rows.push(vec![i]);
            used = w;
        } else {
            used += if last.is_empty() { w } else { w + 2 };
            last.push(i);
        }
    }
    let pitch = button_rows();
    if let Some(frame) = frame {
        for (r, row) in rows.iter().enumerate() {
            let total: u16 = row.iter().map(|i| width(&items[*i])).sum::<u16>()
                + 2 * row.len().saturating_sub(1) as u16;
            let y = area.y + r as u16 * pitch;
            if y + button_rows() > area.bottom() {
                break;
            }
            let mut x = area.x + area.width.saturating_sub(total) / 2;
            for i in row {
                let (label, fill, action, enabled) = items[*i].clone();
                let w = width(&items[*i]);
                if x + w > area.right() {
                    break;
                }
                if slim {
                    chip(
                        frame,
                        hits,
                        (x, y),
                        label,
                        (fill, Color::Reset),
                        action,
                        enabled,
                    );
                } else {
                    button(
                        frame,
                        hits,
                        Rect::new(x, y, w, 3),
                        label,
                        fill,
                        action,
                        enabled,
                    );
                }
                x += w + 2;
            }
        }
    }
    rows.len() as u16 * pitch
}
/// One button in `rect` (three rows), shaped like the typing bar: the label's row filled,
/// half a row of the same colour above and below, and round ends. `fill` is the accent, the
/// red or the surface.
fn button(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    rect: Rect,
    label: Line<'static>,
    fill: Color,
    action: Click,
    enabled: bool,
) {
    let plain = fill == surface();
    let ink = if !enabled {
        Color::DarkGray
    } else if plain {
        Color::Reset
    } else {
        Color::Black
    };
    // A label with colours of its own keeps them only on the surface (on the accent they
    // would not read).
    let label = if enabled && plain {
        label
    } else {
        Line::from(label.to_string())
    };
    let row = rect.y + 1;
    let middle = Rect::new(rect.x + 1, row, rect.width.saturating_sub(2), 1);
    frame.render_widget(ratatui::widgets::Clear, rect);
    if enabled {
        hits.push(Hit { area: rect, action });
    }
    if outline_controls() {
        let edge = if !enabled {
            Color::DarkGray
        } else if plain {
            Color::Gray
        } else {
            fill
        };
        outline_box(frame, rect, edge);
        let ink = if enabled && !plain { fill } else { ink };
        frame.render_widget(
            Paragraph::new(label)
                .style(Style::default().fg(ink))
                .alignment(Alignment::Center),
            middle,
        );
        return;
    }
    // Where pictures are drawn the ends are pictures, in the theme's exact colours: the
    // cells between them must be those colours too.
    let rgb = match fill {
        THEME_ACCENT => theme_rgb().0,
        THEME_RECORD => theme_rgb().1,
        _ => surface_rgb(),
    };
    let fill = if round_theme().is_some() {
        Color::Rgb(rgb[0], rgb[1], rgb[2])
    } else {
        fill
    };
    frame.render_widget(
        Paragraph::new(label)
            .style(Style::default().fg(ink).bg(fill))
            .alignment(Alignment::Center),
        middle,
    );
    if slim_round() {
        // One row with round ends, as every control is in this look.
        pill_caps(frame, rect.x, rect.right() - 1, row, fill);
        return;
    }
    for (y, half) in [(rect.y, "▄"), (rect.y + 2, "▀")] {
        frame.render_widget(
            Paragraph::new(half.repeat(middle.width as usize)).style(Style::default().fg(fill)),
            Rect::new(middle.x, y, middle.width, 1),
        );
    }
    bar_ends(frame, rect.x, rect.right() - 1, row, rgb);
}
/// A one-row text field on a plate: Yiddish (right to left, typed at its left end) or English.
fn field(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    rect: Rect,
    text: &str,
    empty: &str,
    (focused, yiddish, slim): (bool, bool, bool),
    action: Click,
) {
    // `slim`: no room above and below for the plate's half rows; one row with the same ends.
    let (inner, style) = if slim && pad() > 0 {
        let fill = surface();
        let inner = Rect::new(rect.x + 1, rect.y, rect.width.saturating_sub(2), 1);
        frame.render_widget(ratatui::widgets::Clear, rect);
        if outline() {
            let edge = Style::default().fg(Color::DarkGray);
            for (column, glyph) in [(rect.x, "["), (rect.right() - 1, "]")] {
                frame.render_widget(
                    Paragraph::new(glyph).style(edge),
                    Rect::new(column, rect.y, 1, 1),
                );
            }
            (inner, Style::default())
        } else {
            let style = Style::default().bg(fill);
            frame.render_widget(Block::default().style(style), inner);
            if nerd_font() {
                pill_caps(frame, rect.x, rect.right() - 1, rect.y, fill);
            } else {
                let cap = Style::default().fg(fill);
                frame.render_widget(
                    Paragraph::new("▐").style(cap),
                    Rect::new(rect.x, rect.y, 1, 1),
                );
                frame.render_widget(
                    Paragraph::new("▌").style(cap),
                    Rect::new(rect.right() - 1, rect.y, 1, 1),
                );
            }
            (inner, style)
        }
    } else {
        let (inner, style, _) = card(frame, rect, (Color::Reset, surface()));
        (inner, style)
    };
    hits.push(Hit { area: rect, action });
    let room = inner.width.saturating_sub(3) as usize;
    let line = if text.is_empty() && !focused {
        Line::styled(format!(" {empty}"), Style::default().fg(Color::DarkGray))
    } else {
        let chars: Vec<char> = text.chars().collect();
        let tail: String = chars[chars.len().saturating_sub(room)..].iter().collect();
        let cursor = if focused { "▏" } else { " " };
        if yiddish {
            Line::from(format!("{cursor}{} ", shown(&tail)))
        } else {
            Line::from(format!(" {tail}{cursor}"))
        }
    };
    frame.render_widget(
        Paragraph::new(line)
            .style(style)
            .alignment(if yiddish && (!text.is_empty() || focused) {
                Alignment::Right
            } else {
                Alignment::Left
            }),
        inner,
    );
}

/// The playground in `region` (under the top bar; on Record, above the typing bar).
pub(super) fn draw(frame: &mut Frame, state: &Screen, region: Rect, hits: &mut Vec<Hit>) {
    if region.height == 0 || region.width < 20 {
        return;
    }
    let play = &state.play;
    // Fat buttons where the screen has the rows for them; one-row ones in a small window.
    SLIM.set(false);
    let fat = match () {
        _ if play.ask => 10,
        _ if play.send => 18,
        _ if play.page == Page::Review => review_rows(state, region),
        _ if play.page == Page::List => 9,
        _ => 12,
    };
    SLIM.set(region.height < fat);
    if play.ask {
        return draw_ask(frame, state, region, hits);
    }
    if play.send {
        return draw_send(frame, state, region, hits);
    }
    match play.page {
        Page::Record => draw_record(frame, state, region, hits),
        Page::Review if play.take.is_some() => draw_review(frame, state, region, hits),
        Page::Review => {}
        Page::List => draw_list(frame, state, region, hits),
    }
}

fn draw_record(frame: &mut Frame, state: &Screen, region: Rect, hits: &mut Vec<Hit>) {
    #[derive(Clone, Copy)]
    enum Part {
        Title,
        Progress,
        Gap,
        Idea,
        Stage,
        Chip,
    }
    let play = &state.play;
    let col = column(region, 64);
    let pad = pad();
    let busy = play.recording || play.thinking;
    let idea: Vec<String> = if play.idea.is_empty() {
        Vec::new()
    } else {
        visual(&play.idea, col.width.saturating_sub(6).max(8) as usize)
            .lines()
            .take(3)
            .map(str::to_owned)
            .collect()
    };
    // While recording, the typing bar below is the live voice and the timer, as in the chat.
    let stage = if play.recording {
        0
    } else if play.thinking {
        1 + 2 * pad
    } else {
        button_rows() * u16::from(play.retry.is_some())
    };
    let mut parts = vec![(0, 1, 1, Part::Title), (4, 1, 1, Part::Progress)];
    if !idea.is_empty() {
        let rows = idea.len() as u16 + 2 * pad;
        parts.push((6, 1, 1, Part::Gap));
        parts.push((0, rows, rows, Part::Idea));
    }
    if stage > 0 {
        parts.push((6, 1, 1, Part::Gap));
        parts.push((0, stage, stage, Part::Stage));
    }
    parts.push((5, 1, 1, Part::Gap));
    parts.push((1, button_rows(), button_rows(), Part::Chip));
    let parts = fit(region.height, parts);
    stack(region, parts, |part, y, rows| match part {
        Part::Gap => {}
        Part::Title => {
            let done = play.done();
            let rest = match play.goal() {
                None => String::new(),
                Some(goal) if done >= goal => {
                    format!("   The set of {goal} is complete. More is fine.")
                }
                Some(goal) => format!("  of {goal}"),
            };
            bare(
                frame,
                Rect::new(col.x, y, col.width, 1),
                Line::from(vec![
                    Span::styled(
                        format!("Take {}", done + 1),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(rest, Style::default().fg(Color::Gray)),
                ]),
                Alignment::Center,
            );
        }
        Part::Progress => {
            let Some(goal) = play.goal() else {
                return;
            };
            let done = play.done();
            let w = col.width.min(40);
            let full = ((done.min(goal) * w as usize) / goal.max(1)) as u16;
            let color = if done >= goal {
                Color::Green
            } else {
                THEME_ACCENT
            };
            bare(
                frame,
                Rect::new(col.x + (col.width - w) / 2, y, w, 1),
                Line::from(vec![
                    Span::styled("━".repeat(full as usize), Style::default().fg(color)),
                    Span::styled(
                        "─".repeat((w - full) as usize),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Alignment::Left,
            );
        }
        Part::Idea => {
            let (inner, style, _) = card(
                frame,
                Rect::new(col.x, y + pad, col.width, rows - 2 * pad),
                (Color::Reset, surface()),
            );
            let inner = Rect::new(
                inner.x + 1,
                inner.y,
                inner.width.saturating_sub(2),
                inner.height,
            );
            frame.render_widget(
                Paragraph::new(
                    idea.iter()
                        .map(|l| Line::from(l.clone()))
                        .collect::<Vec<_>>(),
                )
                .style(style)
                .alignment(Alignment::Center),
                inner,
            );
        }
        Part::Stage if play.thinking => {
            let words = "Writing down what you said";
            let w = (14 + 2 + width_of(words) + 4).min(col.width);
            let (inner, style, _) = card(
                frame,
                Rect::new(col.x + (col.width - w) / 2, y + pad, w, 1),
                (Color::Reset, surface()),
            );
            let mut spans = thinking();
            spans.push(Span::raw(format!("  {words}")));
            frame.render_widget(
                Paragraph::new(Line::from(spans))
                    .style(style)
                    .alignment(Alignment::Center),
                inner,
            );
        }
        Part::Stage => {
            chips(
                Some(frame),
                hits,
                Rect::new(col.x, y, col.width, rows),
                &[(
                    Line::from("Send it again"),
                    THEME_ACCENT,
                    Click::Play(PlayClick::Retry),
                    true,
                )],
            );
        }
        Part::Chip => {
            chips(
                Some(frame),
                hits,
                Rect::new(col.x, y, col.width, rows),
                &[
                    (
                        Line::from("Another idea"),
                        surface(),
                        Click::Play(PlayClick::Idea),
                        !busy && !play.idea.is_empty(),
                    ),
                    (
                        Line::from("All takes"),
                        surface(),
                        Click::Play(PlayClick::List),
                        !busy,
                    ),
                ]
                .into_iter()
                .chain(play.send_button().filter(|_| !busy))
                .collect::<Vec<_>>(),
            );
        }
    });
}

/// The rows Review needs with fat buttons: the draft's card at its smallest and the buttons,
/// or the mark panel's own controls.
fn review_rows(state: &Screen, region: Rect) -> u16 {
    let play = &state.play;
    let width = column(region, 76).width;
    // Buttons wrap by their widths: label, two columns of padding, two ends, two apart.
    let rows = |labels: &[usize]| {
        let (mut rows, mut used) = (1u16, 0u16);
        for (i, label) in labels.iter().enumerate() {
            let w = *label as u16 + 4;
            if i > 0 && used + 2 + w > width {
                rows += 1;
                used = w;
            } else {
                used += if i == 0 { w } else { w + 2 };
            }
        }
        rows * 3
    };
    if let Some((start, end)) = play.sel {
        let place = start == end;
        let reasons: Vec<usize> = REASONS.iter().map(|(_, label)| label.len() + 2).collect();
        let fields = 1 + u16::from(!place && play.reason == "custom");
        3 + if place { 0 } else { rows(&reasons) } + fields * 3 + 1 + rows(&[6, 11])
    } else {
        5 + rows(&[21, 4, 11])
    }
}

/// The words of the draft (or the typed text) in a card's text area, from row `first`.
fn draw_words(
    frame: &mut Frame,
    hits: &mut Vec<Hit>,
    area: Rect,
    style: Style,
    (pieces, first): (&[Piece], u16),
) {
    for piece in pieces {
        if piece.row < first || piece.row >= first + area.height {
            continue;
        }
        let w = width_of(&piece.text);
        let rect = Rect::new(area.x + piece.x, area.y + piece.row - first, w, 1);
        if rect.right() > area.right() {
            continue;
        }
        frame.render_widget(
            Paragraph::new(piece.text.clone()).style(style.patch(piece.style)),
            rect,
        );
        if let Some(click) = piece.click {
            hits.push(Hit {
                area: rect,
                action: Click::Play(click),
            });
        }
    }
}

fn draw_review(frame: &mut Frame, state: &Screen, region: Rect, hits: &mut Vec<Hit>) {
    #[derive(Clone, Copy)]
    enum Part {
        Player,
        Gap,
        Card,
        Actions,
        SelLabel,
        Sel,
        Reasons,
        Note,
        RightLabel,
        Right,
        Help,
        Insert,
        SheetActions,
    }
    let play = &state.play;
    let Some(take) = &play.take else {
        return;
    };
    let col = column(region, 76);
    let pad = pad();
    let text_w = col.width.saturating_sub(4 + 2 * extra());
    let words = freebench::words(&take.draft);
    let sheet = play.sel.is_some();
    // The draft's rows (or the typed text's), and where the selection is.
    let typed_lines: Vec<String> = play.typed.as_ref().map_or_else(Vec::new, |typed| {
        let cursor = if play.focus == Some(Field::Typed) {
            "▏"
        } else {
            ""
        };
        let mut lines: Vec<String> = visual(typed, text_w.saturating_sub(1).max(4) as usize)
            .lines()
            .map(str::to_owned)
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        if let Some(last) = lines.last_mut() {
            *last = format!("{cursor}{last}");
        }
        lines
    });
    let (pieces, word_rows) = if play.typed.is_some() {
        (Vec::new(), typed_lines.len() as u16)
    } else {
        lay_out(play, text_w)
    };
    let word_rows = word_rows.max(1);
    let help = if play.typed.is_some() {
        "Type exactly what you said."
    } else if words.is_empty() {
        "Nothing was heard. Type what you said, or toss the take."
    } else {
        ""
    };
    let help_lines: Vec<String> = if help.is_empty() {
        Vec::new()
    } else {
        textwrap::wrap(help, text_w.max(8) as usize)
            .into_iter()
            .map(|l| l.into_owned())
            .collect()
    };
    // The card: its head and the line under it, the draft, the help (when there is room and
    // no panel is open).
    let card_most = 2 + word_rows + if sheet { 0 } else { help_lines.len() as u16 };
    // The draft is what Review is for: two of its rows come before the rest (more scroll).
    let card_least = 2 + word_rows.min(2);
    let save_label = if !play.changed() {
        "All correct"
    } else if play.from_list {
        "Save"
    } else {
        "Save & next"
    };
    let actions: Vec<(Line<'static>, Color, Click, bool)> = vec![
        (
            Line::from(if play.typed.is_some() {
                "Back to marking words"
            } else {
                "Type it out instead"
            }),
            surface(),
            Click::Play(PlayClick::TypeOut),
            !play.saving,
        ),
        (
            Line::styled("Toss", Style::default().fg(THEME_RECORD)),
            surface(),
            Click::Play(PlayClick::Toss),
            !play.saving,
        ),
        (
            Line::from(save_label),
            THEME_ACCENT,
            Click::Play(PlayClick::Save),
            !play.saving,
        ),
    ];
    let action_rows = chips(None, hits, col, &actions);
    // The panel for one mark.
    let place = play.sel.is_some_and(|(s, e)| s == e);
    let reason_chips: Vec<(Line<'static>, Color, Click, bool)> = REASONS
        .iter()
        .enumerate()
        .map(|(i, (key, label))| {
            let on = *key == play.reason;
            (
                Line::from(vec![
                    Span::styled("● ", Style::default().fg(tint(key))),
                    Span::raw(*label),
                ]),
                if on { THEME_ACCENT } else { surface() },
                Click::Play(PlayClick::Reason(i)),
                true,
            )
        })
        .collect();
    let reason_rows = chips(None, hits, col, &reason_chips);
    let insert: Vec<(Line<'static>, Color, Click, bool)> = vec![
        (
            Line::from("Add word before"),
            surface(),
            Click::Play(PlayClick::Before),
            true,
        ),
        (
            Line::from("Add word after"),
            surface(),
            Click::Play(PlayClick::After),
            true,
        ),
    ];
    let insert_rows = chips(None, hits, col, &insert);
    let mut sheet_actions: Vec<(Line<'static>, Color, Click, bool)> = Vec::new();
    if play.editing.is_some() {
        sheet_actions.push((
            Line::styled("Remove", Style::default().fg(THEME_RECORD)),
            surface(),
            Click::Play(PlayClick::Remove),
            true,
        ));
    }
    sheet_actions.push((
        Line::from("Cancel"),
        surface(),
        Click::Play(PlayClick::Cancel),
        true,
    ));
    sheet_actions.push((
        Line::from(if play.editing.is_some() {
            "Update mark"
        } else {
            "Save mark"
        }),
        THEME_ACCENT,
        Click::Play(PlayClick::SaveMark),
        true,
    ));
    let sheet_rows = chips(None, hits, col, &sheet_actions);
    // Fields are as tall as the typing bar, unless the panel's controls would then not fit
    // the window: then they are one row each.
    let fields = 1 + u16::from(!place && play.reason == "custom");
    let needed = if place { 0 } else { reason_rows } + fields * (1 + 2 * pad) + sheet_rows + 1;
    let slim = pad > 0 && needed > region.height;
    let field_rows = if slim { 1 } else { 1 + 2 * pad };
    let field_pad = if slim { 0 } else { pad };
    let mut parts: Vec<(u8, u16, u16, Part)> = Vec::new();
    if sheet {
        // The draft stays above the panel (a second click there grows the selection), down to
        // one row of it; only a window too small for the panel's own controls loses it.
        parts.push((1, card_most + 2 * pad, 1 + 2 * pad, Part::Card));
        parts.push((7, 1, 1, Part::Gap));
        parts.push((5, 1, 1, Part::SelLabel));
        parts.push((3, 1, 1, Part::Sel));
        if !place {
            parts.push((8, 1, 1, Part::Gap));
            parts.push((0, reason_rows, reason_rows, Part::Reasons));
            if play.reason == "custom" {
                parts.push((0, field_rows, field_rows, Part::Note));
            }
        }
        parts.push((8, 1, 1, Part::Gap));
        parts.push((4, 1, 1, Part::RightLabel));
        parts.push((0, field_rows, field_rows, Part::Right));
        if !play.help.0.is_empty() {
            parts.push((if play.help.1 { 1 } else { 6 }, 1, 1, Part::Help));
        }
        parts.push((8, 1, 1, Part::Gap));
        if !place && play.editing.is_none() {
            parts.push((2, insert_rows, insert_rows, Part::Insert));
        }
        parts.push((0, sheet_rows, sheet_rows, Part::SheetActions));
    } else {
        parts.push((2, 3, 3, Part::Player));
        parts.push((7, 1, 1, Part::Gap));
        parts.push((0, card_most + 2 * pad, card_least + 2 * pad, Part::Card));
        parts.push((6, 1, 1, Part::Gap));
        parts.push((0, action_rows, action_rows, Part::Actions));
    }
    let parts = fit(region.height, parts);
    stack(region, parts, |part, y, rows| match part {
        Part::Gap => {}
        Part::Player => {
            let index = voice_index(take.n);
            let loaded = state.voices.get(&index);
            let voice = loaded.cloned().unwrap_or(Voice {
                path: PathBuf::new(),
                seconds: take.seconds,
                peaks: Vec::new(),
            });
            let mut mine = Vec::new();
            let redo = width_of("Redo") + if SLIM.get() { 2 } else { 4 };
            // The player and Redo side by side, centered together.
            let w = if round_theme().is_some() {
                VOICE_WIDTH
            } else {
                PEAKS as u16 + 8 + 4
            }
            .min(col.width.saturating_sub(redo + 2));
            let x = col.x + col.width.saturating_sub(w + 2 + redo) / 2;
            if let Some(dark) = round_theme() {
                draw_voice(
                    frame,
                    &mut mine,
                    state,
                    (index, &voice),
                    Rect::new(x, y, w, 3),
                    (theme_rgb().0, dark),
                );
            } else {
                let (inner, style, _) =
                    card(frame, Rect::new(x, y + 1, w, 1), (Color::Reset, surface()));
                let inner = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(1), 1);
                frame.render_widget(
                    Paragraph::new(Line::from(voice_spans(state, index, &voice))).style(style),
                    inner,
                );
                mine.push(Hit {
                    area: Rect::new(inner.x, inner.y, 3, 1),
                    action: Click::VoiceToggle(index),
                });
                for i in 0..PEAKS as u16 {
                    if inner.x + 3 + i < inner.right() {
                        mine.push(Hit {
                            area: Rect::new(inner.x + 3 + i, inner.y, 1, 1),
                            action: Click::VoiceSeek(index, i * 1000 / PEAKS as u16),
                        });
                    }
                }
            }
            // Nothing to play until the recording has come.
            if loaded.is_some() {
                hits.extend(mine);
            }
            chips(
                Some(frame),
                hits,
                Rect::new(x + w + 2, y + (3 - button_rows()) / 2, redo, button_rows()),
                &[(
                    Line::from("Redo"),
                    surface(),
                    Click::Play(PlayClick::Redo),
                    !play.saving,
                )],
            );
        }
        Part::Card => {
            let (inner, style, _) = card(
                frame,
                Rect::new(col.x, y + pad, col.width, rows - 2 * pad),
                (Color::Reset, surface()),
            );
            let inner = Rect::new(
                inner.x + 1,
                inner.y,
                inner.width.saturating_sub(2),
                inner.height,
            );
            let (label, color) = status(&take.status);
            let head = Line::from(vec![
                Span::styled(
                    format!("Take {}", take.n),
                    style.fg(Color::Gray).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{:>w$}",
                        label,
                        w = (inner.width as usize).saturating_sub(5 + take.n.to_string().len())
                    ),
                    style.fg(color),
                ),
            ]);
            // With one row only, the draft has it (under the panel, where room is short).
            let headed = inner.height >= 2;
            if headed {
                frame.render_widget(
                    Paragraph::new(head),
                    Rect::new(inner.x, inner.y, inner.width, 1),
                );
            }
            // A line under the head sets it apart from the draft, where there is a row for it.
            let ruled = headed && inner.height >= 3;
            if ruled {
                frame.render_widget(
                    Paragraph::new("─".repeat(inner.width as usize))
                        .style(style.fg(Color::DarkGray)),
                    Rect::new(inner.x, inner.y + 1, inner.width, 1),
                );
            }
            let headed = u16::from(headed) + u16::from(ruled);
            let room = inner.height - headed;
            let help_rows = if sheet {
                0
            } else {
                room.saturating_sub(word_rows).min(help_lines.len() as u16)
            };
            let shown_rows = room - help_rows;
            let body = Rect::new(inner.x, inner.y + headed, inner.width, shown_rows);
            if play.typed.is_some() {
                let first = (typed_lines.len() as u16).saturating_sub(shown_rows);
                frame.render_widget(
                    Paragraph::new(
                        typed_lines[first as usize..]
                            .iter()
                            .map(|l| Line::from(l.clone()))
                            .collect::<Vec<_>>(),
                    )
                    .style(style)
                    .alignment(Alignment::Right),
                    body,
                );
                hits.push(Hit {
                    area: body,
                    action: Click::Play(PlayClick::Focus(Field::Typed)),
                });
            } else {
                // Not all of it fits: the left two columns are kept for the arrows that
                // scroll it, and the draft is laid out in what is left.
                let narrower;
                let (pieces, word_rows, body) = if word_rows > shown_rows {
                    narrower = lay_out(play, text_w.saturating_sub(2));
                    (
                        &narrower.0,
                        narrower.1,
                        Rect::new(
                            body.x + 2,
                            body.y,
                            body.width.saturating_sub(2),
                            body.height,
                        ),
                    )
                } else {
                    (&pieces, word_rows, body)
                };
                // The rows shown: from the scroll, moved so the selection is in sight.
                let most = word_rows.saturating_sub(shown_rows);
                let mut first = (play.words_scroll as u16).min(most);
                if let Some((start, end)) = play.sel {
                    let rows: Vec<u16> = pieces
                        .iter()
                        .filter(|p| match p.click {
                            Some(PlayClick::Word(i)) => start <= i && i < end.max(start + 1),
                            _ => false,
                        })
                        .map(|p| p.row)
                        .collect();
                    if let (Some(top), Some(low)) = (rows.iter().min(), rows.iter().max()) {
                        if *top < first {
                            first = *top;
                        } else if *low >= first + shown_rows {
                            first = (low + 1).saturating_sub(shown_rows).min(most);
                        }
                    }
                }
                draw_words(frame, hits, body, style, (pieces, first));
                // More above or below: a click (or the wheel) shows it.
                for (more, up, row) in [
                    (first > 0, true, body.y),
                    (first < most, false, body.bottom().saturating_sub(1)),
                ] {
                    if more && body.height > 0 {
                        // One column each, so with a single row shown both still fit.
                        let arrow = Rect::new(body.x - 2 + u16::from(!up), row, 1, 1);
                        frame.render_widget(
                            Paragraph::new(if up { "▲" } else { "▼" }).style(style.fg(Color::Gray)),
                            arrow,
                        );
                        hits.push(Hit {
                            area: arrow,
                            action: Click::Play(PlayClick::Scroll(up)),
                        });
                    }
                }
            }
            if help_rows > 0 {
                frame.render_widget(
                    Paragraph::new(
                        help_lines[..help_rows as usize]
                            .iter()
                            .map(|l| Line::from(l.clone()))
                            .collect::<Vec<_>>(),
                    )
                    .style(style.fg(Color::Gray)),
                    Rect::new(inner.x, body.bottom(), inner.width, help_rows),
                );
            }
        }
        Part::Actions => {
            chips(
                Some(frame),
                hits,
                Rect::new(col.x, y, col.width, rows),
                &actions,
            );
        }
        Part::SelLabel => bare(
            frame,
            Rect::new(col.x, y, col.width, 1),
            Line::styled(
                if place {
                    "A word is missing here"
                } else {
                    "You marked"
                },
                Style::default().fg(Color::Gray),
            ),
            Alignment::Left,
        ),
        Part::Sel => {
            let text = match play.sel {
                Some((p, _)) if place => [
                    p.checked_sub(1).and_then(|i| words.get(i)).copied(),
                    Some("[ + ]"),
                    words.get(p).copied(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" "),
                _ => play.heard(),
            };
            let text: String = {
                let chars: Vec<char> = text.chars().collect();
                chars[..chars.len().min(col.width as usize)]
                    .iter()
                    .collect()
            };
            bare(
                frame,
                Rect::new(col.x, y, col.width, 1),
                Line::styled(shown(&text), Style::default().add_modifier(Modifier::BOLD)),
                Alignment::Right,
            );
        }
        Part::Reasons => {
            chips(
                Some(frame),
                hits,
                Rect::new(col.x, y, col.width, rows),
                &reason_chips,
            );
        }
        Part::Note => field(
            frame,
            hits,
            Rect::new(col.x, y + field_pad, col.width, 1),
            &play.note,
            "Your reason, in a few words",
            (play.focus == Some(Field::Note), false, slim),
            Click::Play(PlayClick::Focus(Field::Note)),
        ),
        Part::RightLabel => bare(
            frame,
            Rect::new(col.x, y, col.width, 1),
            Line::styled(
                if place {
                    "The missing word or words"
                } else {
                    "What it should be"
                },
                Style::default().fg(Color::Gray),
            ),
            Alignment::Left,
        ),
        Part::Right => field(
            frame,
            hits,
            Rect::new(col.x, y + field_pad, col.width, 1),
            &play.right,
            if play.reason == "weird" {
                "Nothing was said there"
            } else {
                "Type it here"
            },
            (play.focus == Some(Field::Right), true, slim),
            Click::Play(PlayClick::Focus(Field::Right)),
        ),
        Part::Help => bare(
            frame,
            Rect::new(col.x, y, col.width, 1),
            Line::styled(
                shorten(&play.help.0, col.width),
                Style::default().fg(if play.help.1 {
                    THEME_RECORD
                } else {
                    Color::Gray
                }),
            ),
            Alignment::Left,
        ),
        Part::Insert => {
            chips(
                Some(frame),
                hits,
                Rect::new(col.x, y, col.width, rows),
                &insert,
            );
        }
        Part::SheetActions => {
            chips(
                Some(frame),
                hits,
                Rect::new(col.x, y, col.width, rows),
                &sheet_actions,
            );
        }
    });
}

fn draw_list(frame: &mut Frame, state: &Screen, region: Rect, hits: &mut Vec<Hit>) {
    #[derive(Clone, Copy)]
    enum Part {
        Title,
        Gap,
        Items,
        Chip,
    }
    let play = &state.play;
    let col = column(region, 76);
    let pad = pad();
    let count = play.items.len().max(1) as u16;
    let parts = fit(
        region.height,
        vec![
            (0, 1, 1, Part::Title),
            (5, 1, 1, Part::Gap),
            (0, count + 2 * pad, 1 + 2 * pad, Part::Items),
            (4, 1, 1, Part::Gap),
            (0, button_rows(), button_rows(), Part::Chip),
        ],
    );
    stack(region, parts, |part, y, rows| match part {
        Part::Gap => {}
        Part::Title => {
            let (done, goal) = (play.done(), play.goal().unwrap_or(usize::MAX));
            let waiting = play.items.iter().filter(|t| t.status == "pending").count();
            let tossed = play.items.iter().filter(|t| t.status == "tossed").count();
            let bold = Style::default().add_modifier(Modifier::BOLD);
            let mut spans = if done >= goal {
                vec![
                    Span::styled(format!("The set of {goal} is complete."), bold),
                    Span::raw(format!(" {done} done; more is fine")),
                ]
            } else {
                vec![
                    Span::styled(done.to_string(), bold),
                    Span::raw(match play.goal() {
                        Some(goal) => format!(" of {goal} done"),
                        None => " done".into(),
                    }),
                ]
            };
            if waiting > 0 {
                spans.push(Span::raw(format!(", {waiting} to review")));
            }
            if tossed > 0 {
                spans.push(Span::raw(format!(", {tossed} tossed")));
            }
            bare(
                frame,
                Rect::new(col.x, y, col.width, 1),
                Line::from(spans).style(Style::default().fg(Color::Gray)),
                Alignment::Center,
            );
        }
        Part::Items => {
            let (inner, style, _) = card(
                frame,
                Rect::new(col.x, y + pad, col.width, rows - 2 * pad),
                (Color::Reset, surface()),
            );
            let inner = Rect::new(
                inner.x + 1,
                inner.y,
                inner.width.saturating_sub(2),
                inner.height,
            );
            if play.items.is_empty() {
                frame.render_widget(
                    Paragraph::new("No takes yet. Record the first one.")
                        .style(style.fg(Color::Gray))
                        .alignment(Alignment::Center),
                    inner,
                );
                return;
            }
            let newest: Vec<&Take> = play.items.iter().rev().collect();
            let room = inner.height as usize;
            let most = newest.len().saturating_sub(room);
            let mut first = play.list_scroll.min(most);
            // Rows for "more" when not all fit.
            let above = first > 0;
            let mut shown_rows = room - usize::from(above);
            let below = first + shown_rows < newest.len();
            if below {
                shown_rows = shown_rows.saturating_sub(1);
            }
            if shown_rows == 0 {
                first = first.min(newest.len() - 1);
                shown_rows = 1;
            }
            let mut row = inner.y;
            if above {
                let rect = Rect::new(inner.x, row, inner.width, 1);
                frame.render_widget(
                    Paragraph::new(format!("▲ {first} more"))
                        .style(style.fg(Color::Gray))
                        .alignment(Alignment::Center),
                    rect,
                );
                hits.push(Hit {
                    area: rect,
                    action: Click::Play(PlayClick::Scroll(true)),
                });
                row += 1;
            }
            for take in newest.iter().skip(first).take(shown_rows) {
                let rect = Rect::new(inner.x, row, inner.width, 1);
                let (label, color) = status(&take.status);
                let number = format!("{:>3}  ", take.n);
                let lead = width_of(&number) + 9 + 2;
                let room = inner.width.saturating_sub(lead);
                let text = if !take.final_text.is_empty() {
                    &take.final_text
                } else if !take.draft.is_empty() {
                    &take.draft
                } else {
                    "(nothing heard)"
                };
                let cut = shorten(text, room);
                let text = if cut.ends_with('…') && cut != text {
                    format!("…{}", shown(cut.trim_end_matches('…')))
                } else {
                    shown(text)
                };
                let text_style = if take.status == "tossed" {
                    style
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::CROSSED_OUT)
                } else {
                    style
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(number, style.fg(Color::Gray).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("{label:<9}  "), style.fg(color)),
                    ])),
                    rect,
                );
                frame.render_widget(
                    Paragraph::new(text)
                        .style(text_style)
                        .alignment(Alignment::Right),
                    Rect::new(inner.x + lead, row, room, 1),
                );
                hits.push(Hit {
                    area: rect,
                    action: Click::Play(PlayClick::Open(take.n)),
                });
                row += 1;
            }
            if below {
                let rest = newest.len() - first - shown_rows;
                let rect = Rect::new(inner.x, row, inner.width, 1);
                frame.render_widget(
                    Paragraph::new(format!("▼ {rest} more"))
                        .style(style.fg(Color::Gray))
                        .alignment(Alignment::Center),
                    rect,
                );
                hits.push(Hit {
                    area: rect,
                    action: Click::Play(PlayClick::Scroll(false)),
                });
            }
        }
        Part::Chip => {
            chips(Some(frame), hits, Rect::new(col.x, y, col.width, rows), &{
                let mut buttons = vec![(
                    Line::from("Record a take"),
                    if play.unsent() > 0 {
                        surface()
                    } else {
                        THEME_ACCENT
                    },
                    Click::Play(PlayClick::Record),
                    true,
                )];
                buttons.extend(play.send_button());
                buttons
            });
        }
    });
}

/// How contributing goes: it may give way in a small window.
const EXPLAIN: [&str; 6] = [
    "3Yiddish gets better only by being tested on real speech. You can help by recording yourself.",
    "",
    "1. Say something in your own words. An idea is shown if you want one.",
    "2. The app writes down what it heard.",
    "3. Click any word it got wrong and fix it, or say that all of it is correct.",
    "4. Press Send when you have a few.",
];
/// What accepting agrees to (`phone::CONSENT` numbers this text): always shown whole, or
/// there is no I accept.
const TERMS: [&str; 3] = [
    "Your takes stay on this computer until you press Send. What you send is used to test, and maybe later to improve, Yiddish speech to text, and may be published as a test set under any name you choose.",
    "",
    "Accepting also switches on sharing: what you transcribe in the rest of the app, your recordings and files with the text the app wrote and technical facts, is sent to 3Yiddish, kept privately, and used to test and maybe later improve it. You can switch it off any time with Sharing at the top, or: 3yiddish share off",
];
/// What contributing means, with Never mind and I accept under it. Shown until accepted.
pub(super) fn draw_consent(frame: &mut Frame, region: Rect, hits: &mut Vec<Hit>) {
    question(
        frame,
        region,
        hits,
        "Help test Yiddish speech to text",
        (&EXPLAIN, &TERMS),
        [
            (
                Line::from("Never mind"),
                surface(),
                Click::Play(PlayClick::Decline),
                true,
            ),
            (
                Line::from("I accept"),
                THEME_ACCENT,
                Click::Play(PlayClick::Accept),
                true,
            ),
        ],
    );
}
/// The sharing card: the first-run question (No thanks is the main button: the default is
/// no), or what is being shared with Stop sharing.
pub(super) fn draw_share(frame: &mut Frame, card: ShareCard, region: Rect, hits: &mut Vec<Hit>) {
    match card {
        ShareCard::Ask => question(
            frame,
            region,
            hits,
            "Share what you transcribe?",
            (&[], &[crate::share::WHAT, "", crate::share::OFF]),
            [
                (
                    Line::from("Yes, share"),
                    surface(),
                    Click::Play(PlayClick::Share(true)),
                    true,
                ),
                (
                    Line::from("No thanks"),
                    THEME_ACCENT,
                    Click::Play(PlayClick::Share(false)),
                    true,
                ),
            ],
        ),
        ShareCard::On => question(
            frame,
            region,
            hits,
            "Sharing is on",
            (
                &[],
                &[
                    crate::share::WHAT,
                    "",
                    "Stop sharing also deletes what is still waiting to be sent.",
                ],
            ),
            [
                (
                    Line::from("Keep sharing"),
                    surface(),
                    Click::Play(PlayClick::ShareClose),
                    true,
                ),
                (
                    Line::from("Stop sharing"),
                    THEME_ACCENT,
                    Click::Play(PlayClick::Share(false)),
                    true,
                ),
            ],
        ),
    }
}
/// A question on a card, centered, with two buttons under it: a bold title, then `extra`
/// paragraphs (which give way first in a small window) and `terms` (never cut: where they
/// do not fit whole, the first button, the one that agrees, is not offered).
fn question(
    frame: &mut Frame,
    region: Rect,
    hits: &mut Vec<Hit>,
    title: &str,
    (extra_text, terms): (&[&str], &[&str]),
    buttons: [(Line<'static>, Color, Click, bool); 2],
) {
    if region.height < 3 || region.width < 20 {
        return;
    }
    let col = column(region, 64);
    let width = col.width.saturating_sub(4 + 2 * extra()).max(8) as usize;
    let wrap = |paragraphs: &[&str]| -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for paragraph in paragraphs {
            // A numbered step's later rows line up under its words.
            let step = paragraph.as_bytes().get(1) == Some(&b'.');
            let options =
                textwrap::Options::new(width).subsequent_indent(if step { "   " } else { "" });
            for line in textwrap::wrap(paragraph, options) {
                lines.push(Line::from(line.into_owned()));
            }
        }
        lines
    };
    let head = vec![
        Line::styled(
            title.to_owned(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    let (mut more, must) = (wrap(extra_text), wrap(terms));
    if !more.is_empty() {
        more.push(Line::default());
    }
    let pad = pad();
    let full = (head.len() + more.len() + must.len()) as u16;
    // The explanation gives way before the buttons go thin.
    SLIM.set(false);
    SLIM.set(region.height < (head.len() + must.len()) as u16 + 2 * pad + 1 + 3);
    let room = region.height.saturating_sub(2 * pad + 1 + button_rows());
    // The explanation gives way first, then the title; the terms never do.
    let mut lines = Vec::new();
    if full <= room {
        lines.extend(head);
        lines.extend(more);
    } else if ((head.len() + must.len()) as u16) <= room {
        lines.extend(head);
    }
    lines.extend(must);
    // Where not all that is agreed to can be read, there is no way to agree.
    let fits = lines.len() as u16 <= room;
    let agrees = |b: &(Line, Color, Click, bool)| {
        matches!(b.2, Click::Play(PlayClick::Accept | PlayClick::Share(true)))
    };
    let buttons: Vec<_> = buttons.into_iter().filter(|b| fits || !agrees(b)).collect();
    let words = (lines.len() as u16).min(room);
    let total = words + 2 * pad + 1 + button_rows();
    let y = region.y + region.height.saturating_sub(total) / 2 + pad;
    if words > 0 {
        let (inner, style, _) = card(
            frame,
            Rect::new(col.x, y, col.width, words),
            (Color::Reset, surface()),
        );
        let inner = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        frame.render_widget(Paragraph::new(lines).style(style), inner);
    }
    let row = y + words + pad + 1;
    if row + button_rows() <= region.bottom() {
        chips(
            Some(frame),
            hits,
            Rect::new(col.x, row, col.width, button_rows()),
            &buttons,
        );
    }
}

/// Send: what it means, the name the takes go under, and Send or Not now.
fn draw_send(frame: &mut Frame, state: &Screen, region: Rect, hits: &mut Vec<Hit>) {
    let play = &state.play;
    let pad = pad();
    let col = column(region, 60);
    let width = col.width.saturating_sub(4 + 2 * extra()).max(8) as usize;
    let n = play.unsent();
    let mut lines = vec![
        Line::styled(
            if n == 1 {
                "Send 1 take to 3Yiddish?".to_owned()
            } else {
                format!("Send {n} takes to 3Yiddish?")
            },
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    let what = "The recordings and the text you fixed are used to test, and maybe later to \
                improve, Yiddish speech to text. They may be published as a test set, with \
                the name you give here. Any name will do.";
    lines.extend(
        textwrap::wrap(what, width)
            .into_iter()
            .map(|l| Line::from(l.into_owned())),
    );
    let buttons = [
        (
            Line::from("Not now"),
            surface(),
            Click::Play(PlayClick::SendCancel),
            !play.sending,
        ),
        (
            Line::from(if play.sending { "Sending" } else { "Send" }),
            THEME_ACCENT,
            Click::Play(PlayClick::SendNow),
            !play.sending,
        ),
    ];
    // The words give way before the name and the buttons do.
    let fixed = 2 * pad + 1 + 3 + 1 + button_rows();
    let words = (lines.len() as u16).min(region.height.saturating_sub(fixed));
    let total = words + fixed;
    let mut y = region.y + region.height.saturating_sub(total) / 2 + pad;
    if words > 0 {
        let (inner, style, _) = card(
            frame,
            Rect::new(col.x, y, col.width, words),
            (Color::Reset, surface()),
        );
        let inner = Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height,
        );
        frame.render_widget(Paragraph::new(lines).style(style), inner);
    }
    y += words + pad + 1;
    if y + 3 <= region.bottom() {
        field(
            frame,
            hits,
            Rect::new(col.x, y + 1, col.width, 1),
            &play.name,
            "Your name",
            (!play.sending, false, false),
            Click::Play(PlayClick::SendOpen),
        );
    }
    y += 3 + 1;
    if y + button_rows() <= region.bottom() {
        chips(
            Some(frame),
            hits,
            Rect::new(col.x, y, col.width, button_rows()),
            &buttons,
        );
    }
}

/// "Record take N again?" before a reviewed take's recording is thrown away.
fn draw_ask(frame: &mut Frame, state: &Screen, region: Rect, hits: &mut Vec<Hit>) {
    let Some(take) = &state.play.take else {
        return;
    };
    let pad = pad();
    let col = column(region, 50);
    let text = format!(
        "Record take {} again? The recording you have now is deleted.",
        take.n
    );
    let lines: Vec<Line> = textwrap::wrap(&text, col.width.saturating_sub(6).max(8) as usize)
        .into_iter()
        .map(|l| Line::from(l.into_owned()))
        .collect();
    // The question on a card, with a row of air above and below it where the card has no half
    // rows of its own; the buttons under the card, on the background like the others (so Keep
    // it shows as a button, not as words on the card).
    let words = lines.len() as u16;
    let h = words + 2 * (1 - pad);
    let total = h + 2 * pad + 1 + button_rows();
    let y = region.y + region.height.saturating_sub(total) / 2 + pad;
    let (inner, style, _) = card(
        frame,
        Rect::new(col.x, y, col.width, h.min(region.height)),
        (Color::Reset, surface()),
    );
    let inner = Rect::new(
        inner.x + 1,
        inner.y + (1 - pad),
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(2 * (1 - pad)),
    );
    frame.render_widget(
        Paragraph::new(lines)
            .style(style)
            .alignment(Alignment::Center),
        inner,
    );
    let buttons = [
        (
            Line::from("Keep it"),
            surface(),
            Click::Play(PlayClick::AskNo),
            true,
        ),
        (
            Line::from("Record again"),
            THEME_RECORD,
            Click::Play(PlayClick::AskYes),
            true,
        ),
    ];
    let row = y + h + pad + 1;
    if row + button_rows() <= region.bottom() {
        chips(
            Some(frame),
            hits,
            Rect::new(col.x, row, col.width, button_rows()),
            &buttons,
        );
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::tui::tests::rendered_text;

    const LONG: &str = "ווען מען רעדט אריין אין מיקראפאן, שרייבט עס זיך אויס דא אויף דער רעכטער זייט, שורה נאך שורה.";

    fn take(n: u32, draft: &str, status: &str, marks: Vec<Mark>) -> Take {
        let final_text = freebench::apply(draft, &marks);
        serde_json::from_value(json!({
            "n": n, "audio": format!("audio/{n:03}.wav"), "seconds": 4.2,
            "created": "2026-11-12T15:04:05Z", "updated": "2026-11-12T15:04:05Z",
            "draft": draft, "draft_model": {"name": "test", "engine": "cpu"},
            "final": final_text, "status": status, "marks": marks, "held_out": true,
        }))
        .unwrap()
    }
    fn mark(start: usize, end: usize, right: Option<&str>, reason: &str) -> Mark {
        Mark {
            start,
            end,
            heard: String::new(),
            right: right.map(str::to_owned),
            reason: reason.into(),
            note: String::new(),
        }
    }
    /// A screen on the playground's Review of a long take with two marks.
    fn reviewing() -> Screen {
        let mut state = Screen::default();
        state.playground = true;
        state.play.view = true;
        state.play.loaded = true;
        state.play.idea = "וואס האסטו היינט געטון?".into();
        let marks = vec![
            mark(2, 4, Some("רעדט"), "wrong"),
            mark(9, 9, Some("אהער"), "dropped"),
        ];
        let reviewed = take(7, LONG, "pending", Vec::new());
        state.play.items = vec![
            take(5, "א גוטן טאג", "correct", Vec::new()),
            take(6, "", "tossed", Vec::new()),
            reviewed.clone(),
        ];
        let (jobs, _rx) = mpsc::unbounded_channel();
        state.play.open_review(reviewed, false, &jobs);
        state.play.marks = marks;
        // Its recording has come (made up: nothing to hear in a test).
        state.voices.insert(
            voice_index(7),
            Voice {
                path: PathBuf::new(),
                seconds: 4.2,
                peaks: (0..48).map(|i| (i * 37 % 256) as u8).collect(),
            },
        );
        state
    }
    type Term = ratatui::Terminal<ratatui::backend::TestBackend>;
    fn draw(state: &Screen, (w, h): (u16, u16)) -> (Term, Vec<Hit>) {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        let mut hits = Vec::new();
        term.draw(|f| hits = draw_plain(f, state)).unwrap();
        (term, hits)
    }
    /// Every word of `paragraphs` is on the screen, in order.
    fn shows(text: &str, paragraphs: &[&str]) -> bool {
        let mut on_screen = text.split_whitespace();
        paragraphs
            .iter()
            .flat_map(|p| p.split_whitespace())
            .all(|word| on_screen.any(|w| w == word))
    }
    /// The text in the cells under a hit box.
    fn under(term: &Term, area: Rect) -> String {
        let buffer = term.backend().buffer();
        (area.x..area.right())
            .map(|x| buffer[(x, area.y)].symbol().to_owned())
            .collect()
    }

    /// Every word's hit box carries its own number from `freebench::words`, also on the second
    /// and third line of a draft that wraps: the cells under it show that very word, and a
    /// click there gives that number.
    #[test]
    fn words_keep_their_number_across_wrapped_lines() {
        let words = freebench::words(LONG);
        for size in [(40, 40), (42, 24), (43, 23), (44, 22), (60, 24), (100, 30)] {
            let mut state = reviewing();
            state.play.marks.clear();
            let mut seen = std::collections::BTreeSet::new();
            let mut rows = std::collections::BTreeSet::new();
            // Where the draft does not fit, the rest comes into sight by scrolling.
            for _ in 0..words.len() {
                let (term, hits) = draw(&state, size);
                for hit in &hits {
                    if let Click::Play(PlayClick::Word(i)) = hit.action {
                        assert_eq!(under(&term, hit.area), shown(words[i]), "{size:?} word {i}");
                        let middle = (hit.area.x + hit.area.width / 2, hit.area.y);
                        assert_eq!(
                            hit_at(&hits, middle.0, middle.1),
                            Some(Click::Play(PlayClick::Word(i)))
                        );
                        seen.insert(i);
                        rows.insert(hit.area.y);
                    }
                }
                if seen.len() == words.len() {
                    break;
                }
                scroll(&mut state, false);
            }
            assert_eq!(
                seen.len(),
                words.len(),
                "{size:?}: every word can be clicked"
            );
            assert!(rows.len() >= 2, "{size:?}: the draft wraps");
            let (_, hits) = {
                let mut state = reviewing();
                state.play.marks.clear();
                draw(&state, size)
            };
            // Right to left: the first word is the rightmost on the first row.
            let first = hits
                .iter()
                .find(|h| h.action == Click::Play(PlayClick::Word(0)))
                .unwrap()
                .area;
            for hit in &hits {
                if let Click::Play(PlayClick::Word(i)) = hit.action
                    && hit.area.y == first.y
                    && i > 0
                {
                    assert!(
                        hit.area.x < first.x,
                        "{size:?}: word {i} is left of the first"
                    );
                }
            }
        }
    }

    /// A marked span shows its words struck through with the correction beside them and its
    /// tag under them, all opening the same mark; a missing word shows where it goes.
    #[test]
    fn marks_are_tinted_and_open_their_mark() {
        let state = reviewing();
        let (term, hits) = draw(&state, (80, 30));
        let text = rendered_text(&term);
        assert!(text.contains(&shown("רעדט")), "{text}");
        assert!(
            text.contains("Wrong word") && text.contains("missing"),
            "{text}"
        );
        assert!(
            text.contains("Save & next") && !text.contains("All correct"),
            "{text}"
        );
        let crossed = hits
            .iter()
            .find(|h| h.action == Click::Play(PlayClick::Word(2)))
            .unwrap()
            .area;
        let cell = &term.backend().buffer()[(crossed.x, crossed.y)];
        assert!(cell.modifier.contains(Modifier::CROSSED_OUT));
        assert_eq!(
            hits.iter()
                .filter(|h| h.action == Click::Play(PlayClick::Mark(0)))
                .count(),
            2,
            "the correction and the tag"
        );
        assert!(
            hits.iter()
                .any(|h| h.action == Click::Play(PlayClick::Mark(1)))
        );
        // The saved text is the draft with the marks applied.
        assert_eq!(
            state.play.final_text(),
            freebench::apply(LONG, &state.play.marks)
        );
    }

    /// The panel for a mark works as on the phone: a click selects a word, a second one grows
    /// the selection to a span, the same word again lets go, a span may not run into another
    /// mark, and a click on a marked word opens that mark.
    #[test]
    fn selection_grows_to_a_span_and_marks_are_saved_as_on_the_phone() {
        let mut state = reviewing();
        let play = &mut state.play;
        play.marks = vec![mark(8, 9, Some("זאגט"), "wrong")];
        play.tap_word(3);
        assert_eq!((play.sel, play.editing), (Some((3, 4)), None));
        assert_eq!(play.right, "אריין");
        assert_eq!(play.focus, Some(Field::Right));
        play.tap_word(3);
        assert_eq!(play.sel, None, "the same word again lets go");
        play.tap_word(3);
        play.pick_reason("weird");
        assert_eq!(play.right, "", "weird starts empty");
        play.tap_word(5);
        assert_eq!(play.sel, Some((3, 6)));
        assert_eq!(play.reason, "weird", "growing keeps the chosen reason");
        play.tap_word(1);
        assert_eq!(play.sel, Some((1, 6)));
        // Over the saved mark at word 8: refused, the selection stays.
        play.tap_word(10);
        assert_eq!(play.sel, Some((1, 6)));
        assert!(play.help.1 && play.help.0.contains("another mark"));
        // Weird with nothing typed: nothing was said there.
        play.save_mark();
        assert_eq!(play.sel, None);
        assert_eq!(play.marks[0].start, 1);
        assert_eq!(play.marks[0].right.as_deref(), Some(""));
        // Wrong word: a correction is needed, and it must differ from what was written.
        play.tap_word(0);
        play.save_mark();
        assert!(play.help.1 && play.help.0.contains("That is what was written"));
        play.right = String::new();
        play.save_mark();
        assert!(play.help.1 && play.help.0.contains("Type what it should be"));
        play.right = "אז".into();
        play.save_mark();
        assert_eq!(play.marks.len(), 3);
        // A custom reason needs its note; the chip moves the keyboard there.
        play.tap_word(12);
        play.pick_reason("custom");
        assert_eq!(play.focus, Some(Field::Note));
        play.save_mark();
        assert!(play.help.0.contains("reason in a few words"));
        play.note = " a name I made up ".into();
        play.save_mark();
        let custom = play.marks.iter().find(|m| m.reason == "custom").unwrap();
        assert_eq!(custom.note, "a name I made up");
        // A marked word opens its mark; Remove takes it away.
        play.tap_word(4);
        assert_eq!(play.editing, Some(1));
        assert_eq!(play.sel, Some((1, 6)));
        let (jobs, _rx) = mpsc::unbounded_channel();
        click(&mut state, PlayClick::Remove, &jobs).unwrap();
        assert_eq!(state.play.marks.len(), 3);
        assert!(state.play.marks.iter().all(|m| m.start != 1));
        // Add word after: a place right after the span; its text is needed.
        let play = &mut state.play;
        play.tap_word(6);
        play.add_place(7);
        assert_eq!((play.sel, play.reason), (Some((7, 7)), "dropped"));
        play.save_mark();
        assert!(play.help.0.contains("missing word"));
        play.right = "  גוט ".into();
        play.save_mark();
        let dropped = play
            .marks
            .iter()
            .find(|m| m.start == 7 && m.end == 7)
            .unwrap();
        assert_eq!(dropped.right.as_deref(), Some("גוט"));
        // A place inside a mark is refused.
        play.marks.push(mark(10, 12, Some("x"), "name"));
        play.marks.sort_by_key(|m| (m.start, m.end));
        play.close_sheet();
        play.add_place(11);
        assert!(play.sel.is_none() && play.help.1);
        // The marks are in order and the service would take them.
        let checked = freebench::check(LONG, play.marks.clone()).unwrap();
        assert_eq!(checked.len(), play.marks.len());
    }

    /// A take reopened from the list, with a mark, in a small window: a click on a word opens
    /// the panel for that word.
    #[test]
    fn a_reopened_take_is_clickable_in_a_small_window() {
        let draft = "דאס איז א פרובע פון דעם עקראן, נישט קיין אמת'ע איבערשרייבונג.";
        for size in [(42, 24), (44, 22), (60, 24)] {
            let mut state = reviewing();
            let reopened = take(1, draft, "fixed", vec![mark(6, 7, Some("עקראן"), "wrong")]);
            state.play.items = vec![reopened.clone()];
            let (jobs, _rx) = mpsc::unbounded_channel();
            state.play.open_review(reopened, true, &jobs);
            let (_, hits) = draw(&state, size);
            let word = hits
                .iter()
                .find(|h| h.action == Click::Play(PlayClick::Word(0)))
                .unwrap_or_else(|| panic!("{size:?}: word 0 has no hit box"))
                .area;
            let action = hit_at(&hits, word.x + word.width / 2, word.y).unwrap();
            let Click::Play(action) = action else {
                panic!("{size:?}: {action:?}")
            };
            click(&mut state, action, &jobs).unwrap();
            assert_eq!(state.play.sel, Some((0, 1)), "{size:?}");
            let (term, hits) = draw(&state, size);
            assert!(
                hits.iter()
                    .any(|h| h.action == Click::Play(PlayClick::SaveMark)),
                "{size:?}\n{}",
                rendered_text(&term)
            );
        }
    }

    #[test]
    fn clashes_match_the_phone_page() {
        let marks = vec![
            mark(1, 3, Some("x"), "wrong"),
            mark(4, 4, Some("y"), "dropped"),
        ];
        assert!(!clashes(&marks, 0, 1, None));
        assert!(!clashes(&marks, 3, 4, None));
        assert!(clashes(&marks, 0, 2, None));
        assert!(clashes(&marks, 2, 5, None));
        assert!(clashes(&marks, 3, 5, None));
        assert!(!clashes(&marks, 1, 1, None));
        assert!(!clashes(&marks, 3, 3, None));
        assert!(clashes(&marks, 2, 2, None));
        assert!(clashes(&marks, 4, 4, None));
        assert!(!clashes(&marks, 1, 3, Some(0)));
    }

    /// In a field Space types a space and never records, Esc leaves the panel, Enter saves the
    /// mark; with no field, Space records on the Record screen only.
    #[test]
    fn typing_in_a_field_never_records() {
        let mut state = reviewing();
        let (jobs, mut asked) = mpsc::unbounded_channel();
        let press = |state: &mut Screen, code: KeyCode| {
            key(state, KeyEvent::new(code, KeyModifiers::NONE), &jobs)
        };
        state.play.tap_word(0);
        state.play.right.clear();
        for c in "אז ער".chars() {
            assert!(press(&mut state, KeyCode::Char(c)));
        }
        assert_eq!(state.play.right, "אז ער");
        assert!(state.play.touched);
        assert!(press(&mut state, KeyCode::Enter));
        assert_eq!(state.play.marks.iter().filter(|m| m.start == 0).count(), 1);
        assert_eq!(state.play.sel, None);
        state.play.tap_word(5);
        assert!(press(&mut state, KeyCode::Esc));
        assert_eq!((state.play.sel, state.play.focus), (None, None));
        // Review without a field: Space does not record; Esc goes back to Record.
        assert!(press(&mut state, KeyCode::Char(' ')));
        assert!(press(&mut state, KeyCode::Esc));
        assert_eq!(state.play.page, Page::Record);
        // Record: Space goes on to the recording.
        assert!(!press(&mut state, KeyCode::Char(' ')));
        // Typed out: Space types; Enter saves the take.
        let reviewed = state.play.items[2].clone();
        state.play.open_review(reviewed, true, &jobs);
        state.play.type_out();
        assert_eq!(state.play.focus, Some(Field::Typed));
        state.play.typed = Some(String::new());
        for c in "א ב".chars() {
            assert!(press(&mut state, KeyCode::Char(c)));
        }
        assert_eq!(state.play.typed.as_deref(), Some("א ב"));
        while asked.try_recv().is_ok() {}
        assert!(press(&mut state, KeyCode::Enter));
        match asked.try_recv() {
            Ok(Job::Review(7, review)) => {
                assert_eq!(review["typed"], "א ב");
                assert_eq!(review["toss"], false);
            }
            _ => panic!("Enter in the typed text saves the take"),
        }
        // Refresh and quit keys go on to the app.
        let control = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert!(!key(&mut state, control, &jobs));
    }

    /// Every screen, in each look the text tiers have (plain characters, a Nerd Font, the
    /// phone look with and without one), from the smallest window up: controls stay on the
    /// screen and never sit on each other, the playground's button is always there, and the
    /// main controls show when there is room.
    #[test]
    fn every_screen_fits_every_size_and_look() {
        let mut screens: Vec<(&str, Screen)> = Vec::new();
        let mut record = reviewing();
        record.play.page = Page::Record;
        record.play.take = None;
        screens.push(("record", record));
        let mut talking = reviewing();
        talking.play.page = Page::Record;
        talking.play.take = None;
        talking.play.recording = true;
        talking.demo_rec = Some(Instant::now());
        talking.rec_started = Some(Instant::now());
        // A voice from quiet to loud, so the wave has every height.
        talking.levels = (0..60).map(|i| i as f32 / 60.0).collect();
        screens.push(("recording", talking));
        let mut thinking = reviewing();
        thinking.play.page = Page::Record;
        thinking.play.take = None;
        thinking.play.thinking = true;
        screens.push(("thinking", thinking));
        screens.push(("review", reviewing()));
        let mut sheet = reviewing();
        sheet.play.tap_word(5);
        sheet.play.tap_word(7);
        screens.push(("sheet", sheet));
        let mut custom = reviewing();
        custom.play.tap_word(0);
        custom.play.pick_reason("custom");
        screens.push(("sheet-custom", custom));
        let mut typed = reviewing();
        typed.play.type_out();
        screens.push(("typed", typed));
        let mut list = reviewing();
        list.play.page = Page::List;
        list.play.take = None;
        for n in 8..40u32 {
            let status = ["pending", "fixed", "correct"][n as usize % 3];
            list.play.items.push(take(n, LONG, status, Vec::new()));
        }
        screens.push(("list", list));
        let mut ask = reviewing();
        ask.play.ask = true;
        screens.push(("ask", ask));
        let mut consent = reviewing();
        consent.play.view = false;
        consent.play.consent = true;
        screens.push(("consent", consent));
        for (name, card) in [("share-ask", ShareCard::Ask), ("share-on", ShareCard::On)] {
            let mut share = reviewing();
            share.play.view = false;
            share.sharing = card == ShareCard::On;
            share.share_card = Some(card);
            screens.push((name, share));
        }
        let dump = std::env::var("TUI_LAYOUT_DUMP_DIR").ok();
        for (phone, nerd) in [(false, false), (false, true), (true, false), (true, true)] {
            PHONE_LOOK.with(|o| o.set(phone));
            NERD_FOR_TEST.with(|n| n.set(Some(nerd)));
            for size in [(40, 16), (44, 22), (60, 24), (80, 30), (120, 40), (160, 50)] {
                for (name, state) in &screens {
                    let at = format!("{name} {size:?} phone={phone} nerd={nerd}");
                    let (term, hits) = draw(state, size);
                    let text = rendered_text(&term);
                    for (i, a) in hits.iter().enumerate() {
                        assert!(
                            a.area.right() <= size.0 && a.area.bottom() <= size.1,
                            "{at}: {:?} off the screen\n{text}",
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
                    let has =
                        |action: PlayClick| hits.iter().any(|h| h.action == Click::Play(action));
                    assert!(
                        has(PlayClick::Toggle)
                            || ["recording", "thinking", "share-ask", "share-on"].contains(name),
                        "{at}: the playground's button\n{text}"
                    );
                    if phone {
                        assert!(
                            !text.contains(['▄', '▀', '▐', '▌', '▗', '▖', '▝', '▘', '▔']),
                            "{at}: block shapes\n{text}"
                        );
                    }
                    let roomy = size.1 >= 24;
                    match *name {
                        "record" => {
                            assert!(text.contains("Take 2"), "{at}\n{text}");
                            assert!(!roomy || has(PlayClick::List), "{at}\n{text}");
                            assert!(!roomy || has(PlayClick::Idea), "{at}\n{text}");
                            assert!(hits.iter().any(|h| h.action == Click::Record), "{at}");
                        }
                        "thinking" => assert!(
                            !roomy || text.contains("Writing down what you said"),
                            "{at}\n{text}"
                        ),
                        "review" => {
                            assert!(has(PlayClick::Save) && has(PlayClick::Toss), "{at}\n{text}");
                            assert!(has(PlayClick::TypeOut), "{at}\n{text}");
                            assert!(has(PlayClick::Word(0)), "{at}\n{text}");
                            assert!(!roomy || has(PlayClick::Redo), "{at}\n{text}");
                            assert!(!hits.iter().any(|h| h.action == Click::Record), "{at}");
                        }
                        "sheet" | "sheet-custom" => {
                            assert!(
                                has(PlayClick::SaveMark) && has(PlayClick::Cancel),
                                "{at}\n{text}"
                            );
                            assert!(has(PlayClick::Focus(Field::Right)), "{at}\n{text}");
                            assert!(
                                has(PlayClick::Reason(0)) && has(PlayClick::Reason(7)),
                                "{at}\n{text}"
                            );
                            // Only the smallest window gives the draft up for the panel.
                            assert!(
                                size.1 < 22
                                    || hits.iter().any(|h| {
                                        matches!(h.action, Click::Play(PlayClick::Word(_)))
                                    }),
                                "{at}: the draft stays clickable above the panel\n{text}"
                            );
                            if *name == "sheet-custom" {
                                assert!(has(PlayClick::Focus(Field::Note)), "{at}\n{text}");
                            }
                        }
                        "list" => {
                            assert!(
                                has(PlayClick::Record) && has(PlayClick::Open(39)),
                                "{at}\n{text}"
                            );
                            // 35 takes: all of them fit only the largest window.
                            assert!(
                                size.1 >= 50 || has(PlayClick::Scroll(false)),
                                "{at}: more below\n{text}"
                            );
                        }
                        "ask" => assert!(
                            has(PlayClick::AskYes) && has(PlayClick::AskNo),
                            "{at}\n{text}"
                        ),
                        // A way to agree only with all that is agreed to on the screen, and
                        // always one to say no.
                        "consent" => {
                            assert!(has(PlayClick::Decline), "{at}\n{text}");
                            assert!(!roomy || has(PlayClick::Accept), "{at}\n{text}");
                            if has(PlayClick::Accept) {
                                assert!(shows(&text, &TERMS), "{at}\n{text}");
                            }
                        }
                        "share-ask" => {
                            assert!(has(PlayClick::Share(false)), "{at}\n{text}");
                            assert!(!roomy || has(PlayClick::Share(true)), "{at}\n{text}");
                            if has(PlayClick::Share(true)) {
                                assert!(
                                    shows(&text, &[crate::share::WHAT, crate::share::OFF]),
                                    "{at}\n{text}"
                                );
                            }
                        }
                        "share-on" => assert!(
                            has(PlayClick::Share(false)) && has(PlayClick::ShareClose),
                            "{at}\n{text}"
                        ),
                        _ => {}
                    }
                    if let Some(folder) = &dump {
                        std::fs::create_dir_all(folder).unwrap();
                        std::fs::write(
                            Path::new(folder).join(format!(
                                "playground-{name}-{}x{}{}{}.txt",
                                size.0,
                                size.1,
                                if phone { "-phone" } else { "" },
                                if nerd { "-nerd" } else { "" }
                            )),
                            &text,
                        )
                        .unwrap();
                    }
                }
            }
        }
        NERD_FOR_TEST.with(|n| n.set(None));
        PHONE_LOOK.with(|o| o.set(false));
        // Without the playground opened, only its button is there.
        let plain = Screen::default();
        let (term, hits) = draw(&plain, (120, 30));
        assert!(
            hits.iter()
                .all(|h| !matches!(h.action, Click::Play(action) if action != PlayClick::Toggle))
        );
        assert!(!rendered_text(&term).contains("Take "));
    }

    /// The Sharing chip shows only while sharing is on, and opens its card; the explanation
    /// accepted before sharing existed is not taken as accepting it.
    #[test]
    fn sharing_shows_in_the_top_bar_only_while_on() {
        for (phone, nerd) in [(false, false), (false, true), (true, false), (true, true)] {
            PHONE_LOOK.with(|o| o.set(phone));
            NERD_FOR_TEST.with(|n| n.set(Some(nerd)));
            for size in [(80, 30), (120, 40)] {
                let mut state = Screen::default();
                let opens = |hits: &[Hit]| {
                    hits.iter()
                        .any(|h| h.action == Click::Play(PlayClick::ShareOpen))
                };
                assert!(!opens(&draw(&state, size).1));
                state.sharing = true;
                let (term, hits) = draw(&state, size);
                assert!(opens(&hits), "{size:?}\n{}", rendered_text(&term));
                assert!(rendered_text(&term).contains("Sharing"));
            }
        }
        let dir = crate::share::tests::Scratch::new();
        let file = dir.path().join("contribute-accepted");
        std::fs::write(&file, "1").unwrap();
        assert!(!crate::phone::accepted(&file));
        std::fs::write(&file, crate::phone::CONSENT).unwrap();
        assert!(crate::phone::accepted(&file));
        assert!(
            TERMS
                .concat()
                .contains("Accepting also switches on sharing")
        );
        assert!(TERMS.concat().contains("3yiddish share off"));
    }

    /// A phone held upright has room for all of the terms, so it can agree; a window too
    /// small for them offers no way to (and Enter, which goes by what is offered, neither).
    #[test]
    fn agreeing_needs_all_of_the_terms_on_the_screen() {
        for (phone, nerd) in [(false, false), (false, true), (true, false), (true, true)] {
            PHONE_LOOK.with(|o| o.set(phone));
            NERD_FOR_TEST.with(|n| n.set(Some(nerd)));
            let mut consent = Screen::default();
            consent.play.consent = true;
            let mut ask = Screen::default();
            ask.share_card = Some(ShareCard::Ask);
            for (size, room) in [((44, 40), true), ((44, 22), false), ((40, 16), false)] {
                let at = format!("{size:?} phone={phone} nerd={nerd}");
                let (term, hits) = draw(&consent, size);
                let text = rendered_text(&term);
                assert_eq!(offered(&hits, PlayClick::Accept), room, "{at}\n{text}");
                assert!(offered(&hits, PlayClick::Decline), "{at}\n{text}");
                if room {
                    assert!(shows(&text, &TERMS), "{at}\n{text}");
                }
                let (term, hits) = draw(&ask, size);
                let text = rendered_text(&term);
                assert_eq!(offered(&hits, PlayClick::Share(true)), room, "{at}\n{text}");
                assert!(offered(&hits, PlayClick::Share(false)), "{at}\n{text}");
                if room {
                    assert!(
                        shows(&text, &[crate::share::WHAT, crate::share::OFF]),
                        "{at}\n{text}"
                    );
                }
            }
        }
    }

    /// Refresh keeps the choice of playground or chat; a workspace from before (or one where
    /// the playground is off) opens the playground first.
    #[test]
    fn refresh_keeps_the_playground_or_the_chat() {
        let mut state = Screen::default();
        state.playground = true;
        state.play.view = false;
        state.plain_log = vec![(true, String::new())];
        for (index, file) in [(0, "/tmp/chat.wav"), (voice_index(7), "/tmp/take.wav")] {
            state.voices.insert(
                index,
                Voice {
                    path: PathBuf::from(file),
                    seconds: 1.0,
                    peaks: vec![],
                },
            );
        }
        let resume = Resume::capture(&state);
        assert_eq!(resume.playground_view, Some(false));
        assert_eq!(
            resume.voices.len(),
            1,
            "a take's recording is not carried over"
        );
        let old: Resume = serde_json::from_value(json!({"input": "x"})).unwrap();
        assert_eq!(old.playground_view, None);
    }

    /// The app's way to the takes is the phone page's own: a take sent from here is in the
    /// state as the phone sees it, a review saved from here is stored with its marks and its
    /// final text, and "another idea" changes the idea for both.
    #[tokio::test]
    async fn the_app_and_the_phone_share_the_takes_and_the_idea() {
        use tower::ServiceExt;
        let folder = std::env::temp_dir().join(format!("3y-play-{}", uuid::Uuid::new_v4()));
        let config = || {
            crate::config::Config::load_or_default(Path::new("/nonexistent/config.toml")).unwrap()
        };
        let mut app = crate::service::App::new(config()).unwrap();
        // Demo drafts: no model is run here.
        app.phone = crate::phone::PairHub::at(&folder).switched_on().demo();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = crate::service::router(app.clone());
        let served = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let code = "0123456789ABCDEFGHIJ";
        let page = crate::phone::gateway(app.clone(), code);
        let phone = |path: &'static str, method: &'static str| {
            let page = page.clone();
            async move {
                let request = axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/{code}/api/{path}"))
                    .body(axum::body::Body::empty())
                    .unwrap();
                let response = page.oneshot(request).await.unwrap();
                let bytes = http_body_util::BodyExt::collect(response.into_body())
                    .await
                    .unwrap()
                    .to_bytes();
                serde_json::from_slice::<Value>(&bytes).unwrap()
            }
        };
        let state = get_state(&client, &base).await.unwrap();
        assert_eq!(state["items"], json!([]));
        assert_eq!(state["demo"], true);
        // The same idea on both, and "another idea" on either changes it for the other.
        let idea = state["idea"].as_str().unwrap().to_owned();
        assert!(!idea.is_empty());
        assert_eq!(phone("state", "GET").await["idea"], idea.as_str());
        let next = another_idea(&client, &base).await.unwrap()["idea"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(next, idea);
        assert_eq!(phone("state", "GET").await["idea"], next.as_str());
        let from_phone = phone("idea", "POST").await["idea"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(from_phone, next);
        assert_eq!(
            get_state(&client, &base).await.unwrap()["idea"],
            from_phone.as_str()
        );
        // A take recorded here (a second and a half of sound) goes up as the app sends it.
        let wav = temp_take();
        let pcm: Vec<f32> = (0..24_000).map(|i| (i as f32 / 9.0).sin() * 0.1).collect();
        write_private(&wav, &crate::asr::wav(&pcm).unwrap()).unwrap();
        let sent: Take =
            serde_json::from_value(upload(&client, &base, &wav).await.unwrap()).unwrap();
        std::fs::remove_file(&wav).unwrap();
        assert_eq!(
            (sent.n, sent.status.as_str(), sent.held_out),
            (1, "pending", true)
        );
        let seen = phone("state", "GET").await;
        assert_eq!(seen["items"][0]["n"], 1);
        assert_eq!(seen["items"][0]["draft"], sent.draft.as_str());
        assert_eq!(
            seen["items"][0]["words"],
            json!(freebench::words(&sent.draft))
        );
        // Its recording comes back whole.
        let bytes = audio(&client, &base, 1).await.unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        // A review made here, through the same steps as the screen.
        let mut state = Screen::default();
        let (jobs, mut asked) = mpsc::unbounded_channel();
        state.play.open_review(sent.clone(), false, &jobs);
        state.play.tap_word(0);
        state.play.right = "גוט".into();
        state.play.save_mark();
        state.play.tap_word(2);
        state.play.add_place(3);
        state.play.right = "נאך".into();
        state.play.save_mark();
        let expected = state.play.final_text();
        click(&mut state, PlayClick::Save, &jobs).unwrap();
        let (n, review) = std::iter::from_fn(|| asked.try_recv().ok())
            .find_map(|job| match job {
                Job::Review(n, review) => Some((n, review)),
                _ => None,
            })
            .unwrap();
        let saved: Take =
            serde_json::from_value(send_review(&client, &base, n, &review).await.unwrap()).unwrap();
        assert_eq!(saved.status, "fixed");
        assert_eq!(saved.final_text, expected);
        assert_eq!(saved.marks.len(), 2);
        assert_eq!(saved.marks[0].heard, freebench::words(&sent.draft)[0]);
        // Stored as such, and the phone sees it so.
        let stored = freebench::Store::new(&folder).load().unwrap();
        assert_eq!(stored[0].final_text, expected);
        assert_eq!(stored[0].marks, saved.marks);
        let seen = phone("state", "GET").await;
        assert_eq!(seen["items"][0]["final"], expected.as_str());
        assert_eq!(seen["items"][0]["status"], "fixed");
        // Redo from here: gone for the phone too.
        remove(&client, &base, 1).await.unwrap();
        assert_eq!(phone("state", "GET").await["items"], json!([]));
        // Where the playground is off, none of it is there (a computer that has it on
        // cannot show that).
        if crate::phone::playground() {
            served.abort();
            std::fs::remove_dir_all(folder).unwrap();
            return;
        }
        let mut off = crate::service::App::new(config()).unwrap();
        off.phone = crate::phone::PairHub::at(&folder);
        let response = crate::service::router(off)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/playground/state")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
        served.abort();
        std::fs::remove_dir_all(folder).unwrap();
    }
}
