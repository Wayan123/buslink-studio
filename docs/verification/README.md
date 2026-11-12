# Verification: app-shell-desktop (E3) and dashboard-ui (E4)

Date: 2026-11-12. Machine: the owner's Windows PC (WebView2 157, Chrome 157), driven from WSL.
**Everything here ran against simulated devices. None of it is hardware evidence** (safety rule 12). The real BGT-SMPS on the CH343G adapter has not been tested in this app yet.

## How it ran

`node scripts/verify-journey.mjs` (Windows node, puppeteer-core 25.17.0):

- **Part A** starts the e2e build (`cargo.exe build --release --features "custom-protocol e2e" --target-dir src-tauri/target/e2e`) with `--remote-debugging-port` for WebView2 and drives the real app over CDP. The `e2e` feature only changes export: it writes to `BUSLINK_E2E_EXPORT_DIR` instead of opening the native save dialog, which CDP cannot click. The installer build does not have it.
- **Part B** serves `dist/` to headless Chrome (no backend) to check layout, language and keyboard focus at 360 and 1280 px.

Machine-readable results: `journey-results.json`. CSV written by the run: `export/`.

## Results (15/15 passed)

| Check | Result |
|---|---|
| App webview reachable over CDP, backend present | pass |
| Training mode → 3 simulated devices GOOD, each card shows "Simulated device" | pass (`01-dashboard-training.png`) |
| TIMEOUT injected on unit 3 for 15 s: tiles show `TIMEOUT · age`, last value struck through, comm alarm in the banner | pass (`02-dashboard-timeout.png`) |
| Trend shows hatched gap "No data: TIMEOUT" and the gap list names it | pass (`03-trend-timeout-gap.png`, `04-trend-after-recovery.png`) |
| Register inspector lists frames including TIMEOUT exchanges | pass (`05-inspector.png`) |
| Alarms view | screenshot only (`06-alarms.png`) |
| CSV export: header has `quality`, `exception_code`, `age_ms`, `profile_version`; 24 TIMEOUT rows for unit 3, GOOD rows for the others | pass |
| App at 360 px: no horizontal page scroll | pass (`07-dashboard-360px.png`; dark: `08-…`) |
| `dist/` in headless Chrome, en/id at 360 px and en at 1280 px: "backend not available" notice, `lang` set, no overflow, no visible target under 24 px | pass (`10-static-*.png`) |
| First Tab focuses the skip link with a visible outline | pass |

The first run showed the sticky header covering content at 360 px (it grew tall when wrapped). The header is now static below 720 px; the second run's screenshot confirms it.

## Other checks run

- `cargo test --workspace` (WSL): all pass, including trend T1–T5, alarm A1–A6 and session S1–S6.
- `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --all --check`: clean.
- `cargo.exe clippy --manifest-path src-tauri/Cargo.toml --all-targets -D warnings`: clean.
- `npm install`, `npm run build` (tsc + i18n check + innerHTML check + vite) via `cmd.exe`: pass. 332 keys in en and id.
- `node --test src/**/*.test.ts`: 13 tests pass (U1–U5).
- `npx tauri build --bundles nsis`: `src-tauri/target/release/bundle/nsis/BusLink Studio_0.1.0_x64-setup.exe` (3.46 MiB, unsigned).

## Measured numbers (N3)

- Trend decimation of 100 000 samples in Rust: well under 1 s (asserted by `hundred_thousand_samples_decimate_quickly`; debug build).
- Live trend render in the app: 0.1–3.9 ms per refresh for 2 tags (shown on the trends page).

## Not verified

- Real serial hardware: COM port open/close, CH343 VID:PID detection on a real port, USB unplug → DISCONNECTED. Only the code paths exist.
- Modbus TCP against a real device (only the core TCP client tests against a loopback server).
- Installing the NSIS installer on a clean machine; the installer was built, not installed.
- XLSX opened in Excel (the core test checks sheets and numeric cells by unzipping).
- The native save dialog (the e2e build bypasses it).
- 60 fps with 50 tags at 1 Hz; 100k-point trend render in the browser (only the Rust decimation is timed).
- Screen readers; contrast was computed for the palette tokens, not measured on the rendered pages.
- Trend drag-zoom and cursor tooltip were not exercised by the script.
- The historian of the run lives in the user's app data directory (`%APPDATA%\id.co.arkamaya.buslink\buslink-history.sqlite3`); the script does not isolate it.

# Verification: commissioning-control (E5), PRD journey 4

Date: 2026-11-13. Same machine and method (`node scripts/verify-control.mjs` against the e2e build). **Simulated VFD only; not hardware evidence.** Results: `control-results.json` (25/25 passed); files written by the run: `export-control/` (audit CSV/XLSX, report HTML, report PDF via CDP `Page.printToPDF`).

