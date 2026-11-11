# BusLink Studio: PRD (BMAD phase 2)

Inputs: `product-brief.md`, `research/00-owner-research-brief.pdf`, `research/01-competitors-and-features.md`, `research/02-standards-and-dashboard-design.md`, `research/03-platform-tauri-android-usb.md`.
Status: approved scope "plus-control" (owner, 2026-11-11). Each requirement has an ID that the Kiro spec cites.

## 1. Goals and non-goals
**Goals:** one Modbus workbench on Windows, Android (USB OTG) and a LAN browser view, sharing one profile format. Data quality and data age are always visible. Writes are safe. Judges can try everything with a simulator.

**Non-goals (competition):**
- Cloud accounts, payments, Play Store publishing.
- Driving a real VFD or heater (simulator profiles only).
- Writes from the LAN browser.
- Linux/macOS builds. Tauri supports them, but they are untested here.

## 2. Users and top jobs
| User | Job | Pain it removes (research/01) |
|---|---|---|
| Commissioning technician | Connect a new device, find the right registers, check the values, hand over a report | #1 addressing, #2 word order, #3 unknown serial settings, #4 wiring vs software |
| Maintenance in the field | Check a device on the plant floor with only a phone | #10 fragile Android apps; no shared profile between PC and phone |
| Supervisor / trainer | Watch live values on any screen; run a lab without hardware | #9 stale values look healthy; labs need a simulator |
| Integrator | Change unit ID/baud or a setpoint safely, with a record | #7 unsafe writes |

## 3. Functional requirements
### Connectivity
- **R1 RTU desktop:** Windows COM ports. Detect WCH adapters by VID:PID (CH343 1A86:55D3 as inbox CDC-ACM, CH340 1A86:7523, CH9102 1A86:55D4), not by friendly name. Baud 300–3 000 000, data 7/8, parity N/E/O, stop 1/2.
- **R2 RTU Android:** USB OTG with a permission flow. Rotation keeps the session (state lives in the Rust core). A `connectedDevice` foreground service keeps polling with the screen off. Detach → DISCONNECTED, and nothing auto-resumes writes. Target API 36.
- **R3 Modbus TCP:** MBAP with transaction IDs, reconnect with backoff, unit ID. RTU-over-TCP is a separate transport type.
- **R4 One master per bus:** one scheduler queue per link. One failing device never blocks the others (backoff on it).

### Protocol and data
- **R5 Function codes:** FC01/02/03/04 read; FC05/06/15/16 write. Spec limits: 125 registers (FC03/04), 2000 coils, 123 registers (FC16). Merge contiguous reads into blocks.
- **R6 Codec:** U16/I16/U32/I32/U64/I64/F32/F64, bit fields, string. Word/byte order ABCD/CDAB/BADC/DCBA, stored explicitly in the profile. Scale, offset, unit, decimals, valid range, sentinel values.
- **R7 Quality:** every sample carries GOOD / TIMEOUT / CRC_ERROR / EXCEPTION(code) / STALE / DECODE_ERROR / DISCONNECTED, plus `age_ms`, source timestamp and receive timestamp. Last-good values are shown as stale, never as fresh. (OPC UA Good/Uncertain/Bad mapping per research/02.)
- **R8 Addressing:** always show the manual/documentation address *and* the PDU offset. Never send 40001-style numbers raw.

### Profiles
- **R9 Device profile:** JSON, versioned, schema-validated. Holds identity, serial defaults, register blocks, tags (address, FC, type, order, scale, unit, range, writable, min/max write), commands, and a support level: `generic | documented | simulated | hardware_verified` with source (manual and page). Profiles and device instances are separate.
- **R10 Bundled profiles:**
  - BGT-SMPS 8-in-1 soil sensor (`documented`, upgraded to `hardware_verified` after the real test).
  - Simulated VFD (`simulated`).
  - Simulated temperature controller (`simulated`).
  - Simulated energy meter (`simulated`).
  - A generic holding-register template.
- **R11 Import:** CSV/XLSX register map with preview and address-base detection. `.mbmap` read-only import (P2).

