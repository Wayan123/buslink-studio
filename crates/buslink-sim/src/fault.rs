//! Fault injection (PRD R24). Faults change how a device answers, never its register map.

use buslink_core::rng::SplitMix64;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    /// No reply.
    Timeout,
    /// Reply with a corrupted CRC (RTU) or a corrupted byte count (MBAP).
    CrcError,
    /// Reply with this exception code.
    Exception(u8),
    /// Process values frozen while replies continue (a gateway serving a cache).
    Stale,
    /// 32/64-bit values word-swapped, 16-bit registers byte-swapped.
    WrongOrder,
    /// Reply after an extra delay (to land after the client timeout).
    LateReply { delay_ms: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Always,
    /// Per request, drawn from the world's seeded fault generator.
    Probability {
        per_mille: u16,
    },
    /// Simulated time window `[from_ms, until_ms)` on the world clock.
    Window {
        from_ms: u64,
        until_ms: u64,
    },
    /// Requests `skip .. skip + count` to this unit (0-based).
    Requests {
        skip: u64,
        count: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultRule {
    pub unit: u8,
    pub kind: FaultKind,
    pub trigger: Trigger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaultId(pub u64);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FaultError {
    #[error("the stale fault needs a deterministic trigger (always, window or request count)")]
    StaleNeedsDeterministicTrigger,
    #[error("probability {0} is above 1000 per mille")]
    Probability(u16),
}

#[derive(Default)]
pub struct FaultTable {
    rules: Vec<(FaultId, FaultRule)>,
    next: u64,
}

/// The effect of the faults active for one request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    pub silent: bool,
    pub corrupt: bool,
    pub exception: Option<u8>,
    pub delay_ms: u64,
    pub wrong_order: bool,
}

impl FaultTable {
    pub fn add(&mut self, rule: FaultRule) -> Result<FaultId, FaultError> {
        if let Trigger::Probability { per_mille } = rule.trigger {
            if rule.kind == FaultKind::Stale {
                return Err(FaultError::StaleNeedsDeterministicTrigger);
            }
            if per_mille > 1000 {
                return Err(FaultError::Probability(per_mille));
            }
        }
        self.next += 1;
        let id = FaultId(self.next);
        self.rules.push((id, rule));
        Ok(id)
    }

    pub fn remove(&mut self, id: FaultId) -> bool {
        let before = self.rules.len();
        self.rules.retain(|(i, _)| *i != id);
        self.rules.len() != before
    }

    pub fn clear_unit(&mut self, unit: u8) {
        self.rules.retain(|(_, r)| r.unit != unit);
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn static_active(t: Trigger, now_ms: u64, request: u64) -> bool {
        match t {
            Trigger::Always => true,
            Trigger::Window { from_ms, until_ms } => (from_ms..until_ms).contains(&now_ms),
            Trigger::Requests { skip, count } => request >= skip && request - skip < count,
            Trigger::Probability { .. } => false,
        }
    }

    /// Is the device frozen right now? (`request` = requests handled so far.)
    pub fn stale(&self, unit: u8, now_ms: u64, request: u64) -> bool {
        self.rules.iter().any(|(_, r)| {
            r.unit == unit && r.kind == FaultKind::Stale && Self::static_active(r.trigger, now_ms, request)
        })
    }

    /// Effect for request number `request` (0-based) to `unit`. The first active
    /// reply-shaping rule (timeout, CRC, exception, late reply) wins.
    pub fn evaluate(&self, unit: u8, now_ms: u64, request: u64, rng: &mut SplitMix64) -> Effect {
        let mut e = Effect::default();
        let mut shaped = false;
        for (_, r) in self.rules.iter().filter(|(_, r)| r.unit == unit) {
            let active = match r.trigger {
                // Every probabilistic rule draws, active or not, so sequences stay reproducible.
                Trigger::Probability { per_mille } => rng.chance(per_mille),
                t => Self::static_active(t, now_ms, request),
            };
            if !active {
                continue;
            }
            match r.kind {
                FaultKind::WrongOrder => e.wrong_order = true,
                FaultKind::Stale => {}
                _ if shaped => {}
                FaultKind::Timeout => {
                    e.silent = true;
                    shaped = true;
                }
                FaultKind::CrcError => {
                    e.corrupt = true;
                    shaped = true;
                }
                FaultKind::Exception(c) => {
                    e.exception = Some(c);
                    shaped = true;
                }
                FaultKind::LateReply { delay_ms } => {
                    e.delay_ms = delay_ms;
                    shaped = true;
                }
            }
        }
        e
    }
}
