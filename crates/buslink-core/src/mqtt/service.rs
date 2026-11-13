//! MQTT 5 telemetry publisher (PRD R23, architecture D7, safety rule 16).
//!
//! bus ─► feeder thread (throttle, topic, payload) ─► SQLite outbox ─► sender task ─► broker
//!
//! The outbox is the only source of what is sent. Each connection attempt builds
//! a fresh rumqttc client with a clean start, so nothing is resent behind our back:
//! unacknowledged rows are simply sent again from the outbox, in order, with their
//! original `ts` and `seq`. This service never subscribes to anything.

use super::config::{Broker, MqttConfig};
use super::outbox::{NewRow, Outbox, Row};
use super::payload;
use super::secrets::{Secret, redact};
use super::{MqttError, tls};
use crate::bus::{BusRecv, BusSubscriber};
use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::mqttbytes::v5::{ConnectReturnCode, LastWill, Packet, PubAckReason, PublishProperties};
use rumqttc::v5::{AsyncClient, Event, EventLoop, MqttOptions};
use rumqttc::{Outgoing, TlsConfiguration, Transport};
use rustls::ClientConfig;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::JoinHandle;

const WINDOW: usize = 10;
const REQUEST_CAPACITY: usize = 64;
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT_S: u64 = 10;
const FEED_PERIOD: Duration = Duration::from_millis(25);
const FEED_BATCH: usize = 500;
const TEST_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MqttState {
    Off,
    Connecting,
    Online,
    Backoff,
}

#[derive(Clone, Debug, Serialize)]
pub struct MqttStatus {
    pub state: MqttState,
    pub broker: String,
    pub client_id: String,
    pub outbox_depth: usize,
    pub outbox_max: usize,
    /// Messages stored for publishing.
    pub enqueued: u64,
    /// Messages the broker acknowledged (PUBACK).
    pub published: u64,
    /// Oldest messages removed because the outbox was full.
    pub dropped_overflow: u64,
    /// Messages dropped because their expiry passed before they could be sent.
    pub expired: u64,
    /// Samples skipped by the per-tag minimum interval.
    pub throttled: u64,
    /// Samples the sample bus overwrote before this publisher read them.
    pub lagged: u64,
    /// Messages the broker refused (negative PUBACK).
    pub rejected: u64,
    pub in_flight: u64,
    pub reconnects: u64,
    pub last_error: Option<String>,
    pub last_ack_ms: Option<i64>,
}

