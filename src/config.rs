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

/// The optional `[phone]` table: other ports for the playground's phone page. Older versions
/// kept a listener address and certificates here; those keys are still accepted, and
/// ignored, so an existing config keeps loading.
#[derive(Clone, Default, Deserialize)]
pub struct PhoneConfig {
    /// The phone page's own port on this computer (8796 when not given).
    pub port: Option<u16>,
    /// The Tailscale HTTPS port in front of it (8443 when not given).
    pub https_port: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config written for an older version (a phone listener with certificates) loads.
    #[test]
    fn an_older_phone_table_is_accepted_and_ignored() {
        let folder = std::env::temp_dir().join(format!("3y-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("config.toml");
        std::fs::write(
            &file,
            r#"
listen = "127.0.0.1:8793"
knowledge_path = ""
default_reasoner = "local"
[schmooze]
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "SCHMOOZE_MODEL_PLACEHOLDER"
[reasoners.local]
url = "http://127.0.0.1:11434/v1/chat/completions"
model = "LOCAL_REASONER_PLACEHOLDER"
[phone]
listen = "10.168.168.99:8796"
public_url = "https://10.168.168.99:8796"
tls_cert = "/x/phone.pem"
tls_key = "/x/phone-key.pem"
certificate_download_listen = "10.168.168.99:8797"
trust_certificate = "/x/ca.crt"
pairing_code_file = "/x/pairing-code"
"#,
        )
        .unwrap();
        let config = Config::load(&file).unwrap();
        assert!(config.phone.unwrap().port.is_none());
        std::fs::write(
            &file,
            std::fs::read_to_string(&file).unwrap() + "port = 9001\nhttps_port = 10000\n",
        )
        .unwrap();
        let phone = Config::load(&file).unwrap().phone.unwrap();
        assert_eq!((phone.port, phone.https_port), (Some(9001), Some(10000)));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
