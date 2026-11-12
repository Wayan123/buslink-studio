# android-otg: design

Implements `requirements.md`. Builds on `core-protocol` (RTU framer, exchanger, `Transport`), `app-shell-desktop` (session, shell, `usb_ids`) and `dashboard-ui` (connect view).

## 1. Layers and where the code lives

```text
webview (TS)  ── invoke ──▶  src-tauri commands (async, spawn_blocking)
                                   │
              session factory ─────┤ serial::open(app, "<dev>#<idx>", settings)
                                   ▼
        android_usb.rs   BufferedUsb<AndroidUsbPort>  ◀── buslink-core/transport/usb.rs  (pure Rust, tested in WSL)
                                   │ run_mobile_plugin (blocking, hex strings)
                                   ▼
        plugins/tauri-plugin-buslink-usb   Rust: typed calls, desktop stub
                                   │ JNI (Tauri mobile plugin)
                                   ▼
        Kotlin BusLinkUsbPlugin ── UsbRuntime ── SerialSession (reader thread) ── PacketQueue
                                   │                         └─ usb-serial-for-android 3.12.0
                                   ├─ PollingService (connectedDevice FGS)
                                   └─ SAF save (ACTION_CREATE_DOCUMENT)
```

| Part | Location | Why there |
|---|---|---|
| `UsbPort` trait, `BufferedUsb`, hex helpers, `FakeUsbPort` | `crates/buslink-core/src/transport/usb.rs` | No Tauri, no Android: unit-tested with `cargo test` in WSL against a fake port (Req 4.3). Same trait would serve a future `tauri-plugin-serialplugin` fallback (tech.md). |
| Plugin Rust API | `plugins/tauri-plugin-buslink-usb/src/` | Typed, blocking calls. Compiles on desktop to stubs returning `Unsupported`, so `cargo.exe check` of the app covers the glue. Exposes **no webview command** (Req 9.2): `build.rs` has an empty command list, the capability file is unchanged. |
| Android glue | `src-tauri/src/android_usb.rs`, `files.rs`, `serial.rs` | `UsbPort` adapter, permission-then-open, foreground-service sync, SAF save. |
| Kotlin | `plugins/.../android/src/main/java/id/co/arkamaya/buslink/usb/` | `BusLinkUsbPlugin`, `UsbRuntime`, `SerialSession`, `PacketQueue`, `PollingService`, `Hex`. JVM tests for `PacketQueue` and `Hex`. |
| Generated project hand edits | `src-tauri/gen/android/` | Only what Gradle/Manifest need; listed in `docs/android-build.md`. |

## 2. The byte bridge (Req 4)

Wire format: hex strings in JSON (`readPort` → `{data, state}`, `writePort {portId, data}`). No base64 or binary channel is needed at 9600–115 200 baud, and hex is easy to read in logs.

**Kotlin side.** `SerialSession` runs one daemon thread: `port.read(buf, 50 ms)`; `n > 0` → `PacketQueue.offer(copy)`. `PacketQueue` is bounded (8 KiB), keeps packet order, splits a packet larger than the caller's `maxBytes`, drops the oldest bytes when full (counted), and `end(DETACHED | CLOSED)` discards queued bytes. `readPort` is `queue.poll(timeout ≤ 5 s, max ≤ 1024)` on a cached executor, so the Tauri command thread never blocks.

**Rust side.** `AndroidUsbPort::read_packet(timeout)` = one `readPort` call. `BufferedUsb` (core) sits on top:

| Case | Behaviour |
|---|---|
| packet bigger than the caller's buffer | the rest stays in a `VecDeque` and is served by the next `read` without another USB call |
| two packets before the caller reads | served in order; nothing lost below the 4 KiB capacity |
| no packet before the timeout | `Ok(0)` (never an error) |
| `Disconnected` from the port | buffer cleared, every later `read`/`write` returns the same `Disconnected` |
| overflow | oldest bytes dropped, `overflowed()` counts them |
| `drain(quiet_ms)` | trait default: read until quiet, through the buffer (no hardware purge, CH340-safe) |

Framing stays where it was: the exchanger's length-based `ResponseFramer` reads whatever arrives and validates unit, CRC, function and length. The adapter never inspects Modbus bytes.

Android link settings: `min_inter_frame_ms = 50` for RTU links, high-baud warning above 115 200 in `ConnectResult`.

