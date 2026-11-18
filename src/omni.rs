//! The second Transcribe engine: Meta's Omnilingual CTC model, run through ONNX Runtime.
//!
//! The model hears up to 40 seconds at a time and gives, for every 20 ms of sound, a score for
//! each letter it knows. Reading the text off those scores is done here: either the plain way
//! (the likeliest letter at each moment) or, with a word list, a search over several readings
//! that prefers the ones made of real words (see `wordcheck`).
use crate::wordcheck::{At, Check};
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, path::Path, sync::Mutex};

const RATE: usize = 16_000;
/// The longest stretch handed to the model at once, and how far back from there a quiet
/// moment to cut at is looked for.
const CHUNK: usize = 28 * RATE;
const CUT_WINDOW: usize = 8 * RATE;
/// Readings kept side by side, and letters tried at each moment, in the search.
const BEAMS: usize = 8;
const TRIED: usize = 5;
/// The first letters of the table are control marks (start, padding, end, unknown); in the
/// scores they all mean "no letter here".
const MARKS: usize = 4;

pub struct Engine {
    session: Mutex<ort::session::Session>,
    letters: Vec<String>,
    check: Option<Check>,
    weight: f32,
}

impl Engine {
    /// `model`: the ONNX file. `letters`: its letter table, a JSON list. `words` and `weight`:
    /// the word check, off without a list.
    pub fn load(model: &Path, letters: &Path, words: Option<&str>, weight: f32) -> Result<Self> {
        let letters: Vec<String> = serde_json::from_slice(
            &std::fs::read(letters)
                .with_context(|| format!("Cannot read the letter table {}", letters.display()))?,
        )
        .context("The letter table is not valid")?;
        if letters.len() <= MARKS {
            bail!("The letter table is empty");
        }
        let env = ort::init().with_telemetry(false).build()?;
        let session = ort::session::Session::builder(&env)?
            .with_intra_threads(crate::asr::threads() as usize)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .commit_from_file(model)
            .with_context(|| format!("Cannot load the Transcribe model {}", model.display()))?;
        let check = words
            .filter(|_| weight > 0.0)
            .map(|w| Check::new(w, Vec::new(), weight, false));
        Ok(Self {
            session: Mutex::new(session),
            letters,
            check,
            weight,
        })
    }

