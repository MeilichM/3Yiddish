//! The playground's phone page: record a phrase on the phone, read our draft, mark what is
//! wrong. Off until the app pairs a phone, and only where the playground is switched on.
//!
//! The page is served by a listener of its own that knows nothing but the playground. Every
//! address on it starts with an unguessable code (the one in the pairing link); anything else
//! gets an empty "not found". Phone browsers give the microphone only to a secure page, so
//! the link goes over Tailscale HTTPS when this computer has it; otherwise it is plain HTTP
//! on the private network and the page records with the phone's own recorder instead.
use crate::{
    freebench::{self, Store},
    service::App,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const AUDIO_LIMIT: usize = 25 * 1024 * 1024;
/// The phone page's own port on this computer, and the Tailscale HTTPS port in front of it.
const PORT: u16 = 8796;
const HTTPS_PORT: u16 = 8443;

/// Is the playground open here: switched on by its owner (`3yiddish playground on`), or by
/// someone who accepted the Contribute explanation in the app (its current text: an earlier
/// one is shown the new one first, and their takes wait for them)?
pub fn playground() -> bool {
    owner() || accepted(&crate::paths::contribute_accepted())
}
/// The number of the Contribute explanation's text; raise it when what a contributor agrees
/// to changes. 2: accepting also switches on sharing (`share`).
pub const CONSENT: &str = "2";
pub fn accepted(file: &std::path::Path) -> bool {
    std::fs::read_to_string(file).is_ok_and(|text| text.trim() == CONSENT)
}

/// The owner's own set (`3yiddish playground on`): a fixed number of takes, kept apart from
/// what contributors record.
pub fn owner() -> bool {
    crate::paths::playground_marker().exists()
}

#[derive(Clone)]
pub struct PairHub {
    state: Arc<Mutex<Hub>>,
    /// One change to the free bench at a time.
    store: Arc<tokio::sync::Mutex<()>>,
    /// Where the takes are kept.
    folder: Arc<std::path::PathBuf>,
    /// Demo mode (THREE_YIDDISH_DEMO): canned drafts in a throwaway folder, never the real set.
    demo: bool,
    /// Tests switch the playground on without the marker file.
    on: Option<bool>,
    /// How many takes make the set; 0 for a contributor, who just records as many as he likes.
    goal: usize,
}
impl Default for PairHub {
    fn default() -> Self {
        if std::env::var_os("THREE_YIDDISH_DEMO").is_some() {
            let folder = std::env::temp_dir().join(format!("3yiddish-demo-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            let mut hub = Self::at(folder).demo();
            if !owner() {
                hub.goal = 0;
            }
            hub.seed_demo();
            return hub;
        }
        if owner() {
            return Self::at(crate::paths::free_bench());
        }
        let mut hub = Self::at(crate::paths::contributions());
        hub.goal = 0;
        hub
    }
}
#[derive(Default)]
struct Hub {
    open: Option<Open>,
    /// When the phone last asked for anything.
    seen: Option<Instant>,
    /// The idea on the Record screen (a line of `IDEAS`), the same on the phone and in the app.
    idea: Option<usize>,
}

/// Everyday prompts for the Record screen, one per line (`#` starts a comment).
const IDEAS: &str = include_str!("../assets/phone/ideas.txt");
fn ideas() -> Vec<&'static str> {
    IDEAS
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}
/// Demo mode's drafts: the app's own demo lines, so nothing here passes for a real one.
const DEMO_DRAFTS: [&str; 3] = [
    "א גוטן טאג, וואס מאכט איר?",
    "דאס איז א פרובע פון דעם עקראן, נישט קיין אמת'ע איבערשרייבונג.",
    "ווען מען רעדט אריין אין מיקראפאן, שרייבט עס זיך אויס דא אויף דער רעכטער זייט, שורה נאך שורה.",
];
/// A pairing that is live: the page is being served at `url`.
struct Open {
    url: String,
    /// `tailscale-https`, `tailscale-http` or `lan-http`.
    transport: &'static str,
    /// What the owner can do for a secure link, when this one is not.
    hint: String,
    /// The Tailscale HTTPS port this pairing set up, to take down again.
    https_port: Option<u16>,
    stop: tokio::sync::oneshot::Sender<()>,
}

#[derive(Debug)]
pub struct PairError(StatusCode, String);
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
fn fail(code: StatusCode, message: impl Into<String>) -> PairError {
    PairError(code, message.into())
}
fn broken(error: impl std::fmt::Display) -> PairError {
    fail(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

/// The code in the pairing link: made once and kept (owner-only file), so the same link
/// keeps working across Refresh and restarts. Unpair deletes it; the next pairing makes a
/// new one and the old link is dead.
fn code() -> anyhow::Result<String> {
    use std::io::Write;
    let path = crate::paths::pair_code();
    if let Ok(code) = std::fs::read_to_string(&path)
        && code.len() == CODE_LENGTH
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Ok(code);
    }
    let code = new_code(uuid::Uuid::new_v4().as_u128());
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?.write_all(code.as_bytes())?;
    Ok(code)
}
/// Twenty capital letters and digits (over a hundred random bits). Capitals, because a QR
/// code of capitals and digits alone is small enough to show in a terminal.
const CODE_LENGTH: usize = 20;
fn new_code(mut random: u128) -> String {
    (0..CODE_LENGTH)
        .map(|_| {
            let digit = (random % 36) as u8;
            random /= 36;
            char::from(if digit < 10 {
                b'0' + digit
            } else {
                b'A' + digit - 10
            })
        })
        .collect()
}

/// This computer on Tailscale, as far as the pairing needs to know.
struct Node {
    name: String,
    ip: IpAddr,
    /// HTTPS certificates are switched on for the tailnet.
    certificates: bool,
}
mod tailscale {
    use super::Node;
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    /// Run the `tailscale` command (or the one named in THREE_YIDDISH_TAILSCALE) without a
    /// keyboard and with a deadline: it can sit waiting for a consent that nobody will give.
    /// None when there is no such command. Otherwise (it succeeded, everything it printed).
    pub fn run(args: &[&str], limit: Duration) -> Option<(bool, String)> {
        let program =
            std::env::var_os("THREE_YIDDISH_TAILSCALE").unwrap_or_else(|| "tailscale".into());
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        // Read both while it runs, so a long answer never blocks it. What was said is
        // gathered in one place and taken when the command is over, without waiting for its
        // output to close: something it started may hold that open for long.
        let said = Arc::new(Mutex::new(Vec::new()));
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|mut pipe| {
            let said = said.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    said.lock().unwrap().extend_from_slice(&chunk[..n]);
                }
            })
        })
        .collect();
        let deadline = Instant::now() + limit;
        let ok = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
            }
        };
        // Its last words, when the output closes promptly (the usual case).
        let settle = Instant::now() + Duration::from_millis(300);
        while Instant::now() < settle && readers.iter().any(|r| !r.is_finished()) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let said = String::from_utf8_lossy(&said.lock().unwrap()).into_owned();
        Some((ok, said))
    }

    /// This computer's Tailscale name and address, when Tailscale is installed and connected.
    pub fn node() -> Option<Node> {
        let (ok, said) = run(
            &["status", "--json", "--peers=false"],
            Duration::from_secs(5),
        )?;
        let status: serde_json::Value = serde_json::from_str(&said).ok().filter(|_| ok)?;
        if status["BackendState"] != "Running" {
            return None;
        }
        let name = status["Self"]["DNSName"].as_str()?.trim_end_matches('.');
        let ip = status["Self"]["TailscaleIPs"]
            .as_array()?
            .iter()
            .filter_map(|ip| ip.as_str()?.parse::<std::net::IpAddr>().ok())
            .find(std::net::IpAddr::is_ipv4)?;
        Some(Node {
            name: name.to_owned(),
            ip,
            certificates: !name.is_empty()
                && status["CertDomains"]
                    .as_array()
                    .is_some_and(|domains| !domains.is_empty()),
        })
    }

    /// Put the page behind Tailscale HTTPS on `https`. On failure: what the owner can do.
    pub fn serve(https: u16, local: u16) -> Result<(), String> {
        let port = format!("--https={https}");
        let target = format!("http://127.0.0.1:{local}");
        if let Some((true, said)) = run(&["serve", "status", "--json"], Duration::from_secs(5))
            && taken(&said, https, &target)
        {
            return Err(format!(
                "Tailscale port {https} already serves something else, so it was left alone.\nSet another https_port under [phone] in config.toml, then pair again."
            ));
        }
        let Some((ok, said)) = run(&["serve", "--bg", &port, &target], Duration::from_secs(15))
        else {
            return Err("Tailscale is not installed.".into());
        };
        if ok {
            return Ok(());
        }
        Err(advice(&said))
    }

    /// Does the serve config (`tailscale serve status --json`) already use this HTTPS port
    /// for something that is not our page? Ours left over from a crash does not count.
    pub fn taken(status: &str, https: u16, target: &str) -> bool {
        let Ok(status) = serde_json::from_str::<serde_json::Value>(status) else {
            return false;
        };
        let port = https.to_string();
        if status["TCP"].get(&port).is_none() {
            return false;
        }
        let suffix = format!(":{port}");
        let mut ours = false;
        for (host, site) in status["Web"].as_object().into_iter().flatten() {
            if !host.ends_with(&suffix) {
                continue;
            }
            for handler in site["Handlers"].as_object().into_iter().flatten() {
                if handler.1["Proxy"] != target {
                    return true;
                }
                ours = true;
            }
        }
        !ours
    }

    /// Tailscale's refusal, as one thing to do.
    pub fn advice(said: &str) -> String {
        if said.contains("--operator") || said.contains("Access denied") {
            let user = std::env::var("USER").unwrap_or_else(|_| "$USER".into());
            return format!(
                "For a secure link, allow Tailscale once, then pair again:\nsudo tailscale set --operator={user}"
            );
        }
        if let Some(link) = said
            .split_whitespace()
            .find(|word| word.starts_with("https://login.tailscale.com/"))
        {
            return format!(
                "For a secure link, switch on Tailscale Serve, then pair again:\n{link}"
            );
        }
        let first = said
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("it gave no answer");
        format!("Tailscale could not open a secure link: {first}")
    }

    /// Take the page off Tailscale again: this port's entry only, never the whole config.
    pub fn unserve(https: u16) {
        let port = format!("--https={https}");
        let _ = run(&["serve", &port, "off"], Duration::from_secs(10));
    }
}

