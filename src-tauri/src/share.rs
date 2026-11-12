//! IPC for the LAN dashboard and MQTT telemetry (lan-mqtt design §5).
//!
//! Both are off at every start. Nothing here carries a frame, an address or a
//! function code, and nothing here writes to a device. The MQTT password goes to
//! the secret store only; it is never saved in a file, logged or returned.

use crate::state::AppState;
use buslink_core::lan::net::{self, DEFAULT_PORT, Interface};
use buslink_core::lan::{Asset, AssetSource, LanConfig, LanServer, LanStatus};
#[cfg(not(windows))]
use buslink_core::mqtt::MemorySecretStore;
use buslink_core::mqtt::{MqttConfig, MqttInit, MqttService, MqttStatus, Secret, SecretStore, TestReport, random_client_id, test_connection};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::async_runtime::Mutex;
use tauri::{AppHandle, Manager, State};

type R<T> = Result<T, String>;

const PASSWORD_KEY: &str = "mqtt-password";
const CONFIG_FILE: &str = "mqtt.json";
const OUTBOX_FILE: &str = "buslink-mqtt-outbox.sqlite3";
#[cfg(windows)]
const KEYRING_SERVICE: &str = "id.co.arkamaya.buslink";

/// Static files of the frontend build, from the Tauri asset resolver (the same files the window shows).
struct TauriAssets(AppHandle);

impl AssetSource for TauriAssets {
    fn get(&self, path: &str) -> Option<Asset> {
        let a = self.0.asset_resolver().get(path.to_string())?;
        // The resolver answers unknown paths with index.html (SPA fallback). Only `index.html`
        // itself may be HTML here, so an unknown path stays a 404.
        if a.mime_type.starts_with("text/html") && path != "index.html" {
            return None;
        }
        Some(Asset {
            mime: a.mime_type,
            bytes: a.bytes,
        })
    }
}

/// Windows Credential Manager.
#[cfg(windows)]
struct WindowsSecrets;

