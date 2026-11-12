//! Android: typed calls into `BusLinkUsbPlugin.kt`. Every call blocks until
//! Kotlin resolves it, so call from a worker thread (never the main thread:
//! the permission dialog result is delivered on it).

use crate::Result;
use crate::models::*;
use serde::Serialize;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

const PLUGIN_IDENTIFIER: &str = "id.co.arkamaya.buslink.usb";

pub fn init<R: Runtime, C: serde::de::DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<UsbSerial<R>> {
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "BusLinkUsbPlugin")?;
    Ok(UsbSerial(handle))
}

pub struct UsbSerial<R: Runtime>(PluginHandle<R>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceArg<'a> {
    device_name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadArg<'a> {
    port_id: &'a str,
    max_bytes: u32,
    timeout_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteArg<'a> {
    port_id: &'a str,
    data: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PortArg<'a> {
    port_id: &'a str,
}

impl<R: Runtime> UsbSerial<R> {
    pub fn list_devices(&self) -> Result<Vec<UsbPortInfo>> {
        #[derive(serde::Deserialize)]
        struct Wrap {
            ports: Vec<UsbPortInfo>,
        }
        let w: Wrap = self.0.run_mobile_plugin("listDevices", ())?;
        Ok(w.ports)
    }

    /// Blocks until the user answers, 60 s at most.
    pub fn request_permission(&self, device_name: &str) -> Result<PermissionOutcome> {
        Ok(self
            .0
            .run_mobile_plugin("requestPermission", DeviceArg { device_name })?)
    }

    pub fn open(&self, req: &OpenRequest) -> Result<OpenedPort> {
        Ok(self.0.run_mobile_plugin("openPort", req)?)
    }

    /// One USB packet (at most `max_bytes`) or an empty result after `timeout_ms`.
    pub fn read(&self, port_id: &str, max_bytes: u32, timeout_ms: u64) -> Result<ReadResult> {
        Ok(self.0.run_mobile_plugin(
            "readPort",
            ReadArg {
                port_id,
                max_bytes,
                timeout_ms,
            },
        )?)
    }

    /// `hex` holds bytes the core already encoded.
    pub fn write(&self, port_id: &str, hex: &str) -> Result<()> {
        Ok(self.0.run_mobile_plugin("writePort", WriteArg { port_id, data: hex })?)
    }

    pub fn close(&self, port_id: &str) -> Result<()> {
        Ok(self.0.run_mobile_plugin("closePort", PortArg { port_id })?)
    }

    /// Android 13+ runtime prompt; a no-op grant on older versions.
    pub fn request_notification_permission(&self) -> Result<NotificationState> {
        #[derive(serde::Deserialize)]
        struct Wrap {
            notifications: NotificationState,
        }
        let w: Wrap = self.0.run_mobile_plugin("requestNotificationPermission", ())?;
        Ok(w.notifications)
    }

    pub fn start_service(&self, req: &ServiceRequest) -> Result<()> {
        Ok(self.0.run_mobile_plugin("startService", req)?)
    }

    pub fn update_service(&self, req: &ServiceRequest) -> Result<()> {
        Ok(self.0.run_mobile_plugin("updateService", req)?)
    }

    pub fn stop_service(&self) -> Result<()> {
        Ok(self.0.run_mobile_plugin("stopService", ())?)
    }

    pub fn service_status(&self) -> Result<ServiceStatus> {
        Ok(self.0.run_mobile_plugin("serviceStatus", ())?)
    }

    pub fn export_dir(&self) -> Result<ExportDir> {
        Ok(self.0.run_mobile_plugin("exportDir", ())?)
    }

    /// Opens the system "save as" picker; blocks until the user chooses or cancels.
    pub fn save_document(&self, req: &SaveRequest) -> Result<SaveOutcome> {
        Ok(self.0.run_mobile_plugin("saveDocument", req)?)
    }
}
