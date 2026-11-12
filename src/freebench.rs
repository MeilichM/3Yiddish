//! The free bench: phrases the speaker says in his own words, the draft our model wrote for
//! each, and his corrections. Kept under `state/free-bench/` and nowhere else.
//!
//! This is a held-out exam. Nothing in it may ever be used as training data: every take is
//! stored with `held_out: true`, and nothing here sends a take anywhere.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// How many accepted takes make the set.
pub const GOAL: usize = 100;
/// Why a span of the draft is marked.
pub const REASONS: [&str; 9] = [
    "wrong",
    "weird",
    "fluency",
    "accent",
    "loshn-koydesh",
    "name",
    "english",
    "dropped",
    "custom",
];
const MAX_TEXT: usize = 4000;
const MAX_MARKS: usize = 200;

/// One marked span of the draft. `start..end` are word numbers in the draft (see `words`),
/// `end` not included; `start == end` marks a place between two words, where the model
/// dropped something.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    /// The draft's words in the span (filled in here, not trusted from the page).
    #[serde(default)]
    pub heard: String,
    /// What it should be: text replaces the span, an empty text means nothing was said
    /// there, and none means no correction was given (the span stays as heard).
    pub right: Option<String>,
    pub reason: String,
    /// The speaker's own words for a custom reason.
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Take {
    pub n: u32,
    /// The recording, relative to the free-bench folder: `audio/NNN.wav`, 16 kHz mono.
    pub audio: String,
    pub seconds: f32,
    pub created: String,
    pub updated: String,
    /// What our model wrote; never changed afterwards.
    pub draft: String,
    /// Which model wrote it and on what: `{"name": ..., "engine": "gpu" | "cpu" | ...}`.
    pub draft_model: Value,
    /// The exact text the speaker considers right: the draft with his marks applied, or what
    /// he typed out.
    #[serde(rename = "final")]
    pub final_text: String,
    /// `final` was typed out by hand instead of derived from the marks.
    #[serde(default)]
    pub typed: bool,
    /// `pending` (not reviewed yet), `correct` (no marks), `fixed`, or `tossed`.
    pub status: String,
    pub marks: Vec<Mark>,
    /// Always true: an exam recording, never training data.
    pub held_out: bool,
    /// Where this take was sent (a contributor's Send), once it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<String>,
}

/// What the speaker decided about a take.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    #[serde(default)]
    pub marks: Vec<Mark>,
    /// The whole text typed out by hand, when the marks were not enough.
    #[serde(default)]
    pub typed: Option<String>,
    /// A bad take: kept, but not counted.
    #[serde(default)]
    pub toss: bool,
}

/// The draft's words, the one way a transcript is split everywhere (the page gets this list
/// instead of splitting by itself).
pub fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// Check the marks against the draft and put them in order. Spans may touch but not overlap,
/// and one place between two words holds one insertion.
pub fn check(draft: &str, mut marks: Vec<Mark>) -> Result<Vec<Mark>> {
    let words = words(draft);
    ensure!(marks.len() <= MAX_MARKS, "Too many marks");
    marks.sort_by_key(|m| (m.start, m.end));
    let mut last: Option<(usize, usize)> = None;
    for mark in &mut marks {
        ensure!(
            mark.start <= mark.end && mark.end <= words.len(),
            "A mark is outside the text"
        );
        ensure!(
            REASONS.contains(&mark.reason.as_str()),
            "Unknown reason for a mark"
        );
        if let Some((start, end)) = last {
            ensure!(
                mark.start >= end && (mark.start, mark.end) != (start, end),
                "Two marks overlap"
            );
        }
        last = Some((mark.start, mark.end));
        mark.heard = words[mark.start..mark.end].join(" ");
        mark.note = mark.note.trim().to_owned();
        mark.right = mark
            .right
            .take()
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "));
        ensure!(
            mark.note.chars().count() <= 300
                && mark
                    .right
                    .as_ref()
                    .is_none_or(|t| t.chars().count() <= MAX_TEXT),
            "A correction is too long"
        );
        if mark.start == mark.end {
            ensure!(
                mark.right.as_ref().is_some_and(|t| !t.is_empty()),
                "A missing word needs its text"
            );
        }
    }
    Ok(marks)
}

/// The draft with the (checked) marks applied.
pub fn apply(draft: &str, marks: &[Mark]) -> String {
    let words = words(draft);
    let mut out: Vec<&str> = Vec::new();
    let mut at = 0;
    for mark in marks {
        out.extend(&words[at..mark.start]);
        match &mark.right {
            Some(right) => out.extend(right.split_whitespace()),
            None => out.extend(&words[mark.start..mark.end]),
        }
        at = mark.end;
    }
    out.extend(&words[at..]);
    out.join(" ")
}

