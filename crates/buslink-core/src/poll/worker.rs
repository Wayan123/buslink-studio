//! The link worker: a dedicated thread that owns the transport and runs the
//! scheduler (blocking serial I/O stays off any async runtime). Every read and
//! write for the link goes through its single queue (safety rule 9).

use super::scheduler::{DeviceCounters, LinkScheduler, Priority, Step, UrgentJob};
use crate::command::{CommandLink, PreparedFrame};
use crate::protocol::pdu::{ReadData, ReadRequest};
use crate::quality::Sample;
use crate::transport::exchange::{ExchangeError, ExchangeRecord};
use crate::transport::{LinkClaim, Transport, TransportError};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

enum Msg {
    Job(UrgentJob),
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Running,
    /// The transport failed (detach, socket closed). Reconnect means a new worker.
    Down,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct LinkStatus {
    pub state: LinkState,
    pub counters: Vec<(String, DeviceCounters)>,
}

pub type SampleSink = Arc<dyn Fn(Sample) + Send + Sync>;

pub struct LinkWorker {
    tx: mpsc::Sender<Msg>,
    status: Arc<Mutex<LinkStatus>>,
    handle: Option<JoinHandle<()>>,
}

/// Cheap, cloneable way to submit jobs to a running worker.
#[derive(Clone)]
pub struct LinkHandle {
    tx: mpsc::Sender<Msg>,
    /// Timeout for jobs submitted through this handle.
    pub timeout_ms: u64,
}

impl LinkWorker {
    /// Start the worker. `claim` proves this worker is the only master on the bus;
    /// it is released when the thread ends.
    pub fn spawn(
        claim: LinkClaim,
        mut scheduler: LinkScheduler,
        mut transport: Box<dyn Transport>,
        sink: SampleSink,
    ) -> std::io::Result<LinkWorker> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let status = Arc::new(Mutex::new(LinkStatus {
            state: LinkState::Running,
            counters: scheduler.counters(),
        }));
        let st = Arc::clone(&status);
        let name = format!("buslink-link-{}", scheduler.link().id);
        let handle = std::thread::Builder::new().name(name).spawn(move || {
            let _claim = claim;
            let mut emit = |s: Sample| sink(s);
            let mut final_state = LinkState::Stopped;
            'run: loop {
                loop {
                    match rx.try_recv() {
                        Ok(Msg::Job(job)) => scheduler.submit(job),
                        Ok(Msg::Stop) | Err(mpsc::TryRecvError::Disconnected) => break 'run,
                        Err(mpsc::TryRecvError::Empty) => break,
                    }
                }
                let step = scheduler.step(transport.as_mut(), &mut emit);
                if let Ok(mut s) = st.lock() {
                    s.counters = scheduler.counters();
                }
                match step {
                    Step::Worked => {}
                    Step::LinkDown => {
                        final_state = LinkState::Down;
                        break 'run;
                    }
                    Step::Idle { until_mono_ms } => {
                        let wait = until_mono_ms.saturating_sub(scheduler.clock_mono());
                        match rx.recv_timeout(Duration::from_millis(wait.clamp(1, 1000))) {
                            Ok(Msg::Job(job)) => scheduler.submit(job),
                            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break 'run,
                            Err(RecvTimeoutError::Timeout) => {}
                        }
                    }
                }
            }
            scheduler.fail_pending();
            // Jobs that raced in after the decision to stop also fail; nothing is replayed.
            while let Ok(msg) = rx.try_recv() {
                if let Msg::Job(job) = msg {
                    scheduler.submit(job);
                    scheduler.fail_pending();
                }
            }
            transport.close();
            if let Ok(mut s) = st.lock() {
                s.state = final_state;
                s.counters = scheduler.counters();
            }
        })?;
        Ok(LinkWorker {
            tx,
            status,
            handle: Some(handle),
        })
    }

    pub fn handle(&self, timeout_ms: u64) -> LinkHandle {
        LinkHandle {
            tx: self.tx.clone(),
            timeout_ms,
        }
    }

    pub fn status(&self) -> LinkStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Stop polling, fail queued jobs, close the transport and wait for the thread.
    pub fn stop(mut self) -> LinkStatus {
        let _ = self.tx.send(Msg::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.status()
    }
}

impl Drop for LinkWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn lost() -> ExchangeError {
    ExchangeError::Link(TransportError::Disconnected("link worker stopped".into()))
}

impl LinkHandle {
    /// Read at the given priority and wait for the result.
    pub fn read(&self, unit: u8, request: ReadRequest, priority: Priority) -> Result<ReadData, ExchangeError> {
        let (tx, rx) = mpsc::sync_channel(1);
        let job = UrgentJob::Read {
            unit,
            request,
            priority,
            timeout_ms: self.timeout_ms,
            reply: Box::new(move |r| {
                let _ = tx.send(r);
            }),
        };
        self.tx.send(Msg::Job(job)).map_err(|_| lost())?;
        rx.recv().unwrap_or_else(|_| Err(lost()))
    }

    /// Hand a confirmed frame to the worker; it is sent once.
    pub fn send_command(&self, frame: PreparedFrame) -> ExchangeRecord {
        let unit = frame.unit();
        let pdu = frame.pdu().to_vec();
        let (tx, rx) = mpsc::sync_channel(1);
        let job = UrgentJob::Command {
            frame,
            timeout_ms: self.timeout_ms,
            reply: Box::new(move |r| {
                let _ = tx.send(r);
            }),
        };
        let not_sent = |request: Vec<u8>| ExchangeRecord {
            unit,
            request,
            response: Vec::new(),
            started_mono_ms: 0,
            elapsed_ms: 0,
            discarded: 0,
            outcome: Err(lost()),
        };
        if self.tx.send(Msg::Job(job)).is_err() {
            return not_sent(Vec::new());
        }
        // If the reply never comes the worker died mid-job: the effect is unknown,
        // so the record carries the PDU (not an empty "not sent" request).
        rx.recv().unwrap_or_else(|_| not_sent(pdu))
    }
}

impl CommandLink for LinkHandle {
    fn read(&mut self, unit: u8, request: ReadRequest) -> Result<ReadData, ExchangeError> {
        LinkHandle::read(self, unit, request, Priority::Command)
    }

    fn send(&mut self, frame: PreparedFrame) -> ExchangeRecord {
        self.send_command(frame)
    }
}
