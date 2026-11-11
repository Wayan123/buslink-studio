//! Time sources. Monotonic milliseconds drive deadlines, ages and scheduling;
//! UTC milliseconds are what gets stored and exported.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    /// Monotonic milliseconds since an arbitrary origin. Never goes backwards.
    fn mono_ms(&self) -> u64;
    /// Wall-clock UTC milliseconds since the Unix epoch.
    fn utc_ms(&self) -> i64;
    /// Wait for `ms` milliseconds. A manual clock advances instead of sleeping.
    fn sleep_ms(&self, ms: u64);
}

/// Real clock for the app and the training simulator.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }

    pub fn shared() -> Arc<dyn Clock> {
        Arc::new(Self::new())
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn mono_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn utc_ms(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
            Err(_) => 0,
        }
    }

    fn sleep_ms(&self, ms: u64) {
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
}

/// Deterministic clock for tests: time only moves when told to (or when a
/// component "sleeps" on it).
#[derive(Debug)]
pub struct ManualClock {
    mono: AtomicU64,
    utc_origin: AtomicI64,
}

impl ManualClock {
    /// `utc_origin_ms` is the UTC time that corresponds to monotonic 0.
    pub fn new(utc_origin_ms: i64) -> Self {
        Self { mono: AtomicU64::new(0), utc_origin: AtomicI64::new(utc_origin_ms) }
    }

    pub fn shared(utc_origin_ms: i64) -> Arc<ManualClock> {
        Arc::new(Self::new(utc_origin_ms))
    }

    pub fn advance(&self, ms: u64) {
        self.mono.fetch_add(ms, Ordering::SeqCst);
    }

    /// Move to `mono_ms` if it is in the future; never goes backwards.
    pub fn advance_to(&self, mono_ms: u64) {
        self.mono.fetch_max(mono_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn mono_ms(&self) -> u64 {
        self.mono.load(Ordering::SeqCst)
    }

    fn utc_ms(&self) -> i64 {
        let mono = i64::try_from(self.mono_ms()).unwrap_or(i64::MAX);
        self.utc_origin.load(Ordering::SeqCst).saturating_add(mono)
    }

    fn sleep_ms(&self, ms: u64) {
        self.advance(ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_forward() {
        let c = ManualClock::new(1_000);
        assert_eq!(c.mono_ms(), 0);
        assert_eq!(c.utc_ms(), 1_000);
        c.advance(250);
        c.sleep_ms(50);
        assert_eq!(c.mono_ms(), 300);
        c.advance_to(100);
        assert_eq!(c.mono_ms(), 300);
        c.advance_to(400);
        assert_eq!(c.utc_ms(), 1_400);
    }

    #[test]
    fn system_clock_is_monotonic() {
        let c = SystemClock::new();
        let a = c.mono_ms();
        let b = c.mono_ms();
        assert!(b >= a);
        assert!(c.utc_ms() > 1_600_000_000_000);
    }
}
