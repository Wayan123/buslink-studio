# Android build (debug APK, arm64)

How the BusLink Studio APK is built from WSL with the Windows toolchain, and every hand edit made under `src-tauri/gen/android/` so it can be re-applied after `tauri android init`.

## Toolchain (checked 2026-11-12 on the owner's PC)

| Item | Value |
|---|---|
| Tauri CLI / crate | 2.12.4 / 2.12.5 |
| Gradle / AGP / Kotlin (Tauri 2.12 template) | 9.6.1 / 9.3.1 / 2.2.10 |
| JDK | `C:\Program Files\Java\jdk-23` (AGP 9.3 needs JDK 17+; 23 worked, no change needed) |
| Android SDK | `C:\Users\wayandadang\AppData\Local\Android\Sdk` (platforms 33–37, build-tools up to 37.0.0) |
| NDK | `28.2.13676358` |
| Rust target | `aarch64-linux-android` (Windows rustup) |
| Node / npm | Windows `node.exe` 24.15.0, `npm.cmd` 11.13.0 |

`JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME` are not set globally on this PC; they are set per call.

## Commands (from WSL, repo root)

```sh
# once, only to (re)generate the project; afterwards re-apply the hand edits below
cmd.exe /c "set JAVA_HOME=C:\Program Files\Java\jdk-23&& set ANDROID_HOME=C:\Users\wayandadang\AppData\Local\Android\Sdk&& set NDK_HOME=C:\Users\wayandadang\AppData\Local\Android\Sdk\ndk\28.2.13676358&& npm.cmd run tauri -- android init --ci"

# debug APK, arm64 only
cmd.exe /c "set JAVA_HOME=C:\Program Files\Java\jdk-23&& set ANDROID_HOME=C:\Users\wayandadang\AppData\Local\Android\Sdk&& set NDK_HOME=C:\Users\wayandadang\AppData\Local\Android\Sdk\ndk\28.2.13676358&& npm.cmd run tauri -- android build --debug --apk --target aarch64 --ci"
```

No space before `&&` in `set X=...&&`: `cmd.exe` would keep the trailing space in the value.

Output: `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk` (contains arm64-v8a only because of `--target aarch64`). Copy it to `release/BusLink-Studio-<version>-android-arm64-debug.apk` and record the SHA-256 in `release/SHA256SUMS.txt`.

Last build (2026-11-12, android-otg): `release/BusLink-Studio-0.1.0-android-arm64-debug.apk`, 182 473 045 bytes, SHA-256 `735ec1001c4c4b9f4ecbc3791f11fe54913580b15ccf691ca8229b46c6e95e8b`. `aapt2 dump badging`: package `id.co.arkamaya.buslink`, versionName 0.1.0, minSdk 24, targetSdk 36, native code `arm64-v8a` only; merged manifest has `PollingService` (`foregroundServiceType` connectedDevice, not exported), `usesCleartextTraffic=false`, `allowBackup=false`, the USB attach filter. Debug signing only; not installed on a phone by this build. The `.apk` is git-ignored (`release/*.apk`); `release/SHA256SUMS.txt` is kept.

Install on the test phone: `adb -s <serial> install -r <apk>`. Never uninstall or clear data to resolve a conflict.

## Hand edits under `src-tauri/gen/android/`

| File | Edit | Why |
|---|---|---|
| `app/build.gradle.kts` | `targetSdk = 36` (template wrote 37) | D10: stay on 36 until the Android 17 `ACCESS_LOCAL_NETWORK` flow exists. `compileSdk` stays 37. |
| `app/build.gradle.kts` | debug `manifestPlaceholders["usesCleartextTraffic"] = "false"` (template: `"true"`) | Req 1.3. The APK loads bundled assets; no dev server. `tauri android dev` would need it back to `true` locally. |
| `build.gradle.kts` | `maven("https://jitpack.io") { content { includeGroup("com.github.mik3y") } }` in `allprojects.repositories` | usb-serial-for-android comes from JitPack; scoped so no other artifact can resolve there. |
| `app/src/main/AndroidManifest.xml` | `<uses-feature android:name="android.hardware.usb.host" android:required="false"/>` | The app still installs and works over TCP on phones without USB host. |
| `app/src/main/AndroidManifest.xml` | `android:allowBackup="false"` on `<application>` | No backup of app data (history, future secrets). |
| `app/src/main/AndroidManifest.xml` | `USB_DEVICE_ATTACHED` intent filter + `<meta-data … android:resource="@xml/device_filter"/>` on `MainActivity` (already `singleTask`) | Auto-launch offer and "use by default" for the listed adapters. |
| `app/src/main/res/xml/device_filter.xml` | new file: `0x1A86:0x55D3` CH343, `0x7523` CH340, `0x5523` CH341, `0x55D4` CH9102 | IDs in `0x` hex (bare numbers are decimal; unparsable = wildcard). |

Not edited: `buildSrc/.../BuildTask.kt`. The 2.12 template calls `npm` with `.exe/.cmd/.bat` fallbacks and worked from WSL through `cmd.exe`; the older `node.bat` problem from the earlier Soil-Sensor project did not occur.

Everything else Android-specific (USB, service, permissions, SAF) lives in `plugins/tauri-plugin-buslink-usb/android/` and is merged by Gradle, so regeneration does not lose it.

## Known build noise

- AGP/Kotlin deprecation warnings (`android.newDsl=false`, legacy variant API) come from the Tauri template; not suppressed.
- `Daemon compilation failed … different roots` for `tauri-android`: the Tauri Kotlin library is compiled from the cargo registry on `C:` while the project is on `D:`; the Kotlin incremental cache cannot relativize the paths. Gradle prints "Using fallback strategy: Compile without Kotlin daemon" and continues. Harmless but slow; no change made.


## LAN dashboard and MQTT (lan-mqtt, E7)

Nothing under `src-tauri/gen/android/` was edited for this epic.

- `buslink-core` type-checks for `aarch64-linux-android` (including the `rustls-platform-verifier` branch): `cargo.exe check -p buslink-core --target aarch64-linux-android --target-dir target\windows` with `CC_aarch64_linux_android` / `AR_aarch64_linux_android` pointing at the NDK 28.2 `aarch64-linux-android24-clang.cmd` and `llvm-ar.exe`. An APK was **not** built and nothing ran on the phone.
- LAN server on Android: binds a private address from `getifaddrs` (if-addrs). `INTERNET` is already declared, and targetSdk 36 needs no `ACCESS_LOCAL_NETWORK`. Not enforced yet: running the server only while the foreground service runs (E6 owns that service). A sleeping phone will stop answering.
- MQTT TLS on Android uses `rustls-platform-verifier`, which needs its Kotlin component and Gradle repository before it works in an APK (see its README, "Android"). **To do**; until then an Android build can only trust a CA the user pastes in the MQTT panel, which also works on the platform verifier path (pasted CA only).
- Password storage on Android: **not implemented**. The shell keeps the MQTT password in memory only and the panel says so. A Kotlin Keystore command in `tauri-plugin-buslink-usb` is the planned fix.