| Check | Screenshot |
|---|---|
| Control panel starts in Monitoring, "Simulated device" banner, commands disabled | `20-control-monitoring.png` |
| Arm with a user name, countdown "Armed: control enabled for N s", software stop visible and labelled "Software stop. Not an emergency stop." | `21-control-armed.png` |
| 60 Hz rejected next to the field, no dialog; button "Apply frequency setpoint 30.00 Hz" | |
| Confirm dialog: device, unit ID, register (doc address + PDU offset), FC06, old → new, raw 0BB8 (3000), frame hex; the script checks the frame bytes against the other fields | `22-confirm-30hz.png` |
| Write acknowledged + read-back verified while process feedback is still "Pending" | `23-ack-pending.png` |
| Output frequency reached 30.00 Hz ± 0.05 (measured) | `24-reached-30hz.png` |
| Software stop: confirmed, read back, output reached 0 Hz | `25-stopped.png` |
| Audit log with results, feedback rows and the sent frame; CSV/XLSX export | `26-audit.png` |
| Session report (devices + profile versions, simulated note, commands with read-back and feedback); HTML without script; PDF | `27-report.png` |
| Disconnect drops the arm with a stated reason | `28-disarmed-after-disconnect.png` |

Journey 1 (`verify-journey.mjs`) was re-run after these changes: 15/15.

Not verified: the commissioning wizard in the UI (covered by core tests J3 only), the WebView print dialog itself (the PDF came from CDP), the `app_background` disarm on Android, any real device.


# Verification: lan-mqtt (E7), PRD journey 3

Date: 2026-11-13. Same machine and method as above (`node scripts/verify-lan.mjs`, Windows node, puppeteer-core): the e2e build runs Training mode, shares the LAN view on this PC's private address `192.168.100.77:8443`, and a second headless Chrome with its own profile plays the colleague's browser. **Simulated devices only; not hardware evidence, and not a test from a second physical device.** Results: `lan-results.json` (33/33 passed). In the screenshots of the host app the link and QR code are blurred with CSS on purpose (a LAN token is a secret).

| Check | Result |
|---|---|
| Sharing is off at start: no link, no header indicator; interface list has private addresses only; default port 8443 | pass (`30-share-off.png`) |
| Link is `https://<ip>:8443/?t=<32 hex>`; QR drawn; SHA-256 fingerprint shown; header indicator on; the "off" form is hidden | pass (`31-share-on.png`) |
| Browser gets the read-only build: 3 devices GOOD, "Simulated device" banners, quality and age on tiles, live stream connected | pass (`32-lan-dashboard.png`) |
| Navigation is overview + trends; buttons are only "Pause live updates" and "Compact tiles"; no connect/arm/write/export element | pass |
| Cookie is Secure, HttpOnly, SameSite=Strict; POST/PUT/DELETE give 405/400; unknown route 404; no token or wrong token 401 | pass (`33-lan-no-token.png`) |
| The app lists the viewer | pass (`34-share-viewer.png`) |
| TIMEOUT injected on the host shows in the browser as `TIMEOUT · n s` with the last value struck through; the alarm banner is display only | pass (`35-lan-timeout.png`) |
| Browser trend shows "No data: TIMEOUT"; the 1 h range is fetched from `GET /api/trend` | pass (`36-lan-trend-gap.png`) |
| 360 px: no horizontal scroll | pass (`37-lan-360px.png`) |
| Stop sharing: the browser stream says reconnecting, the port refuses connections, the indicator disappears; restart gives a new token and the old link gets 401 | pass (`38-lan-after-stop.png`) |

The first run of this script found that the host's asset resolver answered unknown paths like `/api/command` with `index.html` (200). The server now answers 404 for `api/…` and the shell refuses HTML fallbacks (test `unknown_api_paths_stay_404_when_assets_fall_back_to_index`). A later screenshot showed the "Sharing is off" form still visible while sharing was on (a CSS `display` rule beat `hidden`); fixed and now asserted.

## Rust tests (WSL)

`cargo test --workspace` passes, including `lan_server` (L1, L3, L5, L6, limits, origin, 404 fallback), `mqtt_broker` (M4–M6, throttle, expiry, overflow, lost PUBACK, TLS), and unit tests L2, L4, L7, M1–M3, M7. MQTT journey 5 is covered by the in-process broker test, not by a headless browser run and not against a real broker.

## Not verified

- Android: no APK built, no phone run; Keystore and platform-verifier wiring are not done (see `docs/android-build.md`).
- A real MQTT broker (Mosquitto, EMQX, a cloud broker) and TLS roots from the Windows store: the tests use an in-process broker and a pasted CA. `test_connection` against a public broker was not run.
- Windows Credential Manager: it compiles and is wired, but no password was stored and read back in this run.
- A second physical device on the Wi-Fi; browsers other than Chrome; Safari/iOS handling of the self-signed certificate.
- Firewall prompts on first listen (Windows Defender Firewall may ask for permission).
- Load: 8 WebSocket viewers at once and long runs were not measured.
