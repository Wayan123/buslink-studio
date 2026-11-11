# app-shell-desktop: tasks

Depends on core-protocol and simulator (all tasks done). Run `cargo test --workspace` after every core task; `cargo.exe check --manifest-path src-tauri/Cargo.toml` after every shell task. A task is `[x]` only when its checks pass.

- [ ] 1. USB IDs and frame tap
  - `transport/usb_ids.rs` (`classify`), `LinkScheduler::set_frame_tap`.
  - Tests: the three WCH IDs, unknown IDs unlabeled, tap sees poll exchanges with request/response bytes.
  - _Requirements: 2.2, 9.1_

- [ ] 2. Trend decimation
  - `trend.rs`; properties T1–T5 [500 each]; 100 000-sample timing check.
  - _Requirements: 7.1–7.4_

- [ ] 3. Session service
  - `session/{mod,live,frames}.rs`: links, shared transport, workers, hub thread, re-sequencing, live store, frames, historian writer, trend, export, inspector.
  - Tests (`buslink-sim/tests/session.rs`): S1–S6.
  - _Requirements: 4.1–4.8, 5.1–5.4, 6.1–6.5, 8.1, 9.1–9.3_

- [ ] 4. Training bus
  - `buslink-sim/src/training.rs`: training link, transport, fault buttons with time windows, active list.
  - Tests: fault window expires on the simulator clock, clear removes faults, profiles untouched.
  - _Requirements: 10.1–10.4_

- [ ] 5. Tauri shell
  - `package.json`, `vite.config.ts`, `tsconfig.json`, `index.html`; `src-tauri/` (Cargo.toml with exact pins, `tauri.conf.json`, capabilities, icons, `serial.rs`, `commands.rs`, `lib.rs`, emitter thread, save dialog).
  - Checks: `cargo.exe check`, `npm run build`.
  - _Requirements: 1.1–1.4, 2.1–2.7, 3.1–3.3, 5.1, 8.1–8.3_

- [ ] 6. Windows build and journey check
  - NSIS bundle; headless journey (dashboard-ui task 9) against the built app; screenshots in `docs/verification/`.
  - _Requirements: 1.2, 10.1_
