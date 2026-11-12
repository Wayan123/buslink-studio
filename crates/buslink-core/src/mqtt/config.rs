//! MQTT settings that may be saved to a file. There is no password field, and
//! unknown keys are an error, so a hand-edited `"password"` is refused, not read.

use super::topic::sanitize_level;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MqttConfig {
    /// `mqtts://host[:8883]`; `mqtt://host[:1883]` only with `allow_insecure`.
    pub broker_url: String,
    pub site: String,
    pub gateway: String,
    pub client_id: String,
    /// Empty = anonymous. The password lives in the secret store.
    pub username: String,
    /// Extra CA certificate(s) in PEM for a private broker.
    pub ca_pem: String,
    pub allow_insecure: bool,
    pub expiry_s: u32,
    pub keep_alive_s: u16,
    pub outbox_max: usize,
    pub min_interval_ms: u64,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            broker_url: String::new(),
            site: "site1".into(),
            gateway: "gateway1".into(),
            client_id: "buslink".into(),
            username: String::new(),
            ca_pem: String::new(),
            allow_insecure: false,
            expiry_s: 3600,
            keep_alive_s: 30,
            outbox_max: 10_000,
            min_interval_ms: 1000,
        }
    }
}

/// `buslink-` plus 4 random bytes as hex.
pub fn random_client_id() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut b = [0u8; 4];
    match SystemRandom::new().fill(&mut b) {
        Ok(()) => format!("buslink-{}", crate::lan::token::hex(&b)),
        Err(_) => "buslink".into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConfigError {
    #[error("broker URL: {0}")]
    Url(String),
    #[error("{field}: {message}")]
    Field { field: &'static str, message: String },
}

fn field(field: &'static str, message: impl Into<String>) -> ConfigError {
    ConfigError::Field {
        field,
        message: message.into(),
    }
}

impl MqttConfig {
    /// Check every field and return the parsed broker address.
    pub fn validate(&self) -> Result<Broker, ConfigError> {
        let broker = parse_broker(&self.broker_url, self.allow_insecure)?;
        if self.site.trim().is_empty() {
            return Err(field("site", "must not be empty"));
        }
        if self.gateway.trim().is_empty() {
            return Err(field("gateway", "must not be empty"));
        }
        let id = &self.client_id;
        if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(field("client_id", "1 to 64 characters: letters, digits, - and _"));
        }
        if self.username.len() > 128 || self.username.chars().any(char::is_control) {
            return Err(field("username", "at most 128 characters, no control characters"));
        }
        if !(1..=7 * 86_400).contains(&self.expiry_s) {
            return Err(field("expiry_s", "1 s to 7 days"));
        }
        if !(5..=1200).contains(&self.keep_alive_s) {
            return Err(field("keep_alive_s", "5 s to 1200 s"));
        }
        if !(100..=1_000_000).contains(&self.outbox_max) {
            return Err(field("outbox_max", "100 to 1 000 000 messages"));
        }
        if self.min_interval_ms > 3_600_000 {
            return Err(field("min_interval_ms", "at most 1 hour"));
        }
        if !self.ca_pem.is_empty() && (self.ca_pem.len() > 65_536 || !self.ca_pem.contains("BEGIN CERTIFICATE")) {
            return Err(field(
                "ca_pem",
                "must be a PEM certificate (BEGIN CERTIFICATE), at most 64 KB",
            ));
        }
        Ok(broker)
    }

    pub fn status_topic(&self) -> String {
        super::topic::status_topic(&self.site, &self.gateway)
    }

    pub fn tag_topic(&self, device: &str, tag: &str) -> String {
        super::topic::sample_topic(&self.site, &self.gateway, device, tag)
    }

    /// Normalised site and gateway as they appear in topics.
    pub fn levels(&self) -> (String, String) {
        (sanitize_level(&self.site), sanitize_level(&self.gateway))
    }
}

/// M7: TLS is the default; plain `mqtt://` needs the explicit tick; no user-info in the URL.
pub fn parse_broker(url: &str, allow_insecure: bool) -> Result<Broker, ConfigError> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ConfigError::Url("use mqtts://host:8883".into()))?;
    let (tls, default_port) = match scheme.to_ascii_lowercase().as_str() {
        "mqtts" | "ssl" | "tls" => (true, 8883),
        "mqtt" | "tcp" => (false, 1883),
        other => return Err(ConfigError::Url(format!("unsupported scheme '{other}'; use mqtts://"))),
    };
    if !tls && !allow_insecure {
        return Err(ConfigError::Url(
            "unencrypted mqtt:// is refused; use mqtts:// or tick 'Allow unencrypted connection'".into(),
        ));
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') {
        return Err(ConfigError::Url(
            "do not put a user name or password in the URL; use the credential fields".into(),
        ));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>()
                .map_err(|_| ConfigError::Url("port is not a number".into()))?,
        ),
        None => (authority, default_port),
    };
    if port == 0 {
        return Err(ConfigError::Url("port must be 1 to 65535".into()));
    }
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-']);
    if !host_ok {
        return Err(ConfigError::Url("host name or IPv4 address expected".into()));
    }
    Ok(Broker {
        host: host.to_string(),
        port,
        tls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: &str) -> MqttConfig {
        MqttConfig {
            broker_url: url.into(),
            ..MqttConfig::default()
        }
    }

    /// M7
    #[test]
    fn m7_tls_by_default_and_no_userinfo() {
        assert_eq!(
            cfg("mqtts://broker.example").validate().unwrap(),
            Broker {
                host: "broker.example".into(),
                port: 8883,
                tls: true
            }
        );
        assert!(cfg("mqtts://b:8884").validate().is_ok());
        assert!(
            cfg("mqtt://192.168.1.2").validate().is_err(),
            "plain text needs the tick"
        );
        let mut c = cfg("mqtt://192.168.1.2");
        c.allow_insecure = true;
        assert_eq!(c.validate().unwrap().port, 1883);
        for bad in [
            "mqtts://user:pw@host",
            "mqtts://user@host",
            "http://host",
            "host:8883",
            "mqtts://",
            "mqtts://ho st",
            "mqtts://host:0",
            "mqtts://host:99999",
            "mqtts://-host",
        ] {
            assert!(cfg(bad).validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn field_checks() {
        let ok = cfg("mqtts://h");
        assert!(ok.validate().is_ok());
        for (name, c) in [
            (
                "id",
                MqttConfig {
                    client_id: "a b".into(),
                    ..ok.clone()
                },
            ),
            (
                "id empty",
                MqttConfig {
                    client_id: String::new(),
                    ..ok.clone()
                },
            ),
            (
                "site",
                MqttConfig {
                    site: " ".into(),
                    ..ok.clone()
                },
            ),
            (
                "expiry",
                MqttConfig {
                    expiry_s: 0,
                    ..ok.clone()
                },
            ),
            (
                "outbox",
                MqttConfig {
                    outbox_max: 5,
                    ..ok.clone()
                },
            ),
            (
                "ca",
                MqttConfig {
                    ca_pem: "hello".into(),
                    ..ok.clone()
                },
            ),
        ] {
            assert!(c.validate().is_err(), "{name}");
        }
    }

    /// M6 (file part): the saved config has no password, and a file that tries to add one is refused.
    #[test]
    fn m6_config_file_has_no_password_key() {
        let text = serde_json::to_string(&MqttConfig::default()).unwrap();
        assert!(!text.to_lowercase().contains("password") && !text.to_lowercase().contains("secret"));
        assert!(serde_json::from_str::<MqttConfig>(r#"{"password":"x"}"#).is_err());
        assert_eq!(serde_json::from_str::<MqttConfig>("{}").unwrap(), MqttConfig::default());
    }
}
