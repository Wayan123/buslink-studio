# app-shell-desktop: requirements

Epic E3 (`docs/bmad/epics-and-sprint.md`). Covers PRD R1, R3, R4, R12 (backend side), R20, R21, R24 (training transport), N1, N5; architecture D2, D3, D4, D9, D11.
Code: `src-tauri/` (Tauri shell, Windows serial, IPC), `crates/buslink-core/src/session/` (session service), `crates/buslink-core/src/trend.rs`, `crates/buslink-core/src/transport/usb_ids.rs`, `crates/buslink-sim/src/training.rs`.
Rules that always apply: `.kiro/steering/safety.md`, `.kiro/steering/modbus.md`, `.kiro/steering/tech.md`.

Out of scope here: the command service UI and any write IPC (E5), Android USB (E6), LAN server and MQTT (E7), installer signing and release packaging (E8).

## Requirement 1: application shell (N5, D11)

**User story:** As a technician, I want one Windows application called BusLink Studio, so that I can install it and start without extra tools.

1. THE app SHALL be a Tauri 2.12.x application in `src-tauri/` with identifier `id.co.arkamaya.buslink` and product name "BusLink Studio", serving the Vite + TypeScript frontend from `src/`.
2. THE app SHALL build a Windows NSIS installer that bootstraps WebView2.
3. THE webview SHALL run under a content security policy that allows only `'self'` scripts and styles, Tauri IPC, and `data:` images; THE app SHALL enable no shell, generic HTTP or file-system plugin and SHALL expose no file paths chosen by the webview.
4. All npm and cargo dependencies SHALL be pinned to exact versions and recorded in `THIRD_PARTY_NOTICES.md`.

## Requirement 2: Windows serial transport (R1, D2)

**User story:** As a technician, I want BusLink to find my USB-RS485 adapter and talk RTU through it, so that I do not have to guess COM ports or chip names.

1. WHEN the user asks for the port list, THE app SHALL list every serial port with its name, and for USB ports the VID, PID, manufacturer, product and serial number.
2. THE app SHALL classify adapters **by VID:PID only**: `1A86:55D3` WCH CH343 (CDC-ACM), `1A86:7523` WCH CH340, `1A86:55D4` WCH CH9102 (CDC-ACM); any other port SHALL be listed without a chip label. Friendly names SHALL NOT be used for detection.
3. WHEN an RTU link is connected, THE app SHALL open the port with the chosen baud (300–3 000 000), data bits (7/8), parity (N/E/O) and stop bits (1/2), no flow control, and SHALL reject invalid settings before opening.
4. A read that times out SHALL return "no bytes", never an error or end of stream; WHEN the port disappears or fails (USB unplug), THE transport SHALL report `Disconnected` so every tag of the link becomes DISCONNECTED.
5. WHEN the port is in use by another program, THE app SHALL report "port busy or access denied" and SHALL NOT retry in a loop.
6. THE app SHALL allow only one link per physical port (R4, safety rule 9); a second connect to the same port SHALL be refused with a message.
7. THE connection response SHALL include the non-isolation warning for the CH343G (B) adapter (safety rule 18).

## Requirement 3: Modbus TCP and RTU-over-TCP links (R3)

1. WHEN the user connects a TCP link, THE app SHALL open a Modbus TCP (MBAP) connection to the given host and port with a bounded connect timeout and report failure in text.
2. THE user SHALL be able to choose RTU over TCP (raw RTU frames through a serial server) as a separate link kind; it SHALL never be framed as MBAP.
3. TCP device unit IDs SHALL be 1–255; RTU and RTU-over-TCP unit IDs 1–247.

## Requirement 4: session service (R4, D3, D4, N1)

**User story:** As an operator, I want to connect a link, add devices with a profile and unit ID, and start or stop polling, so that I can read values safely.

