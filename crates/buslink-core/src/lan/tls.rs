//! Per-session self-signed certificate (rcgen) and the rustls server config.
//! The key never leaves memory: it lives in the server value and is dropped on stop.

use rcgen::{
    CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType, date_time_ymd,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::IpAddr;
use std::sync::Arc;
use thiserror::Error;

/// Validity of a session certificate.
pub const VALID_DAYS: i64 = 30;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("could not create the certificate: {0}")]
    Cert(String),
    #[error("could not build the TLS configuration: {0}")]
    Config(String),
}

pub struct SessionCert {
    pub cert: CertificateDer<'static>,
    key: Vec<u8>,
    /// SHA-256 of the DER certificate, `AA:BB:…`.
    pub fingerprint: String,
}

/// Civil date (year, month, day) of a day count since 1970-01-01 (Howard Hinnant's algorithm).
pub fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u8, d as u8)
}

pub fn fingerprint(der: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, der);
    d.as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn generate(ip: IpAddr, now_ms: i64) -> Result<SessionCert, TlsError> {
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| TlsError::Cert(e.to_string()))?;
    params.subject_alt_names = vec![SanType::IpAddress(ip)];
    params
        .distinguished_name
        .push(DnType::CommonName, "BusLink Studio LAN view");
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let date = |plus_days: i64| {
        let (y, m, d) = civil_from_days(now_ms.div_euclid(86_400_000) + plus_days);
        date_time_ymd(y, m, d)
    };
    params.not_before = date(-1);
    params.not_after = date(VALID_DAYS);
    let key = KeyPair::generate().map_err(|e| TlsError::Cert(e.to_string()))?;
    let cert = params.self_signed(&key).map_err(|e| TlsError::Cert(e.to_string()))?;
    let der = cert.der().clone();
    Ok(SessionCert {
        fingerprint: fingerprint(der.as_ref()),
        cert: der,
        key: key.serialize_der(),
    })
}

pub fn server_config(c: &SessionCert) -> Result<Arc<ServerConfig>, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(c.key.clone()));
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![c.cert.clone()], key)
        .map_err(|e| TlsError::Config(e.to_string()))?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_513), (2026, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn certificates_differ_per_session_and_config_builds() {
        let ip: IpAddr = "192.168.1.20".parse().unwrap();
        let a = generate(ip, 1_800_000_000_000).unwrap();
        let b = generate(ip, 1_800_000_000_000).unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 32 * 2 + 31);
        server_config(&a).unwrap();
    }
}
