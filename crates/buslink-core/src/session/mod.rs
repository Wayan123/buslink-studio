//! Session service (app-shell-desktop design §2): links, one worker per link,
//! devices, the hub that feeds the live store, alarms and historian, plus
//! trend, export and inspector queries. Platform code supplies transports
//! through a factory, so this module never links serial drivers, Tauri or
//! the simulator.
//!
//! There is no write path here: monitoring only (safety rules 1–2). Control
//! goes through `crate::command` (epic E5).
//!
//! Lock order: `links` → `live` → `alarms` → `log` → `historian`.

pub mod frames;
pub mod live;

use crate::alarm::{AlarmDef, AlarmEngine, AlarmError, AlarmEvent, AlarmSnapshot};
use crate::bus::{BusRecv, SampleBus};
use crate::clock::Clock;
use crate::codec::{self, TypeName};
use crate::device::{DeviceError, DeviceInstance, LinkConfig, LinkKind};
use crate::export::{self, ExportDoc};
use crate::historian::batch::{LogDecimator, SampleBatcher};
use crate::historian::{DeviceRow, EventRow, Historian, SessionInfo};
use crate::poll::scheduler::{DeviceCounters, FrameTap, LinkScheduler, SchedulerError};
use crate::poll::worker::{LinkState as WorkerState, LinkWorker, SampleSink};
use crate::profile::{Profile, Text};
use crate::protocol::pdu::ExceptionCode;
use crate::quality::{Quality, Sample};
use crate::transport::{LinkRegistry, Transport, TransportError};
use crate::trend::{self, TrendInput, TrendSeries};
use frames::{FrameEntry, FrameLog};
use live::{LiveStore, TagView, TrendPoint, tag_key};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use thiserror::Error;

pub type TransportFactory = Arc<dyn Fn(&LinkConfig) -> Result<Box<dyn Transport>, TransportError> + Send + Sync>;

const HUB_PERIOD_MS: u64 = 50;
const FLUSH_PERIOD_MS: u64 = 1000;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum SessionError {
    #[error("unknown link '{0}'")]
    UnknownLink(String),
    #[error("unknown device '{0}'")]
    UnknownDevice(String),
    #[error("unknown profile '{0}'")]
    UnknownProfile(String),
    #[error("link id '{0}' already exists")]
    LinkExists(String),
    #[error("'{0}' is already in use by another link (one master per bus)")]
    BusOwned(String),
    #[error("device id '{0}' already exists")]
    DeviceExists(String),
    #[error("unit ID {0} is already used on this link")]
    UnitTaken(u8),
    #[error("link '{0}' is not connected")]
    NotConnected(String),
    #[error("{0}")]
    Device(DeviceError),
    #[error("cannot open link: {0}")]
    Transport(TransportError),
    #[error("invalid settings: {0}")]
    Settings(String),
    #[error("{0}")]
    Scheduler(SchedulerError),
    #[error("historian unavailable: {0}")]
    Historian(String),
    #[error("export failed: {0}")]
    Export(String),
    #[error("{0}")]
    Alarm(AlarmError),
    #[error("could not start the link worker: {0}")]
    Spawn(String),
}

