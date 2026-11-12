//! Fan-out from the sample bus to WebSocket clients (architecture D4, D6).
//!
//! One std thread per server drains a bus subscriber into a tag store, and every
//! 250 ms turns the changes into one JSON frame that all clients share. A new
//! client gets a snapshot and its broadcast receiver under the same lock, so it
//! can neither miss a frame nor see one older than its snapshot.

use super::{LanSnapshot, LanSource, ProfileEntry};
use crate::bus::BusRecv;
use crate::session::live::{MAX_POINTS_PER_FRAME, TagView, TrendPoint, tag_key, view_of};
use crate::session::{LiveFrame, LiveSnapshot};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub const FRAME_PERIOD: Duration = Duration::from_millis(250);
const HEARTBEAT: Duration = Duration::from_secs(1);
const POLL_PERIOD: Duration = Duration::from_millis(20);
const BROADCAST_CAPACITY: usize = 64;

#[derive(Serialize)]
struct Msg<'a, T: Serialize> {
    #[serde(rename = "type")]
    kind: &'static str,
    data: &'a T,
}

pub fn message<T: Serialize>(kind: &'static str, data: &T) -> String {
    serde_json::to_string(&Msg { kind, data }).unwrap_or_else(|_| "{\"type\":\"error\"}".into())
}

struct Inner {
    tags: BTreeMap<String, TagView>,
    dirty: BTreeSet<String>,
    points: Vec<TrendPoint>,
    dropped: u64,
    tx: broadcast::Sender<Arc<str>>,
}

pub struct Hub {
    inner: Mutex<Inner>,
    source: Arc<dyn LanSource>,
    stop: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
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

impl Hub {
    pub fn start(source: Arc<dyn LanSource>) -> Arc<Hub> {
        // Subscribe before seeding: a sample published in between is applied on top,
        // and the `seq` guard in `apply` stops an older one from replacing a newer view.
        let mut sub = source.subscribe();
        let seed = source.snapshot();
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let hub = Arc::new(Hub {
            inner: Mutex::new(Inner {
                tags: seed.tags.into_iter().map(|t| (t.key.clone(), t)).collect(),
                dirty: BTreeSet::new(),
                points: Vec::new(),
                dropped: 0,
                tx,
            }),
            source,
            stop: AtomicBool::new(false),
            thread: Mutex::new(None),
        });
        let worker = Arc::clone(&hub);
        let spawned = std::thread::Builder::new()
            .name("buslink-lan-hub".into())
            .spawn(move || {
                let mut last_frame = Instant::now();
                let mut last_beat = Instant::now();
                while !worker.stop.load(Ordering::SeqCst) {
                    let mut closed = false;
                    {
                        let mut g = lock(&worker.inner);
                        loop {
                            match sub.try_recv() {
                                BusRecv::Sample(s) => g.apply(&s),
                                BusRecv::Lagged(n) => g.dropped = g.dropped.saturating_add(n),
                                BusRecv::Empty => break,
                                BusRecv::Closed => {
                                    closed = true;
                                    break;
                                }
                            }
                        }
                    }
                    if last_frame.elapsed() >= FRAME_PERIOD {
                        last_frame = Instant::now();
                        let beat = last_beat.elapsed() >= HEARTBEAT;
                        if beat {
                            last_beat = Instant::now();
                        }
                        worker.tick(beat);
                    }
                    if closed {
                        break;
                    }
                    std::thread::sleep(POLL_PERIOD);
                }
            });
        if let Ok(h) = spawned {
            *lock(&hub.thread) = Some(h);
        }
        hub
    }

    /// Build and broadcast one frame; `beat` attaches links and alarms (1 Hz).
    fn tick(&self, beat: bool) {
        let live = beat.then(|| self.source.snapshot());
        let mut g = lock(&self.inner);
        if let Some(s) = &live {
            // A removed device must not linger in the view.
            let devices: BTreeSet<&str> = s
                .links
                .iter()
                .flat_map(|l| l.devices.iter().map(|d| d.id.as_str()))
                .collect();
            g.tags.retain(|_, t| devices.contains(t.device_id.as_str()));
        }
        let dirty = std::mem::take(&mut g.dirty);
        let tags: Vec<TagView> = dirty.iter().filter_map(|k| g.tags.get(k).cloned()).collect();
        let points = std::mem::take(&mut g.points);
        let dropped = std::mem::take(&mut g.dropped);
        let frame = LiveFrame {
            ts_ms: now_ms(),
            tags,
            points,
            dropped,
            links: live.as_ref().map(|s| s.links.clone()),
            alarms: live.as_ref().map(|s| s.alarms.clone()),
            historian_error: live.as_ref().and_then(|s| s.historian_error.clone()),
        };
        if frame.is_empty() {
            return;
        }
        // No receiver is fine: nobody is watching.
        let _ = g.tx.send(Arc::from(message("frame", &frame)));
    }

    fn snapshot_locked(&self, g: &Inner) -> LanSnapshot {
        let base = self.source.snapshot();
        LanSnapshot {
            snapshot: LiveSnapshot {
                tags: g.tags.values().cloned().collect(),
                ..base
            },
            profiles: self
                .source
                .profiles()
                .iter()
                .map(|p| ProfileEntry {
                    key: p.key(),
                    profile: (**p).clone(),
                })
                .collect(),
        }
    }

    /// The current state, for `GET /api/snapshot`.
    pub fn snapshot(&self) -> LanSnapshot {
        let g = lock(&self.inner);
        self.snapshot_locked(&g)
    }

    /// A snapshot message and a receiver for every frame after it.
    pub fn snapshot_and_subscribe(&self) -> (String, broadcast::Receiver<Arc<str>>) {
        let g = lock(&self.inner);
        let rx = g.tx.subscribe();
        let snap = self.snapshot_locked(&g);
        (message("snapshot", &snap), rx)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = lock(&self.thread).take() {
            let _ = h.join();
        }
    }
}

impl Inner {
    fn apply(&mut self, s: &Arc<crate::quality::Sample>) {
        let key = tag_key(&s.device_id, &s.tag_id);
        if self.tags.get(&key).is_some_and(|v| v.seq > s.seq) {
            return;
        }
        if self.points.len() < MAX_POINTS_PER_FRAME {
            self.points.push(TrendPoint {
                k: key.clone(),
                t: s.ts_ms,
                v: s.fresh_value().filter(|v| v.is_finite()),
                q: s.quality.as_str(),
            });
        } else {
            self.dropped += 1;
        }
        self.tags.insert(key.clone(), view_of(s));
        self.dirty.insert(key);
    }
}
