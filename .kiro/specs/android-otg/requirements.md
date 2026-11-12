# android-otg: requirements

Epic E6 (`docs/bmad/epics-and-sprint.md`). Covers PRD R2 and the Android part of R21; architecture D2 (transport trait, Android USB through a Kotlin plugin) and D10 (targetSdk 36, minSdk 24, `connectedDevice` FGS after USB permission).
Code: `crates/buslink-core/src/transport/usb.rs` (buffering adapter), `plugins/tauri-plugin-buslink-usb/` (Rust + Kotlin), `src-tauri/` (Android glue, build config), `src-tauri/gen/android/` (generated; hand edits listed in `docs/android-build.md`).
Rules that always apply: `.kiro/steering/android.md`, `safety.md` (rules 1, 5, 6, 9, 11, 14, 18), `modbus.md` (RTU framing), `tech.md`.

Out of scope here: LAN server and MQTT (E7), release signing, Play Store, iOS, x86/armv7 ABIs, a control panel for real devices, any write path (none is added; writes stay in `command/`).

Honesty rule for this spec: no requirement is "verified on hardware" until the owner fills in `docs/verification/android-hardware.md`. Build and unit-test results are reported as such.

## Requirement 1: Android target (R2, D10)

**User story:** As a technician, I want an APK of the same app, so that I can use the same profiles on my phone.

1. THE app SHALL build an Android project with identifier `id.co.arkamaya.buslink`, minSdk 24, **targetSdk 36**, ABI arm64-v8a only for this release.
2. THE app SHALL NOT request `ACCESS_LOCAL_NETWORK` while targetSdk is 36.
3. THE manifest SHALL set `usb.host` as `required="false"`, `usesCleartextTraffic="false"`, and SHALL NOT request storage permissions.
4. THE generated Android project SHALL stay regenerable: all custom Kotlin lives in the plugin; every hand edit under `src-tauri/gen/android/` SHALL be listed in `docs/android-build.md`.
5. THE debug APK SHALL be built through the Windows toolchain from WSL, copied to `release/BusLink-Studio-<version>-android-arm64-debug.apk`, and its SHA-256 recorded.

## Requirement 2: USB serial enumeration (R2, D2)

**User story:** As a technician, I want to see my adapter with its chip name, so that I know the phone sees it.

1. THE plugin SHALL enumerate USB serial devices with usb-serial-for-android 3.12.x (MIT) and report per port: device name, port index, VID, PID, manufacturer, product, serial number (when readable), driver class name, and whether the app holds USB permission.
2. THE port ID SHALL be `<deviceName>#<portIndex>`; COM names SHALL NOT be used on Android.
3. THE app SHALL label the chip from VID:PID only, using the existing table (`1A86:55D3` CH343, `1A86:7523` CH340, `1A86:5523` CH341, `1A86:55D4` CH9102). CP210x and FTDI devices SHALL be listed and usable but unlabeled by the table.
4. THE driver choice SHALL be made by the library prober: CH343 and CH9102 through `CdcAcmSerialDriver`, CH340 and CH341 through `Ch34xSerialDriver`, plus CP210x and FTDI drivers.
5. `device_filter.xml` SHALL list `1A86:55D3`, `1A86:7523`, `1A86:55D4` (plus `1A86:5523`) in `0x` hexadecimal.
6. A device with no serial driver SHALL be left out of the list and SHALL NOT crash enumeration.

## Requirement 3: USB permission (R2)

1. WHEN the user connects and the app has no permission for the device, THE plugin SHALL call `UsbManager.requestPermission` with an explicit, package-scoped, mutable `PendingIntent` and a receiver registered `RECEIVER_NOT_EXPORTED`.
2. THE plugin SHALL keep at most one pending request; a second request while one is pending SHALL be refused with a message.
3. THE request SHALL time out after 60 s and be cancelled on detach, activity stop and activity destroy; a late grant SHALL NOT open a port.
4. WHEN the user denies, THE connect SHALL fail with a "USB permission denied" message that carries the recovery steps (replug, allow, "use by default"); THE app SHALL NOT prompt again until the user presses Connect again.
5. THE app SHALL re-check permission at every connect, not cache it.

## Requirement 4: serial I/O and the Transport bridge (R2, D2)

**User story:** As a technician, I want RTU replies that arrive split or merged by USB to decode correctly.

