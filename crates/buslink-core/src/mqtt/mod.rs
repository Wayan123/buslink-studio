//! MQTT 5 telemetry out (PRD R23, safety rule 16): plain JSON topics, a bounded
//! SQLite outbox replayed in order, TLS by default, credentials from the OS
//! secret store. Telemetry only: no subscriptions, no command topics.

pub mod config;
pub mod outbox;
pub mod payload;
pub mod secrets;
pub mod service;
pub mod tls;
pub mod topic;

use thiserror::Error;

pub use config::{Broker, ConfigError, MqttConfig, random_client_id};
pub use secrets::{MemorySecretStore, Secret, SecretStore};
pub use service::{MqttInit, MqttService, MqttState, MqttStatus, TestReport, test_connection};

#[derive(Debug, Error)]
pub enum MqttError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("TLS: {0}")]
    Tls(String),
    #[error(transparent)]
    Outbox(#[from] outbox::OutboxError),
    #[error("could not start the publisher: {0}")]
    Spawn(String),
}