/// Seconds since 1970 as `2026-11-12T15:04:05Z`.
fn stamp(unix: u64) -> String {
    let (days, rest) = (unix / 86_400, unix % 86_400);
    // Days to a calendar date (Howard Hinnant's civil_from_days).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
fn now() -> String {
    stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
}

/// The folder with `items.jsonl` and `audio/`.
pub struct Store(PathBuf);
impl Store {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self(folder.into())
    }
    pub fn folder(&self) -> &Path {
        &self.0
    }
    fn items(&self) -> PathBuf {
        self.0.join("items.jsonl")
    }
    pub fn audio(&self, n: u32) -> PathBuf {
        self.0.join(format!("audio/{n:03}.wav"))
    }
    /// Every take, in the order recorded. A line that cannot be read is an error: saving
    /// over a file that was only half understood would lose takes.
    pub fn load(&self) -> Result<Vec<Take>> {
        let text = match std::fs::read_to_string(self.items()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).context("Cannot read the free bench"),
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
            .map(|(i, line)| {
                serde_json::from_str(line)
                    .with_context(|| format!("items.jsonl line {} cannot be read", i + 1))
            })
            .collect()
    }
    /// Write the whole list to a new file and move it into place, so a crash leaves either
    /// the old list or the new one.
    fn save(&self, takes: &[Take]) -> Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.0)?;
        let part = self.0.join("items.jsonl.part");
        let mut file = private(&part)?;
        for take in takes {
            serde_json::to_writer(&mut file, take)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
        std::fs::rename(part, self.items())?;
        Ok(())
    }
    /// (accepted takes, all takes).
    pub fn count(&self) -> (usize, usize) {
        let takes = self.load().unwrap_or_default();
        (
            takes
                .iter()
                .filter(|t| matches!(t.status.as_str(), "correct" | "fixed"))
                .count(),
            takes.len(),
        )
    }
    /// Keep a new recording (16 kHz mono samples) with the model's draft, not reviewed yet.
    pub fn add(&self, pcm: &[f32], draft: &str, model: Value) -> Result<Take> {
        let mut takes = self.load()?;
        // A number is never used twice, also after a Redo threw its take away: a recording
        // called 007.wav is the same recording for good (comparisons keep work by file name).
        let last = self.0.join("last-number");
        let used = std::fs::read_to_string(&last)
            .ok()
            .and_then(|n| n.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let n = takes.iter().map(|t| t.n).max().unwrap_or(0).max(used) + 1;
        let path = self.audio(n);
        std::fs::create_dir_all(path.parent().context("No audio folder")?)?;
        std::fs::write(&last, n.to_string())?;
        std::io::Write::write_all(&mut private(&path)?, &crate::asr::wav(pcm)?)?;
        let draft = words(draft).join(" ");
        let now = now();
        let take = Take {
            n,
            audio: format!("audio/{n:03}.wav"),
            seconds: (pcm.len() as f32 / 16_000.0 * 10.0).round() / 10.0,
            created: now.clone(),
            updated: now,
            final_text: draft.clone(),
            draft,
            draft_model: model,
            typed: false,
            status: "pending".into(),
            marks: Vec::new(),
            held_out: true,
            sent: None,
        };
        takes.push(take.clone());
        self.save(&takes)?;
        Ok(take)
    }
    /// Record the speaker's decision on a take and work out its final text.
    pub fn review(&self, n: u32, review: Review) -> Result<Take> {
        let mut takes = self.load()?;
        let take = takes
            .iter_mut()
            .find(|t| t.n == n)
            .context("No such take")?;
        take.marks = check(&take.draft, review.marks)?;
        let typed = review
            .typed
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "));
        if let Some(text) = &typed {
            ensure!(text.chars().count() <= MAX_TEXT, "The text is too long");
        }
        take.typed = typed.is_some();
        take.final_text = typed.unwrap_or_else(|| apply(&take.draft, &take.marks));
        take.status = if review.toss {
            "tossed"
        } else if take.typed || !take.marks.is_empty() {
            "fixed"
        } else {
            "correct"
        }
        .into();
        if take.status != "tossed" && take.final_text.is_empty() {
            bail!("Nothing would be left: type what you said, or toss the take");
        }
        take.updated = now();
        // Changed after it was sent: it can go again.
        take.sent = None;
        let take = take.clone();
        self.save(&takes)?;
        Ok(take)
    }
    /// The reviewed takes that have not been sent.
    pub fn unsent(&self) -> Result<Vec<Take>> {
        Ok(self
            .load()?
            .into_iter()
            .filter(|t| t.sent.is_none() && matches!(t.status.as_str(), "correct" | "fixed"))
            .collect())
    }
    /// These takes went to `place`, as they were at `updated` (one changed meanwhile stays
    /// unsent).
    pub fn mark_sent(&self, sent: &[(u32, String)], place: &str) -> Result<()> {
        let mut takes = self.load()?;
        for take in &mut takes {
            if sent
                .iter()
                .any(|(n, updated)| *n == take.n && *updated == take.updated)
            {
                take.sent = Some(place.to_owned());
            }
        }
        self.save(&takes)
    }
    /// Throw a take away for good (Redo): its line and its recording.
    pub fn delete(&self, n: u32) -> Result<()> {
        let mut takes = self.load()?;
        let before = takes.len();
        takes.retain(|t| t.n != n);
        ensure!(takes.len() < before, "No such take");
        self.save(&takes)?;
        let _ = std::fs::remove_file(self.audio(n));
        Ok(())
    }
}

