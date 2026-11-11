# dashboard-ui: requirements

Epic E4 (`docs/bmad/epics-and-sprint.md`). Covers PRD R12, R13, R14, R15, R16, R24 (training UI), N3, N4. Code: `src/` (TypeScript UI, shared later by the Android WebView and the LAN view), `crates/buslink-core/src/alarm/` (alarm engine).
Rules that always apply: `.kiro/steering/ui.md`, `.kiro/steering/safety.md`.

Out of scope: control panel and any write UI (E5), LAN-only build (E7). The UI built here SHALL contain no write, arm or command controls.

## Requirement 1: connection center (R12)

**User story:** As a technician, I want one screen to pick an adapter, set serial or TCP settings, or start Training mode, and see link statistics.

1. THE connection center SHALL list serial ports with name, VID:PID and the chip label from the backend (CH343, CH340, CH9102), and a refresh button.
2. THE user SHALL set baud, data bits, parity and stop bits for RTU, or host and port for Modbus TCP / RTU over TCP, and connect; invalid input SHALL be reported in text next to the field.
3. THE screen SHALL offer "Start Training mode (simulated devices)" that works with no hardware.
4. Each link SHALL show its state (connected, polling, down, disconnected) as text, its kind, a "Simulated" label for simulator links, and per device: requests, GOOD, timeouts, CRC errors, exceptions, decode errors, achieved cycle time and last error.
5. THE screen SHALL show the advice that the CH343G (B) adapter is not isolated and that an isolated adapter and a wired GND are recommended for field work.

## Requirement 2: device library (R9, R10, R12)

1. THE library SHALL list the bundled profiles with vendor, model, version, register count and a support-level badge (text + shape) for `generic`, `documented`, `simulated`, `hardware_verified`.
2. A profile marked `simulated_only` SHALL show "Simulated device" wherever it appears.
3. THE user SHALL add a device to a connected link by choosing a profile, unit ID and poll interval; unit ID 0 SHALL be refused in the form (broadcast) and by the backend.

## Requirement 3: live dashboard (R13, ISA-101)

1. THE dashboard SHALL show L1 overview: one card per device, sorted worst state first (link down/disconnected, then active P1→P3 alarms, then bad quality, then normal), with link state shown separately from value quality.
2. Each tag tile SHALL show tag name, value, unit, a quality text label, data age, and a horizontal analog bar with the normal band and alarm-limit ticks when a range is known.
3. Normal values SHALL be neutral grey; only abnormal states get hue, always with a text label and a non-color cue (hatch, strike-through, outline).
4. A non-GOOD tile SHALL show the last good value dimmed with its age and the quality badge; it SHALL never show 0 or a blank without explanation.
5. A density toggle (comfortable / compact) and a "Pause live updates" control SHALL exist; while paused, a visible "Paused" banner SHALL say that values are not live.

## Requirement 4: trends (R14, N3)

1. THE trend view SHALL plot one or more tags against time with uPlot, at most two y-axes (one per unit); tags with more units go to additional charts.
2. Non-GOOD samples and intervals without data SHALL break the line (`null`); gap intervals SHALL be shaded with a hatched band and labeled with the reason.
3. Ranges longer than the live buffer SHALL be loaded from the historian with min-max decimation done in Rust, and the chart SHALL say "min/max per pixel" when decimated.
4. THE chart SHALL have a cursor with a tooltip (local time, UTC, value, quality), drag-to-zoom and a reset-zoom control.
5. Alarm limits of the plotted tag SHALL be drawn as horizontal bands.
6. Series SHALL differ by dash pattern as well as color.
7. Trend rendering SHALL refresh at most 4 times per second.

## Requirement 5: register inspector (R15, R8)

1. Per tag THE inspector SHALL show table, doc address, PDU offset, function code, raw hex, the decoded value in the profile's word order, all four word orders side by side for 32/64-bit types, quality and exception text with a likely-cause hint.
2. THE inspector SHALL show the last request/response frames of the device (time, TX hex, RX hex, outcome, elapsed ms) and the link counters.

