# commissioning-control: requirements

Epic E5 (`docs/bmad/epics-and-sprint.md`). Covers PRD R17, R18, R19 and N1; architecture D8; research/02 §2 (alarms in the report) and §5 (FR2 use control, CR 2.8 auditable events).
Rules that always apply: `.kiro/steering/safety.md` (rules 1–8 in particular), `.kiro/steering/ui.md` (control panel section), `.kiro/steering/modbus.md`.

The command service state machine (`Idle → Armed → Previewed → Confirmed → Sent → ReadBack → Done/Failed`), its audit records and the append-only `command_audit` table already exist (core-protocol Req 16, P10). This epic connects them to the session, the simulator, the Tauri shell and the UI, and adds the commissioning wizard, process feedback, the audit view and the session report.

Out of scope: control of real VFDs or heaters (the control panel exists only for simulated devices), any write through the LAN dashboard or MQTT (E7), user authentication (the role is a local selection and is recorded as such), comm-config registers for the BGT-SMPS soil sensor (its manual in this repo documents none, so its profile declares none).

Decisions recorded with the owner's request (2026-11-13):
- **Arming timeout stays 60 s idle** (safety rule 3 default), not the 120 s in the request. Each preview, confirm or send refreshes it; the countdown shows the time left. `CommandConfig::arm_timeout_ms` makes it configurable for a later decision.
- The stop label is the exact safety-rule text: "Software stop. Not an emergency stop." / "Stop perangkat lunak. Bukan tombol darurat."
- Config registers are declared in the profile's `comm_config` section (schema v1), not in `commands`.

## Requirement 1: arming and session integration (R18, N1)

**User story:** As an operator, I want to arm control for one simulated device for a short time, so that nothing is ever written by accident.

1. THE app SHALL start in Monitoring: WHEN the app starts, a link connects or reconnects, or a profile is loaded, THEN no device SHALL be armed.
2. WHEN the operator arms a device, THE system SHALL require the device's link to be polling, record an `arm` audit row (user, local role, device, profile ID + version, unit ID) and show "Armed: control enabled for N s" with a countdown.
3. WHEN no command activity happens for the arming timeout (default 60 s), THE arm SHALL expire, an `arm_expired` row SHALL be recorded, and any preview or confirmation SHALL be dropped.
4. WHEN the device's link disconnects, reconnects, goes down or changes settings, or the app goes to the background, THE arm SHALL be dropped with a `disarm` row naming the reason; nothing SHALL resume automatically after reconnect.
5. WHILE a device is armed, THE only write path SHALL be the command service through the link worker's queue (one master per bus).

## Requirement 2: control panel for simulated devices (R18)

**User story:** As a trainee, I want to run the simulated VFD and temperature controller through guarded commands, so that I learn the command workflow without risk.

1. THE control panel SHALL exist only for profiles with `simulated_only: true` on a simulator link and SHALL show the banner "Simulated device" on every control view.
2. THE VFD panel SHALL offer run, software stop, frequency setpoint, ramp time and fault reset; THE temperature panel SHALL offer setpoint, heater on, software stop (heater off) and output mode auto/manual. Every command SHALL come from the profile's `commands` list or a `read_write` register.
3. WHEN the operator enters a value, THE system SHALL check it against the profile's `write` bounds (`min`/`max`/`step`/`allowed_values`, `valid_range`) and SHALL reject an out-of-bounds value with a text message next to the field; values SHALL never be clamped.
4. BEFORE sending, THE system SHALL show a confirmation dialog with device, unit ID, register (doc address and PDU offset), function, old value, new value in engineering units, raw register value(s) and the encoded frame in hex; the confirm button SHALL name the action and value (e.g. "Apply setpoint 40.0 °C").
5. WHEN confirmed, THE system SHALL send the frame exactly once, never retry it, then read back and report `verified`, `mismatch` or `unknown`; a timeout SHALL be shown as "effect unknown".
6. THE panel SHALL show three separate fields: command sent, write acknowledged, and process feedback. Process feedback SHALL be judged on the live feedback register named by the command's `feedback` entry, with a tolerance and a timeout, and reported as `pending`, `reached`, `not_reached` or `feedback_unavailable` (feedback quality not GOOD). An acknowledged write SHALL never be shown as "reached".
7. WHILE armed, THE software stop SHALL be visible on the panel and labelled "Software stop. Not an emergency stop." (id: "Stop perangkat lunak. Bukan tombol darurat.").
8. Normal states SHALL stay neutral grey; "running" SHALL never be painted green.

