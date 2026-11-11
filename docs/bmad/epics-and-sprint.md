# BusLink Studio: epics and sprint plan (BMAD phase 3–4)

Each epic becomes one Kiro feature spec in `.kiro/specs/<epic>/` and is executed by the `buslink-builder` agent. `ics-security-reviewer` reviews every epic commit, and the builder fixes its findings.

| # | Epic (Kiro spec) | PRD | Done when | Target (WIB) |
|---|---|---|---|---|
| E1 | `core-protocol`: RTU/TCP framing, CRC, codec, profile schema, quality model, scheduler, fake transport, property tests | R4–R9, R7 | `cargo test` green; PBT for CRC/framing/codec/order; dead-device fairness test | Nov 13 |
| E2 | `simulator`: in-process soil/VFD/temp/energy devices, fault injection, loopback RTU + TCP server | R10, R24 | Sim drives all E1 tests; VFD/temp state machines respond to writes | Nov 14 |
| E3 | `app-shell-desktop`: Tauri 2 app, Windows serial transport (VID:PID detect), sample bus, historian, CSV/XLSX/JSON export | R1, R3, R12, R20, R21 | Windows exe reads the simulator + the real BGT-SMPS on the CH343G; export opens in Excel | Nov 17 |
| E4 | `dashboard-ui`: connection center, ISA-101 live tiles, uPlot gap-aware trends, register inspector, alarms, i18n id/en, training mode | R12–R16, R24, N3, N4 | Headless Chrome journey 1 passes; WCAG checks; 360 px layout | Nov 21 |
| E5 | `commissioning-control`: command service state machine, ID/baud wizard, simulated VFD/temperature control panel, audit, reports | R17–R19, N1 | Journey 4 passes; the guard hook blocks writes outside `command/` | Nov 24 |
| E6 | `android-otg`: Kotlin USB plugin (usb-serial-for-android), permission, FGS, SAF export, APK | R2 | APK reads the real sensor on the A35 via OTG (journey 2) | Nov 26 |
| E7 | `lan-mqtt`: axum read-only LAN dashboard (token + TLS), MQTT 5 telemetry with outbox | R22, R23 | Journeys 3 and 5 pass | Nov 28 |
| E8 | `release`: NSIS + portable exe, APK, README/judge guide, evidence matrix, Remotion video, SolidCraft 3D, submission kit | N5, N6 | Release published; video < 3 min; Devpost submitted | Dec 2 |

**Readiness gate:** PASS.
- PRD, architecture and research are complete, and the hardware is available.
- **Risk to watch:** E6 (Android USB) is the largest unknown. Its spike task runs early, right after E1/E3, so it is never left to the last week.