/// This computer's address on the local network: the one it would reach the internet from.
/// Nothing is sent; connecting a UDP socket only picks the route.
fn lan_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    match ip {
        IpAddr::V4(v4) if v4.is_private() => Some(ip),
        _ => None,
    }
}

/// A Tailscale entry left behind by a run that ended without tidying up (a closed window, a
/// crash) is taken down. Called when the service starts, before anything can pair.
pub fn reconcile() {
    let record = crate::paths::pair_serve();
    if let Ok(port) = std::fs::read_to_string(&record) {
        if let Ok(port) = port.trim().parse() {
            tailscale::unserve(port);
        }
        let _ = std::fs::remove_file(record);
    }
}

/// Unpair from the command line (`3yiddish playground off`): the link dies with its code.
pub fn forget() {
    reconcile();
    let _ = std::fs::remove_file(crate::paths::pair_code());
}

impl PairHub {
    /// A hub that keeps its takes in `folder`.
    pub fn at(folder: impl Into<std::path::PathBuf>) -> Self {
        Self {
            state: Default::default(),
            store: Default::default(),
            folder: Arc::new(folder.into()),
            demo: false,
            on: None,
            goal: freebench::GOAL,
        }
    }
    /// The same hub with the playground switched on whatever the marker file says (tests).
    pub fn switched_on(mut self) -> Self {
        self.on = Some(true);
        self
    }
    /// The same hub in demo mode: canned drafts, nothing is sent to a model.
    pub fn demo(mut self) -> Self {
        self.demo = true;
        self
    }
    fn on(&self) -> bool {
        self.on.unwrap_or_else(playground)
    }
    fn takes(&self) -> Store {
        Store::new(self.folder.as_ref())
    }
    /// Demo mode starts with one take fixed and one to review, so every screen can be judged
    /// at a glance. The sound is quiet and made up: there is nothing to hear.
    fn seed_demo(&self) {
        let store = self.takes();
        let pcm: Vec<f32> = (0..16_000 * 3)
            .map(|i| (i as f32 / 23.0).sin() * (i as f32 / 4000.0).sin() * 0.02)
            .collect();
        let model = json!({"name": "demo", "engine": "demo"});
        let _ = store.add(&pcm, DEMO_DRAFTS[1], model.clone());
        let _ = store.review(
            1,
            freebench::Review {
                marks: vec![freebench::Mark {
                    start: 6,
                    end: 7,
                    heard: String::new(),
                    right: Some("עקראן".into()),
                    reason: "wrong".into(),
                    note: String::new(),
                }],
                typed: None,
                toss: false,
            },
        );
        let _ = store.add(&pcm, DEMO_DRAFTS[2], model);
    }
    /// The idea now on the Record screen; one is picked when there is none yet.
    fn idea(&self) -> &'static str {
        let ideas = ideas();
        if ideas.is_empty() {
            return "";
        }
        let mut hub = self.state.lock().unwrap();
        let at = *hub
            .idea
            .get_or_insert_with(|| (uuid::Uuid::new_v4().as_u128() % ideas.len() as u128) as usize);
        ideas[at % ideas.len()]
    }
    /// Another idea, never the same one twice in a row; both screens show it from now on.
    fn next_idea(&self) -> &'static str {
        let count = ideas().len();
        if count > 1 {
            let mut hub = self.state.lock().unwrap();
            let step = 1 + (uuid::Uuid::new_v4().as_u128() % (count as u128 - 1)) as usize;
            hub.idea = Some(hub.idea.map_or(step, |now| now + step) % count);
        }
        self.idea()
    }
    fn snapshot(&self) -> Value {
        let hub = self.state.lock().unwrap();
        let (takes, all) = self.takes().count();
        let mut value = json!({
            "playground": playground(),
            "active": hub.open.is_some(),
            "connected": hub.open.is_some()
                && hub.seen.is_some_and(|at| at.elapsed() < Duration::from_secs(12)),
            "takes": takes,
            "all": all,
            "goal": self.goal,
        });
        if let Some(open) = &hub.open {
            value["url"] = json!(open.url);
            value["transport"] = json!(open.transport);
            value["secure"] = json!(open.transport == "tailscale-https");
            value["hint"] = json!(open.hint);
        }
        value
    }

    /// Start serving the phone page and say where. Already paired: the same answer again.
    async fn open(&self, app: &App) -> Result<Value> {
        if !playground() {
            return Err(fail(StatusCode::NOT_FOUND, "The playground is off"));
        }
        if self.state.lock().unwrap().open.is_some() {
            return Ok(self.snapshot());
        }
        let code = code().map_err(broken)?;
        let phone = app.config.phone.clone().unwrap_or_default();
        let (port, https) = (
            phone.port.unwrap_or(PORT),
            phone.https_port.unwrap_or(HTTPS_PORT),
        );
        let node = tokio::task::spawn_blocking(tailscale::node)
            .await
            .map_err(broken)?;
        let bind = |ip: IpAddr| async move {
            tokio::net::TcpListener::bind(SocketAddr::new(ip, port))
                .await
                .map_err(|e| {
                    fail(
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!("Cannot open port {port} for the phone: {e}"),
                    )
                })
        };
        let local = IpAddr::from([127, 0, 0, 1]);
        let (listener, origin, transport, hint, https_port) = match node {
            // Tailscale HTTPS in front of a listener nothing else can reach.
            Some(node) if node.certificates => {
                let listener = bind(local).await?;
                match tokio::task::spawn_blocking(move || tailscale::serve(https, port))
                    .await
                    .map_err(broken)?
                {
                    Ok(()) => {
                        let _ = std::fs::write(crate::paths::pair_serve(), https.to_string());
                        let origin = if https == 443 {
                            format!("https://{}", node.name)
                        } else {
                            format!("https://{}:{https}", node.name)
                        };
                        (listener, origin, "tailscale-https", String::new(), Some(https))
                    }
                    Err(hint) => {
                        drop(listener);
                        (
                            bind(node.ip).await?,
                            format!("http://{}:{port}", node.ip),
                            "tailscale-http",
                            hint,
                            None,
                        )
                    }
                }
            }
            Some(node) => (
                bind(node.ip).await?,
                format!("http://{}:{port}", node.ip),
                "tailscale-http",
                "For a secure link, switch on HTTPS Certificates for your tailnet\n(Tailscale admin console, DNS page), then pair again.".to_owned(),
                None,
            ),
            None => {
                let ip = lan_address().ok_or_else(|| {
                    fail(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "This computer is not on a network the phone can reach",
                    )
                })?;
                (
                    bind(ip).await?,
                    format!("http://{ip}:{port}"),
                    "lan-http",
                    String::new(),
                    None,
                )
            }
        };
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let page = gateway(app.clone(), &code);
        tokio::spawn(async move {
            let served = axum::serve(listener, page)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
            if let Err(error) = served {
                eprintln!("Phone page stopped: {error}");
            }
        });
        let mut hub = self.state.lock().unwrap();
        hub.seen = None;
        hub.open = Some(Open {
            url: format!("{origin}/{code}/"),
            transport,
            hint,
            https_port,
            stop,
        });
        drop(hub);
        Ok(self.snapshot())
    }

    /// The service is stopping: the page goes, the link's code stays for next time.
    pub async fn shutdown(&self) {
        self.close(false).await;
    }
    /// Stop serving the page. `forget` also throws the link's code away (Unpair); without it
    /// the same link works again the next time the app pairs (quitting the app).
    async fn close(&self, forget: bool) -> Value {
        let open = self.state.lock().unwrap().open.take();
        if let Some(open) = open {
            let _ = open.stop.send(());
            if let Some(port) = open.https_port {
                let _ = tokio::task::spawn_blocking(move || tailscale::unserve(port)).await;
                let _ = std::fs::remove_file(crate::paths::pair_serve());
            }
        }
        if forget {
            let _ = std::fs::remove_file(crate::paths::pair_code());
        }
        self.snapshot()
    }
}