/// A new file only its owner can read.
fn private(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("Cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mark(start: usize, end: usize, right: Option<&str>, reason: &str) -> Mark {
        Mark {
            start,
            end,
            heard: "from the page, ignored".into(),
            right: right.map(str::to_owned),
            reason: reason.into(),
            note: String::new(),
        }
    }
    fn store() -> Store {
        Store::new(std::env::temp_dir().join(format!("3y-free-bench-{}", uuid::Uuid::new_v4())))
    }

    #[test]
    fn final_text_is_the_draft_with_the_marks_applied() {
        let draft = "איך בין געגאנגען אין שול היינט";
        let marks = check(
            draft,
            vec![
                // Given out of order; a replacement, a removal, a flag and two insertions.
                mark(6, 6, Some("אינדערפרי"), "dropped"),
                mark(5, 6, None, "fluency"),
                mark(2, 3, Some("  געפארן  "), "wrong"),
                mark(3, 4, Some(""), "weird"),
                mark(0, 0, Some("נעכטן"), "dropped"),
            ],
        )
        .unwrap();
        assert_eq!(marks[0].start, 0);
        assert_eq!(marks[1].heard, "געגאנגען");
        assert_eq!(marks[1].right.as_deref(), Some("געפארן"));
        assert_eq!(
            apply(draft, &marks),
            "נעכטן איך בין געפארן שול היינט אינדערפרי"
        );
        assert_eq!(apply(draft, &[]), draft);
    }

    #[test]
    fn marks_that_do_not_fit_are_refused() {
        let draft = "איינס צוויי דריי";
        for bad in [
            vec![
                mark(0, 2, Some("x"), "wrong"),
                mark(1, 3, Some("y"), "wrong"),
            ],
            vec![
                mark(1, 1, Some("x"), "dropped"),
                mark(1, 1, Some("y"), "dropped"),
            ],
            vec![mark(2, 4, Some("x"), "wrong")],
            vec![mark(2, 1, Some("x"), "wrong")],
            vec![mark(0, 1, Some("x"), "because")],
            vec![mark(1, 1, Some(" "), "dropped")],
            vec![mark(1, 1, None, "dropped")],
        ] {
            assert!(check(draft, bad).is_err());
        }
        // Touching spans, and an insertion right before or after a span, are fine.
        assert!(
            check(
                draft,
                vec![
                    mark(0, 1, Some("x"), "wrong"),
                    mark(1, 1, Some("y"), "dropped"),
                    mark(1, 2, Some(""), "weird"),
                    mark(2, 2, Some("z"), "dropped"),
                ]
            )
            .is_ok()
        );
    }

    #[test]
    fn only_reviewed_takes_are_sent_and_only_once() {
        let store = store();
        let pcm = vec![0.1f32; 16_000];
        let model = json!({"name": "test", "engine": "cpu"});
        for _ in 0..3 {
            store.add(&pcm, "איינס צוויי", model.clone()).unwrap();
        }
        let review = |marks| Review {
            marks,
            typed: None,
            toss: false,
        };
        store.review(1, review(vec![])).unwrap();
        let fixed = store
            .review(2, review(vec![mark(0, 1, Some("דריי"), "wrong")]))
            .unwrap();
        // Take 3 was never reviewed: it stays here.
        let unsent = store.unsent().unwrap();
        assert_eq!(unsent.iter().map(|t| t.n).collect::<Vec<_>>(), [1, 2]);
        // Take 2 is changed while the send is under way: only take 1 counts as sent.
        let again = store.review(2, review(vec![])).unwrap();
        let sent: Vec<(u32, String)> = unsent.iter().map(|t| (t.n, t.updated.clone())).collect();
        store.mark_sent(&sent, "takes/x").unwrap();
        let left = store.unsent().unwrap();
        if again.updated == fixed.updated {
            // The same second: both went as they were.
            assert!(left.is_empty());
        } else {
            assert_eq!(left.iter().map(|t| t.n).collect::<Vec<_>>(), [2]);
        }
        // A sent take that is reviewed again can go again.
        store
            .review(1, review(vec![mark(0, 1, Some("פיר"), "wrong")]))
            .unwrap();
        assert!(store.unsent().unwrap().iter().any(|t| t.n == 1));
        std::fs::remove_dir_all(store.folder()).unwrap();
    }

    #[test]
    fn takes_are_kept_reviewed_and_thrown_away() {
        let store = store();
        assert_eq!(store.count(), (0, 0));
        let pcm = vec![0.1f32; 16_000];
        let model = json!({"name": "3Yiddish-Transcribe v9", "engine": "cpu"});
        let first = store.add(&pcm, "  איינס   צוויי ", model.clone()).unwrap();
        assert_eq!(
            (first.n, first.status.as_str(), first.draft.as_str()),
            (1, "pending", "איינס צוויי")
        );
        assert!(first.held_out && first.seconds == 1.0 && first.created.ends_with('Z'));
        // 16 kHz mono 16-bit on disk.
        let wav = std::fs::read(store.audio(1)).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        store.add(&pcm, "דריי", model.clone()).unwrap();
        // All correct.
        let take = store
            .review(
                1,
                Review {
                    marks: vec![],
                    typed: None,
                    toss: false,
                },
            )
            .unwrap();
        assert_eq!(
            (take.status.as_str(), take.final_text.as_str()),
            ("correct", "איינס צוויי")
        );
        // Marked, then typed out, then back to marks: the final text follows each time.
        let marks = vec![mark(1, 2, Some("פיר"), "wrong")];
        let review = |typed: Option<&str>, toss| Review {
            marks: marks.clone(),
            typed: typed.map(str::to_owned),
            toss,
        };
        let take = store.review(1, review(None, false)).unwrap();
        assert_eq!(
            (take.status.as_str(), take.final_text.as_str(), take.typed),
            ("fixed", "איינס פיר", false)
        );
        let take = store
            .review(1, review(Some(" גאר  אנדערש "), false))
            .unwrap();
        assert_eq!(
            (take.final_text.as_str(), take.typed, take.marks.len()),
            ("גאר אנדערש", true, 1)
        );
        let take = store.review(1, review(None, true)).unwrap();
        assert_eq!(
            (take.status.as_str(), take.final_text.as_str()),
            ("tossed", "איינס פיר")
        );
        assert_eq!(store.count(), (0, 2));
        store.review(1, review(None, false)).unwrap();
        assert_eq!(store.count(), (1, 2));
        // The file is one object per line, each held out, with `final` under that name.
        let text = std::fs::read_to_string(store.folder().join("items.jsonl")).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l["held_out"] == true));
        assert_eq!(lines[0]["final"], "איינס פיר");
        assert_eq!(lines[0]["marks"][0]["heard"], "צוויי");
        assert_eq!(lines[0]["draft_model"]["engine"], "cpu");
        assert!(!store.folder().join("items.jsonl.part").exists());
        // A removal that would leave nothing is refused; a toss of it is not.
        assert!(
            store
                .review(
                    2,
                    Review {
                        marks: vec![mark(0, 1, Some(""), "weird")],
                        typed: None,
                        toss: false
                    }
                )
                .is_err()
        );
        // Redo: the take and its recording go; its number is not given out again.
        store.delete(2).unwrap();
        assert!(!store.audio(2).exists() && store.delete(2).is_err());
        assert_eq!(store.add(&pcm, "זעקס", model).unwrap().n, 3);
        assert!(store.review(7, review(None, false)).is_err());
        std::fs::remove_dir_all(store.folder()).unwrap();
    }

    #[test]
    fn a_damaged_list_is_never_written_over() {
        let store = store();
        std::fs::create_dir_all(store.folder()).unwrap();
        std::fs::write(
            store.folder().join("items.jsonl"),
            "{\"n\": 1, half a line\n",
        )
        .unwrap();
        assert!(store.load().is_err());
        assert!(store.add(&[0.0; 1600], "x", json!({})).is_err());
        assert!(!store.audio(1).exists());
        std::fs::remove_dir_all(store.folder()).unwrap();
    }

    #[test]
    fn dates_are_written_in_full() {
        assert_eq!(stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(stamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(stamp(1_794_495_845), "2026-11-12T15:04:05Z");
    }
}
