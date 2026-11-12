//! Cross-platform PC microphone and speaker (rodio/cpal): no ffmpeg or ffplay needed.
use anyhow::{Context, Result, anyhow, bail};
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
    /// Start recording from the microphone. Errors (no mic, no permission) come back here,
    /// not at stop.
    pub fn start() -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (ready, opened) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::spawn(move || {
            let samples = Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
            let opened = open_input(samples.clone());
            let (stream, rate, channels) = match opened {
                Ok(v) => v,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return (Vec::new(), 16_000, 1);
                }
            };
            let _ = ready.send(Ok(()));
            let limit = (rate * MAX_SECONDS) as usize * channels as usize;
            while !flag.load(Ordering::Relaxed) && samples.lock().map_or(0, |s| s.len()) < limit {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            drop(stream);
            let taken = std::mem::take(&mut *samples.lock().unwrap_or_else(|e| e.into_inner()));
            (taken, rate, channels)
        });
        match opened.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(anyhow!("{e}")),
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

/// The microphone: on Linux through PulseAudio/PipeWire (ALSA's "default" capture is often
/// unavailable there), skipping speaker monitors; elsewhere the system default input.
fn open_input(samples: Arc<std::sync::Mutex<Vec<f32>>>) -> Result<(rodio::cpal::Stream, u32, u16)> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let name = |d: &rodio::cpal::Device| {
        d.description()
            .map(|x| x.name().to_string())
            .unwrap_or_default()
    };
    #[cfg(target_os = "linux")]
    let host = rodio::cpal::host_from_id(rodio::cpal::HostId::PulseAudio)
        .unwrap_or_else(|_| rodio::cpal::default_host());
    #[cfg(not(target_os = "linux"))]
    let host = rodio::cpal::default_host();
    let monitor = |d: &rodio::cpal::Device| name(d).starts_with("Monitor of");
    let device = host
        .default_input_device()
        .filter(|d| !monitor(d))
        .or_else(|| host.input_devices().ok()?.find(|d| !monitor(d)))
        .context("No microphone on this PC. Pair your phone and use it as the microphone.")?;
    let config = device
        .default_input_config()
        .context("The microphone has no usable settings")?;
    let (rate, channels) = (config.sample_rate(), config.channels());
    let push = move |data: &mut dyn Iterator<Item = f32>| {
        if let Ok(mut s) = samples.lock() {
            s.extend(data);
        }
    };
    let err = |e| eprintln!("Microphone error: {e}");
    let stream = match config.sample_format() {
        rodio::cpal::SampleFormat::F32 => device.build_input_stream(
            config.config(),
            move |d: &[f32], _| push(&mut d.iter().copied()),
            err,
            None,
        ),
        rodio::cpal::SampleFormat::I16 => device.build_input_stream(
            config.config(),
            move |d: &[i16], _| push(&mut d.iter().map(|s| *s as f32 / 32768.0)),
            err,
            None,
        ),
        rodio::cpal::SampleFormat::U16 => device.build_input_stream(
            config.config(),
            move |d: &[u16], _| push(&mut d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0)),
            err,
            None,
        ),
        other => bail!("Unsupported microphone format {other:?}"),
    }
    .with_context(|| format!("Cannot open the microphone {}", name(&device)))?;
    stream.play()?;
    Ok((stream, rate, channels))
}
