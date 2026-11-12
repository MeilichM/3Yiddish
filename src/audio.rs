//! Microphone and speaker. Computers: rodio/cpal directly, no ffmpeg or ffplay needed.
//! Android (Termux): through the Termux:API add-on, since a terminal app cannot open the
//! phone's audio devices itself.

/// The longest single recording, matching the phone's limit.
pub const MAX_SECONDS: u32 = 120;

#[cfg(not(target_os = "android"))]
pub use native::*;
#[cfg(target_os = "android")]
pub use termux::*;

#[cfg(not(target_os = "android"))]
mod native {
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

    use super::MAX_SECONDS;
    /// The longest to wait after Stop for audio still on its way from the system.
    const MAX_WAIT_MS: u64 = 4000;

    pub struct Recorder {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<(Vec<f32>, u32, u16)>>,
        samples: Arc<std::sync::Mutex<Vec<f32>>>,
        opened: mpsc::Receiver<Result<(), String>>,
        failure: Option<String>,
    }

    impl Recorder {
        /// Start recording and wait until the microphone is open. Errors (no mic, no
        /// permission) come back here, not at stop.
        pub fn start() -> Result<Self> {
            let recorder = Self::start_now();
            match recorder.opened.recv() {
                Ok(Ok(())) => Ok(recorder),
                Ok(Err(e)) => Err(anyhow!("{e}")),
                Err(_) => Err(anyhow!("Cannot open the microphone")),
            }
        }

