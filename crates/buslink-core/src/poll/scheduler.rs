//! One scheduler per link (architecture D3). Synchronous and clock-driven so
//! it can be tested deterministically; [`super::worker`] runs it on a thread.
//!
//! Order: command jobs, then alarm reads, then polling. Polling picks the
//! device whose cycle has been due longest (ties round-robin). A timeout aborts
//! the device's cycle and backs it off exponentially, so a dead device costs at
//! most one (plus retries) timeout per backoff period and never starves the
//! others. Command frames are sent once; there is no retry path for them.

use super::plan::{self, PlanLimits, ReadBlock};
use crate::clock::Clock;
use crate::codec;
use crate::command::PreparedFrame;
use crate::device::{DeviceError, DeviceInstance, LinkConfig};
use crate::profile::Profile;
use crate::protocol::pdu::{ReadData, ReadRequest, ReplyError, decode_read_response};
use crate::quality::{Observation, Quality, Sample, TagIdentity, TagState, Value};
use crate::transport::Transport;
use crate::transport::TransportError;
use crate::transport::exchange::{ExchangeError, ExchangeRecord, Exchanger, Framing, Outgoing};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use thiserror::Error;

pub const DEFAULT_BACKOFF_CAP_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Command,
    Alarm,
}

pub type Reply<T> = Box<dyn FnOnce(T) + Send>;

