//! IPC for control, commissioning, audit and report (commissioning-control design §7).
//!
//! Requests carry device IDs, command IDs, register IDs and engineering values
//! only: no frame, address or function code ever comes from the webview. All
//! checks (arming, bounds, confirmation, single send, read-back, audit) happen
//! in `buslink_core::command`. Calls that may wait for the link worker run off
//! the main thread.

use crate::state::AppState;
use buslink_core::command::commission::Wizard;
use buslink_core::command::control::{ArmRequest, Control, ControlStatus};
use buslink_core::command::{CommSettingKind, CommandOutcome, CommandRequest, DisarmReason, Preview};
use buslink_core::export::report::ReportData;
use buslink_core::historian::AuditRow;
use buslink_core::session::{ExportFormat, Session};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, State};

type R<T> = Result<T, String>;

const MAX_REPORT_BYTES: usize = 5 * 1024 * 1024;

async fn blocking<T: Send + 'static>(
    control: Arc<Control>,
    f: impl FnOnce(&Control) -> R<T> + Send + 'static,
) -> R<T> {
    tauri::async_runtime::spawn_blocking(move || f(&control))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelRequest {
    /// A profile command (run, stop, setpoint, ...).
    Named { command: String, value: Option<f64> },
    /// A `read_write` register by profile ID, in engineering units.
    Register { register: String, value: f64 },
}

impl From<PanelRequest> for CommandRequest {
    fn from(r: PanelRequest) -> Self {
        match r {
            PanelRequest::Named { command, value } => CommandRequest::Named { command, value },
            PanelRequest::Register { register, value } => CommandRequest::SetRegister { register, value },
        }
    }
}

#[tauri::command]
pub async fn control_status(state: State<'_, AppState>) -> R<ControlStatus> {
    blocking(Arc::clone(&state.control), |c| Ok(c.status())).await
}

/// Arm for one device as `Operator`. The request has no role (a forged `role` is
/// refused by `ArmRequest`); this is a single-user local app, so the role is an
/// interlock and not authentication.
#[tauri::command]
pub async fn control_arm(state: State<'_, AppState>, req: ArmRequest) -> R<ControlStatus> {
    blocking(Arc::clone(&state.control), move |c| {
        c.arm(&req.device_id, &req.user).map_err(|e| e.to_string())
    })
    .await
}

/// Separate, audited host action: raise the armed operator to commissioner until the arm ends.
#[tauri::command]
pub async fn commissioner_unlock(state: State<'_, AppState>, device_id: String) -> R<ControlStatus> {
    blocking(Arc::clone(&state.control), move |c| {
        c.commissioner_unlock(&device_id).map_err(|e| e.to_string())
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisarmWhy {
    Operator,
    AppBackground,
}

#[tauri::command]
pub async fn control_disarm(state: State<'_, AppState>, reason: DisarmWhy) -> R<ControlStatus> {
    let reason = match reason {
        DisarmWhy::Operator => DisarmReason::Operator,
        DisarmWhy::AppBackground => DisarmReason::AppBackground,
    };
    blocking(Arc::clone(&state.control), move |c| Ok(c.disarm(reason))).await
}

#[derive(Deserialize)]
pub struct PreviewReq {
    device_id: String,
    request: PanelRequest,
}

#[tauri::command]
pub async fn control_preview(state: State<'_, AppState>, req: PreviewReq) -> R<Preview> {
    blocking(Arc::clone(&state.control), move |c| {
        c.preview(&req.device_id, req.request.into()).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn control_confirm(state: State<'_, AppState>, preview_id: u64) -> R<ControlStatus> {
    blocking(Arc::clone(&state.control), move |c| {
        c.confirm(preview_id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn control_execute(state: State<'_, AppState>) -> R<CommandOutcome> {
    blocking(Arc::clone(&state.control), |c| c.execute().map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn control_cancel(state: State<'_, AppState>) -> R<ControlStatus> {
    blocking(Arc::clone(&state.control), |c| c.cancel().map_err(|e| e.to_string())).await
}

// ---- commissioning wizard ----

#[derive(Serialize)]
pub struct WizardSupport {
    settings: Vec<CommSettingKind>,
    reason: Option<String>,
}

#[tauri::command]
pub async fn wizard_support(state: State<'_, AppState>, device_id: String) -> R<WizardSupport> {
    blocking(Arc::clone(&state.control), move |c| {
        Ok(match c.wizard_support(&device_id) {
            Ok(settings) => WizardSupport { settings, reason: None },
            Err(reason) => WizardSupport {
                settings: Vec::new(),
                reason: Some(reason),
            },
        })
    })
    .await
}

#[derive(Deserialize)]
pub struct WizardStartReq {
    device_id: String,
    setting: CommSettingKind,
    value: u32,
    user: String,
}

#[tauri::command]
pub async fn wizard_start(state: State<'_, AppState>, req: WizardStartReq) -> R<Wizard> {
    blocking(Arc::clone(&state.control), move |c| {
        c.wizard_start(&req.device_id, req.setting, req.value, &req.user)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn wizard_preview(state: State<'_, AppState>) -> R<Preview> {
    blocking(Arc::clone(&state.control), |c| c.wizard_preview().map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn wizard_power_cycled(state: State<'_, AppState>) -> R<Wizard> {
    blocking(Arc::clone(&state.control), |c| {
        c.wizard_power_cycled().map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn wizard_reconnect(state: State<'_, AppState>) -> R<Wizard> {
    blocking(Arc::clone(&state.control), |c| c.wizard_reconnect().map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn wizard_verify(state: State<'_, AppState>) -> R<Wizard> {
    blocking(Arc::clone(&state.control), |c| c.wizard_verify().map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn wizard_revert(state: State<'_, AppState>) -> R<Wizard> {
    blocking(Arc::clone(&state.control), |c| c.wizard_revert().map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn wizard_close(state: State<'_, AppState>) -> R<ControlStatus> {
    blocking(Arc::clone(&state.control), |c| c.wizard_close().map_err(|e| e.to_string())).await
}

// ---- audit and report ----

#[tauri::command]
pub async fn audit_list(state: State<'_, AppState>) -> R<Vec<AuditRow>> {
    let s = Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || s.audit_rows().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct SaveResult {
    saved: bool,
    path: Option<String>,
    bytes: usize,
}

/// Save bytes through the native dialog (or the e2e export directory in the e2e build).
fn save(app: &AppHandle, name: &str, label: &str, ext: &str, bytes: &[u8]) -> R<SaveResult> {
    #[cfg(feature = "e2e")]
    if let Ok(dir) = std::env::var("BUSLINK_E2E_EXPORT_DIR") {
        let path = std::path::Path::new(&dir).join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        return Ok(SaveResult {
            saved: true,
            path: Some(path.display().to_string()),
            bytes: bytes.len(),
        });
    }
    match crate::files::save_bytes(app, name, label, ext, bytes)? {
        Some(saved) => Ok(SaveResult {
            saved: true,
            path: saved.path,
            bytes: bytes.len(),
        }),
        None => Ok(SaveResult {
            saved: false,
            path: None,
            bytes: 0,
        }),
    }
}

#[tauri::command]
pub async fn audit_export(app: AppHandle, state: State<'_, AppState>, format: ExportFormat) -> R<SaveResult> {
    let s: Arc<Session> = Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || {
        let (label, ext) = match format {
            ExportFormat::Csv => ("CSV", "csv"),
            ExportFormat::Xlsx => ("Excel workbook", "xlsx"),
            ExportFormat::Json => return Err("audit export is CSV or XLSX".to_owned()),
        };
        let bytes = s.audit_export(format).map_err(|e| e.to_string())?;
        let name = format!("buslink-audit-{}.{ext}", s.session_id());
        save(&app, &name, label, ext, &bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn report_data(state: State<'_, AppState>) -> R<ReportData> {
    let s = Arc::clone(&state.session);
    tauri::async_runtime::spawn_blocking(move || s.report().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// Save the report the webview rendered (DOM-serialized, so data is already escaped text).
#[tauri::command]
pub async fn report_save_html(app: AppHandle, state: State<'_, AppState>, html: String) -> R<SaveResult> {
    if html.len() > MAX_REPORT_BYTES {
        return Err("report too large".into());
    }
    if !html.starts_with("<!DOCTYPE html>") {
        return Err("not a report document".into());
    }
    let name = format!("buslink-report-{}.html", state.session.session_id());
    tauri::async_runtime::spawn_blocking(move || save(&app, &name, "HTML", "html", html.as_bytes()))
        .await
        .map_err(|e| e.to_string())?
}
