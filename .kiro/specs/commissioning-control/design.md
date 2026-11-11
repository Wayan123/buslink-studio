# commissioning-control: design

## 1. Module layout

| Path | Contents |
|---|---|
| `crates/buslink-core/src/command/mod.rs` | Existing state machine (D8). New: `CommandRequest::CommConfig`, `Compare::Deferred` (read-back after reconnect), `journal()`/`note()` audit rows without arming, rejection of comm-config registers outside the wizard. |
| `crates/buslink-core/src/command/feedback.rs` | `FeedbackWatch`: pure judge of "process reached target" from live samples (tolerance, timeout, bit). |
| `crates/buslink-core/src/command/commission.rs` | `Wizard` state for one unit-ID or baud change: journal → armed → previewed → sent → reconnected → pass/fail. |
| `crates/buslink-core/src/command/control.rs` | `Control`: the app-facing façade over `Session` + `CommandService` + wizard + feedback. The Tauri shell holds one. |
| `crates/buslink-core/src/session/mod.rs` | Link generation counter; `command_access` (target + worker handle); `set_device_unit`; `reconfigure_serial`; audit sink to the historian; audit rows; report data. No write is encoded here. |
| `crates/buslink-core/src/export/report.rs` | `ReportData` (devices, quality counts, counters, alarms, commands, commissioning). |
| `crates/buslink-core/src/export/audit.rs` | Audit CSV and XLSX. |
| `crates/buslink-sim/src/devices/comm.rs` | `CommRegs`: unit-ID and baud-code registers shared by the VFD and the temperature controller. |
| `crates/buslink-sim/src/world.rs`, `transport.rs` | Per-device baud; re-keying a device after a unit-ID change; `SimTransport::rtu_at(baud)`. |
| `src-tauri/src/control.rs` | IPC for control, wizard, audit and report. Requests carry IDs and engineering values only, never frames. |
| `src/views/control.ts`, `commission.ts`, `audit.ts`, `report.ts`, `src/components/confirm.ts` | UI. |

## 2. Arming and link generation

`Session` keeps `generation: u64` per link, bumped by connect, reconnect, disconnect, link down and `reconfigure_serial`. `Control::arm` stores `(link_id, generation)`. Every `Control` call first runs `check_link()`: if the generation changed or the link is no longer polling, it calls `CommandService::disarm(Reconnect | Disconnect | LinkLost)` and remembers the reason for the status view. Pull-based checks cannot miss a reconnect that happened between two calls, because the counter only grows.

`command_access(device)` returns a clone of the device, link config and profile plus the worker's `LinkHandle` (timeout = device response timeout). The handle sends jobs to the same worker queue that polls (safety rule 9). The link must be polling.

App background: the webview sends `control_disarm { reason: "app_background" }` on `visibilitychange → hidden`; the shell also disarms on window close.

Arming timeout: `DEFAULT_ARM_TIMEOUT_MS = 60_000` idle (safety rule 3), refreshed by arm, preview, confirm and execute. The request asked for 120 s; the safety rule wins and the deviation is listed in requirements.md.

## 3. Comm-config requests

`CommandRequest::CommConfig { setting: UnitId | Baud, value: u32 }`:

1. The profile must declare `comm_config.<setting>` and `capabilities.comm_config` ≠ `unsupported`; role must be commissioner.
2. Unit ID: 1–247 (`BroadcastRejected` for 0). Baud: the raw code is looked up in `codes`; a baud with no code is rejected.
3. The raw value then passes the normal register bounds check and encoding.
4. Read-back is `Deferred`: `execute` does not read at the old settings (the device has moved) and reports `readback: unknown` with the note "verified after reconnect at the new settings".
5. `SetRegister`/`Named` on a register named in `comm_config` → `ControlNotAllowed("use the commissioning wizard")`.

Audit events added: `journal` (old settings before arming), `verify` (wizard result), `feedback` (process reached target or not).

## 4. Wizard (`commission.rs` + `Control`)

```
Journaled → (arm) → Armed → (preview) → Previewed → (confirm+execute) → Sent
Sent ─ exception ─────────────────────────────→ Failed("settings unchanged")
Sent ─ ack/unknown, apply=immediate ──────────→ AwaitReconnect
Sent ─ ack/unknown, apply=after_reboot ───────→ AwaitPowerCycle → AwaitReconnect
Sent ─ ack/unknown, apply=unknown ────────────→ NeedsVerification (stop)
AwaitReconnect → (reconnect: set_device_unit | reconfigure_serial; disarm) → Reconnected
Reconnected → (verify: read the setting register at the new settings) → Passed | Failed
Failed/NeedsVerification → (revert: link back to the old settings, no write) → Reverted
```

Each transition is driven by an explicit operator action; nothing advances on a timer and the write is never re-sent. A new write needs a new wizard run. The wizard view carries `old` and `new` settings, so the recovery hint ("The device may still answer at unit 2, 9600 8N1") is always available.

A baud change applies to the whole link: other devices on the same bus stop answering until they are changed too. The wizard says so before arming.

