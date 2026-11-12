# Android hardware test: A35 + OTG + CH343G + BGT-SMPS (manual)

Status: **not run.** This page is the procedure and the record sheet. The result cells are for the owner to fill in after running it on the real hardware. Nothing on this page is a claim that BusLink works on the phone until those cells say so. Simulator runs and the unit tests of the USB bridge are not hardware evidence (safety rule 12).

Epic E6 exit criterion (journey 2): the APK reads the real sensor on the A35 through OTG with the same profile file as Windows.

## Equipment

| Item | Detail |
|---|---|
| Phone | Samsung Galaxy A35 (SM-A356E), Android 16 / API 36 |
| OTG adapter | Vention USB-C OTG (no charging port: the phone runs on battery) |
| RS-485 adapter | Waveshare USB TO RS485 (B), WCH CH343G, VID:PID `1A86:55D3`, **not isolated** |
| Sensor | BGT-SMPS 8-in-1 soil sensor, unit 1, 9600 8N1, FC03 from offset 0, 8 registers |
| Profile | `profiles/bgt-smps-8in1.json` (same file as the Windows test) |
| Power | Sensor supply as in its manual; A/B wired, GND between adapter and sensor |
| APK | `release/BusLink-Studio-<version>-android-arm64-debug.apk`, SHA-256 from `release/SHA256SUMS.txt` |

Before starting: battery ≥ 80 %, set BusLink to "Unrestricted" battery use (Settings → Apps → BusLink Studio → Battery) for step 9, and note the APK hash you installed.

## Steps

| # | Step | Expected (pass condition) |
|---|---|---|
| 1 | `adb -s <serial> install -r <apk>`; open BusLink Studio. | App starts in Monitoring; no port opened; no write. |
| 2 | Plug OTG + CH343G adapter (sensor wired and powered). | Android offers to open BusLink for the device (filter match). Choosing it opens the app; nothing connects by itself. |
| 3 | Connect → Serial (RTU): refresh ports. | One port `/dev/bus/usb/…#0`, `1A86:55D3`, chip `WCH CH343 (CDC-ACM)`, "permission needed" if not granted yet. |
| 4 | Choose the port, 9600 8N1, Connect. Deny the permission dialog. | Error text with recovery steps (replug, Allow, "use by default"). No second dialog appears by itself. Link not created. |
| 5 | Connect again, tick "Use by default", Allow. | Link `connected`. Isolation and OTG battery notes visible. |
| 6 | Add device: profile BGT-SMPS 8-in-1, unit 1. Start polling. Android 13+: answer the notification prompt (Allow). | Notification "Polling · … · 1 device(s)". Tiles show 8 values with units, quality GOOD and age < 3 s. |
| 7 | Inspector for the device. | Request `01 03 00 00 00 08 44 0C`; response 21 bytes, CRC ok. Note the cycle time and counters (requests, GOOD, timeouts, CRC errors). |
| 8 | Rotate the phone twice; switch to another app and back. | Values keep updating; link and device still there; no reconnect needed. Control (if armed) is disarmed on background. |
| 9 | Screen off for 10 minutes, then on. | Trend has samples across the 10 min with no gap (or the gap is shown honestly with its reason). Counters grew. Note battery drop. |
| 10 | Unplug the adapter while polling. | Within a few seconds: link `down`/DISCONNECTED, tiles show last value dimmed with growing age, notification gone. Session data kept. |
| 11 | Plug the adapter back in. | Nothing resumes by itself. Reconnect + Start polling needed; then GOOD again. |
| 12 | Export CSV, XLSX and JSON; save the HTML report. | System "save as" picker each time; files open on the phone (or after copying to a PC) with quality and age columns. Cancelling says "not saved". |
| 13 | Disconnect from the notification action. | Port closed, link down, notification gone, no crash. |
| 14 | Swipe the app away from recents while polling. | Service and notification stop; next launch starts in Monitoring with no open port. |
| 15 | Optional: repeat 3–7 with a CH340 adapter (`1A86:7523`). | Chip `WCH CH340 (CH34x)`, values GOOD. |

## Record (owner fills in)

Date: ____  Tester: ____  APK SHA-256: ____  BusLink version: ____
Phone firmware/build: ____  Adapter: ____  Sensor firmware (if known): ____

| # | Result (pass / fail / not run) | Observed (values, counters, cycle time, battery, notes) |
|---|---|---|
| 1 | | |
| 2 | | |
| 3 | | |
| 4 | | |
| 5 | | |
| 6 | | |
| 7 | | |
| 8 | | |
| 9 | | |
| 10 | | |
| 11 | | |
| 12 | | |
| 13 | | |
| 14 | | |
| 15 | | |

Evidence files (screenshots without notifications or personal data, `adb logcat` excerpt, exported files): ____

Only after steps 3, 5, 6 and 7 pass may `profiles/bgt-smps-8in1.json` get an `evidence.hardware` entry for the Android platform (date, adapter, platform, record, result). Upgrading `support_level` to `hardware_verified` is a separate decision recorded with that entry.

## What was verified without hardware (for context, not evidence)

- Rust: `BufferedUsb` unit tests and RTU exchanges over a fake USB port (`crates/buslink-core/tests/usb_transport.rs`): replies split at every byte, byte-per-packet, coalesced with stray bytes, silence → TIMEOUT, bad CRC → CRC_ERROR, detach → link lost, late reply not matched.
- Kotlin: `PacketQueueTest` (JVM) for the reader-side queue, if run (`gradlew :tauri-plugin-buslink-usb:testDebugUnitTest`).
- The debug APK builds for arm64. Installing it, the permission dialog, the foreground service, screen-off polling, detach handling and SAF on a real provider were **not** observed.
