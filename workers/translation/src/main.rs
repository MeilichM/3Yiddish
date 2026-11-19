//! Native local NLLB translation; independent of the reasoning LLM.
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use ct2rs_sys::{ComputeType, Config, TranslationOptions, Translator};
use sentencepiece::SentencePieceProcessor;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Instant};
use tokio::sync::Semaphore;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    model: PathBuf,
    #[arg(long, default_value = "127.0.0.1:8799")]
    listen: SocketAddr,
    #[arg(long, default_value_t = 4)]
    threads: usize,
}
struct Engine {
    translator: Translator,
    tokenizer: SentencePieceProcessor,
    permit: Arc<Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    text: String,
    direction: String,
}
struct Error(StatusCode, String);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":{"message":self.1,"type":"translation_error"}})),
        )
            .into_response()
    }
}
fn languages(direction: &str) -> Result<(&'static str, &'static str)> {
    match direction {
        "en-yi" => Ok(("eng_Latn", "ydd_Hebr")),
        "yi-en" => Ok(("ydd_Hebr", "eng_Latn")),
        _ => anyhow::bail!("direction must be en-yi or yi-en"),
    }
}
impl Engine {
    fn translate(&self, input: Input) -> Result<Value> {
        let (source, target) = languages(&input.direction)?;
        ensure!(
            !input.text.trim().is_empty() && input.text.chars().count() <= 16000,
            "Text must contain 1..16000 characters"
        );
        let mut tokens = vec![source.to_owned()];
        tokens.extend(
            self.tokenizer
                .encode(&input.text)?
                .into_iter()
                .map(|p| p.piece),
        );
        tokens.push("</s>".into());
        ensure!(
            tokens.len() <= 512,
            "Translation exceeds the 512-token source limit; shorten or split the message"
        );
        let source_tokens = tokens.len();
        let options: TranslationOptions<String, String> = TranslationOptions {
            beam_size: 4,
            max_input_length: 0,
            max_decoding_length: 512,
            return_end_token: true,
            ..Default::default()
        };
        let start = Instant::now();
        let results = self.translator.translate_batch_with_target_prefix(
            &[tokens],
            &[vec![target]],
            &options,
            None,
        )?;
        let output = results
            .first()
            .and_then(|r| r.hypotheses.first())
            .context("No translation returned")?;
        ensure!(
            output.last().is_some_and(|s| s == "</s>"),
            "Translation reached its output limit; shorten the message"
        );
        let target_tokens = output.len();
        let pieces: Vec<_> = output
            .iter()
            .filter(|s| {
                !matches!(
                    s.as_str(),
                    "eng_Latn" | "ydd_Hebr" | "</s>" | "<s>" | "<pad>"
                )
            })
            .collect();
        let text = self.tokenizer.decode_pieces(&pieces)?;
        ensure!(!text.trim().is_empty(), "Translation was empty");
        Ok(
            json!({"text":text,"model":"nllb600m-int8-local","direction":input.direction,"finish_reason":"stop","elapsed_ms":start.elapsed().as_millis(),"usage":{"prompt_tokens":source_tokens,"completion_tokens":target_tokens},"license":"CC-BY-NC-4.0"}),
        )
    }
}
async fn translate(
    State(engine): State<Arc<Engine>>,
    Json(input): Json<Input>,
) -> Result<Json<Value>, Error> {
    if languages(&input.direction).is_err()
        || input.text.trim().is_empty()
        || input.text.chars().count() > 16000
    {
        return Err(Error(
            StatusCode::BAD_REQUEST,
            "Use nonempty text up to 16000 characters and direction en-yi or yi-en".into(),
        ));
    }
    let permit = engine.permit.clone().try_acquire_owned().map_err(|_| {
        Error(
            StatusCode::TOO_MANY_REQUESTS,
            "Translator is busy; retry shortly".into(),
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        engine.translate(input)
    })
    .await
    .map_err(|_| {
        Error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Translation task failed".into(),
        )
    })?
    .map(Json)
    .map_err(|e| Error(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.listen.ip().is_loopback(),
        "Translation worker must bind loopback"
    );
    ensure!((1..=32).contains(&args.threads), "threads must be 1..32");
    let engine = Arc::new(Engine {
        translator: Translator::new(
            &args.model,
            &Config {
                compute_type: ComputeType::INT8,
                num_threads_per_replica: args.threads,
                max_queued_batches: 1,
                ..Default::default()
            },
        )?,
        tokenizer: SentencePieceProcessor::open(args.model.join("sentencepiece.bpe.model"))?,
        permit: Arc::new(Semaphore::new(1)),
    });
    let app=Router::new().route("/healthz",get(||async{Json(json!({"status":"ready","model":"nllb600m-int8-local","license":"CC-BY-NC-4.0","runtime":"Rust/CTranslate2"}))})).route("/v1/translate",post(translate)).layer(DefaultBodyLimit::max(128*1024)).with_state(engine);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    eprintln!(
        "Native translation ready on {}; request text is not logged",
        args.listen
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yiddish_uses_its_own_script_language_tag() {
        assert_eq!(languages("en-yi").unwrap(), ("eng_Latn", "ydd_Hebr"));
        assert_eq!(languages("yi-en").unwrap(), ("ydd_Hebr", "eng_Latn"));
        assert!(languages("he-yi").is_err());
    }
}