**Errors.** Kotlin rejects with text starting `disconnected:` or `permission:`; `map_err` in `android_usb.rs` turns them into `TransportError::Disconnected` / `PermissionDenied`, anything else into `Io`.

## 3. Permission and open (Req 3, 5)

`serial::open` (Android) runs inside `Session::connect`, which runs inside `spawn_blocking` (`connect_link` is now an async command; a sync command would run on the main thread, which must stay free to receive the permission broadcast).

```text
list_devices ─ port found? ─ no ─▶ Disconnected("adapter not connected")
      │ yes
has_permission? ─ no ─▶ requestPermission (blocks ≤ 60 s) ─ granted ─▶ continue
      │ yes                                                └ denied / timeout / busy / unavailable ─▶ Io(message), no retry
openPort(baud, bits, parity, stop)
```

Kotlin `requestPermission`: refuses if the app is not started (foreground), refuses a second request (`busy`), registers a per-request receiver (`RECEIVER_NOT_EXPORTED`), `PendingIntent.getBroadcast` with an explicit package, `FLAG_MUTABLE | FLAG_CANCEL_CURRENT` and the request ID as request code and extra, a 60 s timeout on the main handler, and cancels (state `unavailable`) on detach, `onStop` and `onDestroy`. The receiver checks action, request ID and device name. `openPort` re-checks `hasPermission` itself, so a late grant never opens anything on its own.

Detach: a receiver registered in `load()` for `ACTION_USB_DEVICE_DETACHED` (system broadcast, `RECEIVER_NOT_EXPORTED`) → `UsbRuntime.detached(name)`: sessions of that device are removed and closed with `DETACHED`, the event text is stored (`serviceStatus.lastEvent`), the service is stopped. The next Rust `read`/`write` gets `Disconnected` → the worker goes `Down`, tags become DISCONNECTED (existing session rule). Nothing reopens the port; reattach needs Connect and Start polling again (Req 5.3).

## 4. Foreground service (Req 6)

Manifest (in the plugin, merged): `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `POST_NOTIFICATIONS`, `WAKE_LOCK`, `CHANGE_NETWORK_STATE`, `<service … foregroundServiceType="connectedDevice" exported="false">`.

Start (`start_polling`, a user action, app in foreground):

1. `requestNotificationPermission` (Android 13+ prompt, once per call; no-op when granted or older).
2. `startService` (Kotlin refuses when the activity is not at least STARTED). `ContextCompat.startForegroundService` → `PollingService.onStartCommand` → `ServiceCompat.startForeground(..., FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)`. On exception the service records the error and stops; there is no `dataSync` fallback. The plugin waits ≤ 3 s for running/error and rejects with the text.
3. `session.start_polling`. A service failure does not block polling; `StartResult { background, notifications_denied, error }` tells the UI, which says honestly that polling stops with the screen off or when the app closes.

Keep-alive and stop (`state.rs`, emitter thread, once per second): `sync_service` looks at the non-simulated links. While one is `Polling`, it sends `updateService` (label, state, device count) at most every 5 s; the same call refreshes a 30 s partial wake lock (it expires by itself if the app stops reporting). No polling link → `stopService`. Therefore the service stops on user disconnect, polling stop, link `Down` (detach, I/O failure) and `RunEvent::Exit`. It also stops on task removal (`onTaskRemoved`), on the notification's Disconnect action (closes the ports; the link goes `Down`) and when the activity is finishing. `START_NOT_STICKY`: a system kill never restarts polling by itself (Req 6.8, safety 6).

Notification text comes from Android resources (`values`, `values-in`): "Polling · <link> · N device(s)", "Connected, not polling", "DISCONNECTED". A small icon and a Disconnect action are included.

Updates call the running service instance directly (same process) so no new foreground-service start is attempted from the background (API 31+ rule).

## 5. Session state in Rust (Req 7)

`AppState` (session, control service, training, emitter) is created in the Tauri `setup` and managed by the app, not by the activity. The manifest keeps Tauri's `configChanges` (orientation, screenSize, …) so rotation does not recreate the activity. If the activity or webview is recreated anyway, `main.ts` already calls `live_snapshot` before subscribing to frames (existing behaviour), so the dashboard rebuilds from the core. Backgrounding disarms control (existing `visibilitychange` handler). The plugin closes ports only when the activity is *finishing*, not on recreation.

## 6. SAF export (Req 8)

```text
UI export ─▶ export_session / audit_export / report_save_html
          ─▶ files::save_bytes(app, name, label, ext, bytes)
               desktop: dialog plugin (unchanged)
               android: exportDir() → write cacheDir/exports/export-<n>.<ext> (generated name)
                        saveDocument{cacheFileName, displayName, mime}
                        Kotlin: name regex + extension allow-list (csv, xlsx, json, html) + MIME must match
                                one picker at a time → ACTION_CREATE_DOCUMENT (CATEGORY_OPENABLE, EXTRA_TITLE)
                                OK → copy on an executor ("wt"), failure → DocumentsContract.deleteDocument
                                cancel → saved=false ("not saved"); cache file always deleted
