//! A capability-limited phone audio gateway. Controller routes stay on the PC API.
use crate::service::App;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use qrcode::{QrCode, render::unicode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const AUDIO_LIMIT: usize = 25 * 1024 * 1024;
#[derive(Clone)]
pub struct PairHub(Arc<Mutex<Option<Pair>>>, Arc<String>);
impl Default for PairHub {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(None)), Arc::new(secret()))
    }
}
struct Pair {
    id: String,
    session: Option<String>,
    last_seen: Option<Instant>,
    busy: bool,
    transcript: String,
    transcript_revision: u64,
    error: String,
    playback: Vec<u8>,
    playback_revision: u64,
    played_revision: u64,
    record_requested: bool,
    mic_ready: bool,
    recording: bool,
    /// What the PC game shows (line, last result); the phone mirrors it.
    game: Value,
    /// Last game button tapped on the phone; the PC applies each revision once.
    command: String,
    command_revision: u64,
}
#[derive(Debug)]
pub struct PairError(StatusCode, &'static str);
impl IntoResponse for PairError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":{"message":self.1,"type":"phone_pair_error"}})),
        )
            .into_response()
    }
}
type Result<T> = std::result::Result<T, PairError>;
fn fail(code: StatusCode, message: &'static str) -> PairError {
    PairError(code, message)
}
fn secret() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
impl PairHub {
    pub fn from_code_file(path: &std::path::Path) -> anyhow::Result<Self> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let code = match options.open(path) {
            Ok(mut file) => {
                let code = secret();
                file.write_all(code.as_bytes())?;
                code
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let meta = std::fs::symlink_metadata(path)?;
                anyhow::ensure!(
                    meta.is_file() && meta.len() == 32,
                    "Invalid pairing code file"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    anyhow::ensure!(
                        meta.permissions().mode() & 0o077 == 0,
                        "Pairing code file must be private (0600)"
                    );
                }
                std::fs::read_to_string(path)?
            }
            Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(
            code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid pairing code"
        );
        Ok(Self(Arc::new(Mutex::new(None)), Arc::new(code)))
    }
    fn fresh() -> Pair {
        Pair {
            id: secret(),
            session: None,
            last_seen: None,
            busy: false,
            transcript: String::new(),
            transcript_revision: 0,
            error: String::new(),
            playback: Vec::new(),
            playback_revision: 0,
            played_revision: 0,
            record_requested: false,
            mic_ready: false,
            recording: false,
            game: Value::Null,
            command: String::new(),
            command_revision: 0,
        }
    }
    fn create(&self, origin: &str) -> Result<Value> {
        let url = format!("{}/#{}", origin.trim_end_matches('/'), self.1);
        let qr = QrCode::new(url.as_bytes())
            .map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "Pair URL is too long"))?
            .render::<unicode::Dense1x2>()
            .quiet_zone(true)
            .build();
        let mut guard = self.0.lock().unwrap();
        let id = guard.get_or_insert_with(Self::fresh).id.clone();
        Ok(
            json!({"id":id,"join_url":url,"qr":qr,"reusable":true,"transport":if origin.starts_with("https:"){"https"}else{"lan-http"}}),
        )
    }
    fn connect(&self, invite: &str) -> Result<Value> {
        if invite != self.1.as_str() {
            return Err(fail(StatusCode::UNAUTHORIZED, "Invalid PC pairing code"));
        }
        let mut guard = self.0.lock().unwrap();
        let p = guard.get_or_insert_with(Self::fresh);
        let session = secret();
        p.session = Some(session.clone());
        p.last_seen = Some(Instant::now());
        p.mic_ready = false;
        p.recording = false;
        p.record_requested = false;
        Ok(json!({"session":session,"id":p.id}))
    }
    fn snapshot(p: &Pair) -> Value {
        let online = p.session.is_some()
            && p.last_seen
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5));
        json!({"id":p.id,"connected":online,"mic_ready":online&&p.mic_ready,"recording":online&&p.recording,"record_requested":online&&p.record_requested,"busy":p.busy,"transcript":p.transcript,"transcript_revision":p.transcript_revision,"error":p.error,"playback_revision":p.playback_revision,"played_revision":p.played_revision,"game":p.game,"command":p.command,"command_revision":p.command_revision})
    }
    fn current(&self) -> Value {
        let guard = self.0.lock().unwrap();
        guard
            .as_ref()
            .map(Self::snapshot)
            .unwrap_or(json!({"connected":false}))
    }
    fn authenticated<T>(
        &self,
        token: &str,
        action: impl FnOnce(&mut Pair) -> Result<T>,
    ) -> Result<T> {
        let mut guard = self.0.lock().unwrap();
        let p = guard
            .as_mut()
            .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "Phone is not paired"))?;
        if p.session.as_deref() != Some(token) {
            return Err(fail(
                StatusCode::UNAUTHORIZED,
                "Phone session expired or disconnected",
            ));
        }
        p.last_seen = Some(Instant::now());
        action(p)
    }
    fn revoke(&self, id: &str) -> Result<Value> {
        let mut guard = self.0.lock().unwrap();
        if guard.as_ref().is_none_or(|p| p.id != id) {
            return Err(fail(
                StatusCode::NOT_FOUND,
                "Pair session is no longer current",
            ));
        }
        *guard = None;
        Ok(json!({"disconnected":true}))
    }
    fn queue(&self, id: &str, bytes: Vec<u8>) -> Result<Value> {
        if bytes.len() < 44
            || bytes.len() > AUDIO_LIMIT
            || &bytes[..4] != b"RIFF"
            || &bytes[8..12] != b"WAVE"
        {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Phone playback requires a WAV file up to 25 MiB",
            ));
        }
        let mut guard = self.0.lock().unwrap();
        let p = guard
            .as_mut()
            .filter(|p| p.id == id)
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "Phone pairing expired"))?;
        p.playback = bytes;
        p.playback_revision += 1;
        Ok(json!({"queued":true,"revision":p.playback_revision}))
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .filter(|s| s.len() == 32)
        .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "Phone session required"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invitation {
    invite: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Played {
    revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GameTap {
    action: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MicState {
    ready: bool,
    recording: bool,
    error: Option<String>,
}
struct UploadLease {
    hub: PairHub,
    id: String,
}
impl Drop for UploadLease {
    fn drop(&mut self) {
        if let Some(p) = self
            .hub
            .0
            .lock()
            .unwrap()
            .as_mut()
            .filter(|p| p.id == self.id)
        {
            p.busy = false;
        }
    }
}

/// Phone game taps: plain buttons, or play/toss/redo of one reading by its numeric id.
fn game_action(action: &str) -> bool {
    matches!(action, "record" | "skip" | "back" | "redo")
        || action.split_once(':').is_some_and(|(verb, id)| {
            let digits =
                |s: &str| (1..=24).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
            match verb {
                "play" | "toss" | "redo" => digits(id),
                // playground answer: "<reading number>-<choice>"
                "pick" => id.split_once('-').is_some_and(|(n, c)| {
                    digits(n) && matches!(c, "gold" | "line" | "yl" | "ours" | "toss")
                }),
                _ => false,
            }
        })
}
pub fn controller() -> Router<App> {
    Router::new()
        .route(
            "/v1/pair",
            get(|State(app): State<App>| async move { Json(app.phone.current()) })
                .post(|State(app): State<App>| async move {
                    let cfg = app.config.phone.as_ref().ok_or_else(|| {
                        fail(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Phone gateway is not configured",
                        )
                    })?;
                    app.phone.create(&cfg.public_url).map(Json)
                })
                .delete(
                    |State(app): State<App>, Json(input): Json<Identity>| async move {
                        app.phone.revoke(&input.id).map(Json)
                    },
                ),
        )
        .route(
            "/v1/pair/{id}/record",
            post(|State(app):State<App>, Path(id):Path<String>| async move {
                let mut guard=app.phone.0.lock().unwrap();
                let p=guard.as_mut().filter(|p|p.id==id)
                    .ok_or_else(||fail(StatusCode::NOT_FOUND,"Phone pairing expired"))?;
                if !p.mic_ready || p.last_seen.is_none_or(|t|t.elapsed()>Duration::from_secs(5)) {
                    return Err(fail(StatusCode::CONFLICT,"Phone mic is not ready. Enable it once on the phone over trusted HTTPS."));
                }
                if p.busy {return Err(fail(StatusCode::CONFLICT,"Phone recording is being transcribed"));}
                p.record_requested = !p.record_requested;
                Ok(Json(json!({"text":if p.record_requested{"Phone mic: starting… F10 stops."}else{"Phone mic: stopping and transcribing…"}})))
            }),
        )
        .route(
            "/v1/pair/{id}/game",
            post(
                |State(app): State<App>, Path(id): Path<String>, Json(game): Json<Value>| async move {
                    let mut guard = app.phone.0.lock().unwrap();
                    let p = guard
                        .as_mut()
                        .filter(|p| p.id == id)
                        .ok_or_else(|| fail(StatusCode::NOT_FOUND, "Phone pairing expired"))?;
                    p.game = game;
                    Ok::<_, PairError>(Json(json!({"ok":true})))
                },
            )
            // The view carries the readings list (up to 300 lines).
            .layer(DefaultBodyLimit::max(512 * 1024)),
        )
        .route(
            "/v1/pair/{id}/playback",
            post(
                |State(app): State<App>, Path(id): Path<String>, bytes: Bytes| async move {
                    app.phone.queue(&id, bytes.to_vec()).map(Json)
                },
            ),
        )
}
async fn upload(State(app): State<App>, headers: HeaderMap, bytes: Bytes) -> Response {
    let token = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if bytes.is_empty() || bytes.len() > AUDIO_LIMIT {
        return fail(
            StatusCode::BAD_REQUEST,
            "Recording must contain 1 byte..25 MiB",
        )
        .into_response();
    }
    let id = match app.phone.authenticated(token, |p| {
        if p.busy {
            return Err(fail(
                StatusCode::CONFLICT,
                "A recording is already being transcribed",
            ));
        }
        p.busy = true;
        p.record_requested = false;
        p.recording = false;
        p.error.clear();
        Ok(p.id.clone())
    }) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let _lease = UploadLease {
        hub: app.phone.clone(),
        id: id.clone(),
    };
    let audio = bytes.to_vec();
    match app.transcribe(audio.clone()).await {
        Ok(result) => {
            keep_recent(&audio);
            let text = result["text"].as_str().unwrap_or_default().to_owned();
            let mut guard = app.phone.0.lock().unwrap();
            let Some(p) = guard.as_mut().filter(|p| p.id == id) else {
                return fail(
                    StatusCode::UNAUTHORIZED,
                    "Pair session ended during transcription",
                )
                .into_response();
            };
            p.transcript = text;
            p.transcript_revision += 1;
            Json(json!({"accepted":true})).into_response()
        }
        Err(error) => {
            if let Some(p) = app.phone.0.lock().unwrap().as_mut().filter(|p| p.id == id) {
                p.error = "Transcription failed; see the PC TUI".into();
            }
            error.into_response()
        }
    }
}
/// The read-aloud game scores phone readings from the newest file here (the TUI copies it
/// into the reading log). Last 20 uploads only, owner-only permissions.
fn keep_recent(audio: &[u8]) {
    let dir = std::path::Path::new("state/asr-recent");
    let saved = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            "{}.audio",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        std::io::Write::write_all(&mut options.open(path)?, audio)?;
        let mut old: Vec<_> = std::fs::read_dir(dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "audio"))
            .collect();
        old.sort();
        for path in old.iter().take(old.len().saturating_sub(20)) {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("txt"));
        }
        Ok(())
    })();
    let _ = saved;
}
async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("permissions-policy", "microphone=(self), camera=()"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; media-src 'self' blob: data:; frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        h.insert(
            axum::http::HeaderName::from_static(name),
            axum::http::HeaderValue::from_static(value),
        );
    }
    response
}
async fn authorize(State(app): State<App>, request: Request, next: Next) -> Response {
    let Ok(_permit) = app.permits.clone().try_acquire_owned() else {
        return fail(StatusCode::TOO_MANY_REQUESTS, "Service is busy").into_response();
    };
    if request.uri().path().starts_with("/api/")
        && request.uri().path() != "/api/connect"
        && let Err(e) =
            bearer(request.headers()).and_then(|t| app.phone.authenticated(t, |_| Ok(())))
    {
        return e.into_response();
    }
    next.run(request).await
}
pub fn gateway(app: App) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../assets/phone/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../assets/phone/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../assets/phone/style.css"),
                )
            }),
        )
        .route(
            "/api/connect",
            post(
                |State(app): State<App>, Json(input): Json<Invitation>| async move {
                    app.phone.connect(&input.invite).map(Json)
                },
            )
            .layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/status",
            get(|State(app): State<App>, headers: HeaderMap| async move {
                let token = bearer(&headers)?;
                app.phone
                    .authenticated(token, |p| Ok(Json(PairHub::snapshot(p))))
            }),
        )
        .route("/api/audio", post(upload))
        .route("/api/mic", post(|State(app):State<App>, headers:HeaderMap, Json(input):Json<MicState>|async move {
            let token=bearer(&headers)?;
            app.phone.authenticated(token,|p| {
                p.mic_ready=input.ready;
                p.recording=input.recording;
                if !input.ready || !input.recording {p.record_requested=false;}
                if let Some(error)=input.error {p.error=error.chars().take(300).collect(); p.record_requested=false;}
                else {p.error.clear();}
                Ok(Json(json!({"ok":true})))
            })
        }).layer(DefaultBodyLimit::max(4096)))
        .route(
            "/api/playback/{revision}",
            get(
                |State(app): State<App>, headers: HeaderMap, Path(revision): Path<u64>| async move {
                    let token = bearer(&headers)?;
                    app.phone.authenticated(token, |p| {
                        if revision != p.playback_revision || p.playback.is_empty() {
                            return Err(fail(
                                StatusCode::NOT_FOUND,
                                "Audio is no longer available",
                            ));
                        }
                        Ok(([(header::CONTENT_TYPE, "audio/wav")], p.playback.clone()))
                    })
                },
            ),
        )
        .route(
            "/api/game",
            post(
                |State(app): State<App>, headers: HeaderMap, Json(input): Json<GameTap>| async move {
                    let token = bearer(&headers)?;
                    if !game_action(&input.action) {
                        return Err(fail(StatusCode::BAD_REQUEST, "Unknown game action"));
                    }
                    app.phone.authenticated(token, |p| {
                        p.command = input.action;
                        p.command_revision += 1;
                        Ok(Json(json!({"ok":true,"revision":p.command_revision})))
                    })
                },
            )
            .layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/played",
            post(
                |State(app): State<App>, headers: HeaderMap, Json(input): Json<Played>| async move {
                    let token = bearer(&headers)?;
                    app.phone.authenticated(token, |p| {
                        if input.revision == p.playback_revision {
                            p.played_revision = input.revision;
                        }
                        Ok(Json(json!({"ok":true})))
                    })
                },
            ),
        )
        .route(
            "/api/disconnect",
            post(|State(app): State<App>, headers: HeaderMap| async move {
                let token = bearer(&headers)?;
                let id = app.phone.authenticated(token, |p| Ok(p.id.clone()))?;
                app.phone.revoke(&id).map(Json)
            }),
        )
        .layer(DefaultBodyLimit::max(AUDIO_LIMIT))
        .layer(middleware::from_fn_with_state(app.clone(), authorize))
        .layer(middleware::from_fn(headers))
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_pc_code_survives_new_invitations_and_service_restarts() {
        let folder =
            std::env::temp_dir().join(format!("3yiddish-pair-code-{}", uuid::Uuid::new_v4()));
        let file = folder.join("code");
        let first = PairHub::from_code_file(&file).unwrap();
        let a = first.create("https://192.168.1.2:8796").unwrap();
        let b = first.create("https://192.168.1.2:8796").unwrap();
        assert_eq!(a["join_url"], b["join_url"]);
        let restarted = PairHub::from_code_file(&file).unwrap();
        let c = restarted.create("https://192.168.1.2:8796").unwrap();
        assert_eq!(a["join_url"], c["join_url"]);
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn reusable_code_reconnects_after_unpair_and_does_not_expire() {
        let hub = PairHub::default();
        let p = hub.create("http://192.168.1.2:8796").unwrap();
        let code = p["join_url"].as_str().unwrap().split('#').nth(1).unwrap();
        let first = hub.connect(code).unwrap();
        let second = hub.connect(code).unwrap();
        assert_ne!(first["session"], second["session"]);
        assert!(
            hub.authenticated(first["session"].as_str().unwrap(), |_| Ok(()))
                .is_err()
        );
        assert!(
            hub.authenticated(second["session"].as_str().unwrap(), |_| Ok(()))
                .is_ok()
        );
        // Phone game taps reach the PC as numbered commands.
        let token = second["session"].as_str().unwrap();
        hub.authenticated(token, |p| {
            p.command = "skip".into();
            p.command_revision += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(hub.current()["command_revision"], 1);
        assert_eq!(hub.current()["command"], "skip");
        assert!(game_action("toss:1794422602184341874") && game_action("back"));
        assert!(!game_action("toss:../../etc") && !game_action("rm:1") && !game_action("play:"));
        assert!(
            game_action("pick:12-yl") && !game_action("pick:12-rm") && !game_action("pick:x-gold")
        );
        hub.revoke(p["id"].as_str().unwrap()).unwrap();
        assert!(hub.connect(code).is_ok());
        assert!(hub.connect("wrong").is_err());
    }
}