#[derive(Deserialize)]
struct Closing {
    /// Keep the link's code for next time.
    #[serde(default)]
    keep: bool,
}
/// While the playground is off its addresses on the service do not exist.
async fn switched_on(State(app): State<App>, request: Request, next: Next) -> Response {
    if app.phone.on() {
        next.run(request).await
    } else {
        fail(StatusCode::NOT_FOUND, "The playground is off").into_response()
    }
}
/// The app's side, on the service's own (local) address: pair, look, unpair; and the
/// playground itself, through the very handlers the phone's page uses, so the takes have one
/// writer. These requests do not count as the phone being there.
pub fn controller(app: App) -> Router<App> {
    let playground = Router::new()
        .route("/v1/playground/state", get(state))
        .route(
            "/v1/playground/takes",
            post(upload).layer(DefaultBodyLimit::max(AUDIO_LIMIT)),
        )
        .route("/v1/playground/takes/{n}", post(review).delete(remove))
        .route("/v1/playground/audio/{n}", get(audio))
        .route("/v1/playground/idea", post(another_idea))
        .route("/v1/playground/send", post(send))
        .route_layer(middleware::from_fn_with_state(app, switched_on));
    Router::new().merge(playground).route(
        "/v1/pair",
        get(|State(app): State<App>| async move { Json(app.phone.snapshot()) })
            .post(|State(app): State<App>| async move { app.phone.open(&app).await.map(Json) })
            .delete(
                |State(app): State<App>, Query(closing): Query<Closing>| async move {
                    Json(app.phone.close(!closing.keep).await)
                },
            ),
    )
}

