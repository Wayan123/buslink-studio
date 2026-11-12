//! Android USB glue (android-otg): port listing, permission-then-open, the
//! `UsbPort` adapter over the Kotlin plugin, and the foreground-service sync.
//!
//! Compiled on every platform so `cargo check` on Windows covers it; on desktop
//! the plugin stub returns `Unsupported` and nothing here is reachable.
//! Only the bytes the core already encoded go through `write`: this module
//! builds no Modbus frame (safety rule 2).
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use crate::serial::PortView;
use buslink_core::protocol::rtu::{Parity, SerialSettings};
use buslink_core::transport::usb::{BufferedUsb, UsbPort, from_hex, to_hex};
use buslink_core::transport::{Transport, TransportError};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};
use tauri_plugin_buslink_usb::{
    Error, NotificationState, OpenRequest, PermissionState, ReadState, ServiceRequest, UsbSerialExt,
};

/// Largest chunk asked for per read call; a USB bulk packet is at most 512 bytes.
const READ_MAX: u32 = 512;

/// Baud rates above this lose fragments on Android (research/03 row 37).
pub const MAX_RELIABLE_BAUD: u32 = 115_200;

fn map_err(e: Error) -> TransportError {
    let text = e.to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("disconnected") {
        TransportError::Disconnected(text)
    } else if lower.contains("permission") {
        TransportError::PermissionDenied
    } else {
        TransportError::Io(text)
    }
}

/// `<deviceName>#<portIndex>` → (deviceName, portIndex).
pub fn split_port_id(id: &str) -> Option<(&str, u32)> {
    let (device, index) = id.rsplit_once('#')?;
    if device.is_empty() {
        return None;
    }
    Some((device, index.parse().ok()?))
}

pub fn list<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<PortView>, String> {
    let ports = app.usb_serial().list_devices().map_err(|e| e.to_string())?;
    Ok(ports
        .into_iter()
        .map(|p| {
            let mut v = PortView::usb(
                p.port_id(),
                p.vid,
                p.pid,
                p.manufacturer.clone(),
                p.product.clone(),
                p.serial_number.clone(),
            );
            v.has_permission = Some(p.has_permission);
            v.driver = Some(p.driver.clone());
            v
        })
        .collect())
}

/// One open Kotlin port as a packet source.
pub struct AndroidUsbPort<R: Runtime> {
    app: AppHandle<R>,
    port_id: String,
}

impl<R: Runtime> UsbPort for AndroidUsbPort<R> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.app
            .usb_serial()
            .write(&self.port_id, &to_hex(bytes))
            .map_err(map_err)
    }

    fn read_packet(&mut self, timeout_ms: u64) -> Result<Vec<u8>, TransportError> {
        let r = self
            .app
            .usb_serial()
            .read(&self.port_id, READ_MAX, timeout_ms)
            .map_err(map_err)?;
        match r.state {
            ReadState::Ok => {
                from_hex(&r.data).map_err(|e| TransportError::Io(format!("bad data from USB plugin: {e}")))
            }
            ReadState::Detached => Err(TransportError::Disconnected("USB adapter detached".into())),
            ReadState::Closed => Err(TransportError::Disconnected("USB port closed".into())),
        }
    }

    fn close(&mut self) {
        // Already closed or detached is fine.
        let _ = self.app.usb_serial().close(&self.port_id);
    }
}

fn parity_name(p: Parity) -> &'static str {
    match p {
        Parity::None => "none",
        Parity::Even => "even",
        Parity::Odd => "odd",
    }
}

