use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};
use three_yiddish::{
    config::Config,
    service::{App, router},
};

#[derive(Parser)]
#[command(
    version,
    about = "3Yiddish: Yiddish speech to text, on your own computer. Run `3yiddish` to open the app."
)]
struct Cli {
    #[arg(
        long,
        global = true,
        default_value = "http://127.0.0.1:8793",
        env = "THREE_YIDDISH_URL"
    )]
    server: String,
    #[arg(long, global = true, default_value = "config.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum ModelAction {
    /// Download the latest model from GitHub if it is newer (resumes, verifies checksums).
    Pull,
    /// Show the installed model.
    Status,
    /// Switch back to the previous model.
    Rollback,
}
#[derive(Subcommand)]
enum Command {
    /// Run the HTTP service. Never starts or rents a model automatically.
    Serve,
    /// Open the terminal playground and live training monitor.
    Tui {
        #[arg(long, value_name = "NAME=STATUS_JSON")]
        watch: Vec<String>,
    },
    /// Check service configuration status without loading models or making inference calls.
    Doctor,
    /// Chat once, or enter an interactive conversation when text is omitted.
    #[command(hide = true)]
    Chat {
        text: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Translate yi-en or en-yi.
    #[command(hide = true)]
    Translate {
        #[arg(value_parser=["yi-en","en-yi"])]
        direction: String,
        text: String,
    },
    /// Transcribe an audio file; --reply runs the full language bridge.
    Transcribe {
        file: PathBuf,
        #[arg(long)]
        reply: bool,
        #[arg(long)]
        model: Option<String>,
    },
    /// Download the latest Transcribe model, show the installed one, or roll back.
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// Update 3Yiddish itself to the latest release.
    Update,
    /// Score the installed (or given) model on a test set: gold.json [{n, gold, audio}].
    Bench {
        gold: PathBuf,
        audio: PathBuf,
        #[arg(long)]
        model: Option<PathBuf>,
        #[arg(long)]
        vad: Option<PathBuf>,
        /// Print totals only (for public CI logs).
        #[arg(long)]
        summary: bool,
    },
    /// Search the local cultural notes.
    #[command(hide = true)]
    Knowledge { query: String },
    /// Run the fixed Schmooze conversation suite against the local model worker.
    #[command(hide = true)]
    Eval {
        #[arg(long, default_value = "docs/evaluation/schmooze-suite.toml")]
        suite: PathBuf,
        /// LoRA scales to compare, e.g. --scales 0,0.5,1
        #[arg(long, value_delimiter = ',', default_value = "1")]
        scales: Vec<f32>,
        /// Report path; defaults to docs/evaluation/schmooze-eval-<unix>.json
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Synthesize Yiddish speech to a WAV file using the local Speak model.
    #[command(hide = true)]
    Speak {
        text: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}
async fn response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body: Value = response.json().await.context("Invalid service response")?;
    if !status.is_success() {
        bail!(
            "Service returned {status}: {}",
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("request failed")
        );
    }
    Ok(body)
}
#[tokio::main]
async fn main() -> Result<()> {
    // A bare `3yiddish` opens the app.
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.len() == 1 {
        args.push("tui".into());
    }
    let mut cli = Cli::parse_from(args);
    if matches!(cli.command, Command::Update) {
        match tokio::task::spawn_blocking(three_yiddish::update::update).await?? {
            Some(version) => {
                println!("Updated to 3Yiddish {version}. Restart it to use the new version.")
            }
            None => println!("3Yiddish {} is the latest.", env!("CARGO_PKG_VERSION")),
        }
        return Ok(());
    }
    if let Command::Bench {
        gold,
        audio,
        model,
        vad,
        summary,
    } = &cli.command
    {
        let summary = *summary;
        let (gold, audio, model, vad) = (gold.clone(), audio.clone(), model.clone(), vad.clone());
        let report = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
            let (model, vad) = match model {
                Some(m) => (m, vad),
                None => {
                    let home = three_yiddish::paths::home()?;
                    let (_, m, v) = three_yiddish::model::current(&home.join(three_yiddish::paths::models()))
                        .context("No model installed; pass --model")?;
                    (m, vad.or(v))
                }
            };
            let engine = three_yiddish::asr::Engine::load(&model, vad.as_deref())?;
            let items: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&gold)?)?;
            let mut scores = Vec::new();
            for item in &items {
                let file = audio.join(item["audio"].as_str().context("gold item without audio")?);
                let heard = engine.transcribe(&three_yiddish::asr::decode(std::fs::read(&file)?)?)?;
                let (_, cer) = three_yiddish::tui::errors(item["gold"].as_str().unwrap_or_default(), &heard)?;
                scores.push(serde_json::json!({"n": item["n"], "score": (1.0 - cer).max(0.0), "heard": heard}));
            }
            let mean = scores.iter().filter_map(|s| s["score"].as_f64()).sum::<f64>() / scores.len().max(1) as f64;
            let mut report = serde_json::json!({"readings": scores.len(), "score": (mean * 1000.0).round() / 10.0});
            if !summary {
                report["items"] = scores.into();
            }
            Ok(report)
        })
        .await??;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    // Commands that run fully on this machine, in the app's home folder.
    let file_arg = match &cli.command {
        Command::Transcribe { file, .. } => Some(std::path::absolute(file)?),
        _ => None,
    };
    if matches!(
        cli.command,
        Command::Tui { .. } | Command::Serve | Command::Model { .. } | Command::Transcribe { .. }
    ) {
        let config = std::path::absolute(&cli.config)?;
        three_yiddish::paths::enter_home()?;
        cli.config = config;
    }
    let models = three_yiddish::paths::models();
    if let Command::Model { action } = &cli.command {
        match action {
            ModelAction::Pull => {
                let mut last = 0u64;
                let version = tokio::task::spawn_blocking(move || {
                    three_yiddish::model::pull(&models, &mut |name, done, total| {
                        let pct = done * 100 / total.max(1);
                        if pct != last {
                            last = pct;
                            eprint!("\r{name}: {pct}%   ");
                        }
                    })
                })
                .await??;
                eprintln!();
                println!("Transcribe model {version} is installed and active.");
            }
            ModelAction::Status => {
                match three_yiddish::model::current(&std::env::current_dir()?.join(&models)) {
                    Some((version, path, vad)) => println!(
                        "Transcribe model {version}\n  {}\n  VAD: {}",
                        path.display(),
                        vad.map_or("none".into(), |v| v.display().to_string())
                    ),
                    None => println!("No Transcribe model installed. Run: 3yiddish model pull"),
                }
            }
            ModelAction::Rollback => {
                println!("Now using {}", three_yiddish::model::rollback(&models)?);
            }
        }
        return Ok(());
    }
    if let (Command::Transcribe { reply: false, .. }, Some(file)) = (&cli.command, &file_arg)
        && let Some((_, model, vad)) = three_yiddish::model::current(&models)
    {
        let bytes = std::fs::read(file).context("Cannot read audio file")?;
        let text = tokio::task::spawn_blocking(move || -> Result<String> {
            let engine = three_yiddish::asr::Engine::load(&model, vad.as_deref())?;
            engine.transcribe(&three_yiddish::asr::decode(bytes)?)
        })
        .await??;
        println!("{text}");
        return Ok(());
    }
    if let Command::Eval {
        suite,
        scales,
        output,
    } = &cli.command
    {
        let cases = three_yiddish::eval::load(suite)?;
        let report = three_yiddish::eval::run(Config::load(&cli.config)?, &cases, scales, |line| {
            println!("{line}")
        })
        .await?;
        let output = output.clone().unwrap_or_else(|| {
            format!(
                "docs/evaluation/schmooze-eval-{}.json",
                report["created_unix"]
            )
            .into()
        });
        std::fs::write(&output, serde_json::to_string_pretty(&report)? + "\n")?;
        for run in report["runs"].as_array().into_iter().flatten() {
            println!(
                "scale {}: {}/{} checked passed, {} for review {}",
                run["lora_scale"], run["passed"], run["checked"], run["review"], run["by_kind"]
            );
        }
        println!("Report: {}", output.display());
        return Ok(());
    }
    if matches!(cli.command, Command::Serve) {
        return serve(Config::load_or_default(&cli.config)?).await;
    }
    let url = reqwest::Url::parse(&cli.server)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
    {
        bail!("Invalid service URL");
    }
    if url.scheme() == "http"
        && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    {
        bail!("Use HTTPS for a remote service");
    }
    let token = std::env::var("THREE_YIDDISH_TOKEN").ok();
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(token) = token {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}").parse()?,
        );
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()?;
    let base = cli.server.trim_end_matches('/');
    match cli.command {
        Command::Serve
        | Command::Eval { .. }
        | Command::Model { .. }
        | Command::Update
        | Command::Bench { .. } => unreachable!(),
        Command::Tui { watch } => {
            // One binary: when no 3Yiddish service is running yet, run it inside this process.
            if client
                .get(format!("{base}/v1/status"))
                .send()
                .await
                .is_err()
            {
                let config = Config::load_or_default(&cli.config)?;
                tokio::spawn(async move {
                    if let Err(error) = serve(config).await {
                        eprintln!("3Yiddish service stopped: {error}");
                    }
                });
                for _ in 0..50 {
                    if client.get(format!("{base}/v1/status")).send().await.is_ok() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
            three_yiddish::tui::run(client, base.to_owned(), watch, cli.config).await?;
        }
        Command::Doctor => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &response(client.get(format!("{base}/v1/status")).send().await?).await?
                )?
            );
        }
        Command::Knowledge { query } => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &response(
                        client
                            .get(format!("{base}/v1/knowledge/search"))
                            .query(&[("q", query)])
                            .send()
                            .await?
                    )
                    .await?
                )?
            );
        }
        Command::Translate { direction, text } => {
            let result = response(
                client
                    .post(format!("{base}/v1/translate"))
                    .json(&json!({"direction":direction,"text":text}))
                    .send()
                    .await?,
            )
            .await?;
            println!("{}", result["text"].as_str().unwrap_or_default());
        }
        Command::Speak { text, output } => {
            let mut result = client
                .post(format!("{base}/v1/audio/speech"))
                .json(&json!({"input":text,"voice":"default","response_format":"wav"}))
                .send()
                .await?;
            if !result.status().is_success() {
                response(result).await?;
                unreachable!();
            }
            let mut audio = Vec::new();
            while let Some(chunk) = result.chunk().await? {
                if audio.len() + chunk.len() > 25 * 1024 * 1024 {
                    bail!("Speak response exceeds 25 MiB");
                }
                audio.extend_from_slice(&chunk);
            }
            if audio.len() < 44 || &audio[..4] != b"RIFF" || &audio[8..12] != b"WAVE" {
                bail!("Service returned invalid WAV audio");
            }
            // Never overwrite an existing recording or output file.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            file.write_all(&audio)?;
            println!("{}", output.display());
        }
        Command::Transcribe { file, reply, model } => {
            let file = file_arg.unwrap_or(file);
            let metadata = tokio::fs::metadata(&file)
                .await
                .context("Cannot read audio file")?;
            if metadata.len() > 25 * 1024 * 1024 {
                bail!("Audio exceeds 25 MiB");
            }
            let part = reqwest::multipart::Part::bytes(tokio::fs::read(&file).await?)
                .file_name("recording.wav");
            let mut form = reqwest::multipart::Form::new().part("file", part);
            if let Some(model) = model {
                form = form.text("model", model);
            }
            let endpoint = if reply {
                "/v1/audio/chat"
            } else {
                "/v1/audio/transcriptions"
            };
            let result = response(
                client
                    .post(format!("{base}{endpoint}"))
                    .multipart(form)
                    .send()
                    .await?,
            )
            .await?;
            println!(
                "{}",
                if reply {
                    result.pointer("/choices/0/message/content")
                } else {
                    result.get("text")
                }
                .and_then(Value::as_str)
                .unwrap_or_default()
            );
        }
        Command::Chat {
            text,
            model,
            json: show_json,
        } => {
            let interactive = text.is_none();
            let mut next = text;
            let mut messages = Vec::new();
            loop {
                let text = if let Some(text) = next.take() {
                    text
                } else {
                    print!("you> ");
                    io::stdout().flush()?;
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line)? == 0 {
                        break;
                    }
                    let line = line.trim();
                    if matches!(line, "/quit" | "/exit") {
                        break;
                    }
                    if line == "/clear" {
                        messages.clear();
                        continue;
                    }
                    if line.is_empty() {
                        continue;
                    }
                    line.to_owned()
                };
                let mut attempt = messages.clone();
                attempt.push(json!({"role":"user","content":text}));
                let result = response(
                    client
                        .post(format!("{base}/v1/chat/completions"))
                        .json(&json!({"model":model,"messages":attempt}))
                        .send()
                        .await?,
                )
                .await;
                match result {
                    Ok(value) => {
                        let answer = value
                            .pointer("/choices/0/message/content")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if show_json {
                            println!("{}", serde_json::to_string_pretty(&value)?);
                        } else {
                            println!("{answer}");
                        }
                        messages = attempt;
                        messages.push(json!({"role":"assistant","content":answer}));
                    }
                    Err(error) => {
                        if !interactive {
                            return Err(error);
                        }
                        eprintln!("{error}");
                    }
                }
                if !interactive {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// The HTTP API (plus the phone gateway when configured). `3yiddish serve` runs it in the
/// foreground; the TUI starts it in the background when nothing is listening yet.
async fn serve(config: Config) -> Result<()> {
    let listen = config.listen;
    let phone_config = config.phone.clone();
    let app = App::new(config)?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    if let Some(phone) = phone_config {
        let address = phone.listen;
        if let Some(download_address) = phone.certificate_download_listen {
            let certificate = tokio::fs::read(
                phone
                    .trust_certificate
                    .as_ref()
                    .context("Missing trust_certificate")?,
            )
            .await?;
            let download_listener = tokio::net::TcpListener::bind(download_address)
                .await
                .context("Cannot bind public certificate download")?;
            let download = axum::Router::new().route(
                "/3yiddish-ca.crt",
                axum::routing::get(move || {
                    let certificate = certificate.clone();
                    async move {
                        (
                            [
                                ("content-type", "application/x-x509-ca-cert"),
                                (
                                    "content-disposition",
                                    "attachment; filename=3yiddish-ca.crt",
                                ),
                                ("cache-control", "no-store"),
                            ],
                            certificate,
                        )
                    }
                }),
            );
            tokio::spawn(async move {
                if let Err(error) = axum::serve(download_listener, download).await {
                    eprintln!("Certificate download stopped: {error}");
                }
            });
        }
        let phone_listener = tokio::net::TcpListener::bind(address)
            .await
            .context("Cannot bind the phone audio gateway")?;
        let phone_app = app.clone();
        let tls = if let (Some(cert), Some(key)) = (phone.tls_cert, phone.tls_key) {
            let _ = rustls::crypto::ring::default_provider().install_default();
            Some(
                axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                    .await
                    .context("Cannot load phone HTTPS certificate")?,
            )
        } else {
            None
        };
        tokio::spawn(async move {
            let gateway = three_yiddish::phone::gateway(phone_app);
            let result = if let Some(tls) = tls {
                match phone_listener.into_std() {
                    Ok(listener) => match axum_server::from_tcp_rustls(listener, tls) {
                        Ok(server) => server.serve(gateway.into_make_service()).await,
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                }
            } else {
                axum::serve(phone_listener, gateway).await
            };
            if let Err(error) = result {
                eprintln!("Phone gateway stopped: {error}");
            }
        });
    }
    eprintln!("3Yiddish listening on {listen}; request content and credentials are not logged");
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