## Requirement 3: commissioning wizard (R17)

**User story:** As a commissioning technician, I want to change a device's unit ID or baud rate with a journal and a recovery hint, so that I never lose the device on the bus.

1. THE wizard SHALL be offered only for devices whose profile declares the setting in `comm_config` and whose capability `comm_config` is not `unsupported`; otherwise it SHALL say why it is not available.
2. WHEN the technician starts the wizard, THE system SHALL require the commissioner role, validate the new value (unit ID 1–247 and not used by another device on the link; baud one of the profile's `codes`), and record a `journal` audit row with the old unit ID and serial settings before anything is armed.
3. THE wizard SHALL then arm the device, preview the frame and require confirmation, and send exactly once.
4. IF the device answers with an exception, THEN the wizard SHALL stop at "failed: settings unchanged".
5. WHEN the write was acknowledged or its effect is unknown, AND the profile says `apply: immediate`, THE wizard SHALL offer "Reconnect at the new settings"; for `after_reboot` it SHALL first ask for a power cycle; for `unknown` it SHALL stop at "needs verification". Reconnecting SHALL drop the arm.
6. AFTER reconnecting, THE wizard SHALL read the setting's register at the new settings and mark the run `pass` (value equals the new setting) or `fail`, with a `verify` audit row.
7. IF verification fails, THEN the wizard SHALL show recovery hints with the old settings and offer "Return the link to the old settings" (no write).
8. Writing a `comm_config` register outside the wizard SHALL be rejected.

## Requirement 4: audit log view (N1, CR 2.8)

1. THE audit view SHALL list every audit row of the session (time local + UTC, event, user/role, device, profile version, function, PDU address, old → requested value, request and response hex, exception, acknowledgement, read-back, note).
2. THE user SHALL filter by device, event and text.
3. THE user SHALL export the audit rows as CSV and XLSX with ISO-8601 UTC times, `.` decimals, numbers as numbers and the formula guard on text cells.
4. THE audit table SHALL stay append-only (update and delete are refused by the database).

## Requirement 5: session report (R19)

1. THE report SHALL contain: session ID, start time, app version, platform; devices with profile ID, version, support level and "Simulated device" where it applies; quality statistics per device (sample counts per quality and link counters); alarm activity (counts per priority and the alarm event list); commands with acknowledgement, read-back and process feedback results; commissioning runs with their journal and verification.
2. THE report SHALL be shown in the app, saved as a self-contained HTML file, and printed to PDF through the WebView print dialog.
3. Every device, profile and user string SHALL be rendered as text (never as markup); THE report SHALL say "simulated" for simulated devices and SHALL make no hardware claim.

## Requirement 6: simulator support (R10, R24)

1. THE simulated VFD and temperature controller SHALL expose a unit-ID register and a baud-code register (simulator design choices, documented in their profiles).
2. WHEN the unit-ID register is written, THE simulated device SHALL answer from its old unit ID and then answer only at the new one; WHEN the baud code is written, it SHALL answer at the old baud and then only at the new baud.
3. A simulated RTU link SHALL carry its baud rate so that a device at another baud stays silent.

## Requirement 7: verification

1. Property tests SHALL show that a frame is never sent unarmed, unconfirmed, after its arm expired, or twice for one confirmation, and that bounds are always enforced.
2. Simulator integration tests SHALL cover the VFD journey (arm, set 30.0 Hz, read-back verified, run, reached target, software stop, audit) and the temperature-controller journey (setpoint, heater on, reached target, software stop), plus the wizard for unit ID and baud.
3. A headless check of PRD journey 4 SHALL drive the e2e build and store screenshots and results in `docs/verification/`.
