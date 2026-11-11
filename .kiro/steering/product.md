---
inclusion: always
---

# BusLink Studio: product

BusLink Studio is one open-source (Apache-2.0) Modbus workbench for three screens:

- **Windows .exe**: Modbus RTU over a USB-RS485 adapter (owner's: Waveshare "USB TO RS485 (B)", WCH CH343G; CH340 also supported), plus Modbus TCP.
- **Android APK**: RTU over USB-C OTG to the same adapter, plus Modbus TCP over Wi-Fi.
- **Browser**: the running app (desktop or Android) serves the same live dashboard **read-only** to LAN browsers (token + TLS).

The same versioned JSON **device profile** works on all three. Kiroween 2026, Frankenstein category: USB serial + industrial protocol + phone + web dashboard + MQTT stitched into one tool. Deadline: 4 Dec 2026 14:00 PT (internal target 2 Dec).

Sources: `research/00-owner-research-brief.pdf` (owner's research, Indonesian; working name "Modbus FieldHub", superseded by "BusLink Studio"), `research/01-competitors-and-features.md`, `research/03-platform-tauri-android-usb.md`, `docs/bmad/product-brief.md`.

## Audience

1. **Commissioning technicians and maintenance staff** (primary): connect fast, understand registers, log portably, hand over a test record.
2. **System integrators** who wire many vendors on one bus.
3. **Training labs** (vocational and university): training mode with a built-in simulator, Indonesian and English.

Not the audience in v1: permanent SCADA, critical process control, cloud fleets.

## Core journey

**Connect → Add device → Read → Log → Export → Share.** Every screen should make the next step obvious.

1. Connect: pick adapter/port (USB permission on Android) or TCP host, or Training mode (simulator).
2. Add device: choose a profile (with its support level) or create a custom one; set unit ID.
3. Read: live values with unit, quality and age; register inspector for experts.
4. Log: SQLite historian, alarms, gap-aware trends.
5. Export: CSV / XLSX / JSON with quality columns and profile version.
6. Share: read-only LAN dashboard (URL + QR + token) and optional MQTT telemetry.

Beginner pages show tag names and units. Expert mode opens FC, address, quantity and raw frames.

## Scope for the competition

In: multi-device RTU/TCP polling (one owner per bus); profiles with support levels; quality GOOD / TIMEOUT / CRC_ERROR / EXCEPTION / STALE / DECODE_ERROR plus data age; ISA-101 style live dashboard; trends with gaps; register inspector (manual address vs PDU offset, raw hex, decoded); SQLite historian; CSV/XLSX/JSON export; ISA-18.2-style alarms (hysteresis, delay, acknowledge, shelving); MQTT telemetry; training mode with simulator; commissioning wizard (unit ID / baud with read-back and recovery journal); control panel for a **simulated** VFD and temperature controller through the command service; id/en UI.

Out: cloud accounts, payments, control of real VFDs or heaters, any write through the LAN dashboard or MQTT.

## Hardware we really have

USB TO RS485 (B) CH343G adapter (not isolated), Vention USB-C OTG adapter (no charging port), Samsung A35 (SM-A356E), BGT-SMPS 8-in-1 soil sensor (RTU slave 1, 9600 8N1, FC03 from address 0, 8 registers). There is **no** VFD or temperature controller: those are simulator profiles, labeled "Simulated device" everywhere they appear.

## Differentiation

Desktop pollers are crowded (Modbus Poll, Busloom, Modbench, ModbusLens, OpenModScan...). Write read-back, device libraries and map import already exist elsewhere, so do not pitch them as new. Pitch the combination no surveyed tool has:

1. The **same profile on a Windows PC and an Android phone over USB OTG**.
2. **Quality + age on every value**, carried into logs and exports, with trends that break at outages.
3. A **read-only live view served to LAN browsers** by the app itself.

Plus: free and open source, no trial limits, Indonesian/English, training mode, `.mbmap` import for interoperability instead of a rival format.

- **vs Modbus Poll** (Witte, $129, Windows only): Modbus Poll is already a capable generic master (RTU/TCP, multiple slaves, charts, logging, Excel). Never claim it "can't read sensors". BusLink differs by profiles shared with Android, quality/age everywhere, the LAN view, guarded writes with audit, and being free and open.
- **vs GhostWire** (the owner's other Kiroween entry): GhostWire is a simulator-only troubleshooting trainer. BusLink commissions and logs **physical** devices across PC, phone and browser; its simulator exists so judges and labs can try it without hardware. Do not share code, UI or story beats with GhostWire.

## Tone

Plain, precise, field-technician voice. Short labels, real units, concrete verbs ("Apply setpoint 40 °C", not "Submit"). Honest about limits: say what was tested, on what, and what was not. No hype, no "AI-powered", no "universal sensor support" (the right claim is "supports Modbus devices through profiles you can add"). Indonesian strings use standard technical Indonesian, not literal translations.

## Success criteria (measure, do not assume)

- A judge installs the Windows build and runs the whole journey on the simulator in under 5 minutes.
- The real soil sensor is read on Windows and on Android OTG with the same profile file, recorded as hardware evidence.
- Stale data and communication gaps are always visible in tiles and trends.
- Every write goes through the command service and appears in the audit log with request, response and read-back.