#[derive(Clone, Debug)]
pub struct SessionConfig {
    /// `None` = in-memory historian (tests).
    pub db_path: Option<PathBuf>,
    pub app_version: String,
    pub platform: String,
    pub logging_interval_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// Port open, not polling.
    Connected,
    Polling,
    /// The transport failed; the user must reconnect.
    Down,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Xlsx,
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Json => "json",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub link_id: String,
    pub unit_id: u8,
    pub profile_id: String,
    pub profile_version: String,
    pub simulated_only: bool,
    pub poll_interval_ms: u64,
    pub stale_after_ms: u64,
    pub counters: DeviceCounters,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LinkView {
    pub id: String,
    /// `rtu`, `tcp`, `rtu_over_tcp`, `simulator`
    pub kind: &'static str,
    pub label: String,
    pub simulated: bool,
    pub state: LinkState,
    pub error: Option<String>,
    pub devices: Vec<DeviceView>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiveSnapshot {
    pub ts_ms: i64,
    pub session_id: String,
    pub tags: Vec<TagView>,
    pub links: Vec<LinkView>,
    pub alarms: AlarmSnapshot,
    pub historian_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiveFrame {
    pub ts_ms: i64,
    pub tags: Vec<TagView>,
    pub points: Vec<TrendPoint>,
    /// Points that could not be delivered (frame cap or bus lag). Never silent.
    pub dropped: u64,
    pub links: Option<Vec<LinkView>>,
    pub alarms: Option<AlarmSnapshot>,
    pub historian_error: Option<String>,
}

impl LiveFrame {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
            && self.points.is_empty()
            && self.dropped == 0
            && self.links.is_none()
            && self.alarms.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OrderValue {
    pub order: &'static str,
    pub value: Option<String>,
    pub error: Option<String>,
    pub is_profile: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InspectRow {
    pub tag_id: String,
    pub name: Text,
    pub table: &'static str,
    pub doc_address: String,
    pub pdu_offset: u16,
    pub function: u8,
    pub function_name: &'static str,
    pub type_name: TypeName,
    pub word_order: Option<&'static str>,
    pub width: u16,
    pub unit: Option<String>,
    pub raw: Option<Vec<u16>>,
    pub raw_hex: Option<String>,
    /// Decoded raw value in the profile's word order.
    pub decoded: Option<String>,
    pub value: Option<f64>,
    /// All four orders for 32/64-bit types.
    pub orders: Vec<OrderValue>,
    pub quality: Option<&'static str>,
    pub exception: Option<u8>,
    pub exception_name: Option<&'static str>,
    pub error: Option<String>,
    pub value_ts_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeviceInspection {
    pub device: DeviceView,
    pub rows: Vec<InspectRow>,
    pub frames: Vec<FrameEntry>,
}

struct LinkEntry {
    config: LinkConfig,
    simulated: bool,
    transport: Option<Arc<Mutex<Box<dyn Transport>>>>,
    devices: Vec<(DeviceInstance, Arc<Profile>)>,
    worker: Option<LinkWorker>,
    state: LinkState,
    error: Option<String>,
    counters: Vec<(String, DeviceCounters)>,
}

struct LogState {
    decimator: LogDecimator,
    batcher: SampleBatcher,
}

struct FrameCursor {
    alarm_rev: u64,
    links_rev: u64,
}

struct Shared {
    clock: Arc<dyn Clock>,
    factory: TransportFactory,
    registry: Arc<LinkRegistry>,
    bus: SampleBus,
    /// Global sequence; locked around publish so bus order equals sequence order.
    publish_lock: Arc<Mutex<u64>>,
    session_id: String,
    app_version: String,
    profiles: Mutex<BTreeMap<String, Arc<Profile>>>,
    links: Mutex<BTreeMap<String, LinkEntry>>,
    live: Mutex<LiveStore>,
    alarms: Mutex<AlarmEngine>,
    frames: Arc<Mutex<FrameLog>>,
    log: Mutex<LogState>,
    historian: Option<Mutex<Historian>>,
    historian_error: Mutex<Option<String>>,
    registered: Mutex<HashSet<String>>,
    links_rev: AtomicU64,
    cursor: Mutex<FrameCursor>,
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Transport handed to a worker: delegates to the link's open transport and
/// does not close it, so the worker can be restarted without reopening the port.
struct SharedTransport(Arc<Mutex<Box<dyn Transport>>>);

impl Transport for SharedTransport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        lock(&self.0).write_all(bytes)
    }
    fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        lock(&self.0).read(buf, timeout_ms)
    }
    fn drain(&mut self, quiet_ms: u64) -> Result<usize, TransportError> {
        lock(&self.0).drain(quiet_ms)
    }
    fn close(&mut self) {}
}

fn publish_with(lock_seq: &Mutex<u64>, bus: &SampleBus, mut s: Sample) {
    let mut seq = lock(lock_seq);
    *seq += 1;
    s.seq = *seq;
    bus.publish(s);
}

fn link_kind(c: &LinkConfig) -> &'static str {
    match c.kind {
        LinkKind::Rtu { .. } => "rtu",
        LinkKind::Tcp { .. } => "tcp",
        LinkKind::RtuOverTcp { .. } => "rtu_over_tcp",
        LinkKind::Simulator { .. } => "simulator",
    }
}

fn serial_label(s: &crate::protocol::rtu::SerialSettings) -> String {
    let p = match s.parity {
        crate::protocol::rtu::Parity::None => 'N',
        crate::protocol::rtu::Parity::Even => 'E',
        crate::protocol::rtu::Parity::Odd => 'O',
    };
    format!("{} {}{}{}", s.baud, s.data_bits, p, s.stop_bits)
}

fn link_label(c: &LinkConfig) -> String {
    match &c.kind {
        LinkKind::Rtu { port, serial } => format!("{port} {}", serial_label(serial)),
        LinkKind::Tcp { host, port } => format!("{host}:{port}"),
        LinkKind::RtuOverTcp { host, port, serial } => format!("{host}:{port} RTU {}", serial_label(serial)),
        LinkKind::Simulator { name, serial } => format!("{name} {}", serial_label(serial)),
    }
}

impl Shared {
    fn now(&self) -> i64 {
        self.clock.utc_ms()
    }

    fn set_historian_error(&self, e: impl ToString) {
        *lock(&self.historian_error) = Some(e.to_string());
    }

    fn event(&self, kind: &str, device: Option<&str>, tag: Option<&str>, severity: &str, text: String) {
        let Some(h) = &self.historian else { return };
        let row = EventRow {
            session: Some(self.session_id.clone()),
            ts_ms: self.now(),
            kind: kind.to_owned(),
            device: device.map(str::to_owned),
            tag: tag.map(str::to_owned),
            severity: severity.to_owned(),
            text,
        };
        if let Err(e) = lock(h).add_event(&row) {
            self.set_historian_error(e);
        }
    }

    fn alarm_events_to_historian(&self, events: Vec<AlarmEvent>) {
        let Some(h) = &self.historian else { return };
        let mut h = lock(h);
        for ev in events {
            let row = EventRow {
                session: Some(self.session_id.clone()),
                ts_ms: ev.ts_ms,
                kind: "alarm".into(),
                device: Some(ev.device_id),
                tag: ev.tag_id,
                severity: format!("P{}", ev.priority),
                text: format!("{}: {}", ev.event, ev.text),
            };
            if let Err(e) = h.add_event(&row) {
                drop(h);
                self.set_historian_error(e);
                return;
            }
        }
    }

    fn store_batch(&self, batch: Vec<Sample>) {
        if batch.is_empty() {
            return;
        }
        let Some(h) = &self.historian else { return };
        if let Err(e) = lock(h).insert_samples(&batch) {
            self.set_historian_error(e);
        }
    }

    fn ingest(&self, s: &Arc<Sample>) {
        let now = self.now();
        lock(&self.live).apply(s);
        lock(&self.alarms).observe(s, now);
        let batch = {
            let mut log = lock(&self.log);
            if log.decimator.keep(s) {
                log.batcher.push((**s).clone(), now)
            } else {
                None
            }
        };
        if let Some(b) = batch {
            self.store_batch(b);
        }
    }

    fn flush_log(&self) {
        let batch = lock(&self.log).batcher.flush();
        self.store_batch(batch);
    }

    fn sink(&self) -> SampleSink {
        let bus = self.bus.clone();
        let seq = Arc::clone(&self.publish_lock);
        Arc::new(move |s: Sample| publish_with(&seq, &bus, s))
    }

    fn tap(&self, link_id: &str) -> FrameTap {
        let frames = Arc::clone(&self.frames);
        let clock = Arc::clone(&self.clock);
        let link = link_id.to_owned();
        Arc::new(move |rec| lock(&frames).push(&link, rec, clock.utc_ms()))
    }

    fn bump_links(&self) {
        self.links_rev.fetch_add(1, Ordering::SeqCst);
    }

    fn spawn_worker(&self, e: &mut LinkEntry) -> Result<(), SessionError> {
        let transport = e
            .transport
            .clone()
            .ok_or_else(|| SessionError::NotConnected(e.config.id.clone()))?;
        let mut sched = LinkScheduler::new(e.config.clone(), e.devices.clone(), Arc::clone(&self.clock))
            .map_err(SessionError::Scheduler)?;
        sched.set_frame_tap(self.tap(&e.config.id));
        let key = e.config.owner_key();
        let claim = self.registry.claim(&key).map_err(|_| SessionError::BusOwned(key))?;
        let worker = LinkWorker::spawn(claim, sched, Box::new(SharedTransport(transport)), self.sink())
            .map_err(|err| SessionError::Spawn(err.to_string()))?;
        e.worker = Some(worker);
        e.state = LinkState::Polling;
        e.error = None;
        self.bump_links();
        Ok(())
    }

    fn stop_worker(&self, e: &mut LinkEntry) {
        if let Some(w) = e.worker.take() {
            let st = w.stop();
            e.counters = st.counters;
        }
    }

    fn device_view(&self, e: &LinkEntry, d: &DeviceInstance, p: &Profile) -> DeviceView {
        let live = e.worker.as_ref().map(|w| w.status().counters);
        let counters = live
            .as_ref()
            .unwrap_or(&e.counters)
            .iter()
            .find(|(id, _)| *id == d.id)
            .map(|(_, c)| c.clone())
            .unwrap_or_default();
        DeviceView {
            id: d.id.clone(),
            name: d.name.clone(),
            link_id: d.link_id.clone(),
            unit_id: d.unit_id,
            profile_id: d.profile_id.clone(),
            profile_version: d.profile_version.clone(),
            simulated_only: p.simulated_only,
            poll_interval_ms: d.poll_interval_ms,
            stale_after_ms: d.stale_threshold_ms(),
            counters,
        }
    }

    fn link_views(&self, links: &BTreeMap<String, LinkEntry>) -> Vec<LinkView> {
        links
            .values()
            .map(|e| LinkView {
                id: e.config.id.clone(),
                kind: link_kind(&e.config),
                label: link_label(&e.config),
                simulated: e.simulated,
                state: e.state,
                error: e.error.clone(),
                devices: e.devices.iter().map(|(d, p)| self.device_view(e, d, p)).collect(),
            })
            .collect()
    }

    /// Hub: a worker that died with its transport → link Down (never restarted here).
    fn check_links(&self) {
        let mut down = Vec::new();
        {
            let mut links = lock(&self.links);
            for e in links.values_mut() {
                let finished = e.worker.as_ref().is_some_and(|w| w.is_finished());
                if !finished {
                    continue;
                }
                if let Some(w) = e.worker.take() {
                    let st = w.stop();
                    e.counters = st.counters.clone();
                    if st.state == WorkerState::Down {
                        let why = st
                            .counters
                            .iter()
                            .find_map(|(_, c)| c.last_error.clone())
                            .unwrap_or_else(|| "transport lost".into());
                        e.state = LinkState::Down;
                        e.error = Some(why.clone());
                        if let Some(t) = e.transport.take() {
                            lock(&t).close();
                        }
                        down.push((e.config.id.clone(), why));
                    } else {
                        e.state = LinkState::Connected;
                    }
                    self.bump_links();
                }
            }
        }
        for (id, why) in down {
            self.event("link", None, None, "warning", format!("link {id} down: {why}"));
        }
    }
}

fn hub(shared: Arc<Shared>, mut sub: crate::bus::BusSubscriber) {
    let mut last_flush = shared.clock.mono_ms();
    loop {
        let stopping = shared.stop.load(Ordering::SeqCst);
        for r in sub.drain() {
            match r {
                BusRecv::Sample(s) => shared.ingest(&s),
                BusRecv::Lagged(n) => lock(&shared.live).add_dropped(n),
                BusRecv::Empty | BusRecv::Closed => {}
            }
        }
        let now = shared.now();
        let events = {
            let mut a = lock(&shared.alarms);
            a.tick(now);
            a.take_events()
        };
        shared.alarm_events_to_historian(events);
        let mono = shared.clock.mono_ms();
        if stopping || mono.saturating_sub(last_flush) >= FLUSH_PERIOD_MS {
            last_flush = mono;
            shared.flush_log();
            shared.check_links();
        }
        if stopping {
            break;
        }
        std::thread::sleep(Duration::from_millis(HUB_PERIOD_MS));
    }
}

pub struct Session {
    shared: Arc<Shared>,
    hub: Mutex<Option<JoinHandle<()>>>,
}

impl Session {
    pub fn start(
        cfg: SessionConfig,
        clock: Arc<dyn Clock>,
        factory: TransportFactory,
    ) -> Result<Session, SessionError> {
        let now = clock.utc_ms();
        let session_id = format!("s{now}");
        let (historian, historian_error) = {
            let opened = match &cfg.db_path {
                Some(p) => Historian::open(p),
                None => Historian::open_in_memory(),
            };
            match opened.and_then(|mut h| {
                h.begin_session(
                    &session_id,
                    now,
                    &cfg.app_version,
                    &cfg.platform,
                    cfg.logging_interval_ms,
                )?;
                Ok(h)
            }) {
                Ok(h) => (Some(Mutex::new(h)), None),
                Err(e) => (None, Some(e.to_string())),
            }
        };
        let bus = SampleBus::default();
        let sub = bus.subscribe();
        let shared = Arc::new(Shared {
            clock,
            factory,
            registry: LinkRegistry::new(),
            bus,
            publish_lock: Arc::new(Mutex::new(0)),
            session_id,
            app_version: cfg.app_version.clone(),
            profiles: Mutex::new(BTreeMap::new()),
            links: Mutex::new(BTreeMap::new()),
            live: Mutex::new(LiveStore::default()),
            alarms: Mutex::new(AlarmEngine::new()),
            frames: Arc::new(Mutex::new(FrameLog::default())),
            log: Mutex::new(LogState {
                decimator: LogDecimator::new(cfg.logging_interval_ms),
                batcher: SampleBatcher::new(FLUSH_PERIOD_MS, 2000),
            }),
            historian,
            historian_error: Mutex::new(historian_error),
            registered: Mutex::new(HashSet::new()),
            links_rev: AtomicU64::new(1),
            cursor: Mutex::new(FrameCursor {
                alarm_rev: u64::MAX,
                links_rev: 0,
            }),
            stop: AtomicBool::new(false),
        });
        let hub_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("buslink-hub".into())
            .spawn(move || hub(hub_shared, sub))
            .map_err(|e| SessionError::Spawn(e.to_string()))?;
        Ok(Session {
            shared,
            hub: Mutex::new(Some(handle)),
        })
    }

    pub fn session_id(&self) -> &str {
        &self.shared.session_id
    }

    /// A subscriber for consumers outside the session (tests, LAN, MQTT).
    pub fn subscribe(&self) -> crate::bus::BusSubscriber {
        self.shared.bus.subscribe()
    }

    // ---- profiles ----

    pub fn add_profile(&self, p: Profile) -> String {
        let key = p.key();
        lock(&self.shared.profiles).insert(key.clone(), Arc::new(p));
        key
    }

    pub fn profiles(&self) -> Vec<Arc<Profile>> {
        lock(&self.shared.profiles).values().cloned().collect()
    }

    pub fn profile(&self, key: &str) -> Option<Arc<Profile>> {
        lock(&self.shared.profiles).get(key).cloned()
    }

    // ---- links ----

    /// Open a new link (Monitoring: nothing is polled until `start_polling`).
    pub fn connect(&self, config: LinkConfig, simulated: bool) -> Result<(), SessionError> {
        if let Some(s) = config.serial() {
            s.validate().map_err(|e| SessionError::Settings(e.into()))?;
        }
        let id_ok = (1..=48).contains(&config.id.len())
            && config
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_'));
        if !id_ok {
            return Err(SessionError::Settings(
                "link id must be 1-48 of a-z, 0-9, '-', '_'".into(),
            ));
        }
        if simulated != config.is_simulator() {
            return Err(SessionError::Settings("only simulator links can be simulated".into()));
        }
        let key = config.owner_key();
        let mut links = lock(&self.shared.links);
        if links.contains_key(&config.id) {
            return Err(SessionError::LinkExists(config.id));
        }
        let open_with_key = links
            .values()
            .any(|e| e.transport.is_some() && e.config.owner_key() == key);
        if open_with_key || self.shared.registry.is_owned(&key) {
            return Err(SessionError::BusOwned(key));
        }
        let transport = (self.shared.factory)(&config).map_err(SessionError::Transport)?;
        let id = config.id.clone();
        links.insert(
            id.clone(),
            LinkEntry {
                config,
                simulated,
                transport: Some(Arc::new(Mutex::new(transport))),
                devices: Vec::new(),
                worker: None,
                state: LinkState::Connected,
                error: None,
                counters: Vec::new(),
            },
        );
        drop(links);
        self.shared.bump_links();
        self.shared
            .event("link", None, None, "info", format!("link {id} connected"));
        Ok(())
    }

    /// Reopen a disconnected or failed link with its stored settings. It comes
    /// back in Monitoring (connected, not polling): reconnect never restarts anything.
    pub fn reconnect(&self, link_id: &str) -> Result<(), SessionError> {
        let mut links = lock(&self.shared.links);
        let key = {
            let e = links
                .get(link_id)
                .ok_or_else(|| SessionError::UnknownLink(link_id.into()))?;
            if e.transport.is_some() {
                return Ok(());
            }
            e.config.owner_key()
        };
        let busy = links
            .values()
            .any(|o| o.config.id != link_id && o.transport.is_some() && o.config.owner_key() == key);
        if busy || self.shared.registry.is_owned(&key) {
            return Err(SessionError::BusOwned(key));
        }
        let Some(e) = links.get_mut(link_id) else {
            return Err(SessionError::UnknownLink(link_id.into()));
        };
        let t = (self.shared.factory)(&e.config).map_err(SessionError::Transport)?;
        e.transport = Some(Arc::new(Mutex::new(t)));
        e.state = LinkState::Connected;
        e.error = None;
        drop(links);
        self.shared.bump_links();
        self.shared
            .event("link", None, None, "info", format!("link {link_id} reconnected"));
        Ok(())
    }

    /// Stop polling, close the port and mark every tag of the link DISCONNECTED
    /// (last good value and its time kept).
    pub fn disconnect(&self, link_id: &str) -> Result<(), SessionError> {
        {
            let mut links = lock(&self.shared.links);
            let e = links
                .get_mut(link_id)
                .ok_or_else(|| SessionError::UnknownLink(link_id.into()))?;
            self.shared.stop_worker(e);
            if let Some(t) = e.transport.take() {
                lock(&t).close();
            }
            e.state = LinkState::Disconnected;
        }
        self.shared.bump_links();
        self.publish_disconnected(link_id, "disconnected by the user");
        self.shared
            .event("link", None, None, "info", format!("link {link_id} disconnected"));
        Ok(())
    }

    /// Disconnect (if needed) and forget a link and its devices.
    pub fn remove_link(&self, link_id: &str) -> Result<(), SessionError> {
        self.disconnect(link_id)?;
        let removed = lock(&self.shared.links).remove(link_id);
        if let Some(e) = removed {
            let mut live = lock(&self.shared.live);
            let mut alarms = lock(&self.shared.alarms);
            for (d, _) in &e.devices {
                live.remove_device(&d.id);
                alarms.remove_device(&d.id);
            }
        }
        lock(&self.shared.frames).clear_link(link_id);
        self.shared.bump_links();
        Ok(())
    }

    fn publish_disconnected(&self, link_id: &str, why: &str) {
        let now = self.shared.now();
        let last = lock(&self.shared.live).last_samples_of_link(link_id);
        for s in last {
            if s.quality == Quality::Disconnected {
                continue;
            }
            let mut d = (*s).clone();
            d.quality = Quality::Disconnected;
            d.ts_ms = now;
            d.age_ms = d.value_ts_ms.map(|t| u64::try_from(now.saturating_sub(t)).unwrap_or(0));
            d.raw = None;
            d.error = Some(why.to_owned());
            publish_with(&self.shared.publish_lock, &self.shared.bus, d);
        }
    }

    pub fn start_polling(&self, link_id: &str) -> Result<(), SessionError> {
        let mut links = lock(&self.shared.links);
        let e = links
            .get_mut(link_id)
            .ok_or_else(|| SessionError::UnknownLink(link_id.into()))?;
        match e.state {
            LinkState::Polling => Ok(()),
            LinkState::Connected => self.shared.spawn_worker(e),
            LinkState::Down | LinkState::Disconnected => Err(SessionError::NotConnected(link_id.into())),
        }
    }

    pub fn stop_polling(&self, link_id: &str) -> Result<(), SessionError> {
        let mut links = lock(&self.shared.links);
        let e = links
            .get_mut(link_id)
            .ok_or_else(|| SessionError::UnknownLink(link_id.into()))?;
        if e.state == LinkState::Polling {
            self.shared.stop_worker(e);
            e.state = LinkState::Connected;
            self.shared.bump_links();
        }
        Ok(())
    }

    // ---- devices ----

    /// A free device id derived from the profile and unit ID.
    pub fn suggest_device_id(&self, profile_id: &str, unit: u8) -> String {
        let base: String = profile_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let base = format!("{}-u{unit}", base.trim_matches('-'));
        let taken: HashSet<String> = lock(&self.shared.links)
            .values()
            .flat_map(|e| e.devices.iter().map(|(d, _)| d.id.clone()))
            .collect();
        if !taken.contains(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|c| !taken.contains(c))
            .unwrap_or(base)
    }

    pub fn add_device(&self, d: DeviceInstance) -> Result<(), SessionError> {
        let key = format!("{}@{}", d.profile_id, d.profile_version);
        let profile = self
            .profile(&key)
            .ok_or_else(|| SessionError::UnknownProfile(key.clone()))?;
        let mut links = lock(&self.shared.links);
        if links.values().any(|e| e.devices.iter().any(|(x, _)| x.id == d.id)) {
            return Err(SessionError::DeviceExists(d.id));
        }
        let e = links
            .get_mut(&d.link_id)
            .ok_or_else(|| SessionError::UnknownLink(d.link_id.clone()))?;
        d.check(&profile, &e.config).map_err(SessionError::Device)?;
        if e.devices.iter().any(|(x, _)| x.unit_id == d.unit_id) {
            return Err(SessionError::UnitTaken(d.unit_id));
        }
        e.devices.push((d.clone(), Arc::clone(&profile)));
        if e.state == LinkState::Polling {
            self.shared.stop_worker(e);
            if let Err(err) = self.shared.spawn_worker(e) {
                e.devices.pop();
                e.state = LinkState::Connected;
                let _ = self.shared.spawn_worker(e);
                return Err(err);
            }
        }
        let link = link_label(&e.config);
        drop(links);
        self.shared.bump_links();
        self.register_device(&d, &link);
        Ok(())
    }

    fn register_device(&self, d: &DeviceInstance, link: &str) {
        let Some(h) = &self.shared.historian else { return };
        if !lock(&self.shared.registered).insert(d.id.clone()) {
            return;
        }
        let row = DeviceRow {
            session: self.shared.session_id.clone(),
            device_id: d.id.clone(),
            name: d.name.clone(),
            profile_id: d.profile_id.clone(),
            profile_version: d.profile_version.clone(),
            unit_id: d.unit_id,
            link: link.to_owned(),
        };
        if let Err(e) = lock(h).register_device(&row) {
            self.shared.set_historian_error(e);
        }
    }

    pub fn remove_device(&self, device_id: &str) -> Result<(), SessionError> {
        let mut links = lock(&self.shared.links);
        let e = links
            .values_mut()
            .find(|e| e.devices.iter().any(|(d, _)| d.id == device_id))
            .ok_or_else(|| SessionError::UnknownDevice(device_id.into()))?;
        e.devices.retain(|(d, _)| d.id != device_id);
        if e.state == LinkState::Polling {
            self.shared.stop_worker(e);
            self.shared.spawn_worker(e)?;
        }
        drop(links);
        lock(&self.shared.live).remove_device(device_id);
        lock(&self.shared.alarms).remove_device(device_id);
        self.shared.bump_links();
        Ok(())
    }

    fn find_device(&self, device_id: &str) -> Result<(DeviceInstance, Arc<Profile>, DeviceView), SessionError> {
        let links = lock(&self.shared.links);
        for e in links.values() {
            if let Some((d, p)) = e.devices.iter().find(|(d, _)| d.id == device_id) {
                let view = self.shared.device_view(e, d, p);
                return Ok((d.clone(), Arc::clone(p), view));
            }
        }
        Err(SessionError::UnknownDevice(device_id.into()))
    }

    // ---- live ----

    pub fn links(&self) -> Vec<LinkView> {
        let links = lock(&self.shared.links);
        self.shared.link_views(&links)
    }

    pub fn link_config(&self, link_id: &str) -> Option<LinkConfig> {
        lock(&self.shared.links).get(link_id).map(|e| e.config.clone())
    }

    pub fn snapshot(&self) -> LiveSnapshot {
        let now = self.shared.now();
        let links = self.links();
        LiveSnapshot {
            ts_ms: now,
            session_id: self.shared.session_id.clone(),
            tags: lock(&self.shared.live).all(),
            links,
            alarms: lock(&self.shared.alarms).snapshot(now),
            historian_error: lock(&self.shared.historian_error).clone(),
        }
    }

    /// Changes since the last call (the shell calls this at ≤ 4 Hz).
    pub fn take_frame(&self) -> LiveFrame {
        let now = self.shared.now();
        let links_rev = self.shared.links_rev.load(Ordering::SeqCst);
        let alarm_rev = lock(&self.shared.alarms).revision();
        let (send_links, send_alarms) = {
            let mut c = lock(&self.shared.cursor);
            let l = c.links_rev != links_rev;
            let a = c.alarm_rev != alarm_rev;
            c.links_rev = links_rev;
            c.alarm_rev = alarm_rev;
            (l, a)
        };
        let links = send_links.then(|| self.links());
        let (tags, points, dropped) = lock(&self.shared.live).take_changes();
        let alarms = send_alarms.then(|| lock(&self.shared.alarms).snapshot(now));
        LiveFrame {
            ts_ms: now,
            tags,
            points,
            dropped,
            links,
            alarms,
            historian_error: lock(&self.shared.historian_error).clone(),
        }
    }

    /// Link views with fresh counters (heartbeat); counters change without a link revision.
    pub fn link_status(&self) -> Vec<LinkView> {
        self.links()
    }

    // ---- historian ----

    pub fn flush(&self) {
        self.shared.flush_log();
    }

    pub fn sessions(&self) -> Result<Vec<SessionInfo>, SessionError> {
        let h = self
            .shared
            .historian
            .as_ref()
            .ok_or_else(|| SessionError::Historian(self.historian_error_text()))?;
        lock(h).sessions().map_err(|e| SessionError::Historian(e.to_string()))
    }

    fn historian_error_text(&self) -> String {
        lock(&self.shared.historian_error)
            .clone()
            .unwrap_or_else(|| "not opened".into())
    }

    pub fn trend(
        &self,
        device_id: &str,
        tag_id: &str,
        from_ms: i64,
        to_ms: i64,
        buckets: usize,
    ) -> Result<TrendSeries, SessionError> {
        let (d, _, _) = self.find_device(device_id)?;
        self.shared.flush_log();
        let h = self
            .shared
            .historian
            .as_ref()
            .ok_or_else(|| SessionError::Historian(self.historian_error_text()))?;
        let rows = lock(h)
            .samples(&self.shared.session_id, Some(device_id), Some(tag_id), from_ms, to_ms)
            .map_err(|e| SessionError::Historian(e.to_string()))?;
        let input: Vec<TrendInput> = rows
            .iter()
            .map(|s| TrendInput {
                ts_ms: s.ts_ms,
                value: s.value,
                quality: s.quality,
            })
            .collect();
        Ok(trend::decimate(&input, from_ms, to_ms, buckets, d.stale_threshold_ms()))
    }

    pub fn export(&self, format: ExportFormat) -> Result<Vec<u8>, SessionError> {
        self.shared.flush_log();
        let h = self
            .shared
            .historian
            .as_ref()
            .ok_or_else(|| SessionError::Historian(self.historian_error_text()))?;
        let doc = ExportDoc::from_historian(
            &lock(h),
            &self.shared.session_id,
            self.shared.now(),
            &self.shared.app_version,
        )
        .map_err(|e| SessionError::Export(e.to_string()))?;
        match format {
            ExportFormat::Csv => Ok(export::csv::measurements(&doc).into_bytes()),
            ExportFormat::Json => Ok(export::json::to_string(&doc).into_bytes()),
            ExportFormat::Xlsx => export::xlsx::workbook(&doc).map_err(|e| SessionError::Export(e.to_string())),
        }
    }

    pub fn events(&self, kind: Option<&str>, limit: usize) -> Result<Vec<EventRow>, SessionError> {
        let h = self
            .shared
            .historian
            .as_ref()
            .ok_or_else(|| SessionError::Historian(self.historian_error_text()))?;
        let mut rows = lock(h)
            .events(&self.shared.session_id)
            .map_err(|e| SessionError::Historian(e.to_string()))?;
        if let Some(k) = kind {
            rows.retain(|r| r.kind == k);
        }
        let skip = rows.len().saturating_sub(limit);
        Ok(rows.split_off(skip))
    }

    // ---- inspector ----

    pub fn inspect(&self, device_id: &str) -> Result<DeviceInspection, SessionError> {
        let (d, p, view) = self.find_device(device_id)?;
        let live = lock(&self.shared.live);
        let rows = p
            .registers
            .iter()
            .map(|r| {
                let key = tag_key(&d.id, &r.id);
                let tv = live.get(&key);
                let raw = live.last_raw(&key).map(<[u16]>::to_vec);
                let dt = r.data_type();
                let decoded = match (&raw, dt) {
                    (Some(words), Some(dt)) => codec::decode(dt, r.word_order, words).ok().map(|v| v.display()),
                    _ => None,
                };
                let orders = match (&raw, dt) {
                    (Some(words), Some(dt)) if r.type_name.needs_word_order() => codec::decode_all_orders(dt, words)
                        .into_iter()
                        .map(|(o, res)| OrderValue {
                            order: o.as_str(),
                            is_profile: r.word_order == Some(o),
                            value: res.as_ref().ok().map(|v| v.display()),
                            error: res.err().map(|e| e.to_string()),
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let f = r.table.read_function();
                InspectRow {
                    tag_id: r.id.clone(),
                    name: r.name.clone(),
                    table: r.table.as_str(),
                    doc_address: r.doc_label(),
                    pdu_offset: r.address,
                    function: f.code(),
                    function_name: f.name(),
                    type_name: r.type_name,
                    word_order: r.word_order.map(|o| o.as_str()),
                    width: r.width(),
                    unit: r.unit.clone(),
                    raw_hex: raw.as_deref().map(export::raw_hex),
                    raw,
                    decoded,
                    value: tv.and_then(|t| t.value),
                    orders,
                    quality: tv.map(|t| t.quality),
                    exception: tv.and_then(|t| t.exception),
                    exception_name: tv.and_then(|t| t.exception).map(|c| ExceptionCode(c).name()),
                    error: tv.and_then(|t| t.error.clone()),
                    value_ts_ms: tv.and_then(|t| t.value_ts_ms),
                }
            })
            .collect();
        drop(live);
        let frames = lock(&self.shared.frames).of(&d.link_id, d.unit_id);
        Ok(DeviceInspection {
            device: view,
            rows,
            frames,
        })
    }

    // ---- alarms ----

    pub fn alarm_add(&self, def: AlarmDef) -> Result<(), SessionError> {
        self.find_device(&def.device_id)?;
        let now = self.shared.now();
        lock(&self.shared.alarms).add(def, now).map_err(SessionError::Alarm)
    }

    pub fn alarm_remove(&self, id: &str) -> Result<(), SessionError> {
        lock(&self.shared.alarms).remove(id).map_err(SessionError::Alarm)
    }

    pub fn alarm_ack(&self, id: &str) -> Result<(), SessionError> {
        let now = self.shared.now();
        lock(&self.shared.alarms).ack(id, now).map_err(SessionError::Alarm)
    }

    pub fn alarm_shelve(&self, id: &str, duration_ms: u64) -> Result<(), SessionError> {
        let now = self.shared.now();
        let until = now.saturating_add(i64::try_from(duration_ms).unwrap_or(i64::MAX));
        lock(&self.shared.alarms)
            .shelve(id, until, now)
            .map_err(SessionError::Alarm)
    }

    pub fn alarm_unshelve(&self, id: &str) -> Result<(), SessionError> {
        let now = self.shared.now();
        lock(&self.shared.alarms).unshelve(id, now).map_err(SessionError::Alarm)
    }

    pub fn alarm_set_out_of_service(&self, id: &str, oos: bool) -> Result<(), SessionError> {
        let now = self.shared.now();
        lock(&self.shared.alarms)
            .set_out_of_service(id, oos, now)
            .map_err(SessionError::Alarm)
    }

    pub fn alarms(&self) -> AlarmSnapshot {
        let now = self.shared.now();
        lock(&self.shared.alarms).snapshot(now)
    }

    pub fn alarm_defs(&self) -> Vec<AlarmDef> {
        lock(&self.shared.alarms).defs()
    }

    /// Samples applied to the live store so far (diagnostics and tests).
    pub fn applied_samples(&self) -> u64 {
        lock(&self.shared.live).applied()
    }

    /// Stop every worker, flush the historian and end the hub thread.
    pub fn shutdown(&self) {
        {
            let mut links = lock(&self.shared.links);
            for e in links.values_mut() {
                self.shared.stop_worker(e);
                if let Some(t) = e.transport.take() {
                    lock(&t).close();
                }
                e.state = LinkState::Disconnected;
            }
        }
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(h) = lock(&self.hub).take() {
            let _ = h.join();
        }
        self.shared.flush_log();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}
