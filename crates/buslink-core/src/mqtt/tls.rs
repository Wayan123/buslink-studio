//! Certificate verification for the broker connection (research/03 rows 56–57).
//!
//! Desktop: the operating system's roots (`rustls-native-certs`) plus an optional
//! user CA. Android: `rustls-platform-verifier` (needs its Kotlin component, see
//! docs/android-build.md); with a user CA the connection trusts only that CA.

use super::MqttError;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use std::sync::Arc;

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn parse_pem(pem: &str) -> Result<Vec<CertificateDer<'static>>, MqttError> {
    if pem.trim().is_empty() {
        return Ok(Vec::new());
    }
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|e| MqttError::Tls(format!("the CA PEM could not be read: {e}")))?;
    if certs.is_empty() {
        return Err(MqttError::Tls("the CA PEM contains no certificate".into()));
    }
    Ok(certs)
}

fn tls_err(e: rustls::Error) -> MqttError {
    MqttError::Tls(e.to_string())
}

/// Roots made only of the user's CA (private broker).
fn user_roots(extra: Vec<CertificateDer<'static>>) -> Result<RootCertStore, MqttError> {
    let mut roots = RootCertStore::empty();
    for c in extra {
        roots
            .add(c)
            .map_err(|e| MqttError::Tls(format!("the CA certificate is not usable: {e}")))?;
    }
    Ok(roots)
}

#[cfg(not(target_os = "android"))]
pub fn client_config(extra_ca_pem: &str) -> Result<Arc<ClientConfig>, MqttError> {
    let extra = parse_pem(extra_ca_pem)?;
    let mut roots = user_roots(extra)?;
    let native = rustls_native_certs::load_native_certs();
    for c in native.certs {
        // A few platform roots can be unparsable; skip them rather than fail.
        let _ = roots.add(c);
    }
    if roots.is_empty() {
        return Err(MqttError::Tls(
            "no trusted root certificates were found on this system; add a CA certificate".into(),
        ));
    }
    let cfg = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(tls_err)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

#[cfg(target_os = "android")]
pub fn client_config(extra_ca_pem: &str) -> Result<Arc<ClientConfig>, MqttError> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let extra = parse_pem(extra_ca_pem)?;
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(tls_err)?;
    let cfg = if extra.is_empty() {
        builder.with_platform_verifier().map_err(tls_err)?.with_no_client_auth()
    } else {
        builder.with_root_certificates(user_roots(extra)?).with_no_client_auth()
    };
    Ok(Arc::new(cfg))
}