/// Work that jumps the polling queue.
pub enum UrgentJob {
    Read {
        unit: u8,
        request: ReadRequest,
        priority: Priority,
        timeout_ms: u64,
        reply: Reply<Result<ReadData, ExchangeError>>,
    },
    Command {
        frame: PreparedFrame,
        timeout_ms: u64,
        reply: Reply<ExchangeRecord>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceCounters {
    pub requests: u64,
    pub good: u64,
    pub timeouts: u64,
    pub crc_errors: u64,
    pub exceptions: u64,
    pub decode_errors: u64,
    pub last_error: Option<String>,
    /// Start-to-start time of the last two cycles (achieved poll interval).
    pub cycle_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub backoff_until_mono_ms: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Worked,
    Idle { until_mono_ms: u64 },
    LinkDown,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    #[error("device '{0}': {1}")]
    Device(String, DeviceError),
    #[error("duplicate device id '{0}'")]
    DuplicateDevice(String),
}

#[derive(Clone, Copy, Debug)]
struct Cycle {
    due: u64,
    block: usize,
    retries_used: u8,
}

struct PollDevice {
    instance: DeviceInstance,
    profile: Arc<Profile>,
    blocks: Vec<ReadBlock>,
    unreadable: Vec<usize>,
    tags: Vec<TagState>,
    timeout_ms: u64,
    stale_ms: u64,
    next_due: u64,
    cycle: Option<Cycle>,
    failures: u32,
    last_cycle_start: Option<u64>,
    counters: DeviceCounters,
}

pub struct LinkScheduler {
    link: LinkConfig,
    clock: Arc<dyn Clock>,
    exchanger: Exchanger,
    devices: Vec<PollDevice>,
    commands: VecDeque<UrgentJob>,
    alarms: VecDeque<UrgentJob>,
    seq: u64,
    rr: usize,
    down: bool,
    backoff_cap_ms: u64,
}

fn reply_error(e: ReplyError) -> ExchangeError {
    match e {
        ReplyError::Exception(c) => ExchangeError::Exception(c),
        ReplyError::Frame(f) => ExchangeError::Decode(f),
    }
}

impl LinkScheduler {
    pub fn new(
        link: LinkConfig,
        devices: Vec<(DeviceInstance, Arc<Profile>)>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, SchedulerError> {
        let framing = if link.uses_rtu_framing() {
            Framing::Rtu
        } else {
            Framing::Mbap
        };
        let exchanger = Exchanger::new(framing, Arc::clone(&clock), link.inter_frame_ms());
        let now = clock.mono_ms();
        let mut out: Vec<PollDevice> = Vec::new();
        for (instance, profile) in devices {
            instance
                .check(&profile, &link)
                .map_err(|e| SchedulerError::Device(instance.id.clone(), e))?;
            if out.iter().any(|d| d.instance.id == instance.id) {
                return Err(SchedulerError::DuplicateDevice(instance.id));
            }
            let p = plan::plan(&profile, PlanLimits::for_device(&instance));
            let tags = profile
                .registers
                .iter()
                .map(|r| {
                    TagState::new(TagIdentity {
                        link_id: link.id.clone(),
                        device_id: instance.id.clone(),
                        tag_id: r.id.clone(),
                        profile_id: profile.profile_id.clone(),
                        profile_version: profile.profile_version.clone(),
                        unit: r.unit.clone(),
                    })
                })
                .collect();
            out.push(PollDevice {
                timeout_ms: instance.timeout_ms(&profile, &link),
                stale_ms: instance.stale_threshold_ms(),
                instance,
                profile,
                blocks: p.blocks,
                unreadable: p.unreadable,
                tags,
                next_due: now,
                cycle: None,
                failures: 0,
                last_cycle_start: None,
                counters: DeviceCounters::default(),
            });
        }
        Ok(Self {
            link,
            clock,
            exchanger,
            devices: out,
            commands: VecDeque::new(),
            alarms: VecDeque::new(),
            seq: 0,
            rr: 0,
            down: false,
            backoff_cap_ms: DEFAULT_BACKOFF_CAP_MS,
        })
    }

    pub fn set_backoff_cap_ms(&mut self, ms: u64) {
        self.backoff_cap_ms = ms.max(1);
    }

    /// RTU only: how long the line is drained after a timeout before the next request.
    pub fn set_late_guard_ms(&mut self, ms: u64) {
        self.exchanger.set_late_guard_ms(ms);
    }

    pub fn link(&self) -> &LinkConfig {
        &self.link
    }

    pub fn clock_mono(&self) -> u64 {
        self.clock.mono_ms()
    }

    pub fn is_down(&self) -> bool {
        self.down
    }

    pub fn counters(&self) -> Vec<(String, DeviceCounters)> {
        self.devices
            .iter()
            .map(|d| (d.instance.id.clone(), d.counters.clone()))
            .collect()
    }

    /// Default timeout for urgent jobs addressed to `unit`.
    pub fn timeout_for_unit(&self, unit: u8) -> u64 {
        self.devices
            .iter()
            .find(|d| d.instance.unit_id == unit)
            .map(|d| d.timeout_ms)
            .unwrap_or(1000)
    }

    /// Queue a command or alarm job. On a dead link it fails at once (never replayed later).
    pub fn submit(&mut self, job: UrgentJob) {
        if self.down {
            fail_job(job);
            return;
        }
        match &job {
            UrgentJob::Command { .. }
            | UrgentJob::Read {
                priority: Priority::Command,
                ..
            } => self.commands.push_back(job),
            UrgentJob::Read {
                priority: Priority::Alarm,
                ..
            } => self.alarms.push_back(job),
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Do at most one exchange.
    pub fn step(&mut self, t: &mut dyn Transport, emit: &mut dyn FnMut(Sample)) -> Step {
        if self.down {
            return Step::LinkDown;
        }
        self.emit_stale(emit);
        if let Some(job) = self.commands.pop_front().or_else(|| self.alarms.pop_front()) {
            if self.run_urgent(t, job) {
                self.go_down(emit, "link lost during a command or alarm read");
                return Step::LinkDown;
            }
            return Step::Worked;
        }
        let now = self.clock.mono_ms();
        let Some(i) = self.pick(now) else {
            return Step::Idle {
                until_mono_ms: self.next_wake(now),
            };
        };
        self.poll_block(t, i, emit)
    }

    /// Fail all queued jobs (worker shutdown). Nothing is replayed later.
    pub fn fail_pending(&mut self) {
        for job in self.commands.drain(..).chain(self.alarms.drain(..)) {
            fail_job(job);
        }
    }

    fn emit_stale(&mut self, emit: &mut dyn FnMut(Sample)) {
        let now_mono = self.clock.mono_ms();
        let now_utc = self.clock.utc_ms();
        let mut seq = self.seq;
        for d in &mut self.devices {
            for tag in &mut d.tags {
                if tag.needs_stale(now_mono, d.stale_ms) {
                    seq += 1;
                    let obs = Observation {
                        quality: Quality::Stale,
                        value: None,
                        raw: None,
                        error: Some("no new data within the stale threshold".into()),
                    };
                    emit(tag.observe(obs, seq, now_utc, now_mono));
                }
            }
        }
        self.seq = seq;
    }

    fn pick(&mut self, now: u64) -> Option<usize> {
        let n = self.devices.len();
        let mut best: Option<(u64, usize, usize)> = None;
        for (i, d) in self.devices.iter().enumerate() {
            if d.blocks.is_empty() {
                continue;
            }
            let key = match d.cycle {
                Some(c) => c.due,
                None if d.next_due <= now => d.next_due,
                None => continue,
            };
            let rank = (i + n - self.rr % n.max(1)) % n.max(1);
            if best.is_none_or(|(k, r, _)| (key, rank) < (k, r)) {
                best = Some((key, rank, i));
            }
        }
        let (_, _, i) = best?;
        let d = &mut self.devices[i];
        if d.cycle.is_none() {
            d.cycle = Some(Cycle {
                due: d.next_due,
                block: 0,
                retries_used: 0,
            });
            if let Some(prev) = d.last_cycle_start {
                d.counters.cycle_ms = Some(now.saturating_sub(prev));
            }
            d.last_cycle_start = Some(now);
            self.rr = (i + 1) % n.max(1);
        }
        Some(i)
    }

    fn next_wake(&self, now: u64) -> u64 {
        let mut wake = u64::MAX;
        for d in &self.devices {
            if !d.blocks.is_empty() {
                wake = wake.min(d.next_due.max(now));
            }
            for tag in &d.tags {
                if tag.last_quality() == Some(Quality::Good)
                    && let Some(age) = tag.age_ms(now)
                {
                    wake = wake.min(now + d.stale_ms.saturating_sub(age) + 1);
                }
            }
        }
        if wake == u64::MAX {
            now + 1000
        } else {
            wake.max(now + 1)
        }
    }

    /// Returns true if the link was lost.
    fn run_urgent(&mut self, t: &mut dyn Transport, job: UrgentJob) -> bool {
        match job {
            UrgentJob::Read {
                unit,
                request,
                timeout_ms,
                reply,
                ..
            } => {
                let rec = self.exchanger.transact(t, unit, Outgoing::Read(&request), timeout_ms);
                self.count_unit(unit, &rec);
                let lost = rec.outcome.as_ref().is_err_and(ExchangeError::is_link_lost);
                let result = rec
                    .outcome
                    .and_then(|pdu| decode_read_response(&request, &pdu).map_err(reply_error));
                reply(result);
                lost
            }
            UrgentJob::Command {
                frame,
                timeout_ms,
                reply,
            } => {
                // Exactly one transmission; whatever happens is reported, never retried.
                let rec = self
                    .exchanger
                    .transact(t, frame.unit(), Outgoing::Command(&frame), timeout_ms);
                self.count_unit(frame.unit(), &rec);
                let lost = rec.outcome.as_ref().is_err_and(ExchangeError::is_link_lost);
                reply(rec);
                lost
            }
        }
    }

    fn count_unit(&mut self, unit: u8, rec: &ExchangeRecord) {
        if let Some(d) = self.devices.iter_mut().find(|d| d.instance.unit_id == unit) {
            count(&mut d.counters, rec.outcome.as_ref().err());
        }
    }

    fn poll_block(&mut self, t: &mut dyn Transport, i: usize, emit: &mut dyn FnMut(Sample)) -> Step {
        let Some(cycle) = self.devices[i].cycle else {
            return Step::Worked;
        };
        let (unit, request, timeout) = {
            let d = &self.devices[i];
            (d.instance.unit_id, d.blocks[cycle.block].request, d.timeout_ms)
        };
        let rec = self.exchanger.transact(t, unit, Outgoing::Read(&request), timeout);
        let now_mono = self.clock.mono_ms();
        let now_utc = self.clock.utc_ms();
        count(&mut self.devices[i].counters, rec.outcome.as_ref().err());
        let result = rec
            .outcome
            .and_then(|pdu| decode_read_response(&request, &pdu).map_err(reply_error));
        match result {
            Ok(data) => {
                self.devices[i].counters.good += 1;
                self.emit_block(i, cycle.block, Ok(&data), now_utc, now_mono, emit);
                self.advance(i, now_mono, now_utc, emit);
            }
            Err(ExchangeError::Link(e)) => {
                self.go_down(emit, &e.to_string());
                return Step::LinkDown;
            }
            Err(e @ (ExchangeError::Timeout | ExchangeError::Crc(_))) => {
                let d = &mut self.devices[i];
                if d.failures == 0 && cycle.retries_used < d.instance.read_retries {
                    d.cycle = Some(Cycle {
                        retries_used: cycle.retries_used + 1,
                        ..cycle
                    });
                    return Step::Worked;
                }
                if matches!(e, ExchangeError::Timeout) {
                    self.abort_cycle(i, &e, now_utc, now_mono, emit);
                } else {
                    self.emit_block(i, cycle.block, Err(&e), now_utc, now_mono, emit);
                    self.advance(i, now_mono, now_utc, emit);
                }
            }
            Err(e) => {
                self.emit_block(i, cycle.block, Err(&e), now_utc, now_mono, emit);
                self.advance(i, now_mono, now_utc, emit);
            }
        }
        Step::Worked
    }

    fn emit_block(
        &mut self,
        i: usize,
        block: usize,
        result: Result<&ReadData, &ExchangeError>,
        now_utc: i64,
        now_mono: u64,
        emit: &mut dyn FnMut(Sample),
    ) {
        let tags = self.devices[i].blocks[block].tags.clone();
        for bt in tags {
            let obs = match result {
                Ok(data) => {
                    let words = data.as_words();
                    let start = usize::from(bt.offset);
                    let slice = words.get(start..start + usize::from(bt.width)).unwrap_or(&[]).to_vec();
                    decode_tag(&self.devices[i].profile, bt.register, slice)
                }
                Err(e) => Observation {
                    quality: e.quality(),
                    value: None,
                    raw: None,
                    error: Some(e.to_string()),
                },
            };
            if !obs.quality.is_good() {
                self.devices[i].counters.decode_errors +=
                    u64::from(obs.quality == Quality::DecodeError && result.is_ok());
            }
            let seq = self.next_seq();
            emit(self.devices[i].tags[bt.register].observe(obs, seq, now_utc, now_mono));
        }
    }

    fn advance(&mut self, i: usize, now_mono: u64, now_utc: i64, emit: &mut dyn FnMut(Sample)) {
        let d = &mut self.devices[i];
        let Some(c) = d.cycle else { return };
        if c.block + 1 < d.blocks.len() {
            d.cycle = Some(Cycle {
                block: c.block + 1,
                retries_used: 0,
                ..c
            });
            return;
        }
        d.cycle = None;
        d.failures = 0;
        d.counters.consecutive_failures = 0;
        d.counters.backoff_until_mono_ms = None;
        d.next_due = (c.due + d.instance.poll_interval_ms).max(now_mono);
        let unreadable = d.unreadable.clone();
        for r in unreadable {
            let obs = Observation {
                quality: Quality::DecodeError,
                value: None,
                raw: None,
                error: Some("tag is wider than the device read limit".into()),
            };
            let seq = self.next_seq();
            emit(self.devices[i].tags[r].observe(obs, seq, now_utc, now_mono));
        }
    }

    fn abort_cycle(&mut self, i: usize, e: &ExchangeError, now_utc: i64, now_mono: u64, emit: &mut dyn FnMut(Sample)) {
        let Some(c) = self.devices[i].cycle else { return };
        self.emit_block(i, c.block, Err(e), now_utc, now_mono, emit);
        let later: Vec<usize> = self.devices[i].blocks[c.block + 1..]
            .iter()
            .flat_map(|b| b.tags.iter().map(|t| t.register))
            .collect();
        for r in later {
            let obs = Observation {
                quality: e.quality(),
                value: None,
                raw: None,
                error: Some("not read: device did not respond earlier in this cycle".into()),
            };
            let seq = self.next_seq();
            emit(self.devices[i].tags[r].observe(obs, seq, now_utc, now_mono));
        }
        let cap = self.backoff_cap_ms;
        let d = &mut self.devices[i];
        d.cycle = None;
        d.failures = d.failures.saturating_add(1);
        let factor = 1u64 << (d.failures - 1).min(20);
        let delay = d.instance.poll_interval_ms.saturating_mul(factor).min(cap);
        d.next_due = now_mono + delay;
        d.counters.consecutive_failures = d.failures;
        d.counters.backoff_until_mono_ms = Some(d.next_due);
    }

    fn go_down(&mut self, emit: &mut dyn FnMut(Sample), why: &str) {
        self.down = true;
        let now_mono = self.clock.mono_ms();
        let now_utc = self.clock.utc_ms();
        let mut seq = self.seq;
        for d in &mut self.devices {
            d.cycle = None;
            d.counters.last_error = Some(why.to_owned());
            for tag in &mut d.tags {
                seq += 1;
                let obs = Observation {
                    quality: Quality::Disconnected,
                    value: None,
                    raw: None,
                    error: Some(why.to_owned()),
                };
                emit(tag.observe(obs, seq, now_utc, now_mono));
            }
        }
        self.seq = seq;
        self.fail_pending();
    }
}

fn count(c: &mut DeviceCounters, err: Option<&ExchangeError>) {
    c.requests += 1;
    match err {
        None => {}
        Some(e) => {
            match e {
                ExchangeError::Timeout => c.timeouts += 1,
                ExchangeError::Crc(_) => c.crc_errors += 1,
                ExchangeError::Exception(_) => c.exceptions += 1,
                ExchangeError::Decode(_) | ExchangeError::Encode(_) => c.decode_errors += 1,
                ExchangeError::Link(_) => {}
            }
            c.last_error = Some(e.to_string());
        }
    }
}

/// Fail a job that will never run. Unsent command frames come back with an
/// empty request, which the command service reports as "not sent".
fn fail_job(job: UrgentJob) {
    let lost = || ExchangeError::Link(TransportError::Disconnected("link is down".into()));
    match job {
        UrgentJob::Read { reply, .. } => reply(Err(lost())),
        UrgentJob::Command { frame, reply, .. } => reply(ExchangeRecord {
            unit: frame.unit(),
            request: Vec::new(),
            response: Vec::new(),
            started_mono_ms: 0,
            elapsed_ms: 0,
            discarded: 0,
            outcome: Err(lost()),
        }),
    }
}

/// Decode one tag's words into an observation (GOOD or DECODE_ERROR with the raw words kept).
pub fn decode_tag(profile: &Profile, register: usize, words: Vec<u16>) -> Observation {
    let Some(reg) = profile.registers.get(register) else {
        return Observation {
            quality: Quality::DecodeError,
            value: None,
            raw: Some(words),
            error: Some("no such tag".into()),
        };
    };
    let Some(dt) = reg.data_type() else {
        return Observation {
            quality: Quality::DecodeError,
            value: None,
            raw: Some(words),
            error: Some("bad type".into()),
        };
    };
    let decoded = codec::decode(dt, reg.word_order, &words)
        .and_then(|raw| codec::to_engineering(&raw, &reg.scaling()).map(|eng| (raw, eng)));
    match decoded {
        Ok((raw, engineering)) => {
            let text = match &raw {
                codec::RawValue::Str(s) => Some(s.clone()),
                _ => None,
            };
            Observation {
                quality: Quality::Good,
                value: Some(Value { raw, engineering, text }),
                raw: Some(words),
                error: None,
            }
        }
        Err(e) => Observation {
            quality: Quality::DecodeError,
            value: None,
            raw: Some(words),
            error: Some(e.to_string()),
        },
    }
}