#[cfg(windows)]
impl SecretStore for WindowsSecrets {
    fn get(&self, key: &str) -> Result<Option<Secret>, buslink_core::mqtt::secrets::SecretError> {
        use buslink_core::mqtt::secrets::SecretError;
        let e = keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| SecretError(e.to_string()))?;
        match e.get_password() {
            Ok(p) => Ok(Some(Secret::new(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &Secret) -> Result<(), buslink_core::mqtt::secrets::SecretError> {
        use buslink_core::mqtt::secrets::SecretError;
        keyring::Entry::new(KEYRING_SERVICE, key)
            .and_then(|e| e.set_password(value.expose()))
            .map_err(|e| SecretError(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), buslink_core::mqtt::secrets::SecretError> {
        use buslink_core::mqtt::secrets::SecretError;
        match keyring::Entry::new(KEYRING_SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }

    fn kind(&self) -> &'static str {
        "windows-credential-manager"
    }

    fn persistent(&self) -> bool {
        true
    }
}

/// Windows Credential Manager on Windows; memory only elsewhere (Android Keystore is not wired yet).
pub fn secret_store() -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        Arc::new(WindowsSecrets)
    }
    #[cfg(not(windows))]
    {
        Arc::new(MemorySecretStore::default())
    }
}

/// What the Share and MQTT panels need besides the Core state.
pub struct ShareState {
    lan: Mutex<Option<LanServer>>,
    mqtt: Mutex<Option<MqttService>>,
    secrets: Arc<dyn SecretStore>,
}

impl ShareState {
    pub fn new() -> Self {
        Self {
            lan: Mutex::new(None),
            mqtt: Mutex::new(None),
            secrets: secret_store(),
        }
    }

    /// Close sharing and MQTT (app exit). Blocking; called from the exit handler.
    pub fn shutdown(&self) {
        tauri::async_runtime::block_on(async {
            if let Some(s) = self.lan.lock().await.take() {
                s.stop().await;
            }
            if let Some(m) = self.mqtt.lock().await.take() {
                m.stop().await;
            }
        });
    }
}

impl Default for ShareState {
    fn default() -> Self {
        Self::new()
    }
}

fn share(app: &AppHandle) -> R<tauri::State<'_, ShareState>> {
    app.try_state::<ShareState>().ok_or_else(|| "sharing is not available".to_string())
}

// ---- LAN ----

/// Private IPv4 addresses (and loopback in the verification build only).
fn interfaces() -> Vec<Interface> {
    #[allow(unused_mut)]
    let mut list = net::private_interfaces();
    #[cfg(feature = "e2e")]
    list.push(Interface {
        name: "loopback (verification build)".into(),
        ip: Ipv4Addr::LOCALHOST,
    });
    list
}

#[derive(Serialize)]
pub struct LanInterfaces {
    interfaces: Vec<Interface>,
    default_port: u16,
}

#[tauri::command]
pub fn lan_interfaces() -> LanInterfaces {
    LanInterfaces {
        interfaces: interfaces(),
        default_port: DEFAULT_PORT,
    }
}

#[derive(Deserialize)]
pub struct LanStartReq {
    ip: String,
    port: u16,
}

#[tauri::command]
pub async fn lan_start(app: AppHandle, state: State<'_, AppState>, req: LanStartReq) -> R<LanStatus> {
    let sh = share(&app)?;
    let ip: Ipv4Addr = req.ip.parse().map_err(|_| "not an IPv4 address".to_string())?;
    // Only an address this machine actually has, from the private list.
    if !interfaces().iter().any(|i| i.ip == ip) {
        return Err("choose one of the listed private network addresses".into());
    }
    let mut guard = sh.lan.lock().await;
    if guard.is_some() {
        return Err("sharing is already on; stop it first".into());
    }
    let mut cfg = LanConfig::new(IpAddr::V4(ip), req.port);
    cfg.allow_loopback = cfg!(feature = "e2e");
    let server = LanServer::start(cfg, state.session.clone(), Arc::new(TauriAssets(app.clone())))
        .await
        .map_err(|e| e.to_string())?;
    let status = server.status();
    *guard = Some(server);
    Ok(status)
}

#[tauri::command]
pub async fn lan_stop(app: AppHandle) -> R<LanStatus> {
    let sh = share(&app)?;
    if let Some(s) = sh.lan.lock().await.take() {
        s.stop().await;
    }
    Ok(LanStatus::off())
}

#[tauri::command]
pub async fn lan_status(app: AppHandle) -> R<LanStatus> {
    let sh = share(&app)?;
    Ok(sh.lan.lock().await.as_ref().map(LanServer::status).unwrap_or_else(LanStatus::off))
}

// ---- MQTT ----

fn data_dir(app: &AppHandle) -> R<PathBuf> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

fn load_config(app: &AppHandle) -> R<MqttConfig> {
    let path = data_dir(app)?.join(CONFIG_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{} is not valid: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(MqttConfig {
            client_id: random_client_id(),
            ..MqttConfig::default()
        }),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

fn save_config(app: &AppHandle, cfg: &MqttConfig) -> R<()> {
    let path = data_dir(app)?.join(CONFIG_FILE);
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[derive(Serialize)]
pub struct MqttView {
    config: MqttConfig,
    /// A password is stored; its value is never returned.
    has_password: bool,
    secret_store: &'static str,
    secret_persistent: bool,
    status: MqttStatus,
}

async fn view(app: &AppHandle, sh: &ShareState) -> R<MqttView> {
    Ok(MqttView {
        config: load_config(app)?,
        has_password: sh.secrets.get(PASSWORD_KEY).ok().flatten().is_some(),
        secret_store: sh.secrets.kind(),
        secret_persistent: sh.secrets.persistent(),
        status: sh.mqtt.lock().await.as_ref().map(MqttService::status).unwrap_or_else(MqttStatus::off),
    })
}

#[tauri::command]
pub async fn mqtt_get(app: AppHandle) -> R<MqttView> {
    let sh = share(&app)?;
    view(&app, &sh).await
}

#[tauri::command]
pub async fn mqtt_status(app: AppHandle) -> R<MqttStatus> {
    let sh = share(&app)?;
    Ok(sh.mqtt.lock().await.as_ref().map(MqttService::status).unwrap_or_else(MqttStatus::off))
}

/// Save settings (never a password). Invalid settings are refused with a text for the field.
#[tauri::command]
pub async fn mqtt_save_config(app: AppHandle, config: MqttConfig) -> R<MqttConfig> {
    config.validate().map_err(|e| e.to_string())?;
    save_config(&app, &config)?;
    Ok(config)
}

#[tauri::command]
pub async fn mqtt_set_password(app: AppHandle, password: String) -> R<()> {
    let sh = share(&app)?;
    if password.is_empty() {
        return Err("the password is empty; use Remove password".into());
    }
    sh.secrets.set(PASSWORD_KEY, &Secret::new(password)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mqtt_clear_password(app: AppHandle) -> R<()> {
    let sh = share(&app)?;
    sh.secrets.delete(PASSWORD_KEY).map_err(|e| e.to_string())
}

/// Connect once with the given settings. A typed password is used for this call only.
#[tauri::command]
pub async fn mqtt_test(app: AppHandle, config: MqttConfig, password: Option<String>) -> R<TestReport> {
    let sh = share(&app)?;
    let pw = match password.filter(|p| !p.is_empty()) {
        Some(p) => Some(Secret::new(p)),
        None => sh.secrets.get(PASSWORD_KEY).map_err(|e| e.to_string())?,
    };
    Ok(test_connection(&config, pw.as_ref(), None).await)
}

#[tauri::command]
pub async fn mqtt_start(app: AppHandle, state: State<'_, AppState>) -> R<MqttStatus> {
    let sh = share(&app)?;
    let mut guard = sh.mqtt.lock().await;
    if guard.is_some() {
        return Err("MQTT is already on".into());
    }
    let cfg = load_config(&app)?;
    cfg.validate().map_err(|e| e.to_string())?;
    let password = sh.secrets.get(PASSWORD_KEY).map_err(|e| e.to_string())?;
    let svc = MqttService::start(
        MqttInit {
            cfg,
            password,
            session_id: state.session.session_id().to_owned(),
            outbox_path: Some(data_dir(&app)?.join(OUTBOX_FILE)),
            tls: None,
        },
        state.session.subscribe(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let status = svc.status();
    *guard = Some(svc);
    Ok(status)
}

#[tauri::command]
pub async fn mqtt_stop(app: AppHandle) -> R<MqttStatus> {
    let sh = share(&app)?;
    if let Some(m) = sh.mqtt.lock().await.take() {
        m.stop().await;
    }
    Ok(MqttStatus::off())
}
