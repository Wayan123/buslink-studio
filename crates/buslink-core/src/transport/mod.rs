//! Byte transports (architecture D2) and bus ownership.
//!
//! Serial (Windows, Android) transports live in the app crates; this module
//! has the trait, a scripted fake, a Modbus TCP client and the exchanger that
//! runs one request/response over any transport.

pub mod exchange;
pub mod fake;
pub mod tcp;
pub mod usb_ids;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The port or connection is gone (USB detach, socket closed).
    #[error("link disconnected: {0}")]
    Disconnected(String),
    #[error("permission denied")]
    PermissionDenied,
    #[error("I/O error: {0}")]
    Io(String),
}

pub trait Transport: Send {
    /// Write all bytes or fail.
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Read up to `buf.len()` bytes, waiting at most `timeout_ms`.
    /// `Ok(0)` means nothing arrived before the timeout; it is not end of stream.
    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError>;

    /// Discard stale input by reading until the line has been quiet for
    /// `quiet_ms` (no hardware purge: the CH340 driver on Android throws on it).
    /// Bounded to 16 reads. Returns the number of discarded bytes.
    fn drain(&mut self, quiet_ms: u64) -> Result<usize, TransportError> {
        let mut buf = [0u8; 256];
        let mut total = 0;
        for _ in 0..16 {
            let n = self.read(&mut buf, quiet_ms)?;
            if n == 0 {
                break;
            }
            total += n;
        }
        Ok(total)
    }

    fn close(&mut self) {}
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        (**self).write_all(bytes)
    }
    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        (**self).read(buf, timeout_ms)
    }
    fn drain(&mut self, quiet_ms: u64) -> Result<usize, TransportError> {
        (**self).drain(quiet_ms)
    }
    fn close(&mut self) {
        (**self).close()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("'{0}' is already owned by another worker (one master per bus)")]
pub struct AlreadyOwned(pub String);

/// Process-wide record of which buses have an owner (safety rule 9).
#[derive(Debug, Default)]
pub struct LinkRegistry {
    owned: Mutex<HashSet<String>>,
}

impl LinkRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn claim(self: &Arc<Self>, key: &str) -> Result<LinkClaim, AlreadyOwned> {
        let mut owned = self.owned.lock().unwrap_or_else(|e| e.into_inner());
        if !owned.insert(key.to_owned()) {
            return Err(AlreadyOwned(key.to_owned()));
        }
        Ok(LinkClaim {
            registry: Arc::clone(self),
            key: key.to_owned(),
        })
    }

    pub fn is_owned(&self, key: &str) -> bool {
        self.owned.lock().unwrap_or_else(|e| e.into_inner()).contains(key)
    }
}

/// Ownership of one bus; released on drop.
#[derive(Debug)]
pub struct LinkClaim {
    registry: Arc<LinkRegistry>,
    key: String,
}

impl LinkClaim {
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for LinkClaim {
    fn drop(&mut self) {
        self.registry
            .owned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_per_key() {
        let reg = LinkRegistry::new();
        let a = reg.claim("serial:com7").unwrap();
        assert_eq!(
            reg.claim("serial:com7").unwrap_err(),
            AlreadyOwned("serial:com7".into())
        );
        let _b = reg.claim("serial:com8").unwrap();
        drop(a);
        assert!(!reg.is_owned("serial:com7"));
        assert!(reg.claim("serial:com7").is_ok());
    }
}
