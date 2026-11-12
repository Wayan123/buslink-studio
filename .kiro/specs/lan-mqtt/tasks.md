# lan-mqtt: tasks

Depends on app-shell-desktop (sample bus, session, historian) and dashboard-ui. After core tasks: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`. After UI tasks: `npx tsc --noEmit`, `node --test "src/**/*.test.ts"`, `node scripts/check-i18n.mjs`, `node scripts/check-ui.mjs`. After shell tasks: `cargo.exe check --manifest-path src-tauri/Cargo.toml`. A task is `[x]` only when its checks pass.

- [x] 1. Dependencies and notices
  - Pinned axum, hyper, hyper-util, tokio-rustls, rustls (ring), rcgen, rumqttc (`use-rustls-no-provider`), if-addrs, qrcode, rustls-native-certs; `THIRD_PARTY_NOTICES.md`.
  - _Requirements: 1.10, 3.2_

- [x] 2. LAN primitives
  - `lan/net.rs`, `token.rs`, `tls.rs`, `limits.rs`.
  - Tests: L2, L4, L7, certificate/fingerprint unit tests.
  - _Requirements: 1.2, 1.3, 1.5, 1.7_

- [x] 3. LAN hub and server
  - `LiveStore::seed`, `lan/hub.rs`, `routes.rs`, `serve.rs`, `mod.rs`, `impl LanSource for Session`.
  - Tests: L1, L3, L5, L6 (`tests/lan_server.rs`, in-memory assets and a bus).
  - _Requirements: 1.1–1.10, 2.1–2.4_

- [x] 4. MQTT primitives
  - `mqtt/config.rs`, `topic.rs`, `payload.rs`, `secrets.rs`, `tls.rs`, `outbox.rs`.
  - Tests: M1, M2, M3, M6 (secret parts), M7.
  - _Requirements: 3.2–3.4, 3.7, 3.9, 3.11_

- [x] 5. MQTT service and broker tests
  - `mqtt/service.rs` (feeder, sender, test connection); `tests/common/broker.rs` (in-process MQTT 5 broker on rumqttc's codec).
  - Tests: M4, M5, M6 (password only in CONNECT).
  - _Requirements: 3.1, 3.5, 3.6, 3.8, 3.10, 3.12, 3.13_

- [x] 6. Tauri shell
  - `src-tauri/src/share.rs`, registration, assets over the asset resolver, secret store, config files, stop on exit; `e2e` loopback allowance.
  - Check: `cargo.exe check`, `cargo.exe clippy`.
  - _Requirements: 1.1, 1.2, 1.9, 3.1, 3.11, 4.4_

- [x] 7. Frontend: LAN mode
  - `src/lan.ts`, `api.ts` branch, `main.ts` LAN shell, dashboard/trends read-only, i18n keys.
  - Tests: unit tests for the LAN nav and the read-only API guard.
  - _Requirements: 2.5, 2.6_

- [x] 8. Frontend: Share and MQTT panels
  - `views/share.ts`, `views/mqtt.ts`, QR canvas, header indicators, i18n en/id.
  - _Requirements: 4.1–4.4_

- [x] 9. Verification
  - `scripts/verify-lan.mjs` (journey 3 with screenshots), run against the e2e build; results in `docs/verification/`; `docs/android-build.md` notes; ledger entry.
  - _Requirements: 5.1, 5.2_
