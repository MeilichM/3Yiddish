//! In-process Transcribe: whisper.cpp (via whisper-rs) with Silero VAD, plus audio decoding.
//! Replaces the Python speech worker; settings match the app's benchmarked ones
//! (language yi, beam 5, VAD on, no conditioning on previous text).
use anyhow::{Context, Result, bail};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const RATE: u32 = 16_000;

/// Where the speech model runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compute {
    Gpu,
    Cpu,
}

impl Compute {
    /// The word used in `state/compute`, `$THREE_YIDDISH_COMPUTE` and the service's JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word.trim() {
            "gpu" => Some(Self::Gpu),
            "cpu" => Some(Self::Cpu),
            _ => None,
        }
    }

    /// Plain words for people (`3yiddish model status`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Gpu if cfg!(target_os = "macos") => "GPU (Metal)",
            Self::Gpu => "GPU (Vulkan)",
            Self::Cpu => "CPU",
        }
    }
}

/// Whether this binary can use a GPU at all: Vulkan builds, and every macOS build (Metal).
pub fn gpu_built() -> bool {
    cfg!(feature = "vulkan") || cfg!(target_os = "macos")
}

// The device list of ggml, the library under whisper.cpp (linked in with it).
unsafe extern "C" {
    fn ggml_backend_dev_count() -> usize;
    fn ggml_backend_dev_get(index: usize) -> *mut std::ffi::c_void;
    fn ggml_backend_dev_type(device: *mut std::ffi::c_void) -> std::ffi::c_int;
}

/// How many GPUs (separate or built into the processor) ggml can run on.
fn gpu_devices() -> usize {
    whisper_rs::install_logging_hooks(); // keep the driver's start-up lines off the screen
    // SAFETY: plain queries of ggml's own device list; every index is below its count.
    unsafe {
        (0..ggml_backend_dev_count())
            .filter(|&i| matches!(ggml_backend_dev_type(ggml_backend_dev_get(i)), 1 | 2))
            .count()
    }
}

/// Whether the GPU can really be used: this binary has GPU support and the computer has a
/// GPU it can reach. whisper.cpp quietly runs on the CPU when asked for a GPU that is not
/// there, so the app looks first (once per run).
pub fn gpu_usable() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| gpu_built() && gpu_devices() > 0)
}

/// The choice order: the environment (one run), then the saved choice, then automatic (GPU
/// when there is one). Unknown words are skipped, and GPU is never picked without one.
fn resolve(env: Option<&str>, file: Option<&str>, gpu: bool) -> Compute {
    match env
        .and_then(Compute::parse)
        .or_else(|| file.and_then(Compute::parse))
    {
        Some(Compute::Cpu) => Compute::Cpu,
        _ if gpu => Compute::Gpu,
        _ => Compute::Cpu,
    }
}

/// What to run on: `$THREE_YIDDISH_COMPUTE`, else the choice saved in `state/compute`, else
/// the GPU when it can be used. Call it from the app's home folder.
pub fn preferred() -> Compute {
    preferred_at(&crate::paths::compute())
}

/// `preferred` for a choice file somewhere else (commands that do not enter the home folder).
pub fn preferred_at(file: &Path) -> Compute {
    resolve(
        std::env::var("THREE_YIDDISH_COMPUTE").ok().as_deref(),
        std::fs::read_to_string(file).ok().as_deref(),
        gpu_usable(),
    )
}

/// Save the user's choice for later runs.
pub fn remember(compute: Compute) -> Result<()> {
    remember_at(&crate::paths::compute(), compute)
}

/// `remember` for a choice file somewhere else. The file is swapped in whole, so a reader
/// never sees half a word.
pub fn remember_at(file: &Path, compute: Compute) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options.open(&tmp)?;
    out.write_all(compute.name().as_bytes())?;
    out.sync_all()?;
    std::fs::rename(tmp, file)?;
    Ok(())
}

pub struct Engine {
    model: PathBuf,
    vad: Option<String>,
    // whisper.cpp states are heavy; one transcription at a time per engine. The lock also
    // lets a GPU context that fails on first use be swapped for a CPU one.
    loaded: Mutex<Loaded>,
    /// Running on the GPU right now; readable without waiting for a transcription.
    gpu: AtomicBool,
}

struct Loaded {
    ctx: WhisperContext,
    /// On the GPU and no transcription has succeeded yet: a failure now means the GPU or its
    /// driver does not work, so the CPU takes over.
    unproven_gpu: bool,
}

fn context(model: &Path, gpu: bool) -> Result<WhisperContext> {
    let params = WhisperContextParameters {
        use_gpu: gpu,
        // Faster encoder, same benchmark score on CPU and GPU. THREE_YIDDISH_FLASH=0 turns it
        // off (for checks).
        flash_attn: std::env::var("THREE_YIDDISH_FLASH").map_or(true, |v| v != "0"),
        ..Default::default()
    };
    WhisperContext::new_with_params(model, params)
        .with_context(|| format!("Cannot load the Transcribe model {}", model.display()))
}

