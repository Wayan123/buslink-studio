//! A small in-process MQTT 5 broker for the telemetry tests, built on rumqttc's
//! packet codec. It can cut every connection (a Wi-Fi drop), refuse new ones,
//! withhold PUBACKs, refuse logins, and it records every packet the client sends
//! so the tests can assert what was (and was not) sent.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::type_complexity
)]

use bytes::BytesMut;
use rumqttc::v5::mqttbytes::Error as PacketError;
use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::mqttbytes::v5::{ConnAck, ConnectReturnCode, Packet, PubAck};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

#[derive(Clone, Debug, PartialEq)]
pub struct Will {
    pub topic: String,
    pub payload: Vec<u8>,
    pub retain: bool,
    pub qos: QoS,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rx {
    Connect {
        client_id: String,
        clean_start: bool,
        will: Option<Will>,
        username: Option<String>,
        password: Option<String>,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
        dup: bool,
        expiry: Option<u32>,
    },
    Subscribe,
    Unsubscribe,
    Disconnect,
    /// The broker published the will of a connection that ended without DISCONNECT.
    WillFired {
        topic: String,
        payload: Vec<u8>,
    },
    Other(String),
}

#[derive(Default)]
struct Mode {
    accepting: bool,
    withhold_acks: bool,
    refuse: Option<ConnectReturnCode>,
}

struct Inner {
    log: Mutex<Vec<Rx>>,
    retained: Mutex<HashMap<String, Vec<u8>>>,
    mode: Mutex<Mode>,
    /// Bumped by `cut`; every connection ends when it changes.
    generation: watch::Sender<u64>,
}

pub struct TestBroker {
    pub addr: SocketAddr,
    inner: Arc<Inner>,
    task: tokio::task::JoinHandle<()>,
}

impl TestBroker {
    pub async fn start() -> TestBroker {
        Self::start_with(None).await
    }

    pub async fn start_tls(acceptor: TlsAcceptor) -> TestBroker {
        Self::start_with(Some(acceptor)).await
    }

    async fn start_with(tls: Option<TlsAcceptor>) -> TestBroker {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (generation, _) = watch::channel(0u64);
        let inner = Arc::new(Inner {
            log: Mutex::new(Vec::new()),
            retained: Mutex::new(HashMap::new()),
            mode: Mutex::new(Mode {
                accepting: true,
                ..Mode::default()
            }),
            generation,
        });
        let i2 = Arc::clone(&inner);
        let task = tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    continue;
                };
                if !i2.mode.lock().unwrap().accepting {
                    drop(sock);
                    continue;
                }
                let i3 = Arc::clone(&i2);
                let tls = tls.clone();
                tokio::spawn(async move {
                    match tls {
                        Some(a) => {
                            if let Ok(s) = a.accept(sock).await {
                                conn(s, i3).await;
                            }
                        }
                        None => conn(sock, i3).await,
                    }
                });
            }
        });
        TestBroker { addr, inner, task }
    }

    /// Drop every connection now and refuse new ones until `restore`.
    pub fn cut(&self) {
        self.inner.mode.lock().unwrap().accepting = false;
        self.inner.generation.send_modify(|g| *g += 1);
    }

    pub fn restore(&self) {
        self.inner.mode.lock().unwrap().accepting = true;
    }

    pub fn set_withhold_acks(&self, on: bool) {
        self.inner.mode.lock().unwrap().withhold_acks = on;
    }

    pub fn refuse_with(&self, code: Option<ConnectReturnCode>) {
        self.inner.mode.lock().unwrap().refuse = code;
    }

    pub fn log(&self) -> Vec<Rx> {
        self.inner.log.lock().unwrap().clone()
    }

    pub fn retained(&self, topic: &str) -> Option<String> {
        self.inner
            .retained
            .lock()
            .unwrap()
            .get(topic)
            .map(|v| String::from_utf8_lossy(v).into_owned())
    }

    /// Publishes of the client on topics other than `…/status`, in arrival order.
    pub fn data(&self) -> Vec<(String, serde_json::Value, Rx)> {
        self.log()
            .into_iter()
            .filter_map(|r| match &r {
                Rx::Publish { topic, payload, .. } if !topic.ends_with("/status") => {
                    Some((topic.clone(), serde_json::from_slice(payload).unwrap(), r.clone()))
                }
                _ => None,
            })
            .collect()
    }

    pub fn data_seqs(&self) -> Vec<u64> {
        self.data().iter().map(|(_, v, _)| v["seq"].as_u64().unwrap()).collect()
    }

    pub fn status_publishes(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .filter_map(|r| match r {
                Rx::Publish { topic, payload, .. } if topic.ends_with("/status") => {
                    Some(String::from_utf8_lossy(&payload).into_owned())
                }
                _ => None,
            })
            .collect()
    }
}