## Requirement 6: alarms (R16, ISA-18.2)

**User story:** As an operator, I want alarms that follow ISA-18.2 so that I notice abnormal values without a flood when the link drops.

1. Alarm definitions SHALL have tag, type (HI, HIHI, LO, LOLO), limit, deadband, on-delay, off-delay, priority 1–3, message and latch flag.
2. THE engine SHALL implement the states Normal, Unacknowledged, Acknowledged, Return-to-normal unacknowledged, Shelved (with expiry), Suppressed by design and Out of service.
3. WHEN the condition holds for the on-delay, THE alarm SHALL activate; WHEN the value is back inside the limit by more than the deadband for the off-delay, it SHALL clear. A condition shorter than the on-delay SHALL never activate.
4. An acknowledged alarm SHALL stay active until it clears; a latched alarm SHALL stay unacknowledged after it clears until acknowledged.
5. WHEN a shelve expires, THE alarm SHALL return to its underlying state and the expiry SHALL be logged.
6. Communication alarms (TIMEOUT, CRC_ERROR, EXCEPTION, DECODE_ERROR, STALE, DISCONNECTED) SHALL be a separate class, one per device; WHILE a tag's quality is not GOOD its process alarms SHALL hold their state (an inactive one shows Suppressed).
7. THE banner SHALL show the count per priority, the oldest unacknowledged alarm, and a flood indicator when more than 10 alarms activated in 10 minutes; an alarm that activates 3 or more times in 1 minute SHALL be flagged chattering.
8. Activate, clear, acknowledge, shelve, unshelve, shelve expiry and out-of-service changes SHALL be logged as historian events and shown in the alarm log.
9. Default deadbands SHALL follow research/02 (temperature 1 %, pressure 2 %, flow/level 5 % of range; on/off delay 60 s for temperature, 15 s otherwise).

## Requirement 7: training mode (R24)

1. THE training view SHALL list guided exercises in Indonesian and English (read three devices, recognise a timeout gap, word-order check, stale data, alarm acknowledge and shelve, export a record).
2. Fault injection buttons SHALL exist per simulated device: timeout, CRC error, exception 02, stale values, wrong word order (each for a stated duration), and clear faults; active faults SHALL be listed with their remaining time.
3. Every training screen SHALL show the "Simulated device" banner.

## Requirement 8: i18n, theme, accessibility, responsiveness (N4)

1. All UI strings SHALL come from `src/i18n/en.json` and `src/i18n/id.json` with identical keys; a missing key SHALL fail the build check. Language follows the system, can be switched at runtime and is persisted; `<html lang>` follows it.
2. THE theme SHALL follow the OS (light ISA grey default, dark when the OS asks) and can be overridden (system / light / dark).
3. All text/background pairs SHALL meet WCAG 2.2 AA (4.5:1 text, 3:1 non-text); every status SHALL have a text label and a shape.
4. All controls SHALL be keyboard operable with a visible focus ring; targets ≥ 24 px (44 px for primary actions on narrow screens).
5. Alarm and connection changes SHALL be announced through a polite live region (P1 assertive); samples SHALL NOT be announced.
6. With `prefers-reduced-motion`, nothing SHALL pulse or animate.
7. THE layout SHALL work from 360 CSS px wide without horizontal scrolling of the page.
8. Profile, device and user strings SHALL be rendered as text (never `innerHTML`).

## Requirement 9: export entry point (R21)

1. THE UI SHALL offer CSV, XLSX and JSON export of the session and report "Saved to <path>" or "Not saved" as returned by the backend.

## Requirement 10: performance (N3, measured)

1. With 50 tags at 1 Hz THE dashboard SHALL update only changed tiles (no full re-render per frame).
2. THE verification SHALL record frame-processing time and trend render time measured in the app; numbers that were not measured SHALL be reported as not measured.
