# Dependency audit: TLS stack (2026-11-13)

Source: independent QA run `qa-2026-11-13`, `cargo audit` (RustSec database, 1490 advisories).

## Fixed

`rustls 0.23.40`: RUSTSEC-2026-0285, -0346, -0348, -0349. `crates/buslink-core/Cargo.toml` pins `rustls =0.23.46` (uses `rustls-webpki 0.103.16`, patched). `Cargo.lock` updated.

## Ignored (`.cargo/audit.toml`)

| Crate | Advisories |
|---|---|
| `rustls-webpki 0.102.8` | RUSTSEC-2026-0049, -0098, -0099, -0104, -0382 |
| `rustls-pemfile 2.2.0` (unmaintained) | RUSTSEC-2025-0134 |

Both come only through `rumqttc 0.25.1` (feature `use-rustls-no-provider`); `cargo tree -i rustls-webpki@0.102.8` shows `rumqttc` as the sole dependent.

## Reachability argument

- In rumqttc 0.25.1 `src/tls.rs`, `rustls_pemfile` is called only in the `TlsConfiguration::Simple` branch of `rustls_connector` (CA, client cert and key parsing). The `webpki` reference is the error type for the same path.
- BusLink builds its own `rustls::ClientConfig` (`mqtt/tls.rs`) and passes it as `TlsConfiguration::Rustls` (`mqtt/service.rs`, `Transport::tls_with_config`). In rumqttc that branch returns the config unchanged. `TlsConfiguration::Simple` is never constructed in this repo (grep: no match).
- Certificate verification therefore runs in rustls 0.23.46 with rustls-webpki 0.103.16.

Not verified: the vulnerable code is still compiled and linked, only not called. A future change that uses `Simple` would reopen the exposure, so the call site carries a comment.

## Upstream status

rumqttc latest is 0.25.1 (2025-11-21). No release with webpki >= 0.103.16 yet.

## Removal condition

Drop the `ignore` entries when rumqttc releases a version with `rustls-webpki >= 0.103.16`, then bump the pin and re-run `cargo audit`.
