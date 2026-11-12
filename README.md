# BusLink Studio

One open Modbus workbench for laptop, phone and browser. Read an RS485 or Modbus TCP device on Windows, plug the same USB adapter into an Android phone through USB-C OTG, and share a read-only dashboard over the LAN. The same device profile is used everywhere.

Built for **Kiroween 2026** (category: Frankenstein) with Kiro. Every feature came from a Kiro spec; see [How Kiro was used](#how-kiro-was-used).

> **Status, stated plainly.** All automated checks run against the built-in simulator. The Windows app was driven end to end on Windows. The Android APK builds, but it has **not yet run on a phone with a real sensor**: the procedure and the empty result sheet are in [`docs/verification/android-hardware.md`](docs/verification/android-hardware.md).

## What it does

- **Live dashboard** that follows high-performance HMI rules (ISA-101 style): grey when normal, colour only for abnormal states. Analog bars use the profile's declared range only.
- **Quality on every value.** Each value carries GOOD, TIMEOUT, CRC, EXCEPTION, STALE, DECODE_ERROR or DISCONNECTED, plus its age. A stale value is struck through and never shown as fresh.
- **Trends that break at gaps.** Missing data is never interpolated, and the chart names the reason ("No data: TIMEOUT").
- **Register inspector**: manual address, PDU offset, raw bytes, and all four word orders side by side, with the frame log.
- **Guarded writes.** You arm the device, the value is checked against the profile bounds (rejected, never clamped), you confirm the exact frame, and the app sends it once, reads it back and writes an audit row. Control is enabled only for the simulated devices in this release.
- **Commissioning wizard** for unit ID and baud changes, with read-back and a recovery path.
- **Android USB OTG** through usb-serial-for-android (CH343, CH340/CH341, CP210x, FTDI), with a `connectedDevice` foreground service and export through the system file picker.
- **LAN dashboard**: read-only and off by default. It binds to a private IPv4 address only, uses a per-session TLS certificate and a 128-bit token, and serves GET routes only.
- **MQTT telemetry**: MQTT 5 over TLS, publish only, with a bounded SQLite outbox replayed in order. The password is stored in the OS credential store, never in project files.
- **Exports**: CSV, XLSX and JSON with quality, exception code, age and profile version, plus a printable session report and the audit log.
- **Training mode**: a multi-device simulator (soil sensor, VFD, temperature controller, energy meter) with fault injection: timeout, CRC error, exception and wrong word order.
- English and Bahasa Indonesia UI, light and dark themes, and a layout that works down to 360 px.

## Quick start (no hardware needed)

**Windows installer:** download `BusLink-Studio_0.1.0_x64-setup.exe` from [Releases](../../releases/latest) (SHA-256 in `SHA256SUMS.txt`), install it, then open **Connect → Start training mode**. The installer is unsigned, so SmartScreen may warn: choose *More info → Run anyway*.

**From source** (Rust 1.85+, Node 20+, Windows for the desktop shell):

```bash
npm ci
npm run tauri dev          # desktop app
cargo test --workspace     # core, simulator and session tests
npm test                   # UI unit tests
```

Android build notes: [`docs/android-build.md`](docs/android-build.md).

### Real hardware

A USB to RS485 adapter (tested target: WCH CH343G, `1A86:55D3`, not isolated), the device's own power supply, A/B and GND wired, and 120 Ω termination at both bus ends. Only one Modbus master may poll a bus at a time. Profiles live in [`profiles/`](profiles); the schema is in [`docs/profiles/schema-v1.md`](docs/profiles/schema-v1.md).

## Architecture

```
crates/buslink-core   Modbus RTU/TCP codec, scheduler, quality, alarms, historian (SQLite),
                      command service, exports, LAN server (axum), MQTT (rumqttc)
crates/buslink-sim    multi-device simulator with fault injection (training mode)
src-tauri             Tauri 2 shell: IPC, Windows serial, Android USB bridge
plugins/tauri-plugin-buslink-usb   Kotlin USB serial plugin + foreground service
src                   TypeScript UI (no framework), shared by the app and the LAN view
```

All Modbus write encoding lives in `crates/buslink-core/src/command/`. A Kiro hook blocks write function codes anywhere else.

## Verification

| Check | Result |
|---|---|
| `cargo test --workspace` | 274 passed |
| `npm test` | 37 passed |
| Journey 1, dashboard / timeout / trend gap / export (Windows, e2e build) | 15/15 |
| Journey 4, arm / confirm / read-back / audit (simulated VFD) | 30/30 |
| Journey 3, LAN share / token / read-only / stop | 38/38 |
| Android APK on Samsung A35 + CH343G + soil sensor | **not run yet** |

Details, screenshots and method: [`docs/verification/README.md`](docs/verification/README.md). Property-based tests use a seeded generator, and any failure prints its seed for replay.

## Security

The security review was done by a read-only Kiro agent ([`docs/security/review-07.md`](docs/security/review-07.md)). It found no critical or high issues, 2 medium and 6 low. Fixes, and the three items accepted with reasons, are in [`docs/security/review-07-fixes.md`](docs/security/review-07-fixes.md). Roles are a safety interlock, not authentication: [`docs/security/roles.md`](docs/security/roles.md). The software stop **is not an emergency stop**.

## How Kiro was used

- **Specs:** seven specs in [`.kiro/specs/`](.kiro/specs) (core-protocol, simulator, app-shell-desktop, dashboard-ui, commissioning-control, android-otg, lan-mqtt). Each has EARS requirements, a design with correctness properties, and tasks that were ticked only when their checks passed.
- **Steering:** [`.kiro/steering/`](.kiro/steering) covers product, tech, Modbus facts, safety rules, UI rules and Android.
- **Hooks:** [`.kiro/hooks/`](.kiro/hooks):
  - a write-path guard that blocks FC05/06/15/16 code outside the command service;
  - a profile schema check;
  - tests on save for the core;
  - a type check for the UI;
  - a verify step on stop.
- **Custom agents:** [`.kiro/agents/`](.kiro/agents):
  - `buslink-builder`, which implements the specs;
  - `ics-security-reviewer`, which may only run `git diff` and `git show` and cannot write files;
  - `hmi-designer`;
  - `profile-author`.
- **Session log:** [`docs/kiro-log/`](docs/kiro-log). Every Kiro CLI run is recorded with its prompt, session ID, tools used and result, including the runs that hit a time limit and were resumed.

Planning artifacts (product brief, PRD, architecture, epics) are in [`docs/bmad/`](docs/bmad).

## Pre-existing work

This project was started on 11 Nov 2026. Earlier projects by the same author were read for facts and lessons only, and no code was copied. Everything adopted is listed in [`docs/existing-code-disclosure.md`](docs/existing-code-disclosure.md). Third-party libraries are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

## License

Apache-2.0. See [`LICENSE`](LICENSE).