/// Ask for USB permission if missing (once, no loop), then open the port.
/// Blocking: call from a worker thread, never the main thread (the permission
/// result arrives on the main thread).
pub fn open<R: Runtime>(
    app: &AppHandle<R>,
    port_id: &str,
    s: SerialSettings,
) -> Result<Box<dyn Transport>, TransportError> {
    s.validate().map_err(|e| TransportError::Io(e.into()))?;
    let (device, index) = split_port_id(port_id)
        .ok_or_else(|| TransportError::Io(format!("'{port_id}' is not an Android USB port ID")))?;
    let usb = app.usb_serial();
    let ports = usb.list_devices().map_err(map_err)?;
    let info = ports
        .iter()
        .find(|p| p.device_name == device && p.port_index == index)
        .ok_or_else(|| TransportError::Disconnected(format!("USB adapter {port_id} is not connected")))?;
    if !info.has_permission {
        let outcome = usb.request_permission(device).map_err(map_err)?;
        match outcome.state {
            PermissionState::Granted => {}
            PermissionState::Denied => {
                return Err(TransportError::Io(
                    outcome.message.unwrap_or_else(|| {
                        "USB permission denied. Unplug and replug the adapter, tap Allow, and tick \"Use by default for this USB device\".".into()
                    }),
                ));
            }
            PermissionState::TimedOut | PermissionState::Busy | PermissionState::Unavailable => {
                return Err(TransportError::Io(
                    outcome
                        .message
                        .unwrap_or_else(|| "USB permission was not granted.".into()),
                ));
            }
        }
    }
    let opened = usb
        .open(&OpenRequest {
            device_name: device.to_owned(),
            port_index: index,
            baud: s.baud,
            data_bits: s.data_bits,
            parity: parity_name(s.parity),
            stop_bits: s.stop_bits,
        })
        .map_err(map_err)?;
    Ok(Box::new(BufferedUsb::new(AndroidUsbPort {
        app: app.clone(),
        port_id: opened.port_id,
    })))
}

/// Foreground-service bookkeeping owned by the emitter thread.
pub struct ServiceSync {
    last: Option<(String, &'static str, u32)>,
    sent_at: Option<Instant>,
}

/// Minimum spacing of notification updates (requirement 6.4).
const UPDATE_EVERY: Duration = Duration::from_secs(5);

impl ServiceSync {
    pub fn new() -> Self {
        Self {
            last: None,
            sent_at: None,
        }
    }

    /// `want` = what the notification should say, or `None` to stop the service.
    pub fn sync<R: Runtime>(&mut self, app: &AppHandle<R>, want: Option<(String, &'static str, u32)>) {
        match want {
            None => {
                if self.last.take().is_some() {
                    let _ = app.usb_serial().stop_service();
                }
                self.sent_at = None;
            }
            Some(w) => {
                let due = self.sent_at.is_none_or(|t| t.elapsed() >= UPDATE_EVERY);
                if due {
                    // The wake lock is refreshed by every update, so send even if unchanged.
                    let _ = app.usb_serial().update_service(&ServiceRequest {
                        label: w.0.clone(),
                        state: w.1,
                        devices: w.2,
                    });
                    self.last = Some(w);
                    self.sent_at = Some(Instant::now());
                }
            }
        }
    }
}

/// Result of Start polling on Android, shown to the user (requirement 6.7).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct BackgroundStatus {
    /// The foreground service is running, so polling continues with the screen off.
    pub background: bool,
    pub notifications_denied: bool,
    pub error: Option<String>,
}

/// Ask for the notification permission (Android 13+) and start the service.
/// Must run while the app is in the foreground, after USB permission is held.
pub fn start_service<R: Runtime>(app: &AppHandle<R>, label: &str, devices: u32) -> BackgroundStatus {
    let usb = app.usb_serial();
    let notifications_denied = matches!(usb.request_notification_permission(), Ok(NotificationState::Denied));
    match usb.start_service(&ServiceRequest {
        label: label.to_owned(),
        state: "polling",
        devices,
    }) {
        Ok(()) => BackgroundStatus {
            background: true,
            notifications_denied,
            error: None,
        },
        Err(e) => {
            // The plugin already stopped the service; no other service type is tried.
            let _ = usb.stop_service();
            BackgroundStatus {
                background: false,
                notifications_denied,
                error: Some(e.to_string()),
            }
        }
    }
}

pub fn stop_service<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.usb_serial().stop_service();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_ids_split_on_the_last_hash() {
        assert_eq!(
            split_port_id("/dev/bus/usb/001/002#0"),
            Some(("/dev/bus/usb/001/002", 0))
        );
        assert_eq!(
            split_port_id("/dev/bus/usb/001/002#1"),
            Some(("/dev/bus/usb/001/002", 1))
        );
        assert_eq!(split_port_id("COM3"), None);
        assert_eq!(split_port_id("#0"), None);
        assert_eq!(split_port_id("dev#x"), None);
    }
}