    /// 16 kHz mono samples in, Yiddish text out.
    pub fn transcribe(&self, pcm: &[f32]) -> Result<String> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("Transcribe engine poisoned"))?;
        let mut text = Vec::new();
        for chunk in chunks(pcm) {
            if chunk.len() < RATE / 5 {
                continue; // too short for the model to hear anything
            }
            let sound = normalized(chunk);
            let input =
                ort::value::TensorRef::from_array_view(([1usize, sound.len()], &sound[..]))?;
            let outputs = session.run(ort::inputs!["sound" => input])?;
            let (shape, scores) = outputs["scores"].try_extract_tensor::<f32>()?;
            let width = *shape.last().context("The model gave no scores")? as usize;
            if width != self.letters.len() || width == 0 {
                bail!("The model and its letter table do not match");
            }
            let t = match &self.check {
                Some(check) => self.search(scores, width, check),
                None => self.plain(scores, width),
            };
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() {
                text.push(t);
            }
        }
        Ok(text.join(" "))
    }

    /// The likeliest letter at each moment; the same letter twice in a row is written once.
    fn plain(&self, scores: &[f32], width: usize) -> String {
        let mut out = String::new();
        let mut last = usize::MAX;
        for frame in scores.chunks_exact(width) {
            let best = (0..width).fold(0, |b, i| if frame[i] > frame[b] { i } else { b });
            if best != last && best >= MARKS {
                out.push_str(&self.letters[best]);
            }
            last = best;
        }
        out
    }

    /// Several readings side by side; one that writes a word outside the list loses `weight`
    /// each time it does.
    fn search(&self, scores: &[f32], width: usize, check: &Check) -> String {
        #[derive(Clone)]
        struct Reading {
            /// Chance (as a logarithm) of this text so far, ending in "no letter" / in a letter.
            none: f32,
            letter: f32,
            at: At,
            broke: u32,
        }
        fn add(a: f32, b: f32) -> f32 {
            if a == f32::NEG_INFINITY {
                b
            } else if b == f32::NEG_INFINITY {
                a
            } else {
                a.max(b) + (-(a - b).abs()).exp().ln_1p()
            }
        }
        let never = f32::NEG_INFINITY;
        let mut readings: HashMap<Vec<u32>, Reading> = HashMap::new();
        readings.insert(
            Vec::new(),
            Reading {
                none: 0.0,
                letter: never,
                at: At::default(),
                broke: 0,
            },
        );
        let mut order: Vec<usize> = Vec::with_capacity(width);
        for frame in scores.chunks_exact(width) {
            let top = frame.iter().copied().fold(never, f32::max);
            let total = top + frame.iter().map(|s| (s - top).exp()).sum::<f32>().ln();
            let none = frame[..MARKS].iter().fold(never, |a, &s| add(a, s - total));
            order.clear();
            order.extend(MARKS..width);
            let tried = TRIED.min(order.len());
            order.select_nth_unstable_by(tried - 1, |&a, &b| frame[b].total_cmp(&frame[a]));
            let mut next: HashMap<Vec<u32>, Reading> = HashMap::with_capacity(readings.len() * 4);
            for (text, r) in &readings {
                let both = add(r.none, r.letter);
                let same = next.entry(text.clone()).or_insert(Reading {
                    none: never,
                    letter: never,
                    at: r.at,
                    broke: r.broke,
                });
                same.none = add(same.none, both + none);
                let last = text.last().copied();
                if let Some(l) = last {
                    // The last letter held for another moment.
                    same.letter = add(same.letter, r.letter + frame[l as usize] - total);
                }
                for &c in &order[..tried] {
                    let p = frame[c] - total;
                    if p < -12.0 {
                        continue;
                    }
                    // The same letter again counts as a new one only after "no letter".
                    let from = if last == Some(c as u32) { r.none } else { both };
                    if from == never {
                        continue;
                    }
                    let mut longer = text.clone();
                    longer.push(c as u32);
                    let grown = next.entry(longer).or_insert_with(|| {
                        let mut at = r.at;
                        let broke = r.broke + check.feed(&mut at, self.letters[c].as_bytes());
                        Reading {
                            none: never,
                            letter: never,
                            at,
                            broke,
                        }
                    });
                    grown.letter = add(grown.letter, from + p);
                }
            }
            let worth = |r: &Reading| add(r.none, r.letter) - self.weight * r.broke as f32;
            let mut kept: Vec<(Vec<u32>, Reading)> = next.into_iter().collect();
            if kept.len() > BEAMS {
                kept.select_nth_unstable_by(BEAMS - 1, |a, b| worth(&b.1).total_cmp(&worth(&a.1)));
                kept.truncate(BEAMS);
            }
            readings = kept.into_iter().collect();
        }
        readings
            .into_iter()
            .map(|(text, r)| {
                // The word still open when the sound ends must be a whole word too.
                let mut at = r.at;
                let broke = r.broke + check.end(&mut at);
                (add(r.none, r.letter) - self.weight * broke as f32, text)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, text)| {
                text.iter()
                    .map(|&c| self.letters[c as usize].as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The sound with its average at zero and its spread at one, as the model was trained to hear.
fn normalized(pcm: &[f32]) -> Vec<f32> {
    let n = pcm.len().max(1) as f64;
    let mean = pcm.iter().map(|&s| s as f64).sum::<f64>() / n;
    let spread = (pcm.iter().map(|&s| (s as f64 - mean).powi(2)).sum::<f64>() / n).sqrt();
    pcm.iter()
        .map(|&s| ((s as f64 - mean) / (spread + 1e-7)) as f32)
        .collect()
}

/// Cut a long recording into stretches the model can take, each cut at the quietest moment
/// near the end of its stretch so that no word is split.
fn chunks(pcm: &[f32]) -> Vec<&[f32]> {
    let mut out = Vec::new();
    let mut rest = pcm;
    while rest.len() > CHUNK {
        let from = CHUNK - CUT_WINDOW;
        let window = RATE / 5; // 0.2 s
        let hop = RATE / 20;
        let cut = (from..CHUNK - window)
            .step_by(hop)
            .min_by(|&a, &b| {
                let loud = |i: usize| rest[i..i + window].iter().map(|s| s * s).sum::<f32>();
                loud(a).total_cmp(&loud(b))
            })
            .map_or(CHUNK, |i| i + window / 2);
        out.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    out.push(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_sound_is_cut_at_the_quiet_moment() {
        // 60 s of noise with silence from 25.0 s to 25.6 s.
        let mut pcm: Vec<f32> = (0..60 * RATE)
            .map(|i| ((i * 7919) % 200) as f32 / 100.0 - 1.0)
            .collect();
        for s in &mut pcm[25 * RATE..25 * RATE + 6 * RATE / 10] {
            *s = 0.0;
        }
        let parts = chunks(&pcm);
        assert_eq!(parts.iter().map(|p| p.len()).sum::<usize>(), pcm.len());
        assert!(parts.iter().all(|p| p.len() <= CHUNK));
        let first = parts[0].len() as f32 / RATE as f32;
        assert!((25.0..25.6).contains(&first), "{first}");
        assert_eq!(chunks(&pcm[..RATE]).len(), 1);
        let n = normalized(&[1.0, 3.0]);
        assert!((n[0] + 1.0).abs() < 1e-4 && (n[1] - 1.0).abs() < 1e-4);
    }
}
