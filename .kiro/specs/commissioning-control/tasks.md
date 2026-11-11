# commissioning-control: tasks

Depends on core-protocol (command service), simulator, app-shell-desktop and dashboard-ui. After core tasks: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`. After profile changes: `check_profile.py` on all profiles. After UI tasks: `npx tsc --noEmit`, `node --test`, i18n and UI checks. After shell tasks: `cargo.exe check --manifest-path src-tauri/Cargo.toml`. A task is `[x]` only when its checks pass.

- [ ] 1. Profile schema: command `feedback`
  - `feedback { register, tolerance, timeout_ms, value?, bit? }` in `check_profile.py`, `validate.rs`, JSON schema, `docs/profiles/schema-v1.md`, `profile/model.rs`.
  - _Requirements: 2.6_

- [ ] 2. Simulator comm registers and baud
  - `devices/comm.rs`, VFD and temperature controller registers 10/11, world re-key, per-device baud, `SimTransport::rtu_at`, `Training::transport_for`.
  - Profiles `sim-vfd.json` and `sim-temp-controller.json` → 1.1.0 with `comm_config`, `set_ramp`, feedback entries.
  - _Requirements: 6.1–6.3_

- [ ] 3. Command service additions
  - `CommandRequest::CommConfig`, deferred read-back, `journal`/`note` rows, audit events `journal`/`verify`/`feedback`, comm registers rejected outside the wizard, `feedback.rs`.
  - Tests: C1–C4.
  - _Requirements: 1.3, 2.3, 2.5, 2.6, 3.2, 3.8, 7.1_

- [ ] 4. Session integration and `Control`
  - Link generation, `command_access`, `set_device_unit`, `reconfigure_serial`, historian audit sink, audit rows; `command/commission.rs`, `command/control.rs`; `export/audit.rs`, `export/report.rs`.
  - Tests: J1–J3.
  - _Requirements: 1.1–1.5, 3.1–3.7, 4.1–4.4, 5.1, 7.2_

- [ ] 5. Tauri IPC
  - `src-tauri/src/control.rs`, registration, training factory uses the link baud, disarm on window close.
  - _Requirements: 1.4, 2.*, 3.*, 4.3, 5.2_

- [ ] 6. UI: control panel, confirm dialog, wizard
  - `views/control.ts`, `views/commission.ts`, `components/confirm.ts`, i18n keys, nav.
  - _Requirements: 1.2, 1.4, 2.1–2.8, 3.1–3.7_

- [ ] 7. UI: audit view and session report
  - `views/audit.ts`, `views/report.ts`, print CSS.
  - _Requirements: 4.1–4.3, 5.1–5.3_

- [ ] 8. Verification
  - `scripts/verify-control.mjs` (journey 4 on the e2e build); screenshots and results in `docs/verification/`.
  - _Requirements: 7.3_
