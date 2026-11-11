//! Fixed Schmooze conversation suite: runs every case through the real native
//! chat pipeline (same system prompt and knowledge retrieval as the API) at one or
//! more LoRA scales, with greedy decoding so runs are comparable.
use crate::{
    config::Config,
    service::{App, ChatRequest, Message, has_yiddish_script},
};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    version: u32,
    case: Vec<Case>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub kind: String,
    pub prompt: String,
    #[serde(default)]
    pub history: Vec<String>,
    #[serde(default)]
    pub expect_any: Vec<String>,
    #[serde(default)]
    pub reject_any: Vec<String>,
    pub max_chars: Option<usize>,
}

/// Automatic check failures for one reply; empty means it passed.
pub fn check(case: &Case, reply: &str) -> Vec<String> {
    let mut failures = Vec::new();
    let reply = reply.trim();
    if reply.is_empty() {
        return vec!["empty reply".into()];
    }
    if !has_yiddish_script(reply) {
        failures.push("no Yiddish letters".into());
    }
    if reply.chars().any(|c| c.is_ascii_alphabetic()) {
        failures.push("contains Latin letters".into());
    }
    if !case.expect_any.is_empty() && !case.expect_any.iter().any(|e| reply.contains(e.as_str())) {
        failures.push(format!("expected one of {:?}", case.expect_any));
    }
    for bad in case
        .reject_any
        .iter()
        .filter(|r| reply.contains(r.as_str()))
    {
        failures.push(format!("contains rejected {bad:?}"));
    }
    if let Some(max) = case.max_chars.filter(|&m| reply.chars().count() > m) {
        failures.push(format!("longer than {max} characters"));
    }
    failures
}

pub fn load(suite: &Path) -> Result<Vec<Case>> {
    let text = std::fs::read_to_string(suite)
        .with_context(|| format!("Could not read suite {}", suite.display()))?;
    let suite: Suite = toml::from_str(&text).context("Invalid suite TOML")?;
    anyhow::ensure!(suite.version == 1, "Unsupported suite version");
    Ok(suite.case)
}

/// Runs the suite once per scale. `progress` receives one line per finished case.
pub async fn run(
    mut config: Config,
    cases: &[Case],
    scales: &[f32],
    mut progress: impl FnMut(String),
) -> Result<Value> {
    // Evaluation runs in-process against the model worker; it never serves HTTP.
    config.api_token_env = None;
    config.phone = None;
    let model = config
        .reasoners
        .get(&config.default_reasoner)
        .map(|r| r.model.clone());
    let mut runs = Vec::new();
    for &scale in scales {
        let mut app = App::new(config.clone())?;
        // ponytail: adapter id 0 assumes the worker loads exactly one LoRA, as configured today.
        app.sampling = json!({"lora":[{"id":0,"scale":scale}],"temperature":0.0,"seed":613})
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut results = Vec::new();
        let mut by_kind: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
        for case in cases {
            let mut messages: Vec<Message> = case
                .history
                .iter()
                .enumerate()
                .map(|(i, content)| Message {
                    role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                    content: content.clone(),
                })
                .collect();
            messages.push(Message {
                role: "user".into(),
                content: case.prompt.clone(),
            });
            let started = std::time::Instant::now();
            let request = ChatRequest {
                model: None,
                messages,
                max_tokens: Some(256),
                stream: false,
            };
            let (reply, failures) = match app.chat(request).await {
                Ok(body) => {
                    let reply = body
                        .pointer("/choices/0/message/content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let failures = check(case, &reply);
                    (reply, failures)
                }
                Err(error) => (String::new(), vec![format!("request failed: {error}")]),
            };
            // Basic script/length checks alone cannot tell sense from fluent nonsense.
            let specific = !case.expect_any.is_empty() || !case.reject_any.is_empty();
            let status = match (failures.is_empty(), specific) {
                (false, _) => "fail",
                (true, true) => "pass",
                (true, false) => "review",
            };
            if status != "review" {
                let tally = by_kind.entry(&case.kind).or_default();
                tally.0 += u32::from(status == "pass");
                tally.1 += 1;
            }
            progress(format!(
                "[{scale:.2}] {} {}{}",
                status.to_uppercase(),
                case.id,
                if failures.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", failures.join("; "))
                }
            ));
            results.push(json!({"id":case.id,"kind":case.kind,"prompt":case.prompt,"history":case.history,"reply":reply,"status":status,"failures":failures,"elapsed_ms":started.elapsed().as_millis()}));
        }
        let passed: u32 = by_kind.values().map(|t| t.0).sum();
        let checked: u32 = by_kind.values().map(|t| t.1).sum();
        let scale = (f64::from(scale) * 100.0).round() / 100.0;
        runs.push(json!({"lora_scale":scale,"passed":passed,"checked":checked,"review":cases.len() as u32 - checked,"by_kind":by_kind.iter().map(|(k,(p,t))|(k.to_string(),json!(format!("{p}/{t}")))).collect::<serde_json::Map<_,_>>(),"results":results}));
    }
    Ok(json!({
        "suite_version": 1,
        "created_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "model": model,
        "decoding": {"temperature": 0.0, "seed": 613, "max_tokens": 256},
        "note": "Automatic checks cover script, length and objective answers only; they are not a fluency, dialect or accent judgment.",
        "runs": runs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_parses_and_checks_apply() {
        // The chat suite is private (Schmooze is paused); the public tree has no copy.
        let suite = Path::new("docs/evaluation/schmooze-suite.toml");
        if !suite.exists() {
            return;
        }
        let cases = load(suite).unwrap();
        assert!(cases.len() >= 30);
        let apples = cases.iter().find(|c| c.id == "math-apples").unwrap();
        assert!(check(apples, "איר האסט פינף עפל.").is_empty());
        assert!(!check(apples, "אין דיין פאלען זעסטו דריי").is_empty());
        assert_eq!(check(apples, "dרei 5"), vec!["contains Latin letters"]);
        let greet = cases.iter().find(|c| c.id == "greet-gutn").unwrap();
        assert!(!check(greet, "גוטע נאכט!").is_empty());
        assert_eq!(check(greet, " "), vec!["empty reply"]);
    }
}
