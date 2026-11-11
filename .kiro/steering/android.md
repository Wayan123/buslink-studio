---
inclusion: fileMatch
fileMatchPattern: ["src-tauri/gen/android/**", "plugins/tauri-plugin-buslink-usb/**"]
---

# Android rules (USB OTG, services, export)

Sources: Android USB host guide, Android 14/15/17 behavior changes, FGS service-type docs, usb-serial-for-android README/FAQ, Tauri mobile plugin docs (see `research/03-platform-tauri-android-usb.md` §1.3–1.4). Test device: Samsung A35 (SM-A356E, Android 16 / API 36). The emulator has no USB passthrough: USB features are tested on the phone only, aarch64 builds only.

## Plugin structure

- `plugins/tauri-plugin-buslink-usb/`: Rust crate (commands, `PluginHandle::run_mobile_plugin`) + `android/` Kotlin library: a class extending `app.tauri.plugin.Plugin`, annotated `@TauriPlugin`, with `@Command fun x(invoke: Invoke)` and `@InvokeArg` argument classes. Long work runs on an executor or own CoroutineScope, never on the main thread.
- Dependency: `com.github.mik3y:usb-serial-for-android:3.12.x` from JitPack, scoped: `maven("https://jitpack.io") { content { includeGroup("com.github.mik3y") } }`.
- Tauri 2.12: plugins ship `consumer-rules.pro` (not `proguard-rules.pro`).
- Keep custom code in the plugin, not in `src-tauri/gen/android/`. Re-running `tauri android init` regenerates `gen/android`; every hand edit there (manifest intent filter, Gradle fixes) is listed in `docs/android-build.md` so it can be re-applied.
- Known WSL/Windows build fix (from the owner's earlier project): the generated `buildSrc/.../BuildTask.kt` may call a missing `node.bat`; call `node.exe` with an explicit `node_modules/@tauri-apps/cli/tauri.js` path and pass arguments as a list so paths with spaces survive.

## Manifest

- `<uses-feature android:name="android.hardware.usb.host" android:required="false"/>` (the app still works over TCP without USB host).
- Activity: `launchMode="singleTask"`, intent filter `android.hardware.usb.action.USB_DEVICE_ATTACHED` + `<meta-data android:name="android.hardware.usb.action.USB_DEVICE_ATTACHED" android:resource="@xml/device_filter"/>`.
- Permissions: `INTERNET`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `POST_NOTIFICATIONS` (API 33+, runtime), `WAKE_LOCK`, `CHANGE_NETWORK_STATE` (covers the connectedDevice prerequisite in TCP-only sessions). No storage permissions. `ACCESS_LOCAL_NETWORK` only when targetSdk moves to 37, with a runtime request and rationale.
- `android:usesCleartextTraffic="false"`; `allowBackup` must not back up secrets.

## device_filter.xml

IDs **must be `0x` hex**: bare numbers are read as decimal, and an unparsable value turns the row into a wildcard.

```xml
<resources>
  <usb-device vendor-id="0x1A86" product-id="0x55D3"/> <!-- WCH CH343 (USB TO RS485 (B)), CDC-ACM -->
  <usb-device vendor-id="0x1A86" product-id="0x7523"/> <!-- WCH CH340 -->
  <usb-device vendor-id="0x1A86" product-id="0x5523"/> <!-- WCH CH341 -->
  <usb-device vendor-id="0x1A86" product-id="0x55D4"/> <!-- WCH CH9102, CDC-ACM -->
</resources>
```

The filter only controls auto-launch and the "use by default" offer; the prober decides the driver (CH343/CH9102 → `CdcAcmSerialDriver`, CH340/CH341 → `Ch34xSerialDriver`). Show the chip name and VID:PID in the UI.

## USB permission flow

1. Enumerate with `UsbManager.deviceList` + `UsbSerialProber.getDefaultProber()`. Port IDs are `<deviceName>#<portIndex>`, never COM names (projects moved from Windows must re-map the port).
2. If `hasPermission(device)` is false, call `requestPermission(device, pendingIntent)`:
   - Intent with an app-specific action (`${packageName}.USB_PERMISSION`), **`setPackage(packageName)`** (explicit, required for mutable PendingIntents on API 34+), a request ID extra.
   - `PendingIntent.FLAG_MUTABLE` (the system adds `EXTRA_DEVICE` / `EXTRA_PERMISSION_GRANTED`) `| FLAG_CANCEL_CURRENT`.
   - Receiver registered with `ContextCompat.registerReceiver(..., RECEIVER_NOT_EXPORTED)`; it checks action, device name and request ID.
3. Only one pending request at a time, with a timeout (60 s). Cancel it on detach, `onStop` and `onDestroy`; a late grant must not open the port in the background.
4. Denied → show recovery steps (replug, allow, "use by default"), never loop prompts, never crash.
5. Grants survive replug when VID/PID are in the filter, but may be lost after a reboot with the device attached (Google Won't Fix): re-check permission on start.

## Port I/O

- `openDevice` → `port.open(connection)` → `setParameters(baud, dataBits, stopBits, parity)`; flow control none; the Waveshare (B) switches RS485 direction in hardware.
- Reads on a dedicated thread (or `SerialInputOutputManager` with `setReadQueue`) with short timeouts (≈50 ms); a zero-byte timed read is a timeout, not end of stream. Feed bytes to the Rust RTU parser, which frames by expected length.
- CH340 driver: hardware purge throws `UnsupportedOperationException`; drain input by reading until quiet instead.
- Prefer ≤ 115 200 baud on Android (non-real-time scheduling loses fragments at high rates). Non-standard baud rates may be rejected by CDC devices.
- `ACTION_USB_DEVICE_DETACHED` → cancel in-flight requests, close the port, mark the link DISCONNECTED, log it, keep the session data. Reattach never auto-resumes control or writes.

## Foreground service (background polling and logging)

- One service with `android:foregroundServiceType="connectedDevice"`, `exported="false"`; started from a user action **while the app is in the foreground** and only **after** USB permission is granted (that satisfies the connectedDevice prerequisite; API 31+ forbids starting FGS from the background).
- `ServiceCompat.startForeground(..., FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)`. If it throws (missing permission → `SecurityException` on API 34+), stop and report; do not fall back to `dataSync` (capped at 6 h on API 35).
- Persistent notification with device, state and a Disconnect action; partial wake lock only while polling; stop the service on disconnect, detach, error or session end.
- Battery: ask the user to set the app to "Unrestricted" for long logs; OEMs may still kill it. Phones usually cannot charge in OTG host mode with the Vention adapter: show battery % and warn.
- Session state lives in the Rust core so rotation or activity recreation never loses devices or configuration.

## Export (Storage Access Framework)

- Rust writes the export to the app cache (`cacheDir/exports/`, generated name, allow-listed extension and MIME type), then the plugin fires `ACTION_CREATE_DOCUMENT` (`CATEGORY_OPENABLE`, MIME, `EXTRA_TITLE`).
- On `RESULT_OK`, copy via `contentResolver.openOutputStream(uri, "w")` off the main thread; on failure delete the partial document (`DocumentsContract.deleteDocument`). Cancel resolves as "not saved". One picker at a time. Delete the cache file afterwards.
- No `READ/WRITE_EXTERNAL_STORAGE`, no `MANAGE_EXTERNAL_STORAGE`. Share sheet (FileProvider) is optional and uses the same cache file.

## Network

- The LAN dashboard server on Android binds to the Wi-Fi interface address chosen by the user, never 0.0.0.0 by default; shown only while the FGS runs.
- Modbus TCP over Wi-Fi works with `INTERNET` at targetSdk 36. Client isolation on Wi-Fi can block LAN access: show that hint on connection failure.