## 5. Process feedback (`feedback.rs`)

Command profile entry, new optional key (schema v1 addition, all three validators):

```json
"feedback": { "register": "output_freq", "tolerance": 0.05, "timeout_ms": 65000, "value": 0, "bit": 0 }
```

- Target = `value` if present, else the commanded engineering value (setpoints).
- `bit` (bitfield registers only) compares that bit with `value` (0 or 1).
- `FeedbackWatch::observe(sample, now)`: quality ≠ GOOD → `feedback_unavailable` while pending (it never counts as reached); GOOD within tolerance → `reached` (latched); deadline passed → `not_reached`. Only samples newer than the command are used.
- `Control::status()` evaluates the watch against the live store and records one `feedback` audit row when it resolves.

Feedback mapping in the simulated profiles (version 1.1.0):

| Profile | Command | Feedback |
|---|---|---|
| VFD | run | `status_word` bit 0 = 1, 3 s |
| VFD | stop | `output_freq` = 0 ± 0.05 Hz, 65 s |
| VFD | set_frequency | `output_freq` = setpoint ± 0.05 Hz, 65 s (stays `pending`/`not_reached` while stopped) |
| VFD | reset_fault | `fault_code` = 0, 3 s |
| VFD | set_ramp | none (parameter; read-back only) |
| Temp | set_temperature | `pv` = setpoint ± 1.0 °C, 15 min |
| Temp | heater_off | `output` = 0 ± 0.05 %, 5 s |
| Temp | heater_on, mode_* | none |

## 6. Simulator comm registers

Unit ID (rw 1–247) and baud code (rw `0` 4800, `1` 9600, `2` 19200, `3` 38400) sit right after each map: VFD holding 7/8, temperature controller holding 6/7, so every poll stays one contiguous FC03 read (a gap made two reads per cycle). Both `apply: immediate`, role commissioner, risk high. The model validates, answers the write at the old settings, and leaves a pending change that the world applies after building the reply: a new unit ID re-keys the device (refused if the ID is taken: the write is acknowledged but the device stays, so verification fails, as a real collision would); a new baud sets the device baud. `SimTransport` in RTU mode carries the link baud; a device with another baud stays silent. MBAP links ignore baud.

## 7. IPC (`src-tauri/src/control.rs`)

`control_status`, `control_arm {device_id, user, role}`, `control_disarm {reason}`, `control_preview {device_id, request}`, `control_confirm {preview_id}`, `control_execute`, `control_cancel`, `wizard_start {device_id, setting, value, user}`, `wizard_preview`, `wizard_execute`, `wizard_power_cycled`, `wizard_reconnect`, `wizard_verify`, `wizard_revert`, `wizard_close`, `audit_list`, `audit_export {format}`, `report_data`, `report_save_html {html}`.
`request` is `{kind: "named", command, value?}` or `{kind: "register", register, value}`. No IPC takes a frame, an address or a function code. `report_save_html` accepts only text up to 5 MB and writes through the save dialog.

## 8. UI

- **Control view** (`#/control/<device>`): banner "Simulated device"; arm form (user name, local role); armed border + "Armed: control enabled for 52 s"; command buttons built from the profile; value fields with bounds hints; software stop button with the safety label, separated from Run by other controls; three result fields (sent / acknowledged / process feedback) plus read-back. Status polled at 2 Hz while the view is open.
- **Confirm dialog** (`components/confirm.ts`): `<dialog>` with focus trap and restore; shows every field of safety rule 3; the confirm button label names the action and value.
- **Wizard view** (`#/commission/<device>`): steps list with the current step marked by text and shape; recovery box always visible after sending.
- **Audit view** (`#/audit`): filterable table; CSV/XLSX export.
- **Report view** (`#/report`): rendered with DOM text nodes; "Save HTML" serializes the report element (text already escaped by the DOM) with inline print styles; "Print / save as PDF" calls `window.print()`; print CSS hides navigation.
- The views render nothing but a notice when there is no backend (LAN/static build).

## 9. Correctness properties

- **C1** (`tests/command_props.rs`, extends P10): random operation sequences including clock jumps past the arm timeout; no frame leaves unless the current preview was confirmed while armed and unexpired; one frame per confirmation.
- **C2**: random values for every writable register of the simulated profiles: the service accepts exactly the values inside the profile bounds, and every sent raw value decodes inside them.
- **C3**: comm-config requests: wrong role, unit 0/248, baud without code, comm register through `SetRegister` → rejected and nothing sent.
- **C4** (`feedback.rs` unit tests): non-GOOD samples never resolve to `reached`; old samples are ignored; deadline gives `not_reached`.
- **J1/J2** (`crates/buslink-sim/tests/control_journeys.rs`, manual clock): VFD and temperature journeys over the simulator.
- **J3** (`crates/buslink-sim/tests/control_session.rs`, real clock): `Control` + `Session` + training bus: journey 4, disarm on reconnect, wizard unit ID and baud pass, wizard failure + revert.
