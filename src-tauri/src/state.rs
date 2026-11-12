//! App state: the core session, the training bus and the frame emitter.

use crate::serial;
use buslink_core::clock::{Clock, SystemClock};
use buslink_core::command::CommandConfig;
use buslink_core::command::control::Control;
use buslink_core::device::{LinkConfig, LinkKind};
use buslink_core::profile;
use buslink_core::session::{Session, SessionConfig, TransportFactory};
use buslink_core::transport::tcp::TcpTransport;
use buslink_core::transport::{Transport, TransportError};
use buslink_sim::training::Training;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const FRAME_EVENT: &str = "buslink://frame";
/// ≤ 4 Hz UI refresh, independent of poll rates (ui.md, research/02 §7).
const FRAME_PERIOD: Duration = Duration::from_millis(250);
const HEARTBEAT: Duration = Duration::from_secs(1);
const TCP_CONNECT_TIMEOUT_MS: u64 = 3000;

/// Bundled profiles, compiled in so desktop and Android ship the same files.
const BUNDLED: [(&str, &str); 5] = [
    ("bgt-smps-8in1.json", include_str!("../../profiles/bgt-smps-8in1.json")),
    (
        "generic-holding.json",
        include_str!("../../profiles/generic-holding.json"),
    ),
    (
        "sim-energy-meter.json",
        include_str!("../../profiles/sim-energy-meter.json"),
    ),
    (
        "sim-temp-controller.json",
        include_str!("../../profiles/sim-temp-controller.json"),
    ),
    ("sim-vfd.json", include_str!("../../profiles/sim-vfd.json")),
];

pub struct AppState {
    pub session: Arc<Session>,
    /// The one command service of the app (the only write path, safety rule 2).
    pub control: Arc<Control>,
    pub training: Arc<Mutex<Option<Arc<Training>>>>,
    pub clock: Arc<dyn Clock>,
    pub db_path: Option<PathBuf>,
    pub profile_errors: Vec<String>,
    pub stop: Arc<AtomicBool>,
}

fn factory(app: AppHandle, training: Arc<Mutex<Option<Arc<Training>>>>) -> TransportFactory {
    Arc::new(move |cfg: &LinkConfig| -> Result<Box<dyn Transport>, TransportError> {
        match &cfg.kind {
            LinkKind::Rtu { port, serial } => serial::open(&app, port, *serial),
            LinkKind::Tcp { host, port } | LinkKind::RtuOverTcp { host, port, .. } => {
                TcpTransport::connect(host, *port, TCP_CONNECT_TIMEOUT_MS).map(|t| Box::new(t) as Box<dyn Transport>)
            }
            LinkKind::Simulator { .. } => training
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|t| t.transport_for(cfg))
                .ok_or_else(|| TransportError::Io("Training mode is not started".into())),
        }
    })
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(windows) {
        "windows"
    } else {
        std::env::consts::OS
    }
}

impl AppState {
    pub fn new(app: &AppHandle) -> Result<AppState, String> {
        let clock = SystemClock::shared();
        let training: Arc<Mutex<Option<Arc<Training>>>> = Arc::new(Mutex::new(None));
        let db_path = app.path().app_data_dir().ok().and_then(|dir| {
            std::fs::create_dir_all(&dir).ok()?;
            Some(dir.join("buslink-history.sqlite3"))
        });
        let session = Session::start(
            SessionConfig {
                db_path: db_path.clone(),
                app_version: env!("CARGO_PKG_VERSION").into(),
                platform: platform().into(),
                logging_interval_ms: 1000,
            },
            Arc::clone(&clock),
            factory(app.clone(), Arc::clone(&training)),
        )
        .map_err(|e| e.to_string())?;
        let mut profile_errors = Vec::new();
        for (name, text) in BUNDLED {
            match profile::load_str(text) {
                Ok(p) => {
                    session.add_profile(p.profile);
                }
                Err(e) => profile_errors.push(format!("{name}: {e}")),
            }
        }
        let session = Arc::new(session);
        let control = Arc::new(Control::new(Arc::clone(&session), CommandConfig::default()));
        Ok(AppState {
            session,
            control,
            training,
            clock,
            db_path,
            profile_errors,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }
}

/// Forward session frames to the webview at ≤ 4 Hz, with a 1 s heartbeat for counters.
pub fn spawn_emitter(app: AppHandle, session: Arc<Session>, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("buslink-ui-frames".into())
        .spawn(move || {
            let mut last_emit = Instant::now();
            #[cfg(target_os = "android")]
            let mut service = crate::android_usb::ServiceSync::new();
            #[cfg(target_os = "android")]
            let mut last_sync = Instant::now();
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(FRAME_PERIOD);
                let mut frame = session.take_frame();
                if frame.links.is_none() && last_emit.elapsed() >= HEARTBEAT {
                    frame.links = Some(session.link_status());
                }
                #[cfg(target_os = "android")]
                if last_sync.elapsed() >= Duration::from_secs(1) {
                    last_sync = Instant::now();
                    sync_service(&app, &session, &mut service);
                }
                if frame.is_empty() {
                    continue;
                }
                last_emit = Instant::now();
                // A closed window is not an error worth stopping for; the next frame retries.
                let _ = app.emit(FRAME_EVENT, &frame);
            }
        });
    if let Err(e) = spawned {
        eprintln!("buslink: could not start the frame emitter: {e}");
    }
}

/// Keep the foreground service in step with the links: it runs only while a real
/// (non-simulated) link polls, and stops on disconnect, detach and polling stop.
/// Starting it is a user action (`start_polling`); this only updates or stops it.
#[cfg(target_os = "android")]
fn sync_service(app: &AppHandle, session: &Session, service: &mut crate::android_usb::ServiceSync) {
    use buslink_core::session::LinkState;
    let links = session.link_status();
    let polling: Vec<_> = links
        .iter()
        .filter(|l| !l.simulated && l.state == LinkState::Polling)
        .collect();
    let want = polling.first().map(|l| {
        let devices: usize = polling.iter().map(|l| l.devices.len()).sum();
        (l.label.clone(), "polling", u32::try_from(devices).unwrap_or(u32::MAX))
    });
    service.sync(app, want);
}
