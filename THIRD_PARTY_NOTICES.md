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
