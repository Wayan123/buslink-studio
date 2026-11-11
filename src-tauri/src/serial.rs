//! Desktop serial glue (app-shell-desktop Req 2): port listing with VID:PID
//! chip detection, and a `Transport` over the `serialport` crate.
//! Adapters are identified by VID:PID only, never by friendly name.

use buslink_core::protocol::rtu::SerialSettings;
use buslink_core::transport::usb_ids::{UsbChip, classify, vid_pid_label};
use buslink_core::transport::{Transport, TransportError};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct PortView {
    pub name: String,
    /// `usb`, `pci`, `bluetooth` or `unknown`
    pub kind: &'static str,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub vid_pid: Option<String>,
    pub chip: Option<UsbChip>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

impl PortView {
    fn usb(
        name: String,
        vid: u16,
        pid: u16,
        manufacturer: Option<String>,
        product: Option<String>,
        serial: Option<String>,
    ) -> Self {
        Self {
            name,
            kind: "usb",
            vid: Some(vid),
            pid: Some(pid),
            vid_pid: Some(vid_pid_label(vid, pid)),
            chip: classify(vid, pid),
            manufacturer,
            product,
            serial_number: serial,
        }
    }

    fn other(name: String, kind: &'static str) -> Self {
        Self {
            name,
            kind,
            vid: None,
            pid: None,
            vid_pid: None,
            chip: None,
            manufacturer: None,
            product: None,
            serial_number: None,
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use serialport::{DataBits, ErrorKind, FlowControl, Parity as SpParity, SerialPort, SerialPortType, StopBits};
    use std::io::{Read, Write};
    use std::time::Duration;

    pub fn list() -> Result<Vec<PortView>, String> {
        let ports = serialport::available_ports().map_err(|e| e.to_string())?;
        Ok(ports
            .into_iter()
            .map(|p| match p.port_type {
                SerialPortType::UsbPort(u) => {
                    PortView::usb(p.port_name, u.vid, u.pid, u.manufacturer, u.product, u.serial_number)
                }
                SerialPortType::PciPort => PortView::other(p.port_name, "pci"),
                SerialPortType::BluetoothPort => PortView::other(p.port_name, "bluetooth"),
                SerialPortType::Unknown => PortView::other(p.port_name, "unknown"),
            })
            .collect())
    }

    pub struct SerialTransport {
        port: Option<Box<dyn SerialPort>>,
        timeout_ms: u64,
    }

    fn lost(e: impl ToString) -> TransportError {
        TransportError::Disconnected(e.to_string())
    }

    pub fn open(name: &str, s: SerialSettings) -> Result<Box<dyn Transport>, TransportError> {
        s.validate().map_err(|e| TransportError::Io(e.into()))?;
        let data_bits = if s.data_bits == 7 {
            DataBits::Seven
        } else {
            DataBits::Eight
        };
        let parity = match s.parity {
            buslink_core::protocol::rtu::Parity::None => SpParity::None,
            buslink_core::protocol::rtu::Parity::Even => SpParity::Even,
            buslink_core::protocol::rtu::Parity::Odd => SpParity::Odd,
        };
        let stop_bits = if s.stop_bits == 2 { StopBits::Two } else { StopBits::One };
        let port = serialport::new(name, s.baud)
            .data_bits(data_bits)
            .parity(parity)
            .stop_bits(stop_bits)
            .flow_control(FlowControl::None)
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| match e.kind {
                ErrorKind::NoDevice => TransportError::Disconnected(format!("port {name} not found")),
                ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => TransportError::Io(format!(
                    "port {name} is busy or access was denied (another program may own it)"
                )),
                _ if e.description.to_ascii_lowercase().contains("access is denied") => TransportError::Io(format!(
                    "port {name} is busy or access was denied (another program may own it)"
                )),
                _ => TransportError::Io(format!("{name}: {}", e.description)),
            })?;
        Ok(Box::new(SerialTransport {
            port: Some(port),
            timeout_ms: 50,
        }))
    }

    impl Transport for SerialTransport {
        fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            let port = self.port.as_mut().ok_or_else(|| lost("port closed"))?;
            port.write_all(bytes).map_err(lost)?;
            port.flush().map_err(lost)
        }

        fn read(&mut self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
            let port = self.port.as_mut().ok_or_else(|| lost("port closed"))?;
            let t = timeout_ms.max(1);
            if t != self.timeout_ms {
                port.set_timeout(Duration::from_millis(t)).map_err(lost)?;
                self.timeout_ms = t;
            }
            match port.read(buf) {
                Ok(n) => Ok(n),
                // A timed-out read is "no bytes yet", not end of stream.
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(0),
                Err(e) => Err(lost(format!("serial port error: {e}"))),
            }
        }

        fn close(&mut self) {
            self.port = None;
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn list() -> Result<Vec<PortView>, String> {
        Ok(Vec::new())
    }

    pub fn open(_name: &str, _s: SerialSettings) -> Result<Box<dyn Transport>, TransportError> {
        Err(TransportError::Io(
            "serial ports are not available in this build (Windows only for now)".into(),
        ))
    }
}

pub use imp::{list, open};
