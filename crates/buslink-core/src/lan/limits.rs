//! Per-address rate limit and failed-token lockout (safety rule 15, IEC 62443 FR7).
//! Pure: the caller passes the time, so tests are deterministic.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Connection caps (security review 07, L3). A LAN host that holds idle connections
/// must not be able to starve viewers who have the token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnLimits {
    /// All open connections.
    pub max_total: usize,
    /// Open connections from one address.
    pub per_ip: usize,
    /// Connections that have not yet sent one request with a valid token. They have
    /// their own pool, so a flood of them cannot take the slots authenticated viewers use.
    pub max_unauthenticated: usize,
}

impl ConnLimits {
    pub fn new(max_total: usize) -> Self {
        let max_total = max_total.max(1);
        Self {
            max_total,
            per_ip: 4,
            max_unauthenticated: (max_total / 4).max(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Total,
    PerIp,
    Unauthenticated,
}

#[derive(Default)]
struct ConnCounts {
    total: usize,
    unauthenticated: usize,
    per_ip: HashMap<IpAddr, usize>,
}

/// Counts open connections. [`ConnTracker::admit`] hands out a guard that gives the slot back on drop.
pub struct ConnTracker {
    limits: ConnLimits,
    counts: Mutex<ConnCounts>,
}

/// One admitted connection. Starts unauthenticated; the first request with a valid token moves it
/// to the authenticated pool (which is only bounded by the total and per-address caps).
pub struct ConnGuard {
    tracker: Arc<ConnTracker>,
    ip: IpAddr,
    authenticated: AtomicBool,
}

impl ConnTracker {
    pub fn new(limits: ConnLimits) -> Arc<Self> {
        Arc::new(Self {
            limits,
            counts: Mutex::new(ConnCounts::default()),
        })
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, ConnCounts> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn admit(self: &Arc<Self>, ip: IpAddr) -> Result<Arc<ConnGuard>, Refusal> {
        let mut c = self.counts();
        if c.total >= self.limits.max_total {
            return Err(Refusal::Total);
        }
        if c.per_ip.get(&ip).copied().unwrap_or(0) >= self.limits.per_ip {
            return Err(Refusal::PerIp);
        }
        if c.unauthenticated >= self.limits.max_unauthenticated {
            return Err(Refusal::Unauthenticated);
        }
        c.total += 1;
        c.unauthenticated += 1;
        *c.per_ip.entry(ip).or_insert(0) += 1;
        drop(c);
        Ok(Arc::new(ConnGuard {
            tracker: Arc::clone(self),
            ip,
            authenticated: AtomicBool::new(false),
        }))
    }

    /// `(total, unauthenticated)` open now.
    pub fn open(&self) -> (usize, usize) {
        let c = self.counts();
        (c.total, c.unauthenticated)
    }

    pub fn open_from(&self, ip: IpAddr) -> usize {
        self.counts().per_ip.get(&ip).copied().unwrap_or(0)
    }
}

impl ConnGuard {
    /// Called when a request on this connection carried a valid token.
    pub fn mark_authenticated(&self) {
        if !self.authenticated.swap(true, Ordering::AcqRel) {
            let mut c = self.tracker.counts();
            c.unauthenticated = c.unauthenticated.saturating_sub(1);
        }
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let authed = self.authenticated.load(Ordering::Acquire);
        let mut c = self.tracker.counts();
        c.total = c.total.saturating_sub(1);
        if !authed {
            c.unauthenticated = c.unauthenticated.saturating_sub(1);
        }
        if let Some(n) = c.per_ip.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.per_ip.remove(&self.ip);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LimitConfig {
    /// Bucket size: the largest burst.
    pub capacity: f64,
    /// Requests per second added back.
    pub refill_per_s: f64,
    /// Failed token checks in `fail_window_ms` that trigger a lockout.
    pub fail_limit: usize,
    pub fail_window_ms: i64,
    pub lockout_ms: i64,
    /// Addresses remembered; the least recently seen is dropped beyond this.
    pub max_addrs: usize,
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self {
            capacity: 60.0,
            refill_per_s: 30.0,
            fail_limit: 5,
            fail_window_ms: 60_000,
            lockout_ms: 60_000,
            max_addrs: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Limited { retry_after_s: u64 },
}

#[derive(Debug)]
struct Entry {
    tokens: f64,
    last_ms: i64,
    fails: Vec<i64>,
    locked_until: i64,
}

#[derive(Debug)]
pub struct Limiter {
    cfg: LimitConfig,
    map: HashMap<IpAddr, Entry>,
}

impl Limiter {
    pub fn new(cfg: LimitConfig) -> Self {
        Self {
            cfg,
            map: HashMap::new(),
        }
    }

    pub fn tracked(&self) -> usize {
        self.map.len()
    }

    fn entry(&mut self, ip: IpAddr, now: i64) -> &mut Entry {
        if !self.map.contains_key(&ip) && self.map.len() >= self.cfg.max_addrs.max(1) {
            let oldest = self.map.iter().min_by_key(|(_, e)| e.last_ms).map(|(k, _)| *k);
            if let Some(k) = oldest {
                self.map.remove(&k);
            }
        }
        let capacity = self.cfg.capacity;
        self.map.entry(ip).or_insert(Entry {
            tokens: capacity,
            last_ms: now,
            fails: Vec::new(),
            locked_until: i64::MIN,
        })
    }

    /// Take one request from the address's bucket.
    pub fn check(&mut self, ip: IpAddr, now_ms: i64) -> Verdict {
        let (capacity, refill) = (self.cfg.capacity, self.cfg.refill_per_s);
        let e = self.entry(ip, now_ms);
        if now_ms < e.locked_until {
            return Verdict::Limited {
                retry_after_s: ((e.locked_until - now_ms) as u64).div_ceil(1000).max(1),
            };
        }
        let dt = (now_ms - e.last_ms).max(0) as f64 / 1000.0;
        e.tokens = (e.tokens + dt * refill).min(capacity);
        e.last_ms = now_ms.max(e.last_ms);
        if e.tokens >= 1.0 {
            e.tokens -= 1.0;
            Verdict::Allow
        } else {
            let need = (1.0 - e.tokens) / refill.max(0.001);
            Verdict::Limited {
                retry_after_s: need.ceil().max(1.0) as u64,
            }
        }
    }

    /// Count a failed token check; enough of them lock the address out.
    pub fn record_failure(&mut self, ip: IpAddr, now_ms: i64) {
        let (limit, window, lockout) = (self.cfg.fail_limit, self.cfg.fail_window_ms, self.cfg.lockout_ms);
        let e = self.entry(ip, now_ms);
        e.fails.retain(|t| now_ms - *t < window);
        e.fails.push(now_ms);
        if e.fails.len() >= limit {
            e.fails.clear();
            e.locked_until = now_ms + lockout;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SplitMix64;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([192, 168, 1, n])
    }

    #[test]
    fn per_address_and_unauthenticated_caps() {
        let t = ConnTracker::new(ConnLimits {
            max_total: 8,
            per_ip: 2,
            max_unauthenticated: 3,
        });
        let a1 = t.admit(ip(1)).unwrap();
        let _a2 = t.admit(ip(1)).unwrap();
        assert_eq!(t.admit(ip(1)).err(), Some(Refusal::PerIp), "one address is capped");
        let _b1 = t.admit(ip(2)).unwrap();
        // Three unauthenticated connections are open: a fourth address waits.
        assert_eq!(t.admit(ip(3)).err(), Some(Refusal::Unauthenticated));
        // A connection with a valid token leaves the unauthenticated pool...
        a1.mark_authenticated();
        a1.mark_authenticated(); // idempotent
        assert_eq!(t.open(), (3, 2));
        // ...so a viewer can connect even though the flood holds the rest of that pool.
        let _c = t.admit(ip(3)).unwrap();
        assert_eq!(t.admit(ip(4)).err(), Some(Refusal::Unauthenticated));
        drop(a1);
        assert_eq!(t.open_from(ip(1)), 1);
        assert_eq!(t.open(), (3, 3));
    }

    #[test]
    fn total_cap_and_release_on_drop() {
        let t = ConnTracker::new(ConnLimits {
            max_total: 2,
            per_ip: 4,
            max_unauthenticated: 2,
        });
        let g1 = t.admit(ip(1)).unwrap();
        let g2 = t.admit(ip(2)).unwrap();
        assert_eq!(t.admit(ip(3)).err(), Some(Refusal::Total));
        g1.mark_authenticated();
        drop(g1);
        drop(g2);
        assert_eq!(t.open(), (0, 0));
        assert_eq!(t.open_from(ip(1)), 0);
        assert!(t.admit(ip(3)).is_ok());
    }

    #[test]
    fn burst_then_refill() {
        let mut l = Limiter::new(LimitConfig {
            capacity: 3.0,
            refill_per_s: 1.0,
            ..LimitConfig::default()
        });
        assert!((0..3).all(|_| l.check(ip(1), 0) == Verdict::Allow));
        assert!(matches!(l.check(ip(1), 0), Verdict::Limited { .. }));
        assert_eq!(l.check(ip(1), 1000), Verdict::Allow);
        // Another address has its own bucket.
        assert_eq!(l.check(ip(2), 0), Verdict::Allow);
    }

    #[test]
    fn five_failures_lock_out_for_a_minute() {
        let mut l = Limiter::new(LimitConfig::default());
        for i in 0..5 {
            assert_eq!(l.check(ip(9), i * 100), Verdict::Allow);
            l.record_failure(ip(9), i * 100);
        }
        assert!(matches!(l.check(ip(9), 1000), Verdict::Limited { .. }));
        assert!(matches!(l.check(ip(9), 59_000), Verdict::Limited { .. }));
        assert_eq!(l.check(ip(9), 61_000), Verdict::Allow);
    }

    #[test]
    fn slow_failures_do_not_lock() {
        let mut l = Limiter::new(LimitConfig::default());
        for i in 0..10 {
            l.record_failure(ip(3), i * 20_000);
            assert_eq!(l.check(ip(3), i * 20_000 + 1), Verdict::Allow);
        }
    }

    /// L4: admitted requests never exceed capacity + refill × elapsed; memory stays bounded.
    #[test]
    fn l4_bucket_bound_and_memory() {
        let mut rng = SplitMix64::new(0x4C34);
        for case in 0..300 {
            let mut l = Limiter::new(LimitConfig {
                max_addrs: 4,
                ..LimitConfig::default()
            });
            let mut now = 0i64;
            for _ in 0..200 {
                now += rng.below(150) as i64;
                l.check(ip(rng.below(8) as u8), now);
                assert!(l.tracked() <= 4, "case {case}");
            }
        }
        for _ in 0..300 {
            let cfg = LimitConfig {
                capacity: (1 + rng.below(20)) as f64,
                refill_per_s: (1 + rng.below(30)) as f64,
                ..LimitConfig::default()
            };
            let (cap, refill) = (cfg.capacity, cfg.refill_per_s);
            let mut l = Limiter::new(cfg);
            let mut now = 0i64;
            let mut admitted = 0u64;
            for _ in 0..300 {
                now += rng.below(100) as i64;
                if l.check(ip(1), now) == Verdict::Allow {
                    admitted += 1;
                }
            }
            let bound = cap + refill * (now as f64 / 1000.0);
            assert!(admitted as f64 <= bound + 1e-6, "admitted {admitted} > bound {bound}");
        }
    }
}
