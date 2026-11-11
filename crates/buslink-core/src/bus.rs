//! Sample bus (architecture D4): one broadcast stream of samples, so the UI,
//! historian, LAN view and MQTT all see the same values and qualities.
//! Uses `tokio::sync::broadcast` without a runtime; a lagging subscriber is
//! told how many samples it missed instead of losing them silently.

use crate::quality::Sample;
use std::sync::Arc;
use tokio::sync::broadcast::{self, error::TryRecvError};

pub const DEFAULT_CAPACITY: usize = 4096;

#[derive(Clone)]
pub struct SampleBus {
    tx: broadcast::Sender<Arc<Sample>>,
}

#[derive(Debug, PartialEq)]
pub enum BusRecv {
    Sample(Arc<Sample>),
    /// This many samples were overwritten before this subscriber read them.
    Lagged(u64),
    Empty,
    Closed,
}

pub struct BusSubscriber {
    rx: broadcast::Receiver<Arc<Sample>>,
}

impl SampleBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Publish to every subscriber; returns how many will see it.
    pub fn publish(&self, sample: Sample) -> usize {
        self.tx.send(Arc::new(sample)).unwrap_or(0)
    }

    pub fn subscribe(&self) -> BusSubscriber {
        BusSubscriber {
            rx: self.tx.subscribe(),
        }
    }

    /// A sink for [`crate::poll::worker::LinkWorker::spawn`].
    pub fn sink(&self) -> crate::poll::worker::SampleSink {
        let bus = self.clone();
        Arc::new(move |s| {
            bus.publish(s);
        })
    }
}

impl Default for SampleBus {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl BusSubscriber {
    pub fn try_recv(&mut self) -> BusRecv {
        match self.rx.try_recv() {
            Ok(s) => BusRecv::Sample(s),
            Err(TryRecvError::Lagged(n)) => BusRecv::Lagged(n),
            Err(TryRecvError::Empty) => BusRecv::Empty,
            Err(TryRecvError::Closed) => BusRecv::Closed,
        }
    }

    /// Blocking receive (call from a plain thread, never inside an async runtime).
    pub fn recv_blocking(&mut self) -> BusRecv {
        match self.rx.blocking_recv() {
            Ok(s) => BusRecv::Sample(s),
            Err(broadcast::error::RecvError::Lagged(n)) => BusRecv::Lagged(n),
            Err(broadcast::error::RecvError::Closed) => BusRecv::Closed,
        }
    }

    /// Everything currently queued (lag notices included).
    pub fn drain(&mut self) -> Vec<BusRecv> {
        let mut out = Vec::new();
        loop {
            match self.try_recv() {
                BusRecv::Empty => break,
                BusRecv::Closed => {
                    out.push(BusRecv::Closed);
                    break;
                }
                other => out.push(other),
            }
        }
        out
    }
}