impl MqttStatus {
    pub fn off() -> Self {
        Self {
            state: MqttState::Off,
            broker: String::new(),
            client_id: String::new(),
            outbox_depth: 0,
            outbox_max: 0,
            enqueued: 0,
            published: 0,
            dropped_overflow: 0,
            expired: 0,
            throttled: 0,
            lagged: 0,
            rejected: 0,
            in_flight: 0,
            reconnects: 0,
            last_error: None,
            last_ack_ms: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TestReport {
    pub ok: bool,
    pub detail: String,
    pub elapsed_ms: u64,
}

#[derive(Default)]
struct Counters {
    enqueued: AtomicU64,
    published: AtomicU64,
    expired: AtomicU64,
    throttled: AtomicU64,
    lagged: AtomicU64,
    rejected: AtomicU64,
    in_flight: AtomicU64,
    reconnects: AtomicU64,
}

struct Info {
    state: MqttState,
    last_error: Option<String>,
    last_ack_ms: Option<i64>,
}

struct Shared {
    outbox: Mutex<Outbox>,
    notify: Notify,
    info: Mutex<Info>,
    c: Counters,
    stop_feeder: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl Shared {
    fn set_state(&self, s: MqttState) {
        lock(&self.info).state = s;
    }

    fn fail(&self, msg: String) {
        lock(&self.info).last_error = Some(msg);
    }
}

pub struct MqttInit {
    pub cfg: MqttConfig,
    pub password: Option<Secret>,
    /// Goes into every payload; with `seq` and the gateway it is the dedupe key.
    pub session_id: String,
    /// `None` keeps the outbox in memory (tests).
    pub outbox_path: Option<PathBuf>,
    /// Override the TLS configuration (tests); default is [`tls::client_config`].
    pub tls: Option<Arc<ClientConfig>>,
}

pub struct MqttService {
    shared: Arc<Shared>,
    cfg: MqttConfig,
    broker: Broker,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    feeder: Option<std::thread::JoinHandle<()>>,
}

fn options(
    cfg: &MqttConfig,
    broker: &Broker,
    client_id: &str,
    password: Option<&Secret>,
    tls: Option<Arc<ClientConfig>>,
    will: bool,
) -> MqttOptions {
    let mut o = MqttOptions::new(client_id, broker.host.clone(), broker.port);
    o.set_keep_alive(Duration::from_secs(u64::from(cfg.keep_alive_s)));
    o.set_clean_start(true);
    o.set_connection_timeout(CONNECT_TIMEOUT_S);
    o.set_request_channel_capacity(REQUEST_CAPACITY);
    if let Some(t) = tls {
        // Always `TlsConfiguration::Rustls` with our own ClientConfig (verification in
        // rustls 0.23.46 / webpki 0.103.16). Never `Simple`: that is the only rumqttc
        // path that uses its pinned rustls-webpki 0.102.8 / rustls-pemfile (see
        // docs/security/dependency-audit.md).
        o.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(t)));
    }
    if !cfg.username.is_empty() {
        o.set_credentials(
            cfg.username.clone(),
            password.map(|p| p.expose().to_string()).unwrap_or_default(),
        );
    }
    if will {
        o.set_last_will(LastWill::new(
            cfg.status_topic(),
            "offline",
            QoS::AtLeastOnce,
            true,
            None,
        ));
    }
    o
}

fn resolve_tls(
    cfg: &MqttConfig,
    broker: &Broker,
    given: Option<Arc<ClientConfig>>,
) -> Result<Option<Arc<ClientConfig>>, MqttError> {
    if !broker.tls {
        return Ok(None);
    }
    Ok(Some(match given {
        Some(t) => t,
        None => tls::client_config(&cfg.ca_pem)?,
    }))
}

impl MqttService {
    pub async fn start(init: MqttInit, sub: BusSubscriber) -> Result<MqttService, MqttError> {
        let broker = init.cfg.validate()?;
        let tls = resolve_tls(&init.cfg, &broker, init.tls)?;
        let outbox = Outbox::open(init.outbox_path.as_deref(), init.cfg.outbox_max)?;
        let shared = Arc::new(Shared {
            outbox: Mutex::new(outbox),
            notify: Notify::new(),
            info: Mutex::new(Info {
                state: MqttState::Connecting,
                last_error: None,
                last_ack_ms: None,
            }),
            c: Counters::default(),
            stop_feeder: AtomicBool::new(false),
        });
        let (stop, stop_rx) = watch::channel(false);

        let feeder = {
            let (shared, cfg, session) = (Arc::clone(&shared), init.cfg.clone(), init.session_id.clone());
            std::thread::Builder::new()
                .name("buslink-mqtt-feed".into())
                .spawn(move || feed(shared, cfg, session, sub))
                .map_err(|e| MqttError::Spawn(e.to_string()))?
        };
        // Rows left from an earlier run are sent first.
        shared.notify.notify_one();
        let task = tokio::spawn(run(
            Arc::clone(&shared),
            init.cfg.clone(),
            broker.clone(),
            init.password,
            tls,
            stop_rx,
        ));
        Ok(MqttService {
            shared,
            cfg: init.cfg,
            broker,
            stop,
            task: Some(task),
            feeder: Some(feeder),
        })
    }

    pub fn status(&self) -> MqttStatus {
        let c = &self.shared.c;
        let (depth, max, overflow) = {
            let o = lock(&self.shared.outbox);
            (o.depth(), o.max(), o.dropped)
        };
        let info = lock(&self.shared.info);
        MqttStatus {
            state: info.state,
            broker: format!("{}:{}", self.broker.host, self.broker.port),
            client_id: self.cfg.client_id.clone(),
            outbox_depth: depth,
            outbox_max: max,
            enqueued: c.enqueued.load(Ordering::Relaxed),
            published: c.published.load(Ordering::Relaxed),
            dropped_overflow: overflow,
            expired: c.expired.load(Ordering::Relaxed),
            throttled: c.throttled.load(Ordering::Relaxed),
            lagged: c.lagged.load(Ordering::Relaxed),
            rejected: c.rejected.load(Ordering::Relaxed),
            in_flight: c.in_flight.load(Ordering::Relaxed),
            reconnects: c.reconnects.load(Ordering::Relaxed),
            last_error: info.last_error.clone(),
            last_ack_ms: info.last_ack_ms,
        }
    }

    /// Publish `offline` (retained), disconnect cleanly and stop. Unsent rows stay in the outbox file.
    pub async fn stop(mut self) {
        self.shutdown().await;
    }

    async fn shutdown(&mut self) {
        let _ = self.stop.send(true);
        self.shared.stop_feeder.store(true, Ordering::SeqCst);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        if let Some(f) = self.feeder.take() {
            let _ = tokio::task::spawn_blocking(move || f.join()).await;
        }
        self.shared.set_state(MqttState::Off);
    }
}

impl Drop for MqttService {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.shared.stop_feeder.store(true, Ordering::SeqCst);
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

// ---- feeder: bus → outbox ----

fn feed(shared: Arc<Shared>, cfg: MqttConfig, session: String, mut sub: BusSubscriber) {
    let min_interval = i64::try_from(cfg.min_interval_ms).unwrap_or(i64::MAX);
    // last published ts and quality per tag
    let mut last: HashMap<String, (i64, u8, Option<u8>)> = HashMap::new();
    while !shared.stop_feeder.load(Ordering::SeqCst) {
        let mut batch: Vec<NewRow> = Vec::new();
        let mut closed = false;
        while batch.len() < FEED_BATCH {
            match sub.try_recv() {
                BusRecv::Sample(s) => {
                    let key = format!("{}/{}", s.device_id, s.tag_id);
                    let q = (s.quality.code(), s.quality.exception_code());
                    if let Some((ts, qc, ex)) = last.get(&key)
                        && (*qc, *ex) == q
                        && s.ts_ms - *ts < min_interval
                    {
                        shared.c.throttled.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    last.insert(key, (s.ts_ms, q.0, q.1));
                    let body = payload::from_sample(&s, &session);
                    let Ok(bytes) = serde_json::to_vec(&body) else { continue };
                    batch.push(NewRow {
                        topic: cfg.tag_topic(&s.device_id, &s.tag_id),
                        payload: bytes,
                        enq_ms: now_ms(),
                        sample_seq: s.seq,
                    });
                }
                BusRecv::Lagged(n) => {
                    shared.c.lagged.fetch_add(n, Ordering::Relaxed);
                    shared.fail(format!(
                        "{n} samples were overwritten on the sample bus before they could be queued"
                    ));
                }
                BusRecv::Empty => break,
                BusRecv::Closed => {
                    closed = true;
                    break;
                }
            }
        }
        if !batch.is_empty() {
            let n = batch.len() as u64;
            match lock(&shared.outbox).push_many(&batch) {
                Ok(_) => {
                    shared.c.enqueued.fetch_add(n, Ordering::Relaxed);
                    shared.notify.notify_one();
                }
                Err(e) => shared.fail(e.to_string()),
            }
        }
        if closed {
            break;
        }
        std::thread::sleep(FEED_PERIOD);
    }
}

// ---- sender ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Row(i64),
    Online,
    Offline,
}

enum Ended {
    Stopped,
    Failed { msg: String, reached_online: bool },
}

struct Conn<'a> {
    shared: &'a Shared,
    cfg: &'a MqttConfig,
    client: &'a AsyncClient,
    online: bool,
    window: usize,
    cursor: i64,
    pending: VecDeque<Slot>,
    by_pkid: HashMap<u16, Slot>,
    offline_acked: bool,
}

impl Conn<'_> {
    fn in_flight(&self) -> usize {
        self.pending.len() + self.by_pkid.len()
    }

    fn publish_status(&mut self, text: &'static str, slot: Slot) {
        let ok = self
            .client
            .try_publish(self.cfg.status_topic(), QoS::AtLeastOnce, true, text)
            .is_ok();
        if ok {
            self.pending.push_back(slot);
        }
    }

    /// Hand rows to the client in outbox order, up to the window.
    fn fill(&mut self) {
        if !self.online {
            return;
        }
        let expiry_ms = i64::from(self.cfg.expiry_s) * 1000;
        'outer: while self.in_flight() < self.window {
            let want = self.window - self.in_flight();
            let rows: Vec<Row> = match lock(&self.shared.outbox).peek_after(self.cursor, want) {
                Ok(r) => r,
                Err(e) => {
                    self.shared.fail(e.to_string());
                    break;
                }
            };
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let remaining_ms = expiry_ms - (now_ms() - row.enq_ms);
                if remaining_ms <= 0 {
                    // Too old to be useful; counted, never sent.
                    let _ = lock(&self.shared.outbox).delete(row.id);
                    self.shared.c.expired.fetch_add(1, Ordering::Relaxed);
                    self.cursor = row.id;
                    continue;
                }
                let props = PublishProperties {
                    payload_format_indicator: Some(1),
                    message_expiry_interval: Some(
                        u32::try_from((remaining_ms as u64).div_ceil(1000)).unwrap_or(u32::MAX),
                    ),
                    content_type: Some("application/json".into()),
                    ..PublishProperties::default()
                };
                match self.client.try_publish_with_properties(
                    row.topic.clone(),
                    QoS::AtLeastOnce,
                    false,
                    row.payload.clone(),
                    props,
                ) {
                    Ok(()) => {
                        self.cursor = row.id;
                        self.pending.push_back(Slot::Row(row.id));
                    }
                    Err(_) => break 'outer, // request channel full; retried on the next tick
                }
            }
        }
        self.shared
            .c
            .in_flight
            .store(self.in_flight() as u64, Ordering::Relaxed);
    }

    /// Handle one event; `Err` ends this connection.
    fn on_event(&mut self, ev: Event) -> Result<(), String> {
        match ev {
            Event::Incoming(Packet::ConnAck(ack)) => {
                if ack.code != ConnectReturnCode::Success {
                    return Err(format!("the broker refused the connection: {:?}", ack.code));
                }
                if let Some(max) = ack.properties.as_ref().and_then(|p| p.receive_max) {
                    self.window = self.window.min(usize::from(max.max(1)));
                }
                self.online = true;
                self.shared.set_state(MqttState::Online);
                self.publish_status("online", Slot::Online);
                self.fill();
            }
            Event::Outgoing(Outgoing::Publish(pkid)) => {
                if let Some(slot) = self.pending.pop_front() {
                    self.by_pkid.insert(pkid, slot);
                }
            }
            Event::Incoming(Packet::PubAck(ack)) => {
                if let Some(slot) = self.by_pkid.remove(&ack.pkid) {
                    let accepted = matches!(ack.reason, PubAckReason::Success | PubAckReason::NoMatchingSubscribers);
                    match slot {
                        Slot::Row(id) => {
                            let _ = lock(&self.shared.outbox).delete(id);
                            if accepted {
                                self.shared.c.published.fetch_add(1, Ordering::Relaxed);
                                lock(&self.shared.info).last_ack_ms = Some(now_ms());
                            } else {
                                // A refusal will not change on retry; drop the row and say so.
                                self.shared.c.rejected.fetch_add(1, Ordering::Relaxed);
                                self.shared
                                    .fail(format!("the broker refused a message: {:?}", ack.reason));
                            }
                        }
                        Slot::Online => {}
                        Slot::Offline => self.offline_acked = true,
                    }
                    self.fill();
                }
            }
            Event::Incoming(Packet::Disconnect(d)) => {
                return Err(format!("the broker closed the connection: {:?}", d.reason_code));
            }
            // Nothing else is acted on. In particular no inbound publish: we never subscribe.
            _ => {}
        }
        self.shared
            .c
            .in_flight
            .store(self.in_flight() as u64, Ordering::Relaxed);
        Ok(())
    }
}