impl Drop for TestBroker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn conn<S: AsyncRead + AsyncWrite + Unpin>(mut sock: S, inner: Arc<Inner>) {
    let mut gen_rx = inner.generation.subscribe();
    let mut buf = BytesMut::with_capacity(4096);
    let mut will: Option<Will> = None;
    let mut clean = false;
    'conn: loop {
        // Parse everything that is complete.
        loop {
            // rumqttc's own reader rejects the short DISCONNECT its client writes; any broker accepts it.
            if buf.first().is_some_and(|b| b >> 4 == 14) {
                inner.log.lock().unwrap().push(Rx::Disconnect);
                clean = true;
                break 'conn;
            }
            let packet = match Packet::read(&mut buf, None) {
                Ok(p) => p,
                Err(PacketError::InsufficientBytes(_)) => break,
                Err(_) => break 'conn,
            };
            let mut out = BytesMut::new();
            match packet {
                Packet::Connect(c, w, login) => {
                    will = w.as_ref().map(|w| Will {
                        topic: String::from_utf8_lossy(&w.topic).into_owned(),
                        payload: w.message.to_vec(),
                        retain: w.retain,
                        qos: w.qos,
                    });
                    inner.log.lock().unwrap().push(Rx::Connect {
                        client_id: c.client_id.clone(),
                        clean_start: c.clean_start,
                        will: will.clone(),
                        username: login.as_ref().map(|l| l.username.clone()),
                        password: login.as_ref().map(|l| l.password.clone()),
                    });
                    let refuse = inner.mode.lock().unwrap().refuse;
                    let code = refuse.unwrap_or(ConnectReturnCode::Success);
                    ConnAck {
                        session_present: false,
                        code,
                        properties: None,
                    }
                    .write(&mut out)
                    .unwrap();
                    if refuse.is_some() {
                        let _ = sock.write_all(&out).await;
                        break 'conn;
                    }
                }
                Packet::Publish(p) => {
                    let topic = String::from_utf8_lossy(&p.topic).into_owned();
                    inner.log.lock().unwrap().push(Rx::Publish {
                        topic: topic.clone(),
                        payload: p.payload.to_vec(),
                        qos: p.qos,
                        retain: p.retain,
                        dup: p.dup,
                        expiry: p.properties.as_ref().and_then(|x| x.message_expiry_interval),
                    });
                    if p.retain {
                        inner.retained.lock().unwrap().insert(topic, p.payload.to_vec());
                    }
                    if p.qos == QoS::AtLeastOnce && !inner.mode.lock().unwrap().withhold_acks {
                        Packet::PubAck(PubAck::new(p.pkid, None)).write(&mut out, None).unwrap();
                    }
                }
                Packet::PingReq(_) => {
                    Packet::PingResp(rumqttc::v5::mqttbytes::v5::PingResp)
                        .write(&mut out, None)
                        .unwrap();
                }
                Packet::Subscribe(_) => inner.log.lock().unwrap().push(Rx::Subscribe),
                Packet::Unsubscribe(_) => inner.log.lock().unwrap().push(Rx::Unsubscribe),
                Packet::Disconnect(_) => {
                    inner.log.lock().unwrap().push(Rx::Disconnect);
                    clean = true;
                    break 'conn;
                }
                other => inner.log.lock().unwrap().push(Rx::Other(format!("{other:?}"))),
            }
            if !out.is_empty() && sock.write_all(&out).await.is_err() {
                break 'conn;
            }
        }
        tokio::select! {
            _ = gen_rx.changed() => break 'conn,
            n = sock.read_buf(&mut buf) => match n {
                Ok(0) | Err(_) => break 'conn,
                Ok(_) => {}
            },
            _ = tokio::time::sleep(Duration::from_secs(120)) => break 'conn,
        }
    }
    // A connection that ended without DISCONNECT fires its will.
    if !clean && let Some(w) = will {
        inner.log.lock().unwrap().push(Rx::WillFired {
            topic: w.topic.clone(),
            payload: w.payload.clone(),
        });
        if w.retain {
            inner.retained.lock().unwrap().insert(w.topic, w.payload);
        }
    }
}
