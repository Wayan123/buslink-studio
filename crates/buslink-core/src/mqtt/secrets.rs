//! Secrets (safety rule 14): a redacting wrapper and the store trait the shell implements.
//! A [`Secret`] cannot be serialised, printed or compared by accident.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The only way to read it; used when the value is handed to the broker.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Error)]
#[error("secret store: {0}")]
pub struct SecretError(pub String);

/// OS secret storage: Windows Credential Manager, Android Keystore, or memory only.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Secret>, SecretError>;
    fn set(&self, key: &str, value: &Secret) -> Result<(), SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
    /// Where secrets live, for the UI: `windows-credential-manager`, `android-keystore`, `memory`.
    fn kind(&self) -> &'static str;
    /// False when a secret does not survive an app restart.
    fn persistent(&self) -> bool;
}

/// Process memory only: gone when the app exits.
#[derive(Default)]
pub struct MemorySecretStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<Secret>, SecretError> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Ok(g.get(key).map(|v| Secret::new(v.clone())))
    }

    fn set(&self, key: &str, value: &Secret) -> Result<(), SecretError> {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(key.to_string(), value.expose().to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.remove(key);
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "memory"
    }

    fn persistent(&self) -> bool {
        false
    }
}

/// Replace every occurrence of the password in a text (errors and logs).
pub fn redact(text: &str, secret: Option<&Secret>) -> String {
    match secret {
        Some(s) if !s.is_empty() => text.replace(s.expose(), "[redacted]"),
        _ => text.to_string(),
    }
}
