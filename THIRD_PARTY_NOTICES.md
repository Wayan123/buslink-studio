# Third-party notices

BusLink Studio is Apache-2.0. It uses the libraries below as dependencies under their own licenses. Versions are pinned exactly in `Cargo.toml` and `Cargo.lock`. Nothing is vendored or copied into this repository.

## Direct dependencies (Rust workspace: `crates/buslink-core`, `crates/buslink-sim`)

| Crate | Version | License | Used for |
|---|---|---|---|
| serde | 1.0.229 | MIT OR Apache-2.0 | Profile model, sample and export serialisation |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | Profile parsing (strict, duplicate keys rejected), JSON export |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | Error types |
| tokio | 1.54.1 (feature `sync` only) | MIT | Sample bus (`broadcast` channel); no runtime |
| rusqlite | 0.42.0 (feature `bundled`) | MIT | Historian |
| SQLite (bundled by `libsqlite3-sys` 0.40.0) | 3.x | Public domain | Historian database engine |
| rust_xlsxwriter | 0.102.0 (default features off) | MIT OR Apache-2.0 | XLSX export |

Dev-only: `zip` 9.5.1 (MIT), already a dependency of rust_xlsxwriter, used by tests to inspect exported workbooks.

## Transitive dependencies

Pulled in by the crates above (from `cargo metadata`, normal and build dependencies): bitflags, bumpalo, cc, cfg-if, crc32fast, equivalent, fallible-iterator, fallible-streaming-iterator, find-msvc-tools, flate2, getrandom, hashbrown, indexmap, itoa, js-sys, libc, log, pin-project-lite, pkg-config, proc-macro2, quote, rustversion, serde_core, serde_derive, shlex, smallvec, syn, thiserror-impl, typed-path, vcpkg, wasm-bindgen (+ macro, macro-support, shared), windows-link, windows-sys, zeroize, zeroize_derive (all MIT and/or Apache-2.0); cfg_aliases, libsqlite3-sys, nix, simd-adler32, zip, zmij (MIT); memchr (Unlicense OR MIT); constant_time_eq (CC0-1.0 OR MIT-0 OR Apache-2.0); r-efi (MIT OR Apache-2.0 OR LGPL-2.1-or-later, used under MIT/Apache); unicode-ident ((MIT OR Apache-2.0) AND Unicode-3.0); zlib-rs (Zlib); zopfli (Apache-2.0).

No GPL-only dependency is used. Regenerate this list when dependencies change.

## Desktop app (`src-tauri/`, own `Cargo.lock`)

| Crate | Version | License | Used for |
|---|---|---|---|
| tauri | 2.12.5 | MIT OR Apache-2.0 | App shell, IPC, events |
| tauri-build | 2.7.4 | MIT OR Apache-2.0 | Build script |
| tauri-plugin-dialog | 2.8.4 | MIT OR Apache-2.0 | Native save dialog for exports (opened from Rust only) |
| serialport | 4.10.1 (default features off, Windows only) | MPL-2.0 | Windows COM ports, VID:PID listing. Used unmodified as a library; MPL-2.0 file-level copyleft applies only to its own files. |

Tauri pulls in a large transitive tree (wry, tao, webview2-com, windows-*, etc.), MIT and/or Apache-2.0; see `src-tauri/Cargo.lock`.

## Web UI (`package.json`, exact versions, `package-lock.json`)

| Package | Version | License | Used for |
|---|---|---|---|
| @tauri-apps/api | 2.12.4 | MIT OR Apache-2.0 | IPC from the webview |
| uplot | 1.6.32 | MIT | Trend charts (bundled into `dist/`) |
| @tauri-apps/cli | 2.12.4 (dev) | MIT OR Apache-2.0 | Build tool |
| typescript | 5.9.3 (dev) | Apache-2.0 | Type-check |
| vite | 8.3.8 (dev) | MIT | Bundler |
| @types/node | 24.19.2 (dev) | MIT | Types for build scripts and tests |
| puppeteer-core | 25.17.0 (dev) | Apache-2.0 | Headless verification script only; not shipped |
