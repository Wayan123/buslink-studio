//! Desktop stub: nothing here talks to hardware.

use crate::models::*;
use crate::{Error, Result};
use std::marker::PhantomData;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

pub fn init<R: Runtime, C: serde::de::DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<UsbSerial<R>> {
    Ok(UsbSerial(PhantomData))
}

pub struct UsbSerial<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> UsbSerial<R> {
    pub fn list_devices(&self) -> Result<Vec<UsbPortInfo>> {
        Err(Error::Unsupported)
    }
    pub fn request_permission(&self, _device_name: &str) -> Result<PermissionOutcome> {
        Err(Error::Unsupported)
    }
    pub fn open(&self, _req: &OpenRequest) -> Result<OpenedPort> {
        Err(Error::Unsupported)
    }
    pub fn read(&self, _port_id: &str, _max_bytes: u32, _timeout_ms: u64) -> Result<ReadResult> {
        Err(Error::Unsupported)
    }
    pub fn write(&self, _port_id: &str, _hex: &str) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn close(&self, _port_id: &str) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn request_notification_permission(&self) -> Result<NotificationState> {
        Err(Error::Unsupported)
    }
    pub fn start_service(&self, _req: &ServiceRequest) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn update_service(&self, _req: &ServiceRequest) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn stop_service(&self) -> Result<()> {
        Err(Error::Unsupported)
    }
    pub fn service_status(&self) -> Result<ServiceStatus> {
        Err(Error::Unsupported)
    }
    pub fn export_dir(&self) -> Result<ExportDir> {
        Err(Error::Unsupported)
    }
    pub fn save_document(&self, _req: &SaveRequest) -> Result<SaveOutcome> {
        Err(Error::Unsupported)
    }
}
