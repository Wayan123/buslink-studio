//! IPC commands (app-shell-desktop design §9). Monitoring only: no command here
//! encodes or submits a write; control arrives with the command service (E5).
//! No command accepts a file path, a raw frame or a write request.

use crate::serial::{self, PortView};
use crate::state::{AppState, platform};
use buslink_core::alarm::{AlarmDef, AlarmSnapshot, default_deadband, default_delay_ms};
use buslink_core::device::{DeviceInstance, LinkConfig, LinkKind};
use buslink_core::historian::{EventRow, SessionInfo};
use buslink_core::profile::Profile;
use buslink_core::protocol::rtu::{Parity, SerialSettings};
use buslink_core::session::{DeviceInspection, ExportFormat, LiveSnapshot, SessionError};
use buslink_core::trend::TrendSeries;
use buslink_sim::training::{ActiveFault, TRAINING_DEVICES, TRAINING_LINK_ID, Training, TrainingFault, alarm_presets};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

type R<T> = Result<T, String>;

fn err(e: SessionError) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    platform: &'static str,
    session_id: String,
    historian_path: Option<String>,
    profile_errors: Vec<String>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: "BusLink Studio",
        version: env!("CARGO_PKG_VERSION"),
        platform: platform(),
        session_id: state.session.session_id().to_owned(),
        historian_path: state.db_path.as_ref().map(|p| p.display().to_string()),
        profile_errors: state.profile_errors.clone(),
    }
}

#[tauri::command]
pub fn list_ports() -> R<Vec<PortView>> {
    serial::list()
}

#[derive(Serialize)]
pub struct ProfileView {
    key: String,
    profile: Profile,
}

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> Vec<ProfileView> {
    state
        .session
        .profiles()
        .iter()
        .map(|p| ProfileView {
            key: p.key(),
            profile: (**p).clone(),
        })
        .collect()
}

#[derive(Deserialize)]
pub struct ConnectReq {
    /// `rtu`, `tcp` or `rtu_over_tcp`
    kind: String,
    port: Option<String>,
    baud: Option<u32>,
    data_bits: Option<u8>,
    parity: Option<Parity>,
    stop_bits: Option<u8>,
    host: Option<String>,
    tcp_port: Option<u16>,
}

#[derive(Serialize)]
pub struct ConnectResult {
    link_id: String,
    /// RTU links show the non-isolated adapter advice (safety rule 18).
    isolation_advice: bool,
}

fn slug(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    out.trim_matches('-').chars().take(40).collect()
}

fn serial_of(r: &ConnectReq) -> R<SerialSettings> {
    let s = SerialSettings::new(
        r.baud.unwrap_or(9600),
        r.data_bits.unwrap_or(8),
        r.parity.unwrap_or(Parity::None),
        r.stop_bits.unwrap_or(1),
    );
    s.validate().map_err(str::to_owned)?;
    Ok(s)
}

fn host_of(r: &ConnectReq) -> R<(String, u16)> {
    let host = r.host.as_deref().map(str::trim).unwrap_or_default();
    let ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !ok {
        return Err("host must be an IP address or host name".into());
    }
    let port = r.tcp_port.unwrap_or(502);
    if port == 0 {
        return Err("TCP port must be 1-65535".into());
    }
    Ok((host.to_owned(), port))
}

#[tauri::command]
pub fn connect_link(state: State<'_, AppState>, req: ConnectReq) -> R<ConnectResult> {
    let (config, rtu) = match req.kind.as_str() {
        "rtu" => {
            let port = req.port.clone().unwrap_or_default();
            if port.trim().is_empty() || port.len() > 64 {
                return Err("choose a serial port".into());
            }
            let serial = serial_of(&req)?;
            (
                LinkConfig {
                    id: slug(&port),
                    kind: LinkKind::Rtu { port, serial },
                    min_inter_frame_ms: 0,
                },
                true,
            )
        }
        "tcp" => {
            let (host, port) = host_of(&req)?;
            (
                LinkConfig {
                    id: format!("tcp-{}-{port}", slug(&host)),
                    kind: LinkKind::Tcp { host, port },
                    min_inter_frame_ms: 0,
                },
                false,
            )
        }
        "rtu_over_tcp" => {
            let (host, port) = host_of(&req)?;
            let serial = serial_of(&req)?;
            (
                LinkConfig {
                    id: format!("rtutcp-{}-{port}", slug(&host)),
                    kind: LinkKind::RtuOverTcp { host, port, serial },
                    min_inter_frame_ms: 0,
                },
                false,
            )
        }
        _ => return Err("unknown link kind".into()),
    };
    let link_id = config.id.clone();
    state.session.connect(config, false).map_err(err)?;
    Ok(ConnectResult {
        link_id,
        isolation_advice: rtu,
    })
}

#[tauri::command]
pub fn reconnect_link(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.reconnect(&id).map_err(err)
}

#[tauri::command]
pub fn disconnect_link(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.disconnect(&id).map_err(err)
}

