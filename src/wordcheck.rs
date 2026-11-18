//! Two checks applied while the speech model writes, one piece of a word at a time.
//!
//! The word check: a list of words people really write (from the training text). A piece that
//! would turn the word being written into something no listed word starts with, or that ends
//! it where no listed word ends, is made less likely. It is a push, not a ban: names and new
//! words still come through when the model is sure of them. Only words in Yiddish letters are
//! checked; Latin letters and digits pass untouched.
//!
//! The loop guard: a run of pieces that was already written twice cannot be written a third
//! time, which stops the model from repeating one phrase to the end of the recording.
use std::collections::HashMap;

/// How many pieces in a row count as "the same phrase" for the loop guard.
const RUN: usize = 6;
/// Pieces this far below the likeliest one cannot win even after the push, so they are skipped.
const MARGIN: f32 = 10.0;

/// Where the writing stands inside the current word.
#[derive(Clone, Copy, Default)]
pub struct At {
    /// Running fingerprint of the letters of the word so far.
    hash: u64,
    /// Inside a word in Yiddish letters.
    in_word: bool,
    /// The word already left the list (and was pushed down for it once).
    off: bool,
    /// First byte of a letter whose second byte comes with the next piece.
    lead: u8,
}

fn step(hash: u64, letter: u8) -> u64 {
    (hash ^ (letter as u64 + 1)).wrapping_mul(0x0100_0000_01b3)
}
const START: u64 = 0xcbf2_9ce4_8422_2325;

/// The letters of one character, with the double letters written out (װ as וו) and vowel marks
/// dropped. `None`: not a letter, so it ends the word.
fn letters(lead: u8, second: u8) -> Option<([u8; 2], usize)> {
    match (lead, second) {
        (0xD7, 0x90..=0xAA) => Some(([second - 0x90, 0], 1)),
        (0xD7, 0xB0) => Some(([5, 5], 2)), // װ
        (0xD7, 0xB1) => Some(([5, 9], 2)), // ױ
        (0xD7, 0xB2) => Some(([9, 9], 2)), // ײ
        // Vowel marks and accents sit on a letter and do not end the word.
        (0xD6, 0x91..=0xBD | 0xBF) | (0xD7, 0x81 | 0x82 | 0x84 | 0x85 | 0x87) => Some(([0, 0], 0)),
        _ => None,
    }
}

pub struct Check {
    /// Every beginning of every listed word, and whether a word ends there.
    starts: HashMap<u64, bool>,
    /// The bytes of each piece the model can write; empty for its control marks.
    pieces: Vec<Vec<u8>>,
    /// How hard the word check pushes; 0 switches it off.
    weight: f32,
    /// Whether the loop guard is on.
    loops: bool,
}

impl Check {
    /// `words`: one word per line. `pieces[id]`: the bytes of piece `id` (empty: control mark).
    pub fn new(words: &str, pieces: Vec<Vec<u8>>, weight: f32, loops: bool) -> Self {
        let mut starts = HashMap::new();
        for word in words.split_whitespace() {
            let mut lead = 0u8;
            let mut hash = START;
            let mut any = false;
            let mut whole = true;
            for &b in word.as_bytes() {
                if lead != 0 {
                    match letters(lead, b) {
                        Some((l, n)) => {
                            for &x in &l[..n] {
                                hash = step(hash, x);
                                starts.entry(hash).or_insert(false);
                                any = true;
                            }
                        }
                        None => whole = false,
                    }
                    lead = 0;
                } else if b == 0xD6 || b == 0xD7 {
                    lead = b;
                } else {
                    whole = false;
                }
            }
            // A line with anything but letters in it is not one word; its beginnings stay
            // harmless, and it is not marked as a place where a word ends.
            if any && whole {
                starts.insert(hash, true);
            }
        }
        Self {
            starts,
            pieces,
            weight,
            loops,
        }
    }

    pub fn words(&self) -> usize {
        self.starts.values().filter(|&&end| end).count()
    }

    /// Take in `bytes`; returns how many times a word broke the list on the way.
    pub fn feed(&self, at: &mut At, bytes: &[u8]) -> u32 {
        let mut broke = 0;
        for &b in bytes {
            if at.lead != 0 {
                let lead = std::mem::take(&mut at.lead);
                match letters(lead, b) {
                    Some((l, n)) => {
                        for &x in &l[..n] {
                            if !at.in_word {
                                *at = At {
                                    hash: START,
                                    in_word: true,
                                    ..At::default()
                                };
                            }
                            at.hash = step(at.hash, x);
                            if !at.off && !self.starts.contains_key(&at.hash) {
                                at.off = true;
                                broke += 1;
                            }
                        }
                    }
                    None => broke += self.end(at),
                }
            } else if b == 0xD6 || b == 0xD7 {
                at.lead = b;
            } else {
                broke += self.end(at);
            }
        }
        broke
    }

    /// The word ends here; 1 when no listed word ends at this point.
    pub fn end(&self, at: &mut At) -> u32 {
        let broke = at.in_word && !at.off && self.starts.get(&at.hash) != Some(&true);
        *at = At::default();
        broke as u32
    }

