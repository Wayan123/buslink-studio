//! BusLink USB plugin: the Android side of the `UsbPort` bridge (architecture D2).
//!
//! The Kotlin half owns the USB port (usb-serial-for-android), the foreground
//! service and the SAF picker. This crate only forwards typed calls; it exposes
//! **no webview command** and **no write command**: bytes in `write` were
//! encoded by the core (poller or command service) before they got here.
//! On desktop every call returns [`Error::Unsupported`], so the app crate
//! compiles and checks on Windows too.

mod error;
mod models;

#[cfg(not(mobile))]
mod desktop;
#[cfg(mobile)]
mod mobile;

#[cfg(not(mobile))]
use desktop as imp;
#[cfg(mobile)]
use mobile as imp;

pub use error::{Error, Result};
pub use imp::UsbSerial;
pub use models::*;

use tauri::{Manager, Runtime, plugin::TauriPlugin};

/// Access to the plugin from any `AppHandle`, `App` or `Window`.
pub trait UsbSerialExt<R: Runtime> {
    fn usb_serial(&self) -> &UsbSerial<R>;
}

impl<R: Runtime, T: Manager<R>> UsbSerialExt<R> for T {
    fn usb_serial(&self) -> &UsbSerial<R> {
        self.state::<UsbSerial<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("buslink-usb")
        .setup(|app, api| {
            let usb = imp::init(app, api)?;
            app.manage(usb);
            Ok(())
        })
        .build()
}