### UI
- **R12 Connection center:** ports/adapters, USB permission state, serial settings, TCP host, link statistics (TX/RX, CRC, timeouts, cycle time).
- **R13 Live dashboard:** ISA-101 high-performance style (grey base, color only for abnormal states, analog bar with normal range). Tiles show value, unit, quality text and age. Density toggle; Indonesian and English.
- **R14 Trends:** multiple tags with unit axes, cursor, zoom. **Gaps for lost communication, never interpolated.** Raw samples separate from display decimation (min-max). Alarm bands.
- **R15 Register inspector:** doc address, PDU offset, raw hex, decoded value in all four orders side by side, and the last request/response frame.
- **R16 Alarms (ISA-18.2):** threshold, hysteresis/deadband, on/off delay, priority, acknowledge, shelve with expiry. Communication alarms kept separate from process alarms. Alarm log.
- **R17 Commissioning wizard:** change unit ID or baud on one device, with journal → write → reconnect at the new settings → read-back → pass/fail. A recovery hint shows the old settings.
- **R18 Control panel (simulated VFD + temperature controller):**
  - The command service requires the device to be armed (session timeout), checks bounds, and asks for confirmation with an explicit label ("Apply setpoint 40.0 °C").
  - It encodes a frame preview, writes once (no automatic retry), reads back, and records the audit entry.
  - A software stop is always reachable and labeled "not an E-stop".
- **R19 Reports:** session summary (devices, profile versions, quality statistics, alarms, commands) exported as PDF/HTML.

### Data
- **R20 Historian:** SQLite WAL with tables sessions, devices, tags, samples, events, commands, outbox. Logging interval separate from polling interval. Retention setting.
- **R21 Export:** CSV (UTF-8, ISO-8601 UTC, quality and age columns, formula-injection guard), XLSX (Measurements, Devices, Configuration, Events, Commands sheets, numbers kept as numbers), JSON. Android saves through SAF.

### Sharing
- **R22 LAN dashboard:** opt-in. The app serves the same frontend (read-only routes + WebSocket stream) on a user-chosen interface/port, with a per-session token, TLS (self-signed per session; the URL/QR carries the token), and no write endpoints. Works from Windows and Android.
- **R23 MQTT telemetry:** opt-in. MQTT 5 over TLS, topic `buslink/{site}/{gateway}/{device}/{tag}` or a Sparkplug-B-compatible namespace (decision in architecture), payload with ts/seq/value/unit/quality/profile_version, a bounded offline outbox, deduplication by sequence. Credentials go in the OS secret store and never into project files.

### Training
- **R24 Simulator:** built-in multi-device RTU-over-loopback and TCP simulator with the bundled simulated profiles. Fault injection: timeout, CRC error, exception, stale, wrong word order. A **training mode** with guided exercises in Indonesian and English.

## 4. Non-functional requirements
- **N1 Safety:** read-only by default. Writes only through the command service. No broadcast writes. Audit trail for every command.
- **N2 Security:** IEC 62443-minded. LAN and MQTT off by default. Tokens + TLS. Profiles are data only (no scripts). No secrets in exports or logs. CSV formula guard.
- **N3 Performance (targets, to be measured):** UI keeps 60 fps with 50 tags at 1 Hz. Trend of 100k points renders in under 1 s on desktop. Android stays responsive at 10 tags/s.
- **N4 Accessibility:** WCAG 2.2 AA. Status shown as text plus color. Keyboard operation on desktop. Touch targets ≥ 24 CSS px (AA), with 44 px on primary mobile actions. Reduced motion.
- **N5 Packaging:** Windows NSIS installer (WebView2 bootstrapper) + portable exe. Android APK (arm64). SHA-256 sums.
- **N6 Evidence:** a separate pass/fail matrix for simulator and for hardware. A profile becomes `hardware_verified` only with recorded evidence.

## 5. Acceptance (demo)
1. Fresh Windows install → Training mode → read 3 simulated devices → trend shows a gap when a fault is injected → export XLSX with quality columns.
2. Real BGT-SMPS sensor through the CH343G adapter on Windows, then the same profile on the Samsung A35 via OTG. Values match within one poll.
3. Browser on the LAN shows the phone's live dashboard, read-only.
4. Simulated VFD: arm → set frequency 30.0 Hz → read-back OK → stop → audit log entry.
5. MQTT: messages arrive at a local broker with sequence numbers; disconnect → outbox → replay without duplicates.
