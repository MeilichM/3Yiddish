use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{collections::BTreeMap, net::SocketAddr, path::Path};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub url: String,
    #[serde(default)]
    pub model: String,
    pub api_key_env: Option<String>,
}
impl Endpoint {
    pub fn placeholder(&self) -> bool {
        self.model.contains("PLACEHOLDER")
    }
    pub fn key(&self) -> Result<Option<String>> {
        self.api_key_env
            .as_ref()
            .map(|name| {
                std::env::var(name)
                    .with_context(|| format!("Missing credential environment variable {name}"))
            })
            .transpose()
    }
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.url).context("Invalid endpoint URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Endpoints must be HTTP(S) URLs without credentials, query strings or fragments");
        }
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            || url
                .host_str()
                .is_some_and(|h| h.ends_with(".internal") || h == "host.docker.internal");
        if !local {
            bail!(
                "Only local model endpoints are supported; paid/remote provider endpoints are not allowed"
            );
        }
        if self
            .api_key_env
            .as_ref()
            .is_some_and(|n| n.trim().is_empty())
        {
            bail!("Empty credential environment variable name");
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub phone: Option<PhoneConfig>,
    pub listen: SocketAddr,
    pub api_token_env: Option<String>,
    pub knowledge_path: String,
    pub schmooze: Endpoint,
    pub translation: Option<Endpoint>,
    pub reasoners: BTreeMap<String, Endpoint>,
    pub default_reasoner: String,
    /// Aliases that answer directly in Yiddish with the trained local model.
    #[serde(default)]
    pub native_reasoners: Vec<String>,
    pub transcribe: Option<Endpoint>,
    pub speak: Option<Endpoint>,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
}
fn timeout() -> u64 {
    120
}
/// Transcribe-only defaults for a fresh install: no config file needed. Chat and Speak stay
/// placeholders (disabled) until a config file wires real local endpoints.
const BUILTIN: &str = r#"
listen = "127.0.0.1:8793"
knowledge_path = ""
default_reasoner = "local"
[schmooze]
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "SCHMOOZE_MODEL_PLACEHOLDER"
[reasoners.local]
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "LOCAL_REASONER_PLACEHOLDER"
"#;
impl Config {
    /// The config file when it exists, else the built-in Transcribe-only defaults.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.is_file() {
            Self::load(path)
        } else {
            Ok(toml::from_str(BUILTIN)?)
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut value: Self = toml::from_str(
            &std::fs::read_to_string(path)
                .context("Cannot read config; copy config.example.toml to config.toml")?,
        )?;
        if Path::new(&value.knowledge_path).is_relative() {
            value.knowledge_path = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&value.knowledge_path)
                .to_string_lossy()
                .into_owned();
        }
        if let Some(phone) = &value.phone {
            phone.validate()?;
        }
        value.schmooze.validate()?;
        if value.schmooze.model.is_empty() {
            bail!("Translator model is required");
        }
        for endpoint in value.reasoners.values() {
            endpoint.validate()?;
            if endpoint.model.is_empty() {
                bail!("Reasoner model is required");
            }
        }
        if !value.reasoners.contains_key(&value.default_reasoner) {
            bail!("Unknown default_reasoner");
        }
        if value
            .native_reasoners
            .iter()
            .any(|alias| !value.reasoners.contains_key(alias))
        {
            bail!("Unknown native_reasoners alias");
        }
        for endpoint in [&value.transcribe, &value.speak, &value.translation]
            .into_iter()
            .flatten()
        {
            endpoint.validate()?;
        }
        if !(1..=600).contains(&value.timeout_seconds) {
            bail!("timeout_seconds must be 1..600");
        }
        Ok(value)
    }
    pub fn token(&self) -> Result<Option<String>> {
        let token = self
            .api_token_env
            .as_ref()
            .map(|n| {
                std::env::var(n)
                    .with_context(|| format!("Missing service token environment variable {n}"))
            })
            .transpose()?;
        if token.as_ref().is_some_and(|s| s.len() < 24) {
            bail!("Service token must contain at least 24 characters");
        }
        if !self.listen.ip().is_loopback() && token.is_none() {
            bail!("A service token is required when listening outside loopback");
        }
        Ok(token)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneConfig {
    pub pairing_code_file: Option<String>,
    pub listen: SocketAddr,
    pub public_url: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub certificate_download_listen: Option<SocketAddr>,
    pub trust_certificate: Option<String>,
}
impl PhoneConfig {
    pub fn validate(&self) -> Result<()> {
        if self.tls_cert.is_some() != self.tls_key.is_some() {
            bail!("Phone TLS needs both tls_cert and tls_key");
        }
        if self.tls_cert.is_some() && !self.public_url.starts_with("https://") {
            bail!("Phone TLS requires an HTTPS public_url");
        }
        if let Some(address) = self.certificate_download_listen {
            if self.tls_cert.is_none() || self.trust_certificate.is_none() {
                bail!("Certificate download listener requires phone TLS and trust_certificate");
            }
            if address.ip() != self.listen.ip() || address.port() == self.listen.port() {
                bail!("Certificate download must use the same private IP and a different port");
            }
        }
        let private = match self.listen.ip() {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
            std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
        };
        if !private {
            bail!("Phone listener must bind a private LAN or loopback address");
        }
        let url = reqwest::Url::parse(&self.public_url)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!("Invalid phone public_url; use an HTTP(S) origin without credentials or a path");
        }
        if url.scheme() == "http" {
            let host = url.host_str().unwrap_or_default();
            let private = host == "localhost"
                || host.parse::<std::net::IpAddr>().is_ok_and(|ip| match ip {
                    std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
                    std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
                });
            if !private {
                bail!("Unencrypted phone links must stay on a private LAN");
            }
        }
        Ok(())
    }
}