/// The one line a GPU failure leaves on stderr before the CPU takes over.
fn report_gpu_failure(error: &anyhow::Error) {
    eprintln!("3Yiddish: the GPU could not be used ({error:#}); using the CPU instead.");
}

impl Engine {
    /// Load the model for `compute`. A GPU that cannot be used falls back to the CPU (here or
    /// on the first transcription) so the request still succeeds; `compute()` tells which one
    /// the engine ended up on.
    pub fn load(model: &Path, vad: Option<&Path>, compute: Compute) -> Result<Self> {
        whisper_rs::install_logging_hooks(); // route whisper.cpp's console spam to (silent) log
        let gpu = compute == Compute::Gpu && gpu_usable();
        let (ctx, gpu) = match context(model, gpu) {
            Err(e) if gpu => {
                // A model that cannot load at all fails here too, before anything is logged.
                let ctx = context(model, false)?;
                report_gpu_failure(&e);
                (ctx, false)
            }
            loaded => (loaded?, gpu),
        };
        Ok(Self {
            model: model.to_owned(),
            vad: vad.map(|p| p.to_string_lossy().into_owned()),
            loaded: Mutex::new(Loaded {
                ctx,
                unproven_gpu: gpu,
            }),
            gpu: AtomicBool::new(gpu),
        })
    }

    /// What the engine runs on now (CPU after a GPU fallback).
    pub fn compute(&self) -> Compute {
        if self.gpu.load(Ordering::Relaxed) {
            Compute::Gpu
        } else {
            Compute::Cpu
        }
    }

    /// 16 kHz mono samples in, Yiddish text out.
    pub fn transcribe(&self, pcm: &[f32]) -> Result<String> {
        let mut loaded = self
            .loaded
            .lock()
            .map_err(|_| anyhow::anyhow!("Transcribe engine poisoned"))?;
        let result = self.run(&loaded.ctx, pcm);
        match result {
            Err(e) if loaded.unproven_gpu => {
                // Blame the GPU only when the CPU manages the same audio; otherwise the
                // audio or the model is the problem and the GPU engine stays.
                let cpu = context(&self.model, false)?;
                let text = self.run(&cpu, pcm)?;
                report_gpu_failure(&e);
                loaded.ctx = cpu;
                loaded.unproven_gpu = false;
                self.gpu.store(false, Ordering::Relaxed);
                Ok(text)
            }
            result => {
                loaded.unproven_gpu = false;
                result
            }
        }
    }

    fn run(&self, ctx: &WhisperContext, pcm: &[f32]) -> Result<String> {
        let mut state = ctx.create_state()?;
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
    fn compute_choice_is_env_then_file_then_automatic() {
        use Compute::{Cpu, Gpu};
        // Automatic: the GPU when there is one.
        assert_eq!(resolve(None, None, true), Gpu);
        assert_eq!(resolve(None, None, false), Cpu);
        // The saved choice, with the newline an editor leaves.
        assert_eq!(resolve(None, Some("cpu\n"), true), Cpu);
        assert_eq!(resolve(None, Some("gpu"), true), Gpu);
        // The environment wins over the file.
        assert_eq!(resolve(Some("cpu"), Some("gpu"), true), Cpu);
        assert_eq!(resolve(Some("gpu"), Some("cpu"), true), Gpu);
        // GPU is never picked by a binary without one.
        assert_eq!(resolve(Some("gpu"), Some("gpu"), false), Cpu);
        // Unknown words fall through to the next source.
        assert_eq!(resolve(Some("fast"), Some("cpu"), true), Cpu);
        assert_eq!(resolve(None, Some("GPU please"), true), Gpu);
        assert_eq!(resolve(Some(""), Some(""), false), Cpu);
    }

    #[test]
    fn gpu_is_usable_only_when_built_in_and_present() {
        // Asks ggml for its devices (no model is loaded); the answer depends on the computer.
        let found = gpu_devices();
        assert_eq!(gpu_usable(), gpu_built() && found > 0);
    }

    #[test]
    fn compute_choice_is_saved_whole_and_read_back() {
        let dir = std::env::temp_dir().join(format!("3y-compute-{}", uuid::Uuid::new_v4()));
        let file = dir.join("state").join("compute");
        for choice in [Compute::Cpu, Compute::Gpu, Compute::Cpu] {
            remember_at(&file, choice).unwrap();
            let saved = std::fs::read_to_string(&file).unwrap();
            assert_eq!(saved, choice.name());
            assert_eq!(resolve(None, Some(&saved), true), choice);
        }
        assert!(!file.with_extension("tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

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