```

The dialog plugin is a desktop-only dependency (`cfg(not(target_os = "android"))`), so the APK carries neither its Kotlin code nor its permissions. Nothing from the webview chooses a path or file name on disk.

## 7. Build and Gradle (Req 1)

- `tauri android init` generated `src-tauri/gen/android` (Gradle 9.6.1, AGP 9.3.1, Kotlin 2.2.10, compileSdk 37 platform installed). Hand edits: `targetSdk = 36`, debug `usesCleartextTraffic=false`, JitPack scoped to `com.github.mik3y`, manifest (USB attach filter, `usb.host` optional, `allowBackup=false`), `res/xml/device_filter.xml`. `BuildTask.kt` is the 2.12 template (`npm` with `.exe/.cmd/.bat` fallbacks) and is unchanged unless the build proves it broken.
- The plugin's `android/build.gradle.kts` uses `consumer-rules.pro` (Tauri 2.12), compileSdk 36, minSdk 24, `usb-serial-for-android:3.12.0`.
- Debug APK, `--target aarch64` only (the emulator ABI x86_64 has no USB and an unverified rusqlite link, research/03 row 59).
- Build from WSL through `cmd.exe /c` with `JAVA_HOME` (JDK 23 installed; AGP 9.3 needs 17+), `ANDROID_HOME`, `NDK_HOME` (28.2).

## 8. Properties (correctness)

| ID | Property | Checked by |
|---|---|---|
| A1 | For any split of a valid RTU reply into USB packets (every single cut, byte-per-packet), the exchange decodes the same registers as the unsplit reply. | `tests/usb_transport.rs` |
| A2 | `BufferedUsb` returns exactly the bytes it was given, in order, whatever the caller's buffer size, below capacity. | `transport::usb::tests` |
| A3 | A read with no packet returns `Ok(0)`; silence yields quality TIMEOUT, never a value. | unit + integration |
| A4 | After `Disconnected`, buffered bytes are gone and all I/O fails the same way; a link loss is reported by the exchanger as link lost. | unit + integration |
| A5 | A late reply after a timeout is not matched to the next request. | integration |
| A6 | `PacketQueue` keeps order, splits large packets without loss, reports timeout as `Timeout`, discards on end. | Kotlin JVM tests |
| A7 | Port IDs round-trip: `<device>#<index>`; anything else is refused. | `android_usb::tests` |
| A8 | Export names/types outside the allow-list are refused on both sides. | `files::tests`, Kotlin regex/map |
| A9 | The plugin and the bridge contain no Modbus write encoding. | review + `guard-write-paths` hook (no FC05/06/15/16 builder outside `command/`) |

Not property-testable here, therefore manual (docs/verification/android-hardware.md): permission dialog, real CH343/CH340 behaviour, OTG power, screen-off polling, FGS survival, SAF with a real provider.

## 9. Risks

| Risk | Handling |
|---|---|
| Blocking Rust↔Kotlin calls per read at 9600 baud add latency | One call per USB packet with a 50 ms-class timeout; polling at ≥ 500 ms intervals is far above the IPC cost. Measure on the A35 (hardware doc step 7). |
| `run_mobile_plugin` on the main thread would deadlock the permission flow | Every blocking command is async + `spawn_blocking`; documented in code. |
| JitPack availability at build time | Scoped repository; the dependency is cached by Gradle after the first build. |
| Foreground-service start refused (OEM, battery policy) | Reported in the UI; polling still runs in the foreground. |
| Process killed by the OS with screen off | Not claimed otherwise: docs ask for "Unrestricted" battery mode; OEMs may still kill. |