#[tauri::command]
pub fn remove_link(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.remove_link(&id).map_err(err)
}

#[tauri::command]
pub fn start_polling(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.start_polling(&id).map_err(err)
}

#[tauri::command]
pub fn stop_polling(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.stop_polling(&id).map_err(err)
}

#[derive(Deserialize)]
pub struct AddDeviceReq {
    link_id: String,
    profile_key: String,
    unit_id: u8,
    name: Option<String>,
    poll_interval_ms: Option<u64>,
}

#[tauri::command]
pub fn add_device(state: State<'_, AppState>, req: AddDeviceReq) -> R<String> {
    let s = &state.session;
    let profile = s
        .profile(&req.profile_key)
        .ok_or_else(|| format!("unknown profile '{}'", req.profile_key))?;
    let link = s
        .link_config(&req.link_id)
        .ok_or_else(|| format!("unknown link '{}'", req.link_id))?;
    let id = s.suggest_device_id(&profile.profile_id, req.unit_id);
    let name = req
        .name
        .map(|n| n.trim().chars().take(80).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{} #{}", profile.model.get("en"), req.unit_id));
    let mut d = DeviceInstance::from_profile(&id, &name, &link, &profile);
    d.unit_id = req.unit_id;
    if let Some(ms) = req.poll_interval_ms {
        d.poll_interval_ms = ms;
    }
    s.add_device(d).map_err(err)?;
    Ok(id)
}

#[tauri::command]
pub fn remove_device(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.remove_device(&id).map_err(err)
}

#[tauri::command]
pub fn live_snapshot(state: State<'_, AppState>) -> LiveSnapshot {
    state.session.snapshot()
}

#[derive(Deserialize)]
pub struct TrendReq {
    device_id: String,
    tag_id: String,
    from_ms: i64,
    to_ms: i64,
    buckets: usize,
}

#[tauri::command]
pub async fn trend_query(state: State<'_, AppState>, req: TrendReq) -> R<TrendSeries> {
    let s = Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || {
        s.trend(
            &req.device_id,
            &req.tag_id,
            req.from_ms,
            req.to_ms,
            req.buckets.clamp(10, 4000),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err)
}

#[tauri::command]
pub fn historian_sessions(state: State<'_, AppState>) -> R<Vec<SessionInfo>> {
    state.session.sessions().map_err(err)
}

#[derive(Serialize)]
pub struct ExportResult {
    saved: bool,
    path: Option<String>,
    bytes: usize,
}

#[tauri::command]
pub async fn export_session(app: AppHandle, state: State<'_, AppState>, format: ExportFormat) -> R<ExportResult> {
    let s = Arc::clone(&state.session);
    let name = format!("buslink-{}.{}", s.session_id(), format.extension());
    tauri::async_runtime::spawn_blocking(move || -> R<ExportResult> {
        let bytes = s.export(format).map_err(err)?;
        #[cfg(feature = "e2e")]
        if let Ok(dir) = std::env::var("BUSLINK_E2E_EXPORT_DIR") {
            let path = std::path::Path::new(&dir).join(&name);
            std::fs::write(&path, &bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
            return Ok(ExportResult {
                saved: true,
                path: Some(path.display().to_string()),
                bytes: bytes.len(),
            });
        }
        let (label, ext) = match format {
            ExportFormat::Csv => ("CSV", "csv"),
            ExportFormat::Xlsx => ("Excel workbook", "xlsx"),
            ExportFormat::Json => ("JSON", "json"),
        };
        let picked = app
            .dialog()
            .file()
            .set_file_name(&name)
            .add_filter(label, &[ext])
            .blocking_save_file();
        let Some(fp) = picked else {
            return Ok(ExportResult {
                saved: false,
                path: None,
                bytes: 0,
            });
        };
        let path = fp.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path, &bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        Ok(ExportResult {
            saved: true,
            path: Some(path.display().to_string()),
            bytes: bytes.len(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn inspect_device(state: State<'_, AppState>, id: String) -> R<DeviceInspection> {
    state.session.inspect(&id).map_err(err)
}

// ---- alarms ----

#[tauri::command]
pub fn alarm_list(state: State<'_, AppState>) -> AlarmSnapshot {
    state.session.alarms()
}

#[tauri::command]
pub fn alarm_add(state: State<'_, AppState>, def: AlarmDef) -> R<()> {
    state.session.alarm_add(def).map_err(err)
}

#[tauri::command]
pub fn alarm_remove(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.alarm_remove(&id).map_err(err)
}

#[tauri::command]
pub fn alarm_ack(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.alarm_ack(&id).map_err(err)
}

#[derive(Deserialize)]
pub struct ShelveReq {
    id: String,
    minutes: u32,
}

#[tauri::command]
pub fn alarm_shelve(state: State<'_, AppState>, req: ShelveReq) -> R<()> {
    state
        .session
        .alarm_shelve(&req.id, u64::from(req.minutes) * 60_000)
        .map_err(err)
}

#[tauri::command]
pub fn alarm_unshelve(state: State<'_, AppState>, id: String) -> R<()> {
    state.session.alarm_unshelve(&id).map_err(err)
}

#[derive(Deserialize)]
pub struct OosReq {
    id: String,
    out_of_service: bool,
}

#[tauri::command]
pub fn alarm_set_oos(state: State<'_, AppState>, req: OosReq) -> R<()> {
    state
        .session
        .alarm_set_out_of_service(&req.id, req.out_of_service)
        .map_err(err)
}

#[tauri::command]
pub fn alarm_events(state: State<'_, AppState>) -> R<Vec<EventRow>> {
    state.session.events(Some("alarm"), 500).map_err(err)
}

#[derive(Serialize)]
pub struct AlarmDefaults {
    deadband: f64,
    delay_ms: u64,
}

#[derive(Deserialize)]
pub struct DefaultsReq {
    unit: Option<String>,
    span: f64,
}

#[tauri::command]
pub fn alarm_defaults(req: DefaultsReq) -> AlarmDefaults {
    AlarmDefaults {
        deadband: default_deadband(req.unit.as_deref(), req.span),
        delay_ms: default_delay_ms(req.unit.as_deref()),
    }
}

// ---- training ----

#[derive(Serialize)]
pub struct TrainingStatus {
    started: bool,
    link_id: &'static str,
    units: Vec<u8>,
    faults: Vec<ActiveFault>,
}

fn training_of(state: &AppState) -> Option<Arc<Training>> {
    state.training.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub fn training_status(state: State<'_, AppState>) -> TrainingStatus {
    match training_of(&state) {
        Some(t) => TrainingStatus {
            started: true,
            link_id: TRAINING_LINK_ID,
            units: t.world().lock().unwrap_or_else(|e| e.into_inner()).units(),
            faults: t.active(),
        },
        None => TrainingStatus {
            started: false,
            link_id: TRAINING_LINK_ID,
            units: Vec::new(),
            faults: Vec::new(),
        },
    }
}

/// Start Training mode: a simulated bus with three devices, alarm presets, polling on.
#[tauri::command]
pub fn training_start(state: State<'_, AppState>) -> R<TrainingStatus> {
    let s = &state.session;
    {
        let mut t = state.training.lock().unwrap_or_else(|e| e.into_inner());
        if t.is_none() {
            let seed = u64::try_from(state.clock.utc_ms()).unwrap_or(1);
            *t = Some(Arc::new(Training::new(seed, Arc::clone(&state.clock))));
        }
    }
    let existing = s.links().into_iter().find(|l| l.id == TRAINING_LINK_ID);
    match existing {
        Some(l) if l.state == buslink_core::session::LinkState::Polling => {}
        Some(l) => {
            if matches!(
                l.state,
                buslink_core::session::LinkState::Disconnected | buslink_core::session::LinkState::Down
            ) {
                s.reconnect(TRAINING_LINK_ID).map_err(err)?;
            }
            s.start_polling(TRAINING_LINK_ID).map_err(err)?;
        }
        None => {
            let link = Training::link_config();
            s.connect(link.clone(), true).map_err(err)?;
            let mut ids = Vec::new();
            for (unit, profile_id) in TRAINING_DEVICES {
                let Some(p) = s.profiles().into_iter().find(|p| p.profile_id == profile_id) else {
                    return Err(format!("bundled profile {profile_id} missing"));
                };
                let id = s.suggest_device_id(&p.profile_id, unit);
                let mut d = DeviceInstance::from_profile(&id, &format!("{} (simulated)", p.model.get("en")), &link, &p);
                d.unit_id = unit;
                d.response_timeout_ms = Some(300);
                s.add_device(d).map_err(err)?;
                ids.push(id);
            }
            if let [soil, vfd, temp] = ids.as_slice() {
                for a in alarm_presets(soil, vfd, temp) {
                    s.alarm_add(a).map_err(err)?;
                }
            }
            s.start_polling(TRAINING_LINK_ID).map_err(err)?;
        }
    }
    Ok(training_status(state))
}

#[derive(Deserialize)]
pub struct FaultReq {
    unit: u8,
    fault: TrainingFault,
    seconds: u32,
}

/// Fault injection exists only on the training bus (it acts on the simulator world).
#[tauri::command]
pub fn training_fault(state: State<'_, AppState>, req: FaultReq) -> R<TrainingStatus> {
    let t = training_of(&state).ok_or("Training mode is not started")?;
    t.inject(req.unit, req.fault, u64::from(req.seconds) * 1000)
        .map_err(|e| e.to_string())?;
    Ok(training_status(state))
}

#[tauri::command]
pub fn training_clear(state: State<'_, AppState>, unit: u8) -> R<TrainingStatus> {
    let t = training_of(&state).ok_or("Training mode is not started")?;
    t.clear(unit);
    Ok(training_status(state))
}
