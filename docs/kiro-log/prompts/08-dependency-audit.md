Fix two issues found by the independent QA run (`../qa-2026-11-13/`), as the buslink-builder agent. Keep the change small.

## 1. Dependency advisories (cargo audit)
`cargo audit` on this repo reports advisories in the TLS stack:

- `rustls 0.23.40`: RUSTSEC-2026-0285, -0346, -0348, -0349. The orchestrator already bumped the pin in `crates/buslink-core/Cargo.toml` to `=0.23.46` and updated `Cargo.lock`. Keep that, and verify it builds on Windows and Android.
- `rustls-webpki 0.102.8` (RUSTSEC-2026-0049, -0098, -0099, -0104, -0382) and `rustls-pemfile 2.2.0` (unmaintained, RUSTSEC-2025-0134). Both come only through `rumqttc 0.25.1`, feature `use-rustls-no-provider`. BusLink passes its own `rustls::ClientConfig` through `TlsConfiguration::Rustls`, so rumqttc's webpki/pemfile code path (`TlsConfiguration::Simple`) is never called. Certificate verification happens in rustls 0.23.46 with `rustls-webpki 0.103.16` (patched).

Tasks:
1. Confirm in code that BusLink never constructs `TlsConfiguration::Simple`, and add a short comment at the `Transport::tls_with_config` call site saying why.
2. Add `.cargo/audit.toml` with an `[advisories] ignore = [...]` list for exactly those five webpki IDs plus RUSTSEC-2025-0134. Each line gets a comment naming the reason: "rumqttc 0.25.1 pins it; the Simple TLS path is unused; verification runs in rustls 0.23.46 / webpki 0.103.16". Add the condition to remove the ignore: drop it when rumqttc releases with webpki ≥ 0.103.16.
3. Record the decision in `docs/security/dependency-audit.md`: the advisories, the reachability argument, the date, and the upstream status (rumqttc latest is 0.25.1, 2025-11-21).
4. `cargo audit` must exit 0. `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` must pass. `cargo.exe check` for `src-tauri` must be clean.

## 2. Nothing else
Do not change behaviour. The QA run found no functional bug in BusLink: 64/64 black-box checks passed against an independent virtual Modbus device, and 35/35 LAN security checks passed.

Do not commit. Report what you changed and the check results.