1. THE session SHALL load the bundled profiles from `profiles/` (validated by the core loader) and SHALL refuse a device whose profile ID and version are unknown.
2. WHEN a device is added, THE session SHALL validate it with the core device check (unit ID 0 rejected, unit range per link kind, poll interval 100 ms–24 h) and return the reason as text on failure.
3. Each link SHALL have exactly one worker thread that owns its transport; start polling spawns it, stop polling stops it and fails its queued jobs; nothing is replayed later.
4. Adding or removing a device on a polling link SHALL restart its worker with the new device set without reopening the port.
5. A new session, a reconnect and an app restart SHALL start in Monitoring: THE app SHALL expose **no IPC command that writes to a device** in this epic.
6. WHEN the user disconnects a link, THE session SHALL publish a DISCONNECTED sample for every tag of that link that keeps the last good value and its original timestamp.
7. WHEN a link fails (transport lost), THE link state SHALL become DOWN and SHALL stay down until the user reconnects; THE session SHALL NOT reconnect automatically and SHALL NOT restart polling by itself after reconnect.
8. Sample sequence numbers SHALL be unique and increasing across all links of a session.

## Requirement 5: live frames to the UI (D4, R13)

1. THE app SHALL forward the sample bus to the webview as Tauri events at most 4 times per second, independent of poll rates.
2. Each frame SHALL contain the latest state of every changed tag (quality, value, unit, raw registers, value time, age, error) and every sample published since the previous frame as a trend point (time, value or null, quality).
3. WHEN more points arrive than a frame can carry, or the bus subscriber lags, THE frame SHALL report the number of dropped points; points SHALL never be dropped silently.
4. THE UI SHALL be able to fetch a full snapshot (all tags, links, counters) at any time, so a reloaded page shows the same state.

## Requirement 6: historian (R20, D9)

1. THE session SHALL write every sample through the logging decimator into a SQLite WAL database in the app data directory, in batches of at most 1 s.
2. Non-GOOD samples SHALL be stored with a NULL value and their quality, exception code and age (safety rule 11).
3. Each started session SHALL be recorded with the app version and platform; each device with profile ID + version, unit ID and link.
4. Link changes (connected, disconnected, down) and alarm events SHALL be stored as events.
5. WHEN the historian cannot be opened, THE app SHALL still poll and show live values and SHALL report "historian unavailable" instead of hiding it.

## Requirement 7: trend queries (R14, N3)

1. THE app SHALL answer a trend query for one tag and a time range with at most `2 × buckets` points plus gap markers, computed by **min-max decimation per bucket** on the Rust side.
2. Every non-GOOD sample SHALL produce a gap (`null`); two GOOD samples further apart than the device stale threshold SHALL also be separated by a gap. Gaps SHALL be returned as intervals with their reason quality (or `NO_DATA`).
3. Decimation SHALL keep the minimum and maximum of every bucket of every GOOD run; it SHALL never place a value inside a gap.
4. A trend query of 100 000 samples SHALL return in under 1 s on the development PC (measured, N3).

## Requirement 8: export (R21)

**User story:** As a technician, I want to save the session as CSV, XLSX or JSON from a normal save dialog, so that I can hand over a record.

1. WHEN the user exports, THE app SHALL flush pending samples, build the export with the core exporters (quality, exception code, age, profile ID + version, ISO-8601 UTC, formula guard), and open the native save dialog with a suggested file name and the matching filter.
2. WHEN the user cancels the dialog, THE app SHALL report "not saved"; WHEN writing fails, it SHALL report the error; it SHALL report the saved path only after the write succeeded.
3. The path SHALL come only from the native dialog, never from the webview.

## Requirement 9: inspector and diagnostics backend (R12, R15)

1. THE session SHALL keep the last 32 request/response exchanges per device (time, request hex, response hex, outcome, elapsed ms).
2. THE app SHALL return, per tag: table, doc address, PDU offset, function code, raw registers, the decoded value in the profile's word order and in all four orders, quality and error text.
3. THE app SHALL return link statistics per device: requests, GOOD, timeouts, CRC errors, exceptions, decode errors, last error, achieved cycle time.

## Requirement 10: training mode (R24, safety rule 12)

1. WHEN the user starts Training mode, THE app SHALL create an in-process simulated RTU bus with no hardware and add three simulated devices (BGT-SMPS soil sensor at unit 1, VFD at unit 2, temperature controller at unit 3), ready to poll.
2. THE app SHALL inject faults on a simulated device for a bounded time: timeout, CRC error, exception 02, stale (frozen values), wrong word order; and clear them.
3. Fault injection SHALL exist only for simulator links; it SHALL never touch a real link.
4. Simulator results SHALL never change a profile's support level; the training link SHALL be labeled simulated in every response.