    /// Adjust the model's scores for the next piece, given the pieces written so far.
    pub fn filter(&self, written: &[i32], logits: &mut [f32]) {
        let text = self.pieces.len().min(logits.len());
        if self.loops && written.len() >= RUN {
            // What followed the last RUN-1 pieces each earlier time they were written.
            let tail = &written[written.len() - (RUN - 1)..];
            let mut seen: Vec<(i32, u32)> = Vec::new();
            for w in written.windows(RUN) {
                if &w[..RUN - 1] == tail {
                    match seen.iter_mut().find(|(id, _)| *id == w[RUN - 1]) {
                        Some((_, n)) => *n += 1,
                        None => seen.push((w[RUN - 1], 1)),
                    }
                }
            }
            for (id, n) in seen {
                if n >= 2 && (id as usize) < text {
                    logits[id as usize] = f32::NEG_INFINITY;
                }
            }
        }
        if self.weight <= 0.0 {
            return;
        }
        let mut at = At::default();
        for &id in written {
            match self.pieces.get(id as usize) {
                Some(p) if !p.is_empty() => {
                    self.feed(&mut at, p);
                }
                _ => {
                    self.end(&mut at);
                }
            }
        }
        let top = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let floor = top - self.weight - MARGIN;
        for (id, logit) in logits.iter_mut().enumerate() {
            if *logit < floor {
                continue;
            }
            let mut next = at;
            let broke = match self.pieces.get(id) {
                Some(p) if !p.is_empty() => self.feed(&mut next, p),
                // A control mark (the end of the text, a time mark) ends the word.
                _ => self.end(&mut next),
            };
            *logit -= self.weight * broke as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(weight: f32, loops: bool) -> Check {
        // Pieces: 0 " גע", 1 "זאגט", 2 "זאקט", 3 " איך", 4 ".", 5 first byte of a letter,
        // 6 its second byte (ט), 7 " ok", 8 "װאס", 9 control mark.
        let pieces: Vec<Vec<u8>> = vec![
            " גע".into(),
            "זאגט".into(),
            "זאקט".into(),
            " איך".into(),
            ".".into(),
            vec![0xD7],
            vec![0x98],
            " ok".into(),
            " װאָס".into(),
            vec![],
        ];
        Check::new("געזאגט\nאיך\nגעט\nוואס\nכ'האב\n", pieces, weight, loops)
    }

    #[test]
    fn a_piece_that_leaves_the_word_list_is_pushed_down() {
        let c = check(3.0, false);
        assert_eq!(c.words(), 4, "the line with an apostrophe is not a word");
        let mut logits = vec![0.0f32; 10];
        c.filter(&[0], &mut logits); // " גע" is written
        assert_eq!(logits[1], 0.0, "געזאגט is a word");
        assert_eq!(logits[2], -3.0, "געזאקט is not");
        assert_eq!(
            logits[3], -3.0,
            "גע alone is not a word, so ending it costs"
        );
        assert_eq!(logits[4], -3.0);
        assert_eq!(logits[9], -3.0, "the end of the text ends the word too");
        assert_eq!(logits[5], 0.0, "half a letter decides nothing yet");
        assert_eq!(logits[7], -3.0);
        // The half letter is carried over: גע + ט is a word.
        let mut logits = vec![0.0f32; 10];
        c.filter(&[0, 5], &mut logits);
        assert_eq!(logits[6], 0.0);
        // After a whole word everything that starts a listed word, or no word at all, is free.
        let mut logits = vec![0.0f32; 10];
        c.filter(&[0, 1], &mut logits);
        assert_eq!(&logits[3..5], &[0.0, 0.0]);
        assert_eq!(logits[7], 0.0, "Latin letters are not checked");
        assert_eq!(logits[8], 0.0, "װ counts as וו and vowel marks are skipped");
        assert_eq!(logits[2], -3.0);
        // A word that already left the list is not pushed down a second time.
        let mut logits = vec![0.0f32; 10];
        c.filter(&[0, 2], &mut logits);
        assert_eq!(logits[1], 0.0);
        assert_eq!(logits[4], 0.0);
    }

    #[test]
    fn pieces_far_below_the_best_are_left_alone_and_zero_weight_is_off() {
        let c = check(3.0, false);
        let mut logits = vec![-50.0f32; 10];
        logits[1] = 0.0;
        c.filter(&[0], &mut logits);
        assert_eq!(logits[2], -50.0);
        let c = check(0.0, false);
        let mut logits = vec![0.0f32; 10];
        c.filter(&[0], &mut logits);
        assert!(logits.iter().all(|&l| l == 0.0));
    }

    #[test]
    fn a_phrase_written_twice_cannot_be_written_a_third_time() {
        let c = check(0.0, true);
        let phrase = [3, 0, 1, 4, 7, 8];
        let mut written: Vec<i32> = Vec::new();
        written.extend(phrase);
        written.extend(phrase);
        // Twice so far: the second time was still allowed.
        let mut logits = vec![0.0f32; 10];
        c.filter(&written[..11], &mut logits);
        assert!(logits.iter().all(|&l| l == 0.0));
        // The third time through, the piece that would finish the run is shut out.
        written.extend(&phrase[..5]);
        let mut logits = vec![0.0f32; 10];
        c.filter(&written, &mut logits);
        assert_eq!(logits[8], f32::NEG_INFINITY);
        assert_eq!(logits.iter().filter(|l| l.is_finite()).count(), 9);
    }
}
