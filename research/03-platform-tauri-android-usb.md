# Research 03: Platform facts — Tauri 2, Android USB-OTG, CH343/CH340, Rust crates

Checked 2026-11-11. Every row cites the source that was opened. "Interpretation" means I drew the conclusion from the source but the source does not state it outright. **UNVERIFIED** means I could not confirm it from a primary source.

## Summary

- The Waveshare USB TO RS485 (B) uses a CH343G and an SP485EEN. Direction switching is automatic, and the board has TVS/ESD protection and a 120 Ω resistor. It is **not isolated**.
- The CH343 (1A86:55D3) enumerates as standard CDC-ACM. On Windows 10/11 it gets a COM port from the inbox driver without any install. On Android, usb-serial-for-android and the Tauri serial plugin both drive it with their generic CDC-ACM driver, not the CH34x driver.
- Tauri is at 2.12.3. `tauri-plugin-serialplugin` 4.2.1 covers desktop and Android USB serial with drivers written in Rust.
- `tokio-modbus` 0.17 can run RTU over any async byte stream, and `rmodbus` 0.13 is a transport-free frame codec. A from-scratch codec is not needed.
- On Android, three things need planning: the `connectedDevice` foreground-service rules, the `ACCESS_LOCAL_NETWORK` permission that applies once the app targets API 37, and the fact that most phones cannot charge while in OTG host mode.

## 1. Fact table

### 1.1 USB-serial chips (WCH CH343G / CH340)