type PolledEvent = Result<Event, rumqttc::v5::ConnectionError>;

/// Wait for the DISCONNECT packet to be written (or a second to pass).
async fn finish_disconnect(rx: &mut mpsc::Receiver<PolledEvent>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while let Ok(Some(Ok(ev))) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if matches!(ev, Event::Outgoing(Outgoing::Disconnect)) {
            break;
        }
    }
}

/// Aborts the poller task when a session ends, however it ends.
struct PollerGuard(JoinHandle<()>);

impl Drop for PollerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// One connection. The event loop runs in its own task: `EventLoop::poll` is not
/// cancel-safe while it connects, so it must never sit in a `select!` that can
/// drop it. Events come through a channel instead.
async fn session(
    shared: &Shared,
    cfg: &MqttConfig,
    client: &AsyncClient,
    mut el: EventLoop,
    stop: &mut watch::Receiver<bool>,
) -> Ended {
    let (tx, mut rx) = mpsc::channel::<PolledEvent>(64);
    let _poller = PollerGuard(tokio::spawn(async move {
        loop {
            let ev = el.poll().await;
            let failed = ev.is_err();
            if tx.send(ev).await.is_err() || failed {
                break;
            }
        }
    }));
    let mut conn = Conn {
        shared,
        cfg,
        client,
        online: false,
        window: WINDOW,
        cursor: 0,
        pending: VecDeque::new(),
        by_pkid: HashMap::new(),
        offline_acked: false,
    };
    let mut reached_online = false;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if conn.online {
                    conn.publish_status("offline", Slot::Offline);
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                    while !conn.offline_acked {
                        match tokio::time::timeout_at(deadline, rx.recv()).await {
                            Ok(Some(Ok(ev))) => { let _ = conn.on_event(ev); }
                            _ => break,
                        }
                    }
                    let _ = client.try_disconnect();
                    finish_disconnect(&mut rx).await;
                }
                return Ended::Stopped;
            }
            ev = rx.recv() => match ev {
                Some(Ok(ev)) => {
                    if let Err(msg) = conn.on_event(ev) {
                        return Ended::Failed { msg, reached_online };
                    }
                    reached_online |= conn.online;
                }
                Some(Err(e)) => return Ended::Failed { msg: e.to_string(), reached_online },
                None => return Ended::Failed { msg: "the connection task ended".into(), reached_online },
            },
            _ = shared.notify.notified() => conn.fill(),
            _ = tick.tick() => conn.fill(),
        }
    }
}

