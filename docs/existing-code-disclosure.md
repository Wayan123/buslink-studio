# Existing code disclosure

BusLink Studio is a new project written for Kiroween 2026 (started 11 Nov 2026). The rules require new work and disclosure of anything that existed before. This file lists every pre-existing source we looked at and every fact, constant or design idea we deliberately adopted. Kiro sessions and humans must add a row whenever they adopt something.

## Reuse policy

1. **Prior projects by the owner are reference only.** BusLink is written fresh. Do not copy files or code blocks from them. Learning patterns, pitfalls and verified facts is allowed; each adopted item is recorded below.
2. **Soil-Sensor-Android** (`/mnt/d/KERJA/PT AGS/Proyek 2026/Soil-Sensor-Android`, owner's earlier Tauri 2 + Rust + Android OTG app, PT AGS, no license file): may be read for facts and lessons. No code is copied.
3. **multipoll-studio** (`https://github.com/Wayan123/multipoll-studio`, GPL-3.0, .NET WinForms + Python): **nothing is copied, ported or translated from it**, including code, profile files, UI text and data tables. GPL-3.0 is incompatible with BusLink's Apache-2.0 license. During setup only its public README was read to confirm the license and feature list; no source files were opened.
4. Public standards, datasheets and vendor docs (Modbus specs, Android/Tauri docs, WCH datasheets) are cited, not copied beyond short facts.
5. Third-party libraries are used as dependencies under their own licenses and listed in `THIRD_PARTY_NOTICES.md` (to be created with the first dependency).
6. Orchestrator (non-Kiro) edits are limited to `tools/`, `docs/kiro-log/` and release packaging, and are listed in the last table.

## Adopted from Soil-Sensor-Android (facts and ideas, no code)

| # | Item | Kind | Where it was seen | Where BusLink uses it |
|---|---|---|---|---|
| 1 | BGT-SMPS 8-in-1 register map: FC03, start 0, 8 registers; temperature i16 ÷10 °C, moisture u16 ÷10 %, EC u16 µS/cm, salinity u16, N/P/K u16 mg/kg, pH u16 ÷100; defaults unit 1, 9600 8N1. Registers 8–9 undocumented and unused. pH accuracy ±1 despite 0.01 resolution. | Fact (from the sensor manual, also confirmed by the owner's earlier Windows test) | `README.md` register table, `crates/soil-core/src/protocol.rs` | Future `profiles/` soil sensor profile (must still cite the manual page; `hardware_verified` only after a BusLink test) |
| 2 | Known request frame `01 03 00 00 00 08 44 0C` | Fact (re-computed independently with CRC-16/MODBUS) | `README.md` | `.kiro/steering/modbus.md` test vector |
| 3 | Frame RTU responses by expected length from the header and accumulate partial USB reads to a deadline; treat a zero-byte timed read as a timeout | Design idea | `crates/soil-core/src/reader.rs`, `usb_transport.rs` | `.kiro/steering/modbus.md`, `android.md` |
| 4 | CH340 driver in usb-serial-for-android throws on hardware purge; drain input by reading until quiet instead | Pitfall | `CHANGELOG.md`, `UsbInput.kt` | `.kiro/steering/modbus.md`, `android.md` |
| 5 | USB permission flow: app-specific action, explicit `setPackage`, `FLAG_MUTABLE`, request ID check, `RECEIVER_NOT_EXPORTED`, one pending request with 60 s timeout, cancel on stop/detach | Pattern (matches Android docs) | `UsbSerialPlugin.kt` | `.kiro/steering/android.md` |
| 6 | `connectedDevice` foreground service with partial wake lock, started after permission; `CHANGE_NETWORK_STATE` declared | Pattern | `AndroidManifest.xml`, `RecordingService.kt` | `.kiro/steering/android.md` |
| 7 | SAF export: write to cache with allow-listed name/MIME, `ACTION_CREATE_DOCUMENT`, copy off main thread, delete partial document on failure | Pattern | `DocumentsPlugin.kt`, `src-tauri/src/documents.rs` | `.kiro/steering/android.md` |
| 8 | LAN gateway hardening: bind only to a private IPv4 (not 0.0.0.0), per-session self-signed cert + random 32-byte token, constant-time compare, GET-only, no bodies, connection cap, token invalidated on stop | Design idea | `crates/soil-core/src/gateway.rs` | `.kiro/steering/safety.md` (rule 15). BusLink plans axum instead of a hand-written server. |
| 9 | Pitfall: accepted sockets on Windows inherit non-blocking mode from the listener | Pitfall | `CHANGELOG.md` | Note for the LAN server design |
| 10 | Freshness threshold age ≤ max(3 × poll interval, 3 s) | Constant | `crates/soil-core/src/reader.rs` | `.kiro/steering/modbus.md` (default STALE threshold) |
| 11 | WSL/Windows build fix: generated `BuildTask.kt` should call `node.exe` with an explicit `tauri.js` path and list arguments; Windows `node_modules` has Windows-only native bindings so Vite builds use the Windows toolchain | Pitfall | `docs/operations/android-build-diagnosis.md`, `docs/bmad/progress.md` | `.kiro/steering/tech.md`, `android.md` |
| 12 | Do not re-run `tauri android init` over hand edits; keep custom Android code out of `gen/android` | Lesson | `README.md` | `.kiro/steering/android.md` (BusLink puts Kotlin in a separate plugin) |
| 13 | CSV formula-injection escape for text starting with `= + - @` | Pattern (also OWASP guidance) | `README.md` export section | `.kiro/steering/safety.md` (rule 17) |

Not adopted: the Soil-Sensor `device_filter.xml` (decimal IDs, CH340 only); BusLink writes `0x` hex IDs and adds CH343 `1A86:55D3` from the WCH datasheet / Linux cdc-acm patch cited in `research/03-platform-tauri-android-usb.md`.

## Adopted from other pre-existing material

| # | Item | Source | Use |
|---|---|---|---|
| 1 | Product research, scope, profile field groups, support levels, commissioning wizard steps, VFD state machine, audit fields | `research/00-owner-research-brief.pdf` (owner, 11 Nov 2026, written for this entry) | Steering, future spec |
| 2 | Competitor and platform research | `research/01-*.md`, `research/03-*.md` (written for this entry) | Steering |
| 3 | `tools/kiro_bridge.py` | Orchestrator tool, written for this entry | Runs headless Kiro sessions and keeps the ledger |

## Orchestrator edits outside Kiro

| Date | Files | Reason |
|---|---|---|
| 2026-11-11 | `tools/kiro_bridge.py`, `docs/kiro-handoff.md`, `docs/kiro-log/` | Bridge and ledger setup before the first Kiro session |
