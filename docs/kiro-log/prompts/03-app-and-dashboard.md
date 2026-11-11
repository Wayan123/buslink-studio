Create specs and implement epics E3 `app-shell-desktop` and E4 `dashboard-ui` from docs/bmad/epics-and-sprint.md, as the buslink-builder agent.

Read first: docs/bmad/prd.md (R1, R3, R12–R16, R20, R21, R24, N3, N4), docs/bmad/architecture.md (D2, D4, D5, D9, D11), research/02-standards-and-dashboard-design.md (ISA-101, ISA-18.2, charting choice, palettes), research/03-platform-tauri-android-usb.md, .kiro/steering/*, and the existing crates (buslink-core, buslink-sim) and profiles/. Reference (read only): /mnt/d/KERJA/PT AGS/Proyek 2026/Soil-Sensor-Android/src-tauri (Tauri 2 shell structure, Windows serial port open/close, VID:PID detection) and its .windows-tools build notes for building from WSL with the Windows toolchain.

## Step 1: specs
`.kiro/specs/app-shell-desktop/` and `.kiro/specs/dashboard-ui/` (EARS requirements citing PRD IDs, design with correctness properties, numbered tasks).

## Step 2: implement
**app-shell-desktop**
- Tauri 2.12.x app in `src-tauri/` (identifier `id.co.arkamaya.buslink`, product name "BusLink Studio"), Vite + TypeScript frontend in `src/`.
- Transports from buslink-core's trait: Windows serial (`serialport`; list ports with VID:PID and mark WCH CH343 1A86:55D3, CH340 1A86:7523, CH9102 1A86:55D4) and Modbus TCP; the in-process simulator for Training mode.
- IPC commands: list ports, connect/disconnect link, add device (profile + unit ID), start/stop polling, subscribe to the sample bus (Tauri events, throttled UI updates), historian queries, export CSV/XLSX/JSON via the save dialog.
- Training mode must work with zero hardware.

**dashboard-ui**
- Screens:
  - Connection center.
  - Device library (bundled profiles with support-level badges).
  - Live dashboard with ISA-101 tiles: grey base, color only for abnormal, analog bar with normal range, value + unit + quality text + age.
  - Trends: uPlot unless research/02 recommends otherwise; justify in design.md. Null gaps for non-GOOD samples, min-max decimation, cursor, zoom, alarm bands.
  - Register inspector: doc address + PDU offset, raw hex, all four word orders side by side, last frames.
  - Alarms: ISA-18.2 states with acknowledge, shelve with expiry, hysteresis, delay; comm alarms separate from process alarms.
  - Training mode: guided exercises in id/en, with simulator fault injection buttons.
- i18n id/en, light/dark following the OS with an ISA-101 grey default, WCAG 2.2 AA, keyboard, reduced motion, responsive down to 360 px (it is also the Android UI).

Build checks: `cargo test --workspace` (core); `cmd.exe /c "npm.cmd install"` then `cmd.exe /c "npm.cmd run build"` and `cargo.exe check` in src-tauri via cmd.exe; finally build the Windows app with `cmd.exe /c "npx.cmd tauri build --bundles nsis"`. If a full build is too slow, at least `cargo.exe check` must pass, and say so. Pin all npm and cargo versions. Run a headless check of the UI in Windows Chrome against `npm run dev` or the built dist: Training mode → 3 devices → trend shows a gap after an injected timeout → export CSV. Save screenshots to docs/verification/. Mark tasks `[x]`. Report results honestly, including what did not run. Do not commit.