| # | Fact | Value | Source | Confidence |
|---|------|-------|--------|------------|
| 1 | CH343 VID:PID | `1A86:55D3`. A Linux cdc-acm kernel quirk patch (Mar 2026) targets `USB_DEVICE(0x1a86, 0x55d3)` and calls the CH343 "as buggy as it is popular" | [LKML patch](https://lkml.iu.edu/2603.0/00185.html) | High |
| 2 | CH340 VID:PID | `1A86:7523`. CH341A is `1A86:5523` | [CH340 datasheet, config table](https://www.bitsavers.org/components/wch/_dataSheets/CH340DS1.PDF), [usb-serial-for-android UsbId.java](https://raw.githubusercontent.com/mik3y/usb-serial-for-android/master/usbSerialForAndroid/src/main/java/com/hoho/android/usbserial/driver/UsbId.java) | High |
| 3 | CH9102F VID:PID (related chip, also CDC) | `1A86:55D4` | [usb-serial-for-android example device_filter.xml](https://raw.githubusercontent.com/mik3y/usb-serial-for-android/master/usbSerialExamples/src/main/res/xml/device_filter.xml) | High |
| 4 | CH343 driver model | Works with the OS's built-in CDC driver, or with WCH's "high-speed VCP vendor driver" | [CH343 datasheet v1F §2, §5.3](https://cdn-learn.adafruit.com/assets/assets/000/134/549/original/CH343DS1.PDF?1737477957=) | High |
| 5 | Windows 10/11 inbox driver | Win10+ has a built-in CDC serial driver, so the COM port appears without any install. WCH still recommends its VCP driver (CH343SER.EXE) for full features and performance. In Device Manager, the VCP driver shows the friendly name "USB-Enhanced-SERIAL CH343". Any other name means the CDC driver is in use | [WCH SoftGroup blog (official WCH)](https://www.cnblogs.com/WCH-SoftGroup/p/16520875.html), [Waveshare wiki (archived 2025-11-20)](https://web.archive.org/web/20251120012124/https://www.waveshare.com/wiki/USB_TO_RS485_(B)) | High |
| 6 | Why the inbox driver binds | `usbser.sys` loads automatically on compatible ID `USB\Class_02&SubClass_02` | [Microsoft Learn: Usbser.sys](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/usb-driver-installation-based-on-compatible-ids) | High |
| 7 | CH343 baud range | 50 bps to 6 Mbps. Listed standard rates include 1M, 1.5M, 2M, 3M, 4M and 6M. Rx tolerance ≤2%, Tx error <1.5% | [CH343 datasheet §2, §5.3](https://cdn-learn.adafruit.com/assets/assets/000/134/549/original/CH343DS1.PDF?1737477957=) | High |
| 8 | CH343 RS485 support | Half-duplex mode. A 4.7 kΩ pull-down on DTR at power-up turns that pin into TNOW, which drives the RS485 DE/RE pins | [CH343 datasheet §5.3, §7.3](https://cdn-learn.adafruit.com/assets/assets/000/134/549/original/CH343DS1.PDF?1737477957=) | High |
| 9 | CH340 baud range | 50 bps to 2 Mbps | [CH340 datasheet §2](https://www.bitsavers.org/components/wch/_dataSheets/CH340DS1.PDF) | High |
| 10 | usb-serial-for-android: CH343 driver | Driven by **CdcAcmSerialDriver**, not Ch34xSerialDriver. The README lists "Qinheng CH343, CH9102" under generic CDC/ACM devices. Ch34xSerialDriver only handles 7523/5523 | [README](https://github.com/mik3y/usb-serial-for-android/), [UsbId.java](https://raw.githubusercontent.com/mik3y/usb-serial-for-android/master/usbSerialForAndroid/src/main/java/com/hoho/android/usbserial/driver/UsbId.java) | High |
| 11 | When CH343 started working there | v3.5.0 (commit 2023-03-11) switched CDC detection from a VID/PID list to USB interface class, so the CH343 is detected without a custom prober. Auto-launch on attach still needs VID+PID in `device_filter.xml`. I found no release note that names the CH343 specifically; "since 3.5.0" is my inference | [v3.5.0 release](https://github.com/mik3y/usb-serial-for-android/releases/tag/v3.5.0), [commit 5db4554](https://github.com/mik3y/usb-serial-for-android/commit/5db45548ba5241afa759c51dd9212c591afbea7a) | Medium (interpretation) |
| 12 | usb-serial-for-android current version | v3.12.0 (2026-11-10), MIT. It adds CH34x RTS/CTS flow control; this only affects the CH340 path | [Releases feed](https://github.com/mik3y/usb-serial-for-android/releases) | High |
| 13 | CDC-ACM feature limits on Android | Non-standard baud rates are "not supported by all devices". There is no CTS input line, and input-line state is unknown until the first change | [usb-serial-for-android FAQ, feature matrix](https://github-wiki-see.page/m/mik3y/usb-serial-for-android/wiki/FAQ) | High |

### 1.2 Waveshare USB TO RS485 (B) (SKU 22456)

The Waveshare wiki and product pages return HTTP 403 to automated fetches (Cloudflare). I cited archived copies from the Wayback Machine and Waveshare's own Chinese shop page.

| # | Fact | Value | Source | Confidence |
|---|------|-------|--------|------------|
| 14 | Chips | Original CH343G plus SP485EEN transceiver | [Waveshare product page (archived 2026-02-13)](https://web.archive.org/web/20260213112151/https://www.waveshare.com/usb-to-rs485-b.htm) | High |
| 15 | Direction control | "Hardware automatic control" (no RTS toggling needed) | same | High |
| 16 | Protection | TVS: 600 W lightning/surge suppression, 15 kV ESD on RS485. USB side: 200 mA self-recovery fuse and ESD. Also a lightning tube and protection diodes | same; [Waveshare CN shop](https://www.waveshare.net/shop/USB-TO-RS485-B.htm) | High |
| 17 | Isolation | **Not isolated.** Waveshare labels its isolated models "(隔离型)" and sells them as separate SKUs: RS485 (C), (D), (E), 2CH, and others. The (B) page never claims isolation and its sidebar entry is just "USB转RS485 (CH343方案)". One reseller URL slug says "isolated", which conflicts with the manufacturer | [Waveshare CN shop sidebar](https://www.waveshare.net/shop/USB-TO-RS485-B.htm), [archived product page](https://web.archive.org/web/20260213112151/https://www.waveshare.com/usb-to-rs485-b.htm) | Medium-high (interpretation) |
| 18 | Terminal pinout | Screw terminal: A+, B-, GND | [archived product page](https://web.archive.org/web/20260213112151/https://www.waveshare.com/usb-to-rs485-b.htm), [Switch Science listing](https://www.switch-science.com/products/8426) | High |
| 19 | 120 Ω resistor | Spec says "onboard 120R balancing resistor". **UNVERIFIED** whether a jumper or switch can disconnect it. Neither the wiki nor the product pages mention one. Check the board photo or schematic before writing "jumper" in the docs | same | Low for the jumper claim |
| 20 | Rated baud (board) | 300 bps to 3 Mbps on the product page; the wiki says 50 bps to 3 Mbps. The wiki FAQ lists rates up to 3M, including 250000 → no (only 256000 is listed) | [archived wiki](https://web.archive.org/web/20251120012124/https://www.waveshare.com/wiki/USB_TO_RS485_(B)) | High |
| 21 | Windows default | The wiki says the CDC driver is the default and the VCP driver (CH343SER) is optional | same | High |
| 22 | Bus | Up to 32 nodes, repeater recommended above 16; about 1.2 km at low rate; USB-A plug; operating range -15 to 70 °C | [archived product page](https://web.archive.org/web/20260213112151/https://www.waveshare.com/usb-to-rs485-b.htm) | High |
| 23 | OS list | Mac, Linux, Android, WinCE, Windows 11/10/8.1/8/7 | same | High |

### 1.3 Android USB host, OTG, foreground service

| # | Fact | Value | Source | Confidence |
|---|------|-------|--------|------------|
| 24 | Manifest basics | `<uses-feature android:name="android.hardware.usb.host"/>`, plus an intent filter for `USB_DEVICE_ATTACHED` and `<meta-data>` pointing to `res/xml/device_filter.xml`. Filter rows use `vendor-id`/`product-id` or `class`/`subclass`/`protocol` | [Android USB host guide](https://developer.android.com/develop/connectivity/usb/host) | High |
| 25 | Permission flow | If the app is launched through the attach intent filter, it gets permission automatically until the device is unplugged. Otherwise call `UsbManager.requestPermission(device, PendingIntent)` and handle `EXTRA_PERMISSION_GRANTED` in a BroadcastReceiver | same | High |
| 26 | Android 14 PendingIntent / receiver rules | Targeting API 34+: a mutable PendingIntent must use an explicit intent (component or package set), otherwise it throws. Runtime-registered receivers must pass `RECEIVER_EXPORTED` or `RECEIVER_NOT_EXPORTED` | [Android 14 behavior changes](https://developer.android.com/about/versions/14/behavior-changes-14) | High |
| 27 | Persisting permission | With VID/PID in `device_filter.xml`, the dialog offers "Use by default for this USB device", and the grant survives replug and reboot. If the device stays plugged in across a reboot, the grant is only sometimes kept (Google bug 77658221, Won't Fix). `android:directBootAware="true"` helps on some devices | [usb-serial-for-android FAQ](https://github-wiki-see.page/m/mik3y/usb-serial-for-android/wiki/FAQ) | High |
| 28 | device_filter values | Write IDs as `0x`-prefixed hex. Android reads bare numbers as decimal, and an unparsable ID turns that row into a wildcard. The Tauri serial plugin injects a filter that includes `<usb-device class="2"/>`, which matches CDC-ACM devices such as the CH343 | [serialplugin device_filter.xml](https://raw.githubusercontent.com/s00d/tauri-plugin-serialplugin/main/android/src/main/res/xml/device_filter.xml) | High |
| 29 | `connectedDevice` FGS requirements | Declare `android:foregroundServiceType="connectedDevice"`, plus the `FOREGROUND_SERVICE` and `FOREGROUND_SERVICE_CONNECTED_DEVICE` permissions. Pass `FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE` to `startForeground()`. **Prerequisite (at least one):** declare `CHANGE_NETWORK_STATE`, `CHANGE_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE`, `NFC` or `TRANSMIT_IR`; or be granted a Bluetooth/UWB runtime permission; or **call `UsbManager.requestPermission()`** | [FGS types](https://developer.android.com/develop/background-work/services/fgs/service-types) | High |
| 30 | API 34 enforcement | Targeting API 34+: a missing type-specific FGS permission throws `SecurityException` | [FGS changes](https://developer.android.com/develop/background-work/services/fgs/changes) | High |
| 31 | API 35 / 36 changes | API 35: `dataSync` FGS is capped at 6 h per 24 h, and `BOOT_COMPLETED` cannot start dataSync, camera, mediaPlayback, phoneCall, mediaProjection or microphone FGS. `connectedDevice` is **not** on that list. Android 16: jobs started from an FGS obey normal job quotas. Since API 31, an FGS generally cannot be started from the background | [Android 15 behavior changes](https://developer.android.com/about/versions/15/behavior-changes-15), [FGS changes](https://developer.android.com/develop/background-work/services/fgs/changes) | High |
| 32 | Play Console FGS declaration | Apps targeting 14+ must declare each FGS type in Play Console with a description and a video. For `connectedDevice`, the video must show in-app connection to the external device | [Play Console help](https://support.google.com/googleplay/android-developer/answer/13392821?hl=en) | High |
| 33 | Play target API (now) | Since 2026-08-31, new apps and updates must target API 36 (Android 16). Extensions run to 2026-11-01 | [Play target API requirements](https://support.google.com/googleplay/android-developer/answer/11926878) | High |
| 34 | Android 17 local-network permission | Apps **targeting API 37** need the runtime permission `ACCESS_LOCAL_NETWORK` (in the NEARBY_DEVICES group) for outgoing **and incoming** LAN TCP/UDP. This covers Modbus TCP clients and a LAN dashboard server. Apps targeting ≤36 keep implicit access through `INTERNET` and should not request it | [Local network permission](https://developer.android.com/privacy-and-security/local-network-permission), [Android 17 behavior changes](https://developer.android.com/about/versions/17/behavior-changes-17) | High |
| 35 | Vention OTG adapter (CDQ / CDQH0) | USB-C male to USB 3.0 (5 Gbps) female OTG adapter. It has **no charging port** | [Vention VN product page](https://vention.vn/products/dau-chuyen-otg-usb-c-male-to-usb-3-0-female-vention-cdq), [CDQ datasheet (TME)](https://www.tme.eu/Document/feefb28731501a015859ae7085bd5069/CDQ.pdf) | Medium (spec page text is thin) |
| 36 | Charging while in OTG | Usually not possible. It needs a PD pass-through adapter (Vention sells one: CCBHB, "USB-C M to USB-A F & 100W PD") **and** a phone that implements USB-C PD power-role negotiation. Many tablets and phones lack it, and some ROMs drop OTG data when charger power is applied | [LAVA: why some devices can't do simultaneous power+data](https://lavalink.com/lavablog/articles/why-some-usb-c-mobile-devices-cant-do-simultaneous-power-and-data/), [usb-serial-for-android FAQ](https://github-wiki-see.page/m/mik3y/usb-serial-for-android/wiki/FAQ), [Android SE](https://android.stackexchange.com/questions/237441/usb-c-hub-and-charging-at-the-same-time), [Vention CCBHB page](https://ventiontech.com/products/vention-2-in-1-usb-3-2-gen-2-adapter) | Medium |
| 37 | Android data loss at high baud | Android is not a real-time OS, so continuous reads at ≥115200 can lose fragments. Mitigations: higher thread priority, `setReadQueue` (v3.10.0+) | [FAQ](https://github-wiki-see.page/m/mik3y/usb-serial-for-android/wiki/FAQ), [releases](https://github.com/mik3y/usb-serial-for-android/releases) | High |
| 38 | Emulator | The Android emulator has no USB passthrough. Test on a real phone, or use `tcp://` to a bridge on the host (`10.0.2.2`) | [serialplugin Android docs](https://s00d.github.io/tauri-plugin-serialplugin/platforms/android) | High |

### 1.4 Tauri 2

| # | Fact | Value | Source | Confidence |
|---|------|-------|--------|------------|
| 39 | Current stable | `tauri` 2.12.3 (2026-10-10); `tauri-cli` 2.12.2; `@tauri-apps/api` 2.12.3; `tauri-bundler` 2.10.2 | [Tauri releases](https://v2.tauri.app/release/) | High |
| 40 | 2.12.0 notes that matter here | MSRV 1.90, edition 2024. Windows 7 dropped (`windows` 0.62). Android Gradle template moved to Gradle 9.6.1 / AGP 9.3.1 / Kotlin 2.2, with a minimum Gradle of 8.13. Plugins must rename `proguard-rules.pro` to `consumer-rules.pro`. New `app > appDirectoriesOverride` config supports portable data directories | [tauri release notes](https://v2.tauri.app/release/tauri/) | High |
| 41 | Android minimum | Android 7.0 (SDK 24). Set it with `bundle > android > minSdkVersion`. `tauri android build --apk`/`--aab`, `--target aarch64` and `--split-per-abi` are available | [Tauri Google Play docs](https://v2.tauri.app/distribute/google-play/) | High |
| 42 | Mobile plugin pattern (Kotlin) | Write a class extending `app.tauri.plugin.Plugin`, annotated `@TauriPlugin`, with `@Command fun x(invoke: Invoke)` methods. Arguments come from `@InvokeArg` classes via `invoke.parseArgs`. Rust calls them with `PluginHandle::run_mobile_plugin("x", payload)`. The `onNewIntent` lifecycle hook is Android-only. Wrap suspend functions in your own CoroutineScope | [Tauri mobile plugin docs](https://v2.tauri.app/develop/plugins/develop-mobile/) | High |
| 43 | Community serial plugin | `tauri-plugin-serialplugin` 4.2.1 (2026-11-08), MIT OR Apache-2.0, MSRV 1.90. Desktop uses the `serialport` crate. Android uses USB host with **drivers in Rust** (`android-usb-serial`, a port of usb-serial-for-android on `nusb`); Kotlin only handles permission and passes the fd. Supports FTDI, CP210x, PL2303, CH340/341 and any CDC-ACM device, explicitly including CH343 and CH9102. It also offers `tcp://` and `ble://` transports. Rust API methods are **blocking**, so call them via `spawn_blocking` | [crates.io](https://crates.io/crates/tauri-plugin-serialplugin), [README](https://github.com/s00d/tauri-plugin-serialplugin), [Android doc](https://s00d.github.io/tauri-plugin-serialplugin/platforms/android), [Rust guide](https://s00d.github.io/tauri-plugin-serialplugin/guides/rust) | High |
| 44 | Plugin maturity | `android-usb-serial` was first published 2026-07-08 and is at 0.1.4 with about 1.7k total downloads. `serialplugin` has about 54k total downloads and 177 stars. The plugin's `serial-codec` dependency is MPL-2.0 (file-level copyleft) | [android-usb-serial crate](https://crates.io/crates/android-usb-serial), [README license section](https://github.com/s00d/tauri-plugin-serialplugin) | High |
| 45 | Kotlin path if the plugin fails | Wrap usb-serial-for-android 3.12.0 (JitPack `com.github.mik3y:usb-serial-for-android:3.12.0`) in your own Tauri Kotlin plugin | [usb-serial-for-android README](https://github.com/mik3y/usb-serial-for-android/) | High |
| 46 | Official localhost plugin | `tauri-plugin-localhost` serves app assets over a localhost HTTP server on **Windows, Linux and macOS only (no Android/iOS)**. It is meant for the app's own webview, not for LAN clients | [Tauri localhost plugin](https://v2.tauri.app/plugin/localhost/) | High |
| 47 | Embedding axum for LAN browsers | Feasible. Tauri's async runtime is tokio, so an axum server bound to `0.0.0.0:<port>` can be spawned in `setup()` and can serve the built frontend (for example `tower-http` ServeDir or embedded assets) plus a WebSocket stream. `tauri-plugin-axum` (0.10.0) is a different thing: it routes the webview's own requests to an axum Router through a custom protocol, with no network socket. Several public projects embed axum in Tauri 2 | [tauri-plugin-axum docs](https://docs.rs/tauri-plugin-axum/latest/tauri_plugin_axum/), [example issue: mokumo](https://github.com/breezy-bays-labs/mokumo/issues/4) | Medium (interpretation; not documented by Tauri) |
| 48 | Windows bundles | MSI (WiX v3) can **only be built on Windows**. NSIS `-setup.exe` can be cross-built from Linux/macOS with `cargo-xwin`, but this is "not tested as much". WebView2 install modes: `downloadBootstrapper` (default, 0 MB), `embedBootstrapper` (~1.8 MB), `offlineInstaller` (~127 MB), `fixedVersion` (~180 MB), `skip` | [Tauri Windows installer docs](https://v2.tauri.app/distribute/windows-installer/) | High |
| 49 | Single-exe / portable | Tauri has no official portable target. A maintainer says you can ship the raw `.exe` if you use no resources or sidecars, but the user must already have the WebView2 runtime. The 2.12 `appDirectoriesOverride` setting lets data live next to the exe | [tauri issue #3109](https://github.com/tauri-apps/tauri/issues/3109) (2021, old), [tauri 2.12.0 notes](https://v2.tauri.app/release/tauri/) | Medium |
| 50 | WebView2 presence | The Evergreen runtime is part of Windows 11. Microsoft says "the vast majority" of Win10 devices have it, but some do not, so check for it | [MS Learn: Distribute WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution) | High |

### 1.5 Rust Modbus crates

| # | Fact | Value | Source | Confidence |
|---|------|-------|--------|------------|
| 51 | tokio-modbus version / license | 0.17.0 (2025-10-22), **MIT OR Apache-2.0**, MSRV 1.76. Default features: `rtu`, `tcp`. Server features: `rtu-server`, `tcp-server`, `rtu-over-tcp-server` | [crates.io API](https://crates.io/api/v1/crates/tokio-modbus) | High |
| 52 | tokio-modbus RTU transport | `rtu::attach_slave(transport, slave)` accepts any `AsyncRead + AsyncWrite + Unpin + Send`. It does not require `tokio-serial`; only the `rtu-sync` and `rtu-server` features pull in `tokio-serial`. That means an Android USB port can feed it through an async adapter | [src/client/rtu.rs](https://raw.githubusercontent.com/slowtec/tokio-modbus/main/src/client/rtu.rs), [features](https://crates.io/api/v1/crates/tokio-modbus) | High |
| 53 | rmodbus | 0.13.0 (2026-11-02), **Apache-2.0**. A transport-independent request/response frame codec with context storage, supporting TCP/UDP, RTU and ASCII. `no_std` capable; no server is bundled | [crates.io API](https://crates.io/api/v1/crates/rmodbus), [README](https://github.com/alttch/rmodbus) | High |
| 54 | Own codec? | Reasonable but unnecessary. Encoding PDUs and CRC16 is small. The hard part of RTU is frame delimitation on USB-serial, where bytes arrive in USB packets and not at character timing. Use `rmodbus` for encoding/parsing and write only the transaction/timeout layer, or use `tokio-modbus` as-is | Interpretation from [usb-serial FAQ on fragmented reads](https://github-wiki-see.page/m/mik3y/usb-serial-for-android/wiki/FAQ) | Medium |

### 1.6 MQTT, SQLite, XLSX

| # | Fact | Value | Source | Confidence |
|---|------|-------|--------|------------|
| 55 | rumqttc | 0.25.1 (2025-11-21), Apache-2.0. Default feature `use-rustls`; optional `use-native-tls` and `websocket` | [crates.io API](https://crates.io/api/v1/crates/rumqttc?include=default_version) | High |
| 56 | rumqttc TLS roots | `TlsConfiguration::Simple { ca, .. }` requires a PEM CA (otherwise `NoValidCertInChain`). `TlsConfiguration::Rustls(Arc<ClientConfig>)` accepts any rustls config | [tls.rs source](https://docs.rs/rumqttc/latest/src/rumqttc/tls.rs.html) | High |
| 57 | System roots on Android | `rustls-native-certs` does not support Android. Use `rustls-platform-verifier`, which needs a small Kotlin component wired into Gradle from its Maven repo, or bundle `webpki-roots`, which are static and need manual updates | [rustls-native-certs #3](https://github.com/rustls/rustls-native-certs/issues/3), [rustls-platform-verifier README](https://github.com/rustls/rustls-platform-verifier) | High |
| 58 | rusqlite | 0.42.0 (published 2026-11-11), MIT. The `bundled` feature compiles SQLite from source | [crates.io API](https://crates.io/api/v1/crates/rusqlite?include=default_version) | High |
| 59 | rusqlite bundled on Android | aarch64 builds. `x86_64-linux-android` (emulator ABI) failed to link with soft-float symbols in open issue #1380 (2023), which has a workaround. A fix PR #1592 exists; **UNVERIFIED** whether it shipped in 0.42 | [rusqlite #1380](https://github.com/rusqlite/rusqlite/issues/1380), [PR #1592](https://github.com/rusqlite/rusqlite/pull/1592) | Medium |
| 60 | XLSX writer | `rust_xlsxwriter` 0.102.0 (2026-10-29), MIT OR Apache-2.0, MSRV 1.88. Pure Rust; the `constant_memory` feature handles large logs | [crates.io API](https://crates.io/api/v1/crates/rust_xlsxwriter?include=default_version) | High |

## 2. Recommended stack (derived from the table)

| Layer | Choice | Why |
|-------|--------|-----|
| Serial, desktop + Android | `tauri-plugin-serialplugin` 4.2.x (from the Rust side, via `serialplugin-core`) | One API on both targets. Handles CH343 (CDC) and CH340. Kotlin only does permission. Fallback: a custom Kotlin plugin over usb-serial-for-android 3.12.0 |
| Modbus | `rmodbus` for framing + own async transaction loop, or `tokio-modbus` RTU/TCP client | Both are permissive licenses. tokio-modbus accepts any async stream |
| LAN dashboard | axum + WebSocket spawned on the Tauri tokio runtime, read-only routes, bound to an explicit interface | Same frontend build served to browsers. On Android, keep targetSdk 36 until local-network permission handling exists |
| Storage | `rusqlite` with `bundled`, WAL | Same code on Windows and Android aarch64 |
| Export | `rust_xlsxwriter` + CSV | Pure Rust, no C dependencies |
| MQTT | `rumqttc` with a `TlsConfiguration::Rustls` config from `rustls-platform-verifier` (Android) or native roots (Windows) | Avoids shipping static roots |
| Android background | One `connectedDevice` FGS, started from user action after `UsbManager.requestPermission()` | Meets the FGS prerequisite and avoids dataSync's 6 h cap |

## 3. Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Waveshare (B) is non-isolated, so ground loops or surges on site can damage the phone or laptop | Hardware damage, noisy comms | Say this clearly in docs and demo. Wire the GND terminal. Recommend an isolated adapter (Waveshare RS485 (C)/(E), FT232RNL isolated) for field work. Lab demo is fine |
| 120 Ω termination may be fixed on the board (row 19, UNVERIFIED) | A mid-bus adapter adds an extra terminator; marginal signal on long buses | Inspect the board. If fixed, document "connect adapter at bus end only" |
| CH343 bound to the inbox CDC driver vs the VCP driver gives different COM names and features | Users confused about which driver they have | Detect by VID:PID 1A86:55D3, not by friendly name. Document that no driver install is needed on Win10/11. VCP is optional |
| `android-usb-serial` is 4 months old with few users | Unknown driver bugs on real phones | Test early on 2–3 phones with CH343 and CH340. Keep the usb-serial-for-android Kotlin fallback behind a trait so you can switch |
| Plugin APIs block | UI or tokio stalls | Run port I/O in `spawn_blocking` or a dedicated thread. Bridge to async channels for tokio-modbus |
| RTU frame boundaries over USB-serial (fragmented reads, Android scheduling jitter) | CRC errors, false timeouts | Parse by expected length from function code and byte count, not by 3.5-char silence alone. Use per-request timeout ≥200 ms. Use `setReadQueue`-style buffering. Prefer ≤115200 baud on Android |
| Android FGS misconfiguration (API 34+) | `SecurityException` crash when logging starts | Declare `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, type `connectedDevice`. Start the service only after USB permission is granted and while in foreground. Also declaring `CHANGE_NETWORK_STATE` covers the prerequisite in the no-USB (Modbus TCP only) case |
| Play Console FGS declaration | Review delay if publishing | Record the required demo video (in-app connect to the adapter). For the hackathon, sideload the APK and skip Play |
| Android 17 local-network enforcement at targetSdk 37 | Modbus TCP and the LAN dashboard break silently (TCP timeouts) | Target API 36 (meets the current Play rule). Before moving to 37, add an `ACCESS_LOCAL_NETWORK` runtime request with a rationale screen |
| Phone cannot charge during OTG (Vention CDQ has no PD port) | Long logging sessions drain the battery | Document the limit. Offer a PD pass-through adapter (e.g. Vention CCBHB) only as "may work, device-dependent". Use a low-power logging mode (screen off, FGS running). Show battery % in the UI |
| OTG bus power to adapter | Brown-outs, disconnects ("Queueing USB request failed") | The CH343 board draws little (200 mA fuse). Avoid unpowered hubs. Show a reconnect state in the UI |
| USB permission lost after reboot with device attached (Google Won't Fix) | Logger does not auto-resume | Add VID/PID rows in `0x` hex to `device_filter.xml` (CH343 55D3, CH340 7523, CH9102 55D4). Set `directBootAware`. Re-request permission on app start |
| MSI cannot be built on Linux CI | No MSI from a Linux runner | Build on a `windows-latest` GitHub runner, or ship NSIS only |
| WebView2 missing on some Win10 PCs; "single exe" is not official | App fails to start | Ship the NSIS installer with `embedBootstrapper` (or `offlineInstaller` for air-gapped plants). Offer a raw exe as "portable, needs WebView2" and set `appDirectoriesOverride` |
| Tauri 2.12 bumped Gradle/Kotlin/MSRV | Build breaks with older toolchains or plugins | Pin Rust ≥1.90 and Tauri 2.12.x. Re-run `tauri android init`. Make sure custom plugins use `consumer-rules.pro` |
| rusqlite bundled on x86_64 Android emulator may fail to link | Emulator builds break | Build aarch64 only (`--target aarch64`). Test on hardware, since the emulator has no USB anyway |
| TLS roots on Android for MQTT | TLS handshake fails against public brokers | Use `rustls-platform-verifier` with its Gradle snippet, or bundle `webpki-roots` as a fallback. Allow a user-supplied CA PEM for private brokers |
| MPL-2.0 file in the plugin dependency tree | License surprise in an OSS release | MPL is file-level and compatible with MIT/Apache apps. Note it in THIRD_PARTY notices |
| Read-only LAN dashboard exposed on plant networks | Data leak or unwanted access | Off by default, explicit bind address, optional token in URL, no write routes, LAN-only warning in the UI |

## 4. Contradictions

- **Baud range:** the CH343 datasheet says up to 6 Mbps. The Waveshare (B) board is rated to 3 Mbps; the SP485EEN is likely the limit, though that is my inference. The low end is listed as 50 bps on the wiki and 300 bps on the product page.
- **Isolation:** one reseller (Kamami) URL slug says "isolated". The manufacturer pages do not, and Waveshare sells isolated models as separate SKUs.
- **Portable exe guidance** comes from a 2021 Tauri v1-era issue comment. Tauri 2 docs don't cover a portable mode beyond `appDirectoriesOverride`.

## 5. Missing evidence

- Whether the Waveshare (B) 120 Ω resistor can be disconnected (jumper or switch): UNVERIFIED.
- The exact first usb-serial-for-android version that handled a real CH343: inferred as v3.5.0. No release note names the CH343.
- Whether rusqlite PR #1592 (x86_64 Android) is in 0.42.0: UNVERIFIED.
- Vention CDQ current rating and whether any Vention OTG adapter does PD pass-through with data on a specific test phone: needs a bench test.
- Whether `tauri-plugin-serialplugin`'s Android path holds up on long (multi-hour) RTU polling: no field reports found.
- I did not see the Waveshare wiki live (403). The archived copy is from 2025-11-20 and the product page copy from 2026-02-13.

## 6. Next steps

1. Bench test: Windows 11 inbox CDC + CH343, and Android phone + Vention CDQ + Waveshare (B) + one Modbus slave. Use `tauri-plugin-serialplugin` and log CRC and timeout rates at 9600, 19200 and 115200 baud.
2. Photograph the (B) board to settle the 120 Ω jumper question.
3. Spike a Tauri 2.12 Android build with rusqlite (aarch64), rumqttc + rustls-platform-verifier, and axum bound to 0.0.0.0, to confirm the toolchain before feature work.
