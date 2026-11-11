//! In-process Transcribe: whisper.cpp (via whisper-rs) with Silero VAD, plus audio decoding.
//! Replaces the Python speech worker; settings match the app's benchmarked ones
//! (language yi, beam 5, VAD on, no conditioning on previous text).
use anyhow::{Context, Result, bail};
use std::{io::Cursor, path::Path, sync::Mutex};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const RATE: u32 = 16_000;

pub struct Engine {
    ctx: WhisperContext,
    vad: Option<String>,
    // whisper.cpp states are heavy; one transcription at a time per engine.
    busy: Mutex<()>,
}

impl Engine {
    pub fn load(model: &Path, vad: Option<&Path>) -> Result<Self> {
        whisper_rs::install_logging_hooks(); // route whisper.cpp's console spam to (silent) log
        let params = WhisperContextParameters {
            flash_attn: true, // faster encoder on CPU, same output
            ..Default::default()
        };
        let ctx = WhisperContext::new_with_params(model, params)
            .with_context(|| format!("Cannot load the Transcribe model {}", model.display()))?;
        Ok(Self {
            ctx,
            vad: vad.map(|p| p.to_string_lossy().into_owned()),
            busy: Mutex::new(()),
        })
    }

    /// 16 kHz mono samples in, Yiddish text out.
    pub fn transcribe(&self, pcm: &[f32]) -> Result<String> {
        let _one = self
            .busy
            .lock()
            .map_err(|_| anyhow::anyhow!("Transcribe engine poisoned"))?;
        let mut state = self.ctx.create_state()?;
        let beam = std::env::var("THREE_YIDDISH_BEAM")
            .ok()
            .and_then(|b| b.parse().ok())
            .unwrap_or(5);
        let mut p = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: beam,
            patience: -1.0,
        });
        p.set_language(Some("yi"));
        p.set_no_context(true);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_special(false);
        p.set_print_timestamps(false);
        p.set_n_threads(threads());
        if let Some(vad) = &self.vad {
            p.set_vad_model_path(Some(vad)); // must come before enable_vad
            p.enable_vad(true);
        }
        state.full(p, pcm)?;
        let mut text = Vec::new();
        for i in 0..state.full_n_segments() {
            if let Some(s) = state.get_segment(i) {
                let t = s.to_str_lossy()?.trim().to_owned();
                if !t.is_empty() {
                    text.push(t);
                }
            }
        }
        Ok(text.join(" "))
    }
}

/// Threads for whisper.cpp: `$THREE_YIDDISH_THREADS`, else most of the cores (leave two free).
fn threads() -> i32 {
    std::env::var("THREE_YIDDISH_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(4, |n| n.get().saturating_sub(2).clamp(1, 16)) as i32
        })
}

/// Any common audio (WAV, MP3, FLAC, OGG Vorbis, M4A/AAC) to 16 kHz mono f32.
pub fn decode(bytes: Vec<u8>) -> Result<Vec<f32>> {
    use symphonia::core::{
        codecs::audio::AudioDecoderOptions,
        formats::{FormatOptions, TrackType, probe::Hint},
        io::MediaSourceStream,
        meta::MetadataOptions,
    };
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Hint::new(),
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .context("Unsupported audio; use WAV, MP3, FLAC, OGG or M4A")?;
    let track = format
        .default_track(TrackType::Audio)
        .context("No audio track")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|c| c.audio())
        .context("No audio parameters")?
        .clone();
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .context("Unsupported audio codec (Opus/WebM is not supported; use WAV, MP3 or M4A)")?;
    let (mut mono, mut rate, mut buf) = (Vec::new(), 0u32, Vec::<f32>::new());
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        rate = audio.spec().rate();
        let channels = audio.spec().channels().count().max(1);
        buf.resize(audio.samples_interleaved(), 0.0);
        audio.copy_to_slice_interleaved(&mut buf);
        mono.extend(
            buf.chunks(channels)
                .map(|f| f.iter().sum::<f32>() / channels as f32),
        );
        if mono.len() > 11 * 60 * 48_000 {
            bail!("Audio exceeds ten minutes; split it into shorter recordings");
        }
    }
    if mono.is_empty() || rate == 0 {
        bail!("Audio could not be decoded");
    }
    Ok(resample(&mono, rate))
}

/// Resample to 16 kHz: windowed-sinc low-pass when downsampling, then linear interpolation.
// ponytail: simple FIR + linear interpolation is plenty for speech recognition; swap for a
// polyphase resampler only if a benchmark shows a loss on 44.1/48 kHz input.
pub fn resample(input: &[f32], rate: u32) -> Vec<f32> {
    if rate == RATE {
        return input.to_vec();
    }
    let filtered = if rate > RATE {
        let cutoff = 0.95 * RATE as f32 / 2.0 / rate as f32; // normalized to the input rate
        let taps = 63i32;
        let kernel: Vec<f32> = (-(taps / 2)..=taps / 2)
            .map(|n| {
                let x = n as f32;
                let sinc = if n == 0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
                };
                let w = 0.42
                    + 0.5 * (std::f32::consts::PI * x / (taps / 2) as f32).cos()
                    + 0.08 * (2.0 * std::f32::consts::PI * x / (taps / 2) as f32).cos();
                sinc * w
            })
            .collect();
        let half = taps as isize / 2;
        (0..input.len() as isize)
            .map(|i| {
                kernel
                    .iter()
                    .enumerate()
                    .map(|(k, c)| {
                        c * input
                            .get((i + k as isize - half) as usize)
                            .copied()
                            .unwrap_or(0.0)
                    })
                    .sum()
            })
            .collect::<Vec<f32>>()
    } else {
        input.to_vec()
    };
    let step = rate as f64 / RATE as f64;
    let n = (filtered.len() as f64 / step) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * step;
            let j = x as usize;
            let f = (x - j as f64) as f32;
            let a = filtered[j];
            let b = *filtered.get(j + 1).unwrap_or(&a);
            a + (b - a) * f
        })
        .collect()
}

/// 16 kHz mono f32 to a 16-bit WAV file's bytes (recordings, phone uploads, playback).
pub fn wav(pcm: &[f32]) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = Cursor::new(Vec::new());
    let mut w = hound::WavWriter::new(&mut out, spec)?;
    for s in pcm {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    w.finalize()?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_wav_and_resamples_to_16k() {
        let tone: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut out = Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        for s in &tone {
            w.write_sample((s * 30000.0) as i16).unwrap();
            w.write_sample((s * 30000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        let pcm = decode(out.into_inner()).unwrap();
        assert!((pcm.len() as i32 - 16_000).abs() < 10, "{}", pcm.len());
        assert!(pcm.iter().map(|s| s.abs()).fold(0.0, f32::max) > 0.3);
        assert_eq!(decode(wav(&pcm).unwrap()).unwrap().len(), pcm.len());
        assert!(decode(b"not audio".to_vec()).is_err());
    }
}
