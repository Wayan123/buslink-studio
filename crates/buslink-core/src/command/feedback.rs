//! "Process reached target" (safety rule 7, commissioning-control design §5).
//!
//! A write being acknowledged says nothing about the process. This watch judges
//! the live feedback register named by the command's `feedback` entry: GOOD
//! samples newer than the command, within the tolerance, before the deadline.
//! A sample that is not GOOD never counts as reached.

use crate::profile::Feedback;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackState {
    /// Waiting for the process.
    Pending,
    /// Waiting, and the latest feedback sample is not GOOD.
    FeedbackUnavailable,
    Reached,
    /// The deadline passed without a GOOD sample within the tolerance.
    NotReached,
}

impl FeedbackState {
    pub fn is_final(self) -> bool {
        matches!(self, FeedbackState::Reached | FeedbackState::NotReached)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FeedbackState::Pending => "pending",
            FeedbackState::FeedbackUnavailable => "feedback_unavailable",
            FeedbackState::Reached => "reached",
            FeedbackState::NotReached => "not_reached",
        }
    }
}

/// One observation of the feedback register.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observation {
    /// Time the sample was taken (UTC ms).
    pub ts_ms: i64,
    pub good: bool,
    /// Engineering value (for bit feedback: the raw register as a number).
    pub value: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FeedbackWatch {
    pub command_id: String,
    pub device_id: String,
    pub register: String,
    pub target: f64,
    pub tolerance: f64,
    pub bit: Option<u8>,
    pub started_ms: i64,
    pub deadline_ms: i64,
    pub state: FeedbackState,
    pub last_value: Option<f64>,
    pub last_ts_ms: Option<i64>,
    pub resolved_ms: Option<i64>,
}

impl FeedbackWatch {
    /// `commanded` is the engineering value that was sent (used when the feedback has no fixed target).
    pub fn new(command_id: &str, device_id: &str, fb: &Feedback, commanded: f64, started_ms: i64) -> Self {
        Self {
            command_id: command_id.to_owned(),
            device_id: device_id.to_owned(),
            register: fb.register.clone(),
            target: fb.value.unwrap_or(commanded),
            tolerance: fb.tolerance.max(0.0),
            bit: fb.bit,
            started_ms,
            deadline_ms: started_ms.saturating_add(i64::from(fb.timeout_ms)),
            state: FeedbackState::Pending,
            last_value: None,
            last_ts_ms: None,
            resolved_ms: None,
        }
    }

    fn within(&self, v: f64) -> bool {
        match self.bit {
            Some(b) if b < 16 && v.is_finite() && v >= 0.0 => {
                let raw = v as u64;
                let bit = ((raw >> b) & 1) as f64;
                bit == self.target
            }
            Some(_) => false,
            None => v.is_finite() && (v - self.target).abs() <= self.tolerance + 1e-9,
        }
    }

    /// Feed the latest sample (or `None` if there is none) at time `now_ms`. Returns the new state.
    pub fn observe(&mut self, obs: Option<Observation>, now_ms: i64) -> FeedbackState {
        if self.state.is_final() {
            return self.state;
        }
        if let Some(o) = obs
            && o.ts_ms >= self.started_ms
        {
            self.last_ts_ms = Some(o.ts_ms);
            if o.good {
                self.last_value = o.value;
                if o.value.is_some_and(|v| self.within(v)) && o.ts_ms <= self.deadline_ms {
                    self.state = FeedbackState::Reached;
                    self.resolved_ms = Some(o.ts_ms);
                    return self.state;
                }
                self.state = FeedbackState::Pending;
            } else {
                self.state = FeedbackState::FeedbackUnavailable;
            }
        }
        if now_ms > self.deadline_ms {
            self.state = FeedbackState::NotReached;
            self.resolved_ms = Some(now_ms);
        }
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fb(value: Option<f64>, bit: Option<u8>) -> Feedback {
        Feedback {
            register: "output_freq".into(),
            tolerance: 0.05,
            timeout_ms: 10_000,
            value,
            bit,
        }
    }

    fn good(ts: i64, v: f64) -> Option<Observation> {
        Some(Observation {
            ts_ms: ts,
            good: true,
            value: Some(v),
        })
    }

    #[test]
    fn setpoint_target_and_tolerance() {
        let mut w = FeedbackWatch::new("set_frequency", "vfd", &fb(None, None), 30.0, 1000);
        assert_eq!(w.observe(good(1500, 12.0), 1500), FeedbackState::Pending);
        assert_eq!(w.observe(good(2500, 29.96), 2500), FeedbackState::Reached);
        // Latched.
        assert_eq!(w.observe(good(3000, 0.0), 3000), FeedbackState::Reached);
        assert_eq!(w.resolved_ms, Some(2500));
    }

    #[test]
    fn bad_quality_never_reaches() {
        let mut w = FeedbackWatch::new("stop", "vfd", &fb(Some(0.0), None), 0.0, 0);
        let bad = Some(Observation {
            ts_ms: 100,
            good: false,
            value: Some(0.0),
        });
        assert_eq!(w.observe(bad, 100), FeedbackState::FeedbackUnavailable);
        assert_eq!(w.observe(bad, 10_001), FeedbackState::NotReached);
    }

    #[test]
    fn samples_older_than_the_command_are_ignored() {
        let mut w = FeedbackWatch::new("stop", "vfd", &fb(Some(0.0), None), 0.0, 5000);
        assert_eq!(w.observe(good(4999, 0.0), 5000), FeedbackState::Pending);
        assert_eq!(w.observe(good(5200, 0.0), 5200), FeedbackState::Reached);
    }

    #[test]
    fn deadline_and_bits() {
        let mut w = FeedbackWatch::new("run", "vfd", &fb(Some(1.0), Some(0)), 1.0, 0);
        assert_eq!(w.observe(good(100, 2.0), 100), FeedbackState::Pending); // bit 1 set, bit 0 not
        assert_eq!(w.observe(good(200, 3.0), 200), FeedbackState::Reached);
        let mut late = FeedbackWatch::new("run", "vfd", &fb(Some(1.0), Some(0)), 1.0, 0);
        assert_eq!(late.observe(None, 10_001), FeedbackState::NotReached);
        assert_eq!(late.observe(good(10_002, 1.0), 10_002), FeedbackState::NotReached);
    }

    /// C4: random sequences never resolve to `reached` without a GOOD in-tolerance sample.
    #[test]
    fn reached_only_from_good_samples_in_tolerance() {
        crate::testkit::check("feedback_watch", 300, |r| {
            let mut w = FeedbackWatch::new("c", "d", &fb(None, None), 20.0, 0);
            let mut ts = 0i64;
            let mut witness = false;
            for _ in 0..r.range_u64(1, 30) {
                ts += r.range_u64(1, 2000) as i64;
                let good_q = r.chance(600);
                let v = r.range_u64(0, 4000) as f64 / 100.0;
                let obs = Observation {
                    ts_ms: ts,
                    good: good_q,
                    value: Some(v),
                };
                let before = w.state;
                let st = w.observe(Some(obs), ts);
                if st == FeedbackState::Reached && before != FeedbackState::Reached {
                    assert!(good_q && (v - 20.0).abs() <= 0.05 + 1e-9 && ts <= 10_000);
                    witness = true;
                }
            }
            if w.state == FeedbackState::Reached {
                assert!(witness);
            }
        });
    }
}
