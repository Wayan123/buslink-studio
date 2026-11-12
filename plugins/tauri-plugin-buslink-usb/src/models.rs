//! Wire types shared with the Kotlin plugin (camelCase JSON). Bytes travel as
//! lowercase hex strings.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsbPortInfo {
    /// Android device node name, e.g. `/dev/bus/usb/001/002`.
    pub device_name: String,
    pub port_index: u32,
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    /// Simple class name of the usb-serial-for-android driver, e.g. `CdcAcmSerialDriver`.
    pub driver: String,
    pub has_permission: bool,
}

impl UsbPortInfo {
    /// Port ID used by the app: `<deviceName>#<portIndex>`.
    pub fn port_id(&self) -> String {
        format!("{}#{}", self.device_name, self.port_index)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    /// No answer within 60 s.
    TimedOut,
    /// Another request is still pending.
    Busy,
    /// The device is gone or the app is not in the foreground.
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOutcome {
    pub state: PermissionState,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    pub device_name: String,
    pub port_index: u32,
    pub baud: u32,
    pub data_bits: u8,
    /// `none`, `even` or `odd`
    pub parity: &'static str,
    pub stop_bits: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedPort {
    pub port_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadState {
    Ok,
    /// `data` is empty and the adapter was unplugged.
    Detached,
    /// `data` is empty and the port was closed.
    Closed,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResult {
    /// One USB packet (or nothing on timeout), hex.
    pub data: String,
    pub state: ReadState,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRequest {
    /// Adapter or link name for the notification text, e.g. "CH343 1A86:55D3".
    pub label: String,
    /// `polling`, `stopped` or `disconnected`; the Kotlin side words it in the system language.
    pub state: &'static str,
    pub devices: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationState {
    Granted,
    Denied,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub running: bool,
    pub notifications: NotificationState,
    /// Last foreground-service or USB event, text for the log (detach, error).
    pub last_event: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRequest {
    /// File name inside the plugin export cache directory.
    pub cache_file_name: String,
    /// Suggested name in the picker.
    pub display_name: String,
    pub mime: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOutcome {
    pub saved: bool,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDir {
    pub path: String,
}