/// A take as the page gets it: the stored fields plus the draft split into words.
fn shown(take: &freebench::Take) -> Value {
    let mut value = serde_json::to_value(take).unwrap_or_default();
    value["words"] = json!(freebench::words(&take.draft));
    value
}
async fn state(State(app): State<App>) -> Result<Json<Value>> {
    let _guard = app.phone.store.lock().await;
    let takes = app.phone.takes().load().map_err(broken)?;
    Ok(Json(json!({
        "goal": app.phone.goal,
        "reasons": freebench::REASONS,
        "items": takes.iter().map(shown).collect::<Vec<_>>(),
        "idea": app.phone.idea(),
        "demo": app.phone.demo,
        // A contributor's takes can be sent; the owner's own set never is.
        "can_send": app.phone.goal == 0,
        "name": std::fs::read_to_string(crate::paths::contributor_name()).unwrap_or_default().trim(),
    })))
}

/// Where contributors' takes go: a small relay that hands them to the project's dataset for
/// review. `$THREE_YIDDISH_RELAY` names another one (tests).
pub(crate) fn relay() -> String {
    std::env::var("THREE_YIDDISH_RELAY")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| RELAY.to_owned())
        .trim_end_matches('/')
        .to_owned()
}
const RELAY: &str = "https://3yiddish.tripleu.org";
/// The most takes in one request to the relay; a Send makes as many requests as it takes.
const SEND_MOST: usize = 20;
/// The most requests one Send makes (the rest go with the next Send).
const SEND_ROUNDS: usize = 10;