        /// Start recording without waiting: opening the device happens on its own thread, so
        /// the caller (the screen) never stalls. Ask `failed()` later.
        pub fn start_now() -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let (ready, opened) = mpsc::channel::<Result<(), String>>();
            let samples = Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
            let shared = samples.clone();
            let thread = std::thread::spawn(move || {
                let opened = open_input(samples.clone());
                let (stream, rate, channels) = match opened {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ready.send(Err(e.to_string()));
                        return (Vec::new(), 16_000, 1);
                    }
                };
                let _ = ready.send(Ok(()));
                let opened_at = std::time::Instant::now();
                let limit = (rate * MAX_SECONDS) as usize * channels as usize;
                while !flag.load(Ordering::Relaxed) && samples.lock().map_or(0, |s| s.len()) < limit
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                // Sound reaches us late (the system's audio buffer can hold a second or more),
                // so at Stop the last words have not arrived yet. Keep listening until as much
                // audio has come in as time has passed since the microphone opened; that is
                // exactly everything said up to the moment of Stop, whatever the delay.
                let stopped_at = std::time::Instant::now();
                let want = (((stopped_at - opened_at).as_secs_f64() * rate as f64) as usize
                    * channels as usize)
                    .min(limit);
                let give_up = stopped_at + std::time::Duration::from_millis(MAX_WAIT_MS);
                while samples.lock().map_or(0, |s| s.len()) < want
                    && std::time::Instant::now() < give_up
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                drop(stream);
                let mut taken =
                    std::mem::take(&mut *samples.lock().unwrap_or_else(|e| e.into_inner()));
                taken.truncate(want.max(channels as usize));
                (taken, rate, channels)
            });
            Self {
                stop,
                thread: Some(thread),
                samples: shared,
                opened,
                failure: None,
            }
        }

        /// Why the microphone could not be opened, once that is known.
        pub fn failed(&mut self) -> Option<String> {
            if self.failure.is_none()
                && let Ok(Err(e)) = self.opened.try_recv()
            {
                self.failure = Some(e);
            }
            self.failure.clone()
        }

        /// How loud the microphone is right now, 0 to 1 (the peak of the last moment).
        pub fn level(&self) -> f32 {
            self.samples.lock().map_or(0.0, |s| {
                s[s.len().saturating_sub(2400)..]
                    .iter()
                    .fold(0.0f32, |peak, v| peak.max(v.abs()))
            })
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
            Ok(declick(crate::asr::resample(&mono, rate)))
        }
    }

    /// Microphones pop when they switch on. Drop that first instant and ease the take in and
    /// out, so there is no click at either end.
    fn declick(mut pcm: Vec<f32>) -> Vec<f32> {
        const SKIP: usize = 16_000 * 12 / 100; // the first 0.12 s
        const FADE: usize = 16_000 * 3 / 100; // 30 ms
        if pcm.len() > SKIP * 4 {
            pcm.drain(..SKIP);
        }
        let fade = FADE.min(pcm.len() / 2);
        let n = pcm.len();
        for i in 0..fade {
            let gain = i as f32 / fade as f32;
            pcm[i] *= gain;
            pcm[n - 1 - i] *= gain;
        }
        pcm
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Plays one file on the PC speakers. WAV/MP3/FLAC/OGG/M4A play natively; anything else
    /// (old WebM/Opus phone recordings) falls back to ffplay when it is installed.
    pub enum Playback {
        /// The output, the player and the length in seconds.
        Native(rodio::MixerDeviceSink, rodio::Player, f32),
        External(std::process::Child),
    }

    impl Playback {
        pub fn start(path: &Path) -> Result<Self> {
            match std::fs::read(path)
                .map_err(anyhow::Error::from)
                .and_then(crate::asr::decode)
            {
                Ok(pcm) => {
                    let seconds = pcm.len() as f32 / 16_000.0;
                    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()
                        .context("No speaker available")?;
                    // Stopping is normal here; the library's warning would land on the screen.
                    sink.log_on_drop(false);
                    let player = rodio::Player::connect_new(sink.mixer());
                    player.append(rodio::buffer::SamplesBuffer::new(
                        std::num::NonZero::new(1).unwrap(),
                        std::num::NonZero::new(16_000).unwrap(),
                        pcm,
                    ));
                    Ok(Self::Native(sink, player, seconds))
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

        /// The length in seconds, when known.
        pub fn seconds(&self) -> Option<f32> {
            match self {
                Self::Native(_, _, seconds) => Some(*seconds),
                Self::External(_) => None,
            }
        }

        pub fn set_paused(&self, paused: bool) {
            if let Self::Native(_, player, _) = self {
                if paused {
                    player.pause();
                } else {
                    player.play();
                }
            }
        }

        /// Jump to `seconds` from the start.
        pub fn seek(&self, seconds: f32) {
            if let Self::Native(_, player, _) = self {
                let _ = player.try_seek(std::time::Duration::from_secs_f32(seconds.max(0.0)));
            }
        }

        /// True when playback has ended.
        pub fn done(&mut self) -> bool {
            match self {
                Self::Native(_, player, _) => player.empty(),
                Self::External(child) => child.try_wait().ok().flatten().is_some(),
            }
        }

        pub fn stop(&mut self) {
            match self {
                Self::Native(_, player, _) => player.stop(),
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
    fn open_input(
        samples: Arc<std::sync::Mutex<Vec<f32>>>,
    ) -> Result<(rodio::cpal::Stream, u32, u16)> {
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
            .context(
                "No microphone found. Plug one in (or check your sound settings) and try again.",
            )?;
        let config = device
            .default_input_config()
            .context("The microphone has no usable settings")?;
        let (rate, channels) = (config.sample_rate(), config.channels());
        let push = move |data: &mut dyn Iterator<Item = f32>| {
            if let Ok(mut s) = samples.lock() {
                s.extend(data);
            }
        };
        use rodio::cpal::SampleFormat as F;
        let cfg = config.config();
        let stream = match config.sample_format() {
            F::F32 => build::<f32>(&device, &cfg, push),
            F::F64 => build::<f64>(&device, &cfg, push),
            F::I8 => build::<i8>(&device, &cfg, push),
            F::I16 => build::<i16>(&device, &cfg, push),
            F::I32 => build::<i32>(&device, &cfg, push),
            F::I64 => build::<i64>(&device, &cfg, push),
            F::U8 => build::<u8>(&device, &cfg, push),
            F::U16 => build::<u16>(&device, &cfg, push),
            F::U32 => build::<u32>(&device, &cfg, push),
            F::U64 => build::<u64>(&device, &cfg, push),
            other => bail!("Unsupported microphone format {other:?}"),
        }
        .with_context(|| format!("Cannot open the microphone {}", name(&device)))?;
        stream.play()?;
        Ok((stream, rate, channels))
    }

    /// Open an input stream for any sample type, converting every sample to f32.
    fn build<T>(
        device: &rodio::cpal::Device,
        config: &rodio::cpal::StreamConfig,
        push: impl Fn(&mut dyn Iterator<Item = f32>) + Send + 'static,
    ) -> Result<rodio::cpal::Stream>
    where
        T: rodio::cpal::SizedSample,
        f32: rodio::cpal::FromSample<T>,
    {
        use rodio::cpal::traits::DeviceTrait;
        Ok(device.build_input_stream(
            *config,
            move |d: &[T], _| push(&mut d.iter().map(|s| s.to_sample::<f32>())),
            // A dropped buffer is not worth printing over the screen; the take goes on.
            |_| {},
            None,
        )?)
    }
}

#[cfg(target_os = "android")]
mod termux {
    use super::MAX_SECONDS;
    use anyhow::{Context, Result, bail};
    use std::{
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::Instant,
    };

    const NEEDS_API: &str =
        "Install the Termux:API app, then run: pkg install termux-api (and allow the microphone)";

    fn termux(program: &str, args: &[&str]) -> Result<std::process::Output> {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .context(NEEDS_API)
    }

    pub struct Recorder {
        path: PathBuf,
        started: Instant,
        failure: Option<String>,
    }

    impl Recorder {
        /// Same as `start`, but a failure is reported through `failed()`.
        pub fn start_now() -> Self {
            Self::start().unwrap_or_else(|e| Self {
                path: PathBuf::new(),
                started: Instant::now(),
                failure: Some(e.to_string()),
            })
        }

        pub fn failed(&mut self) -> Option<String> {
            self.failure.clone()
        }

        pub fn start() -> Result<Self> {
            let path = std::path::absolute(format!("state/.take-{}.m4a", uuid::Uuid::new_v4()))?;
            let limit = MAX_SECONDS.to_string();
            let out = termux(
                "termux-microphone-record",
                &["-e", "aac", "-l", &limit, "-f", &path.to_string_lossy()],
            )?;
            let said = String::from_utf8_lossy(&out.stdout).to_lowercase();
            if !out.status.success() || said.contains("error") || said.contains("denied") {
                bail!("The microphone did not start. {NEEDS_API}");
            }
            Ok(Self {
                path,
                started: Instant::now(),
                failure: None,
            })
        }

        /// No live level from the phone's recorder.
        pub fn level(&self) -> f32 {
            0.0
        }

        pub fn finished(&self) -> bool {
            self.started.elapsed().as_secs() >= MAX_SECONDS as u64
        }

        pub fn stop(self) -> Result<Vec<f32>> {
            let _ = termux("termux-microphone-record", &["-q"]);
            // The recorder closes the file a moment after it is told to stop.
            let mut bytes = Vec::new();
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(150));
                if let Ok(b) = std::fs::read(&self.path)
                    && !b.is_empty()
                    && crate::asr::decode(b.clone()).is_ok()
                {
                    bytes = b;
                    break;
                }
            }
            let _ = std::fs::remove_file(&self.path);
            if bytes.is_empty() {
                bail!("The recording came back empty. {NEEDS_API}");
            }
            crate::asr::decode(bytes)
        }
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            let _ = Command::new("termux-microphone-record")
                .arg("-q")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    /// Plays through the `play-audio` package (pkg install play-audio).
    pub struct Playback(Child);

    impl Playback {
        pub fn start(path: &Path) -> Result<Self> {
            Ok(Self(
                Command::new("play-audio")
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .context("To listen back, run: pkg install play-audio")?,
            ))
        }
        pub fn seconds(&self) -> Option<f32> {
            None
        }
        pub fn set_paused(&self, _paused: bool) {}
        pub fn seek(&self, _seconds: f32) {}
        pub fn done(&mut self) -> bool {
            self.0.try_wait().ok().flatten().is_some()
        }
        pub fn stop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    impl Drop for Playback {
        fn drop(&mut self) {
            self.stop();
        }
    }
}