async fn run(
    shared: Arc<Shared>,
    cfg: MqttConfig,
    broker: Broker,
    password: Option<Secret>,
    tls: Option<Arc<ClientConfig>>,
    mut stop: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    while !*stop.borrow() {
        shared.set_state(MqttState::Connecting);
        let opts = options(&cfg, &broker, &cfg.client_id, password.as_ref(), tls.clone(), true);
        let (client, el) = AsyncClient::new(opts, REQUEST_CAPACITY);
        let ended = session(&shared, &cfg, &client, el, &mut stop).await;
        drop(client);
        shared.c.in_flight.store(0, Ordering::Relaxed);
        match ended {
            Ended::Stopped => break,
            Ended::Failed { msg, reached_online } => {
                shared.fail(redact(&msg, password.as_ref()));
                shared.c.reconnects.fetch_add(1, Ordering::Relaxed);
                shared.set_state(MqttState::Backoff);
                if reached_online {
                    backoff = Duration::from_secs(1);
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = stop.changed() => break,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
    shared.set_state(MqttState::Off);
}

// ---- test connection ----

/// Connect, wait for the CONNACK, disconnect. Uses its own client ID so a running
/// publisher is never evicted from the broker.
pub async fn test_connection(
    cfg: &MqttConfig,
    password: Option<&Secret>,
    tls: Option<Arc<ClientConfig>>,
) -> TestReport {
    let started = Instant::now();
    let report = |ok: bool, detail: String| TestReport {
        ok,
        detail: redact(&detail, password),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    let broker = match cfg.validate() {
        Ok(b) => b,
        Err(e) => return report(false, e.to_string()),
    };
    let tls = match resolve_tls(cfg, &broker, tls) {
        Ok(t) => t,
        Err(e) => return report(false, e.to_string()),
    };
    let suffix = super::config::random_client_id();
    let base: String = cfg.client_id.chars().take(48).collect();
    let id = format!("{base}-test-{}", suffix.trim_start_matches("buslink-"));
    let (client, mut el) = AsyncClient::new(options(cfg, &broker, &id, password, tls, false), 8);
    let deadline = tokio::time::Instant::now() + TEST_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, el.poll()).await {
            Err(_) => return report(false, "timed out waiting for the broker".into()),
            Ok(Err(e)) => return report(false, e.to_string()),
            Ok(Ok(Event::Incoming(Packet::ConnAck(ack)))) => {
                if ack.code != ConnectReturnCode::Success {
                    return report(false, format!("the broker refused the connection: {:?}", ack.code));
                }
                let _ = client.try_disconnect();
                let end = tokio::time::Instant::now() + Duration::from_secs(1);
                while let Ok(Ok(ev)) = tokio::time::timeout_at(end, el.poll()).await {
                    if matches!(ev, Event::Outgoing(Outgoing::Disconnect)) {
                        break;
                    }
                }
                let how = if broker.tls { "TLS" } else { "unencrypted" };
                return report(
                    true,
                    format!("Connected to {}:{} ({how}, MQTT 5).", broker.host, broker.port),
                );
            }
            Ok(Ok(_)) => {}
        }
    }
}