#[derive(Deserialize)]
struct Sending {
    name: String,
}
/// Send: the contributor's reviewed takes that were not sent yet go to the relay, under the
/// name given. Only what he pressed Send for, and never the owner's own set.
async fn send(State(app): State<App>, Json(sending): Json<Sending>) -> Result<Json<Value>> {
    if app.phone.goal != 0 {
        return Err(fail(StatusCode::FORBIDDEN, "This set is not sent anywhere"));
    }
    if app.phone.demo && std::env::var_os("THREE_YIDDISH_RELAY").is_none() {
        return Err(fail(StatusCode::FORBIDDEN, "Demo mode: nothing is sent"));
    }
    let name: String = sending
        .name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() || name.chars().count() > 40 {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Give a name, up to 40 letters (any name will do)",
        ));
    }
    let _ = std::fs::write(crate::paths::contributor_name(), &name);
    // The relay may be waking up: its first answer can take a while.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(broken)?;
    let mut sent = 0;
    for round in 0..SEND_ROUNDS {
        let takes = {
            let _guard = app.phone.store.lock().await;
            let mut takes = app.phone.takes().unsent().map_err(broken)?;
            takes.truncate(SEND_MOST);
            takes
        };
        if takes.is_empty() {
            break;
        }
        match send_round(&app, &client, &name, &takes).await {
            Ok(place) => {
                let done: Vec<(u32, String)> =
                    takes.iter().map(|t| (t.n, t.updated.clone())).collect();
                let _guard = app.phone.store.lock().await;
                app.phone.takes().mark_sent(&done, &place).map_err(broken)?;
                sent += takes.len();
            }
            // What went before this round stays sent; the rest goes with the next Send.
            Err(_) if round > 0 => break,
            Err(error) => return Err(error),
        }
    }
    if sent == 0 {
        return Err(fail(StatusCode::BAD_REQUEST, "Nothing new to send"));
    }
    let left = {
        let _guard = app.phone.store.lock().await;
        app.phone.takes().unsent().map_err(broken)?.len()
    };
    Ok(Json(json!({"sent": sent, "left": left})))
}