1. THE plugin SHALL open a port with baud, data bits (7/8), parity (N/E/O) and stop bits (1/2), no flow control, and SHALL reject invalid settings before opening.
2. THE plugin SHALL read on its own thread with ≈50 ms timeouts into a bounded packet queue; a timed-out read SHALL be an empty result, never an error.
3. THE Rust side SHALL implement `Transport` over a packet-level `UsbPort` trait with a buffering adapter (`BufferedUsb`) that (a) returns bytes of one USB packet over several `read` calls when the caller's buffer is smaller, (b) returns the bytes of several packets over one call without losing the rest, (c) reports a read with no packet before the timeout as `Ok(0)`, (d) never reorders or drops bytes below its capacity, and (e) reports overflow above the capacity instead of growing without bound.
4. WHEN the device detaches or the port closes, THE next `read` or `write` SHALL return `Disconnected` and buffered bytes SHALL be discarded; every tag of the link SHALL become DISCONNECTED (existing session rule).
5. THE drain operation SHALL work by reading until quiet (no hardware purge).
6. THE Android link SHALL use an inter-frame floor of 50 ms (modbus.md) and the existing length-based RTU framer; baud above 115 200 SHALL show a warning.
7. THE bridge SHALL never build a write frame; `write_all` carries the bytes the command service or poller already encoded (safety rule 2).
8. ONE port SHALL have one owner: the plugin SHALL refuse a second `open` of the same port ID (safety rule 9).

## Requirement 5: detach and reattach (R2, safety 6)

1. WHEN `ACTION_USB_DEVICE_DETACHED` arrives for the open device, THE plugin SHALL cancel in-flight reads, close the port, record a detach event, and the link SHALL show DISCONNECTED.
2. THE session data SHALL be kept after detach.
3. Reattach SHALL NOT resume polling and SHALL NOT arm or write anything; the user reconnects and starts polling again.

## Requirement 6: foreground service (R2, D10)

**User story:** As a technician, I want logging to continue with the screen off, and to know the link status from the notification.

1. THE plugin SHALL declare one service with `foregroundServiceType="connectedDevice"` (`exported="false"`) and the permissions `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `POST_NOTIFICATIONS`, `WAKE_LOCK` and `CHANGE_NETWORK_STATE`.
2. THE app SHALL start the service only from a user action (Start polling) **while the app is in the foreground** and, for a USB link, **after** USB permission was granted for the open port.
3. IF `startForeground` throws, THE plugin SHALL stop the service, report the error and SHALL NOT fall back to another service type.
4. THE notification SHALL show the link status in text (device or chip, polling / stopped / disconnected, number of devices) and a Disconnect action, and SHALL be updated at most every 5 s.
5. THE service SHALL hold a partial wake lock only while polling.
6. THE service SHALL stop on user disconnect, polling stop with no other polling link, detach, link failure, task removal and app exit.
7. ON Android 13+ THE app SHALL request `POST_NOTIFICATIONS` before the first service start; IF denied, polling SHALL still start (the service runs, the notification may be hidden by the system) and the UI SHALL say so.
8. THE service SHALL NOT start any polling by itself; it only keeps the process alive (session state is in Rust).

## Requirement 7: session survives the activity (R2)

1. THE session, links, devices, historian writer and alarm state SHALL live in the Rust core (`AppState`), not in the activity or the webview.
2. WHEN the activity is recreated or the webview reloads, THE UI SHALL rebuild from `live_snapshot` and the connection and polling SHALL continue.
3. THE manifest SHALL keep the Tauri `configChanges` so rotation does not recreate the activity, and recreation SHALL still be safe (2).

## Requirement 8: export through SAF (R21)

1. THE app SHALL save CSV, XLSX, JSON, the audit export and the HTML report on Android through `ACTION_CREATE_DOCUMENT` (`CATEGORY_OPENABLE`, MIME type, `EXTRA_TITLE`).
2. Rust SHALL write the bytes to `cacheDir/exports/<generated name>` (allow-listed extension, no path from the webview); the plugin copies them to the chosen URI off the main thread and deletes the cache file afterwards.
3. ON failure the plugin SHALL delete the partial document; cancelling SHALL be reported as "not saved".
4. ONE picker SHALL be open at a time.
5. THE app SHALL NOT use `WRITE_EXTERNAL_STORAGE`, `MANAGE_EXTERNAL_STORAGE` or the `dialog` plugin on Android.

## Requirement 9: safety and honesty on Android

1. WHEN an RTU link is connected on Android, THE connect response SHALL carry the non-isolation advice and the UI SHALL add the battery note (no charging in OTG host mode) (safety rule 18).
2. THE plugin SHALL expose no write command and the webview SHALL have no direct access to plugin commands (only Rust calls the plugin).
3. NOTHING in the bridge SHALL retry a write; read retries stay in the poller (safety rule 5).
4. THE docs SHALL say that the A35 + OTG + CH343G + sensor test is a manual procedure and SHALL leave the result cells empty until the owner fills them.
