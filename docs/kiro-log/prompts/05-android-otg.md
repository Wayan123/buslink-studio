Create the spec and implement epic E6 `android-otg` (docs/bmad/epics-and-sprint.md), as the buslink-builder agent.

Read: docs/bmad/prd.md (R2, R21 Android export), docs/bmad/architecture.md (D2, D10), research/03-platform-tauri-android-usb.md (all Android rows and the risks table), .kiro/steering/android.md, and the transport trait in buslink-core.

Reference, read only: /mnt/d/KERJA/PT AGS/Proyek 2026/Soil-Sensor-Android. It holds the verified Android build path from WSL with the Windows toolchain (docs/operations/android-build-diagnosis.md, .windows-tools/), the Kotlin USB permission and foreground service pattern, SAF export, and BufferedUsb packet handling. Learn from it; write fresh code; record adopted facts in docs/existing-code-disclosure.md.

## Spec
`.kiro/specs/android-otg/` with EARS requirements, correctness properties and tasks.

## Implement
1. **Android target:** `tauri android init` (identifier `id.co.arkamaya.buslink`), minSdk 24, **targetSdk 36**, arm64-v8a first.
2. **Kotlin Tauri mobile plugin** `plugins/tauri-plugin-buslink-usb`:
   - Enumerate USB serial devices with usb-serial-for-android 3.12.x (MIT). CH343 is handled by its CDC-ACM driver, CH340/CH341 by Ch34x; also CP210x and FTDI.
   - `device_filter.xml` lists VID/PID in 0x hex: 1A86:55D3, 1A86:7523, 1A86:55D4.
   - Request permission. Handle denial without loops. Report detach events.
   - Open with serial settings, read with timeout, write, close.
   - Bridge the bytes to the Rust `Transport` trait, with buffering so that fragmented USB packets do not break RTU length-based parsing.
3. **Foreground service** of type `connectedDevice`. Declare the `FOREGROUND_SERVICE` and `FOREGROUND_SERVICE_CONNECTED_DEVICE` permissions. Start it only after permission is granted and the user starts polling, while the app is in the foreground. It keeps polling with the screen off and shows a notification with the link status. It stops on disconnect, detach or app exit. Request POST_NOTIFICATIONS on Android 13+.
4. **Session state lives in Rust,** so rotation or activity recreation keeps the connection and the dashboard.
5. **Export through SAF** (`ACTION_CREATE_DOCUMENT`) for CSV/XLSX/JSON and the report.
6. **Build a debug APK** for arm64 through the Windows toolchain, following the reference project's notes (NDK under `C:\Users\wayandadang\AppData\Local\Android\Sdk`, JDK `C:\Program Files\Java\jdk-23` or a compatible JDK 17/21 if the AGP version needs it; check before changing anything). Put it at `release/BusLink-Studio-<version>-android-arm64-debug.apk` and record the SHA-256.
7. **Tests:**
   - Unit tests for the buffering adapter: fragmented and coalesced packets, timeout reported as zero bytes.
   - Rust tests with a fake USB transport.
   - Document a manual hardware test procedure in docs/verification/android-hardware.md (Samsung A35 + Vention USB-C OTG + CH343G adapter + BGT-SMPS sensor) with a pass/fail table. Leave the result cells for the owner to fill in, and do not claim hardware results.

Run `cargo test --workspace` and the APK build. Mark tasks `[x]`. Report what built, what did not, and why. Do not commit.