/// One request to the relay: 4 bytes (the manifest's length), the manifest (who, and each
/// take with its recording's size and SHA-256), then the recordings. Says where they went.
async fn send_round(
    app: &App,
    client: &reqwest::Client,
    name: &str,
    takes: &[freebench::Take],
) -> Result<String> {
    use sha2::Digest;
    let mut entries = Vec::new();
    let mut sound = Vec::new();
    for take in takes {
        let bytes = tokio::fs::read(app.phone.takes().audio(take.n))
            .await
            .map_err(broken)?;
        let sum: String = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        entries.push(json!({"take": take, "size": bytes.len(), "sha256": sum}));
        sound.extend(bytes);
    }
    let manifest = serde_json::to_vec(&json!({
        "name": name,
        "app": env!("CARGO_PKG_VERSION"),
        "takes": entries,
    }))
    .map_err(broken)?;
    let mut body = (manifest.len() as u32).to_be_bytes().to_vec();
    body.extend(manifest);
    body.extend(sound);
    let response = client
        .post(format!("{}/send", relay()))
        .header("content-type", "application/octet-stream")
        .body(body)
        .send()
        .await
        .map_err(|e| {
            fail(
                StatusCode::BAD_GATEWAY,
                if e.is_timeout() || e.is_connect() {
                    "Could not reach 3Yiddish. Check the internet connection and try again."
                        .to_owned()
                } else {
                    format!("Sending failed: {e}")
                },
            )
        })?;
    let status = response.status();
    let answer: Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        let why = answer["error"].as_str().unwrap_or("try again later");
        return Err(fail(
            StatusCode::BAD_GATEWAY,
            format!("3Yiddish did not take them: {why}"),
        ));
    }
    Ok(answer["id"].as_str().unwrap_or("sent").to_owned())
}
/// "Another idea", from the phone or the app: the one idea both show changes.
async fn another_idea(State(app): State<App>) -> Json<Value> {
    Json(json!({"idea": app.phone.next_idea()}))
}
/// A new recording: stored as 16 kHz mono whatever the phone sent, with our model's draft.
async fn upload(State(app): State<App>, bytes: Bytes) -> Result<Json<Value>> {
    if bytes.is_empty() {
        return Err(fail(StatusCode::BAD_REQUEST, "The recording is empty"));
    }
    let pcm = tokio::task::spawn_blocking(move || crate::asr::decode(bytes.to_vec()))
        .await
        .map_err(broken)?
        .map_err(|_| {
            fail(
                StatusCode::BAD_REQUEST,
                "This recording's format cannot be read",
            )
        })?;
    let seconds = pcm.len() as f32 / 16_000.0;
    if seconds < 0.4 {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Too short: nothing was recorded",
        ));
    }
    if seconds > 600.0 {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Too long: keep a take under ten minutes",
        ));
    }
    let (draft, model) = if app.phone.demo {
        let n = app.phone.takes().count().1;
        (
            DEMO_DRAFTS[n % DEMO_DRAFTS.len()].to_owned(),
            json!({"name": "demo", "engine": "demo"}),
        )
    } else {
        let wav = crate::asr::wav(&pcm).map_err(broken)?;
        app.draft(wav)
            .await
            .map_err(|e| fail(StatusCode::BAD_GATEWAY, e.to_string()))?
    };
    let _guard = app.phone.store.lock().await;
    let take = app.phone.takes().add(&pcm, &draft, model).map_err(broken)?;
    Ok(Json(shown(&take)))
}
async fn review(
    State(app): State<App>,
    Path(n): Path<u32>,
    Json(review): Json<freebench::Review>,
) -> Result<Json<Value>> {
    let _guard = app.phone.store.lock().await;
    let take = app
        .phone
        .takes()
        .review(n, review)
        .map_err(|e| fail(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(shown(&take)))
}
async fn remove(State(app): State<App>, Path(n): Path<u32>) -> Result<Json<Value>> {
    let _guard = app.phone.store.lock().await;
    app.phone
        .takes()
        .delete(n)
        .map_err(|e| fail(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({"deleted": n})))
}
async fn audio(State(app): State<App>, Path(n): Path<u32>) -> Result<Response> {
    let bytes = tokio::fs::read(app.phone.takes().audio(n))
        .await
        .map_err(|_| fail(StatusCode::NOT_FOUND, "No such recording"))?;
    Ok(([(header::CONTENT_TYPE, "audio/wav")], bytes).into_response())
}

async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-robots-tag", "noindex"),
        ("permissions-policy", "microphone=(self), camera=()"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; media-src 'self' blob:; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    ] {
        h.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}
/// Runs for addresses that carry the code only: notes that the phone is there, and keeps the
/// number of requests at once small.
async fn admit(State(app): State<App>, request: Request, next: Next) -> Response {
    let Ok(_permit) = app.permits.clone().try_acquire_owned() else {
        return fail(StatusCode::TOO_MANY_REQUESTS, "Busy; try again").into_response();
    };
    app.phone.state.lock().unwrap().seen = Some(Instant::now());
    next.run(request).await
}
fn file(kind: &'static str, body: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, kind)], body)
}
/// The phone page and its few requests, all under `/<code>/`. Everything else: not found.
pub fn gateway(app: App, code: &str) -> Router {
    const JS: &str = "text/javascript; charset=utf-8";
    let at = |path: &str| format!("/{code}{path}");
    let home = at("/");
    Router::new()
        .route(
            &at(""),
            get(move || async move { Redirect::permanent(&home) }),
        )
        .route(
            &at("/"),
            get(|| async { Html(include_str!("../assets/phone/index.html")) }),
        )
        .route(
            &at("/app.js"),
            get(|| async { file(JS, include_str!("../assets/phone/app.js")) }),
        )
        .route(
            &at("/recorder.js"),
            get(|| async { file(JS, include_str!("../assets/phone/recorder.js")) }),
        )
        .route(
            &at("/style.css"),
            get(|| async {
                file(
                    "text/css; charset=utf-8",
                    include_str!("../assets/phone/style.css"),
                )
            }),
        )
        .route(&at("/api/state"), get(state))
        .route(
            &at("/api/takes"),
            post(upload).layer(DefaultBodyLimit::max(AUDIO_LIMIT)),
        )
        .route(&at("/api/takes/{n}"), post(review).delete(remove))
        .route(&at("/api/audio/{n}"), get(audio))
        .route(&at("/api/idea"), post(another_idea))
        .route_layer(middleware::from_fn_with_state(app.clone(), admit))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(middleware::from_fn(headers))
        .with_state(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn call(
        page: &Router,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
        let response = page.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes().to_vec();
        (parts.status, parts.headers, bytes)
    }

    /// The page answers only under its code, and its few requests keep the free bench.
    #[tokio::test]
    async fn the_page_lives_under_its_code_and_keeps_the_takes() {
        let folder = std::env::temp_dir().join(format!("3y-phone-{}", uuid::Uuid::new_v4()));
        let config = crate::config::Config::load_or_default(std::path::Path::new(
            "/nonexistent/config.toml",
        ))
        .unwrap();
        let mut app = App::new(config).unwrap();
        app.phone = PairHub::at(&folder);
        let code = "0123456789ABCDEFGHIJ";
        let page = gateway(app.clone(), code);
        // Without the code there is nothing: no page, no script, no data.
        for path in [
            "/",
            "/app.js",
            "/api/state",
            "/0123456789ABCDEFGHII/",
            "/0123456789ABCDEFGHIJ0/api/state",
            "/0123456789abcdefghij/",
        ] {
            let (status, _, body) = call(&page, "GET", path, vec![]).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(body.is_empty(), "{path}");
        }
        assert!(app.phone.state.lock().unwrap().seen.is_none());
        let (status, headers, _) = call(&page, "GET", &format!("/{code}"), vec![]).await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
        assert_eq!(headers["location"], format!("/{code}/").as_str());
        let (status, headers, body) = call(&page, "GET", &format!("/{code}/"), vec![]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            String::from_utf8(body)
                .unwrap()
                .contains("3Yiddish playground")
        );
        assert!(
            headers["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("script-src 'self'")
        );
        assert_eq!(headers["cache-control"], "no-store");
        assert!(app.phone.state.lock().unwrap().seen.is_some());
        for file in ["app.js", "recorder.js", "style.css"] {
            let (status, _, body) = call(&page, "GET", &format!("/{code}/{file}"), vec![]).await;
            assert!(status == StatusCode::OK && body.len() > 200, "{file}");
        }
        let api = |path: &str| format!("/{code}/api/{path}");
        let json = |body: &[u8]| serde_json::from_slice::<Value>(body).unwrap();
        let (_, _, body) = call(&page, "GET", &api("state"), vec![]).await;
        assert_eq!(json(&body)["items"], json!([]));
        assert_eq!(json(&body)["goal"], 100);
        // Something that is not a recording is turned away before any model is asked.
        let (status, _, body) = call(&page, "POST", &api("takes"), b"not audio".to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            json(&body)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("format")
        );
        // A take (put there as an upload would) is shown with its words, reviewed and removed.
        app.phone
            .takes()
            .add(
                &[0.2; 16_000],
                "איינס צוויי דריי",
                json!({"name": "test", "engine": "cpu"}),
            )
            .unwrap();
        let (_, _, body) = call(&page, "GET", &api("state"), vec![]).await;
        assert_eq!(
            json(&body)["items"][0]["words"],
            json!(["איינס", "צוויי", "דריי"])
        );
        assert_eq!(json(&body)["items"][0]["status"], "pending");
        let review = json!({"marks": [{"start": 1, "end": 2, "right": "פיר", "reason": "wrong", "note": ""}], "typed": null, "toss": false});
        let (status, _, body) = call(
            &page,
            "POST",
            &api("takes/1"),
            review.to_string().into_bytes(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["final"], "איינס פיר דריי");
        assert_eq!(json(&body)["status"], "fixed");
        assert_eq!(json(&body)["held_out"], true);
        let bad = json!({"marks": [{"start": 2, "end": 9, "right": "x", "reason": "wrong"}]});
        let (status, _, _) =
            call(&page, "POST", &api("takes/1"), bad.to_string().into_bytes()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Anything bigger than a review has no business here.
        let (status, _, _) = call(&page, "POST", &api("takes/1"), vec![b' '; 300 * 1024]).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        let (status, headers, body) = call(&page, "GET", &api("audio/1"), vec![]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["content-type"], "audio/wav");
        assert_eq!(&body[..4], b"RIFF");
        assert_eq!(app.phone.snapshot()["takes"], 1);
        let (status, _, _) = call(&page, "DELETE", &api("takes/1"), vec![]).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = call(&page, "GET", &api("audio/1"), vec![]).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = call(&page, "GET", &api("audio/..%2f..%2fx"), vec![]).await;
        assert_ne!(status, StatusCode::OK);
        // Not paired, as the app sees it; and pairing is refused where the playground is off.
        assert_eq!(app.phone.snapshot()["active"], false);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn pairing_codes_are_long_and_fit_a_small_qr_code() {
        let code = new_code(uuid::Uuid::new_v4().as_u128());
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(
            code.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        );
        assert_ne!(code, new_code(uuid::Uuid::new_v4().as_u128()));
        assert_eq!(new_code(0), "0".repeat(CODE_LENGTH));
        assert_eq!(&new_code(35 + 36 * 10)[..2], "ZA");
    }
    /// A command that prints, then hangs with something else holding its output open, is cut
    /// off at the deadline and what it printed is still read.
    #[cfg(unix)]
    #[test]
    fn a_command_that_hangs_is_cut_off_and_still_heard() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("3y-ts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let script = folder.join("tailscale");
        std::fs::write(
            &script,
            "#!/bin/sh\necho visit https://login.tailscale.com/f/serve?node=x\nsleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Only this test sets the variable, and only it runs the command.
        unsafe { std::env::set_var("THREE_YIDDISH_TAILSCALE", &script) };
        let started = Instant::now();
        let (ok, said) = tailscale::run(&["serve"], Duration::from_millis(400)).unwrap();
        unsafe { std::env::remove_var("THREE_YIDDISH_TAILSCALE") };
        assert!(!ok && started.elapsed() < Duration::from_secs(5));
        assert!(tailscale::advice(&said).ends_with("https://login.tailscale.com/f/serve?node=x"));
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn a_tailscale_port_in_other_use_is_left_alone() {
        let ours = "http://127.0.0.1:8796";
        let status = |proxy: &str| {
            json!({"TCP": {"8443": {"HTTPS": true}}, "Web": {"work.example.ts.net:8443": {"Handlers": {"/": {"Proxy": proxy}}}}}).to_string()
        };
        assert!(!tailscale::taken("{}", 8443, ours));
        assert!(!tailscale::taken("", 8443, ours));
        assert!(!tailscale::taken(&status(ours), 8443, ours));
        assert!(tailscale::taken(
            &status("http://127.0.0.1:3000"),
            8443,
            ours
        ));
        assert!(!tailscale::taken(
            &status("http://127.0.0.1:3000"),
            8444,
            ours
        ));
        // A port forwarded as plain TCP, with no web handler at all.
        assert!(tailscale::taken(
            r#"{"TCP": {"8443": {"TCPForward": "127.0.0.1:22"}}}"#,
            8443,
            ours
        ));
    }
    #[test]
    fn tailscale_refusals_become_one_thing_to_do() {
        let denied = "Access denied: serve config denied\n\nUse 'sudo tailscale serve --bg --https=8443 http://127.0.0.1:8796'\nTo not require root, use 'sudo tailscale set --operator=$USER' once.";
        let advice = tailscale::advice(denied);
        assert!(advice.contains("sudo tailscale set --operator="));
        assert!(!advice.contains("$USER") || std::env::var("USER").is_err());
        let off = "Serve is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/serve?node=abc\n";
        assert!(tailscale::advice(off).ends_with("https://login.tailscale.com/f/serve?node=abc"));
        assert!(
            tailscale::advice("\n  something else broke\nmore").ends_with("something else broke")
        );
    }
}
