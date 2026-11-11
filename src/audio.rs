//! Cross-platform PC microphone and speaker (rodio/cpal): no ffmpeg or ffplay needed.
use anyhow::{Context, Result, anyhow};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

/// The longest single recording, matching the phone's limit.
pub const MAX_SECONDS: u32 = 120;

pub struct Recorder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<(Vec<f32>, u32, u16)>>,
}

impl Recorder {
    /// Start recording from the default microphone. Errors (no mic, no permission) come back
    /// here, not at stop.
    pub fn start() -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (ready, opened) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::spawn(move || {
            let mic = match rodio::microphone::MicrophoneBuilder::new()
                .default_device()
                .and_then(|b| b.default_config())
                .map_err(|e| e.to_string())
                .and_then(|b| b.open_stream().map_err(|e| e.to_string()))
            {
                Ok(mic) => mic,
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return (Vec::new(), 16_000, 1);
                }
            };
            let config = mic.config();
            let (rate, channels) = (config.sample_rate.get(), config.channel_count.get());
            let _ = ready.send(Ok(()));
            let limit = (rate * MAX_SECONDS) as usize * channels as usize;
            let mut samples = Vec::with_capacity(rate as usize * channels as usize * 10);
            for s in mic {
                samples.push(s);
                if samples.len() >= limit
                    || (samples.len() % 512 == 0 && flag.load(Ordering::Relaxed))
                {
                    break;
                }
            }
            (samples, rate, channels)
        });
        match opened.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(anyhow!("Cannot open the microphone: {e}")),
            Err(_) => Err(anyhow!("Cannot open the microphone")),
        }
    }

    /// True once the recording hit its time limit by itself.
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Stop and return 16 kHz mono samples.
    pub fn stop(mut self) -> Result<Vec<f32>> {
        self.stop.store(true, Ordering::Relaxed);
        let (samples, rate, channels) = self
            .thread
            .take()
            .context("Recording already stopped")?
            .join()
            .map_err(|_| anyhow!("The recording thread crashed"))?;
        let ch = channels.max(1) as usize;
        let mono: Vec<f32> = samples
            .chunks(ch)
            .map(|f| f.iter().sum::<f32>() / ch as f32)
            .collect();
        Ok(crate::asr::resample(&mono, rate))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Plays one file on the PC speakers. WAV/MP3/FLAC/OGG/M4A play natively; anything else
/// (old WebM/Opus phone recordings) falls back to ffplay when it is installed.
pub enum Playback {
    Native(rodio::MixerDeviceSink, rodio::Player),
    External(std::process::Child),
}

impl Playback {
    pub fn start(path: &Path) -> Result<Self> {
        match std::fs::read(path)
            .map_err(anyhow::Error::from)
            .and_then(crate::asr::decode)
        {
            Ok(pcm) => {
                let sink = rodio::DeviceSinkBuilder::open_default_sink()
                    .context("No speaker available")?;
                let player = rodio::Player::connect_new(sink.mixer());
                player.append(rodio::buffer::SamplesBuffer::new(
                    std::num::NonZero::new(1).unwrap(),
                    std::num::NonZero::new(16_000).unwrap(),
                    pcm,
                ));
                Ok(Self::Native(sink, player))
            }
            Err(_) => Ok(Self::External(
                std::process::Command::new("ffplay")
                    .args(["-nodisp", "-autoexit", "-loglevel", "error", "-i"])
                    .arg(path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .context("This recording's format needs ffplay to play")?,
            )),
        }
    }

    /// True when playback has ended.
    pub fn done(&mut self) -> bool {
        match self {
            Self::Native(_, player) => player.empty(),
            Self::External(child) => child.try_wait().ok().flatten().is_some(),
        }
    }

    pub fn stop(&mut self) {
        match self {
            Self::Native(_, player) => player.stop(),
            Self::External(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}
