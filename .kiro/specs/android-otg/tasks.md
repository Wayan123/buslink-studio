# android-otg: tasks

Depends on core-protocol, simulator, app-shell-desktop, dashboard-ui and commissioning-control (all done). Checks: `cargo test --workspace` (WSL) after core tasks; `cargo.exe check --manifest-path src-tauri/Cargo.toml` after shell tasks; `npx tsc --noEmit` and `node scripts/check-i18n.mjs` after UI tasks; the APK build for Android tasks. A task is `[x]` only when its checks pass. Nothing here is marked as hardware-verified.

- [x] 1. USB bridge in the core
  - `transport/usb.rs`: `UsbPort`, `BufferedUsb`, `to_hex`/`from_hex`, `FakeUsbPort` + `UsbScript`.
  - Tests: small-buffer pieces, coalesced packets, timeout = `Ok(0)`, detach clears and sticks, overflow counted, close, drain, hex round trip.
  - _Requirements: 4.2–4.5, 4.8_

- [x] 2. RTU over the bridge (fake USB transport)
  - `tests/usb_transport.rs`: reply cut at every byte position, byte-per-packet, reply + stray bytes in one packet, silence = TIMEOUT, bad CRC in fragments = CRC_ERROR, detach mid-exchange = link lost, late reply after timeout not matched.
  - _Requirements: 4.3, 4.4, 5.1_ · Properties A1–A5

- [x] 3. Plugin crate (Rust)
  - `plugins/tauri-plugin-buslink-usb`: models, `UsbSerial` (mobile) and desktop stub, `UsbSerialExt`, empty command list, own `[workspace]`, exact pins.
  - Checks: `cargo.exe check` / `clippy -D warnings` (plugin and app).
  - _Requirements: 9.2_

- [x] 4. Plugin (Kotlin)
  - `BusLinkUsbPlugin` (enumeration, permission, open/read/write/close, notification permission, service control, SAF save), `UsbRuntime`, `SerialSession`, `PacketQueue`, `Hex`, `PollingService`; manifest (service, permissions), strings en/id, notification icon, `consumer-rules.pro`.
  - Tests: `PacketQueueTest` (JVM).
  - _Requirements: 2.1–2.6, 3.1–3.5, 4.1, 4.2, 4.8, 5.1, 6.1, 6.3–6.6, 8.1–8.4_

- [x] 5. App glue
  - `android_usb.rs` (list, permission-then-open, `AndroidUsbPort`, `ServiceSync`, `start_service`), `files.rs` (dialog on desktop, SAF on Android), `serial.rs` per platform, async `connect_link`/`list_ports`/`start_polling`, 50 ms inter-frame floor, emitter-driven service sync, `mobile_entry_point`, dialog plugin desktop-only.
  - UI: Android note, permission label in the port list, `StartResult` messages, id/en strings.
  - Checks: `cargo.exe check` + `clippy -D warnings` + `cargo.exe test --lib`; `tsc`, i18n and UI checks.
  - _Requirements: 3.4, 4.6, 6.2, 6.7, 7.1–7.3, 8.5, 9.1_

- [x] 6. Android target and debug APK
  - `tauri android init` (identifier `id.co.arkamaya.buslink`), minSdk 24, targetSdk 36, aarch64; manifest, `device_filter.xml`, JitPack, hand edits listed in `docs/android-build.md`.
  - Debug APK copied to `release/BusLink-Studio-<version>-android-arm64-debug.apk` with SHA-256.
  - _Requirements: 1.1–1.5, 2.5_

- [x] 7. Documents
  - `docs/android-build.md`, `docs/verification/android-hardware.md` (manual procedure, result cells empty), `docs/existing-code-disclosure.md` rows, `THIRD_PARTY_NOTICES.md`.
  - _Requirements: 1.4, 9.4_
