//! Interface choice and bind rules for the LAN dashboard (R22, safety rule 15).
//!
//! Only private IPv4 addresses are ever offered or accepted. `0.0.0.0`, public,
//! link-local, multicast and IPv6 addresses are refused. Loopback is accepted
//! only when the caller opts in (tests and the e2e build); it is never listed.

use qrcode::{Color, QrCode};
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use thiserror::Error;

pub const DEFAULT_PORT: u16 = 8443;
pub const MIN_PORT: u16 = 1024;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BindError {
    #[error("only private IPv4 addresses (10.x, 172.16-31.x, 192.168.x) can be shared")]
    NotPrivate,
    #[error("port must be between {MIN_PORT} and 65535")]
    BadPort,
}

/// One selectable interface address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Interface {
    pub name: String,
    pub ip: Ipv4Addr,
}

/// True for the RFC 1918 ranges.
pub fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// Validate the address and port the user picked.
pub fn check_bind(ip: IpAddr, port: u16, allow_loopback: bool) -> Result<SocketAddr, BindError> {
    let IpAddr::V4(v4) = ip else {
        return Err(BindError::NotPrivate);
    };
    let ok = is_private_v4(v4) || (allow_loopback && v4.is_loopback());
    if !ok || v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast() {
        return Err(BindError::NotPrivate);
    }
    // Port 0 (ephemeral) exists for tests, behind the same opt-in as loopback.
    if port < MIN_PORT && !(allow_loopback && port == 0) {
        return Err(BindError::BadPort);
    }
    Ok(SocketAddr::new(ip, port))
}

/// Private IPv4 addresses of this machine, sorted by name then address.
pub fn private_interfaces() -> Vec<Interface> {
    let mut out: Vec<Interface> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) if is_private_v4(v4) && !i.is_loopback() => Some(Interface { name: i.name, ip: v4 }),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.ip).cmp(&(&b.name, b.ip)));
    out.dedup();
    out
}

/// A QR code as rows of `'1'` (dark) and `'0'` (light), without quiet zone.
/// The UI draws it on a canvas; no HTML or SVG string is produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct QrMatrix {
    pub size: usize,
    pub rows: Vec<String>,
}

pub fn qr_matrix(text: &str) -> Option<QrMatrix> {
    let code = QrCode::new(text.as_bytes()).ok()?;
    let size = code.width();
    let colors = code.to_colors();
    let rows = colors
        .chunks(size)
        .map(|row| row.iter().map(|c| if *c == Color::Dark { '1' } else { '0' }).collect())
        .collect();
    Some(QrMatrix { size, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::check;

    /// L7: only RFC 1918 (and loopback on request) binds; nothing else, ever.
    #[test]
    fn l7_bind_accepts_exactly_private_ranges() {
        check("l7_bind", 500, |rng| {
            let ip = Ipv4Addr::from(rng.next_u32());
            let port = 1024 + rng.below(60_000) as u16;
            let private = ip.octets()[0] == 10
                || (ip.octets()[0] == 172 && (16..=31).contains(&ip.octets()[1]))
                || (ip.octets()[0] == 192 && ip.octets()[1] == 168);
            assert_eq!(check_bind(IpAddr::V4(ip), port, false).is_ok(), private, "{ip}");
            let with_loop = check_bind(IpAddr::V4(ip), port, true).is_ok();
            assert_eq!(with_loop, private || ip.is_loopback(), "{ip}");
        });
    }

    #[test]
    fn refuses_unspecified_v6_and_low_ports() {
        let any = IpAddr::from([0, 0, 0, 0]);
        assert_eq!(check_bind(any, 8443, true), Err(BindError::NotPrivate));
        assert_eq!(check_bind(any, 8443, false), Err(BindError::NotPrivate));
        assert_eq!(
            check_bind(IpAddr::from([0u16; 8]), 8443, true),
            Err(BindError::NotPrivate)
        );
        assert_eq!(
            check_bind(IpAddr::from([239, 1, 1, 1]), 8443, true),
            Err(BindError::NotPrivate)
        );
        assert_eq!(
            check_bind(IpAddr::from([169, 254, 1, 1]), 8443, true),
            Err(BindError::NotPrivate)
        );
        let lan = IpAddr::from([192, 168, 1, 5]);
        assert_eq!(check_bind(lan, 80, false), Err(BindError::BadPort));
        assert_eq!(check_bind(lan, 0, false), Err(BindError::BadPort));
        assert!(check_bind(lan, DEFAULT_PORT, false).is_ok());
    }

    #[test]
    fn listed_interfaces_are_private_and_not_loopback() {
        for i in private_interfaces() {
            assert!(is_private_v4(i.ip) && !i.ip.is_loopback());
        }
    }

    #[test]
    fn qr_matrix_is_square_and_binary() {
        let q = qr_matrix("https://192.168.1.5:8443/?t=0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(q.rows.len(), q.size);
        assert!(
            q.rows
                .iter()
                .all(|r| r.len() == q.size && r.bytes().all(|c| c == b'0' || c == b'1'))
        );
    }
}
