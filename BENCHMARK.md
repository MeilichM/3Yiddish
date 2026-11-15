# 3Yiddish benchmark

How good is it? Honest numbers.

Not the best yet. We tested 16 Yiddish speech-to-text tools on 25 takes of free heimish
speech (1,191 words): websites, programs and open models, paid and free.

**Wrong words per 100** is the main number: lower is better, and it does not change when
more tools are added. The best in each column is bold.

| | What it is | Wrong words per 100 | Score out of 100 | Best in a take (of 25) | Takes with nothing wrong (of 25) | Loshn koydesh wrong (of 13) | English words wrong (of 14) | Names wrong (of 3) |
|---|---|---|---|---|---|---|---|---|
| Yiddish Labs | Paid website | **2.9** | **82.9** | **15** | **14** | **1** | **0** | **0** |
| Yiddish Tools | Paid website | 4.7 | 67.6 | 6 | 4 | 2 | 3 | **0** |
| **3Yiddish** | Free, on your own computer | 12.6 | 59.8 | 3 | 2 | 12 | 14 | 3 |
| Devniks Voice | Free website | 13.5 | 60.7 | 4 | 4 | 5 | 6 | **0** |
| ivrit-ai turbo | Free open model | 15.6 | 55.8 | 2 | 2 | 10 | 13 | 1 |
| ivrit-ai large-v3 | Free open model | 18.3 | 59.2 | 5 | 5 | 11 | 8 | 2 |
| Red Flow | Free website and app, online | 18.6 | 53.2 | 2 | 2 | 13 | 12 | 2 |
| Yiddi Type, fast model | Free Windows program, small built-in model | 31.6 | 40.2 | 0 | 0 | 13 | 13 | 3 |
| Heimish Geredt | Paid website (free trial) | 32.9 | 39.6 | 0 | 0 | 11 | 6 | 3 |
| Omnilingual ASR CTC 300M | Free open model (small) | 61.3 | 26.8 | 0 | 0 | 13 | 14 | 3 |
| Whisper large-v3 (plain) | Free open model | 92.9 | 12.9 | 0 | 0 | 13 | 14 | 3 |
| Whisper medium (plain) | Free open model (small) | 95.9 | 10.5 | 0 | 0 | 13 | 14 | 3 |
| Whisper small (plain) | Free open model (small) | 97.1 | 9.2 | 0 | 0 | 13 | 14 | 3 |
| Whisper turbo (plain) | Free open model | 110.4 | 3.9 | 0 | 0 | 13 | 14 | 3 |
| Whisper base, Yiddish word-list cut (alphaedge) | Free open model (small) | 226 | 3.4 | 0 | 0 | 13 | 14 | 3 |
| Whisper tiny, Yiddish word-list cut (alphaedge) | Free open model (small) | 302 | 2.3 | 0 | 0 | 13 | 14 | 3 |

The score is the plain average of three parts: words right, the place in each take (best
gets full points, worst gets none), and the share of takes with nothing wrong. It depends
on who else is in the table, so it moves when tools are added.

**How it was judged**

- One speaker read the transcripts of Yiddish Labs, Yiddish Tools and 3Yiddish without
  knowing which was which, ranked them in every take, and judged every word.
- The others were added later. Where one of them wrote the same words he had already
  judged, his verdict stands. Everything else was judged by AI agents following his
  earlier answers. He has not gone over those yet.
- A tool he did not rank is placed in each take by its wrong words. That rule matched his
  own order in 58 of 63 pairs.

**How each one was run**

- Websites got one take at a time, with Yiddish selected, and the text was saved exactly
  as shown.
- Devniks Voice returns its own result plus a second pass through Google's Gemini; that
  combined result is what was scored.
- Red Flow takes a live microphone only, up to 30 seconds, so each take was played into
  it. Two takes ran slightly over and lost their last words.
- Heimish Geredt stopped early or skipped a stretch in six takes, which counts as wrong
  words.
- Yiddi Type is a Windows program. We scored the small model that comes inside it, with
  the program's own settings, without the program's extra clean-up steps. It also offers
  a larger model, which the program describes as ivrit-ai's.
- Open models ran with the settings their authors give. 3Yiddish ran as the app runs it.

**What to keep in mind**

- It's 25 takes from one speaker. More takes and more speakers are coming.
- English words written in English letters, and numbers in digits, are not counted as
  mistakes when they are the words that were said.
- Plain Whisper, and the small cuts of it that only had their word list trimmed to Yiddish, were never trained on heimish speech. They
  spell by rough sound, write other languages, or repeat themselves, so a tool can go far
  over 100 wrong words per 100.
- The answer key grew out of 3Yiddish's own draft, corrected by the speaker. That can
  help 3Yiddish and tools built on the same ivrit-ai models where a wrong word was missed
  in the key.
- This is where we are, and it's the number every new model has to beat.
