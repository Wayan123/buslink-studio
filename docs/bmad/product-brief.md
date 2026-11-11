# BusLink Studio: product brief (BMAD phase 1)

Source: the owner's research `research/00-owner-research-brief.pdf` (11 Nov 2026) and owner decisions in the chat on 11 Nov 2026. The competitor, standards and platform briefs `research/01–03` refine this.

## Problem
Commissioning and maintaining Modbus devices still means one vendor tool per device, Modbus Poll on a laptop, register maps typed by hand from PDF manuals, and no easy way to carry the same setup to a phone in the field. Values that have gone stale look the same as fresh ones. Writes (unit ID, baud rate, setpoints) are risky and leave no audit trail.

## Product
**BusLink Studio** is one open-source Modbus workbench across three screens:
- **Windows .exe**: RTU over USB-RS485 (CH343G/CH340), plus Modbus TCP.
- **Android APK**: RTU through USB-C OTG → the same USB-RS485 adapter, plus TCP over Wi-Fi.
- **Browser**: the running app serves the same live dashboard read-only on the LAN.

The same **device profile** (an open, versioned JSON register map) works on all three.

## Users
1. Commissioning technicians and maintenance staff (primary).
2. System integrators who wire many vendors on one bus.
3. Vocational and university labs: a training mode with a built-in simulator, available in Indonesian and English.

## MVP for Kiroween (owner-approved scope "plus-control")
P0: multi-device RTU/TCP polling (one master per bus), profiles with support levels, data quality and data age, an ISA-101 style live dashboard, gap-aware trends, a register inspector, a SQLite historian, CSV/XLSX/JSON export, a simulator.
P1: ISA-18.2-style alarms, the LAN dashboard, MQTT telemetry, a commissioning wizard (ID/baud with read-back and a recovery journal), and a control panel for a **simulated** VFD and temperature controller with arming, bounds, confirmation, read-back and audit.
Out: cloud accounts, payments, real VFD/heater control on hardware, writes over the LAN dashboard.

## Hardware for tests
CH343G "USB TO RS485 (B)" adapter, Vention USB-C OTG, Samsung A35, BGT-SMPS 8-in-1 soil sensor (RTU slave 1, 9600 8N1, FC03 addresses 0–7). There is no VFD or temperature controller, so those are simulator profiles only, labeled as such.

## Success criteria (to be measured, not assumed)
- A judge can install the Windows build and run the full journey on the simulator in under 5 minutes.
- The real soil sensor is read on Windows and on Android through OTG with the same profile file. Recorded as hardware evidence.
- Stale data and communication gaps are always visible, both in tiles and in trends.
- Every write goes through the command service and appears in the audit log with request, response and read-back.

## Differentiation (updated from `research/01-competitors-and-features.md`)
Desktop pollers are crowded. Busloom (Tauri + Rust, desktop, $149 Pro) already has write read-back and XLSX map import. Modbench and the mbmap library (CC0, 621 maps) cover device libraries. So BusLink does **not** pitch those as new. No surveyed tool combines all three of:
1. **The same profile on a Windows PC and an Android phone over USB OTG.**
2. **Quality + age on every value**, carried into logs, with trends that break at outages.
3. **A read-only live view served to LAN browsers** from the app itself.

Plus: free and open source (Apache-2.0), Indonesian/English, `.mbmap` import for interoperability instead of a rival format, and a training mode with a simulator.

vs GhostWire (the owner's other entry): GhostWire is a simulator-only troubleshooting trainer. BusLink commissions and logs **physical** devices across PC, phone and browser.

## Constraints
- New-project rule: the earlier Soil-Sensor (Tauri) and multipoll-studio (GPL) projects are reference only. Kiro writes everything fresh, and any adopted facts are disclosed.
- Kiro must do the building; it is orchestrated through `tools/kiro_bridge.py`.
- Deadline: Dec 4, 2026, 14:00 PT. Internal target: Dec 2.
