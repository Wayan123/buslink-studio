---
inclusion: always
---

# Tech: stack, layout, commands

## Stack

| Layer | Choice | Notes |
|---|---|---|
| App shell | Tauri 2.12.x (Rust ≥ 1.90, edition 2024) | Windows NSIS installer + Android APK (aarch64). minSdk 24, **targetSdk 36** until the Android 17 `ACCESS_LOCAL_NETWORK` runtime flow exists. |
| UI | TypeScript + Vite, plain DOM/Web Components, CSS custom properties | No heavy framework unless the spec justifies it. Charts: canvas renderer or a small MIT library (e.g. uPlot) chosen in the spec. The same build serves the webview and the LAN dashboard. |
| Core | `crates/buslink-core` (pure Rust, no Tauri dependency) | Own Modbus codec: read PDUs in `protocol/`, **write PDUs only in `command/`**. External Modbus crates (`rmodbus` Apache-2.0, `tokio-modbus` MIT/Apache) only if the spec design justifies them. |
| Async | tokio | Serial I/O is blocking: run it on a dedicated thread per bus, bridge with channels. |
| Desktop serial | `serialport` crate | Detect CH343 by VID:PID `1A86:55D3`, CH340 by `1A86:7523`, never by friendly name. |
| Android serial | Kotlin Tauri plugin over **usb-serial-for-android 3.12.x** (MIT, JitPack) | Owner decision. CH343 goes through `CdcAcmSerialDriver`, CH340 through `Ch34xSerialDriver`. Keep transport behind a trait so `tauri-plugin-serialplugin` stays a possible fallback. |
| Historian | `rusqlite` (bundled), WAL | Same code on Windows and Android. Integers keep full precision; no lossy f64 for raw values. |
| Export | CSV (own writer), `rust_xlsxwriter`, serde_json | Real XLSX workbooks, numbers as numbers, ISO-8601 UTC times. |
| MQTT | `rumqttc` with rustls | Android TLS roots via `rustls-platform-verifier` or a user-supplied CA. |
| LAN dashboard | axum (HTTP + WebSocket) with rustls, per-session self-signed cert (`rcgen`) | Spawned on the Tauri tokio runtime; read-only routes only. |
| Simulator | `crates/buslink-sim` | Deterministic devices: soil sensor, VFD, temperature controller; in-process transport + optional Modbus TCP server for training. |

Pin exact versions in `Cargo.lock` / `package-lock.json`; record licenses in `THIRD_PARTY_NOTICES.md`.

## Folder layout

```
Cargo.toml                    root workspace: members = crates/*  (src-tauri and plugins are NOT members)
crates/buslink-core/src/
  protocol/                   CRC, RTU/MBAP framing, read PDUs, exceptions, decode (word order, scale)
  transport/                  Transport trait; serial, TCP, simulator adapters
  poll/                       one worker per bus, block-merged reads, backoff, quality + age
  profile/                    schema v1 loader/validator, .mbmap import
  command/                    THE write path: arming, bounds, confirm token, encode, read-back, audit
  historian/  alarm/  export/  mqtt/  lan/  i18n/
crates/buslink-core/tests/    integration tests (fixtures: frames, partial frames, bad CRC, timeouts)
crates/buslink-sim/           simulator devices + TCP server
src/                          TypeScript UI (views, components, i18n/en.json, i18n/id.json, styles)
src-tauri/                    Tauri app crate (own Cargo.toml/Cargo.lock), Tauri commands, desktop serial glue
src-tauri/gen/android/        generated Android project; keep hand edits minimal and listed in docs/android-build.md
plugins/tauri-plugin-buslink-usb/   Rust side + android/ (Kotlin @TauriPlugin, usb-serial-for-android)
profiles/                     device profiles (schema: docs/profiles/schema-v1.md)
docs/                         design, profiles, evidence, kiro-log
tools/hooks/                  Kiro hook scripts + tests/
```

Why src-tauri is outside the root workspace: Linux `cargo` in WSL can then test and check the core without webkit2gtk or the Android toolchain, and Windows `cargo.exe` builds of the app use `src-tauri/target`, so the two toolchains never share a target directory.

## Environment

The repo lives on `/mnt/d` (Windows drive) and is edited from WSL.

- **Pure-Rust core (crates/*)**: Linux toolchain in WSL (`cargo`, Rust 1.95 installed). Uses `target/`.
- **Tauri desktop and Android builds**: Windows toolchain (`cargo.exe`, `node.exe`, `npm.cmd`, Android SDK/NDK/JBR on Windows). Set `JAVA_HOME` (Android Studio `jbr`), `ANDROID_HOME`, `NDK_HOME` (NDK 28.2+) in the Windows environment.
- `npm.cmd` cannot be executed directly from bash (bash parses it as a script and fails). From WSL call it through `cmd.exe /c`. In a Windows terminal, call `npm.cmd` / `npm` directly.
- `node_modules` installed by Windows npm contains Windows-only native binaries (esbuild/rolldown): `vite build` must run with the Windows toolchain. Pure-JS tools (`tsc`) work with Linux node.
- If you ever run `cargo.exe` at the repo root, pass `--target-dir target/windows`.

## Commands

Core (WSL, repo root):

```sh
cargo test --workspace                                  # all core + sim tests
cargo test -p buslink-core <filter>                     # focused
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 -m unittest discover -s tools/hooks/tests -v    # hook script self-tests
find profiles -name '*.json' -exec python3 tools/hooks/check_profile.py {} +
```

UI and app (Windows toolchain, from WSL at repo root):

```sh
cmd.exe /c "npm.cmd ci"
cmd.exe /c "npm.cmd run build"                          # tsc --noEmit && vite build
npx tsc --noEmit                                        # type-check only (Linux node is fine)
cargo.exe check --manifest-path src-tauri/Cargo.toml
cmd.exe /c "npm.cmd run tauri -- build --bundles nsis"  # Windows installer
cmd.exe /c "npm.cmd run tauri -- android build --debug --apk --target aarch64"
```

Long-running (the user starts these in their own terminal): `cmd.exe /c "npm.cmd run tauri -- dev"`, `cmd.exe /c "npm.cmd run dev"`.

Android install for tests: `adb -s <serial> install -r <apk>` (never uninstall or clear data to resolve conflicts).

## Conventions

- Rust: `thiserror` errors, no `unwrap()` outside tests, no panics on device input. Every decode path is fuzz-friendly (length checks before indexing).
- Time: store UTC epoch ms + monotonic sequence; show local time in UI.
- All UI strings come from `src/i18n/{en,id}.json`; no hard-coded text.
- Codec, framing, quality and command-service rules get unit tests with byte-level fixtures, as listed in the spec tasks. Write-related tests live in `tests/` directories (inline `#[cfg(test)]` modules outside `command/` that build write frames are blocked by the guard hook).
- Kiro hooks in `.kiro/hooks/` run core tests on save, type-check UI on save, guard write paths and secrets, validate profiles, and block Stop when `cargo check --workspace` fails.
