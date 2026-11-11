# core-protocol: tasks

Run `cargo test --workspace` after every task. A task is `[x]` only when its tests pass.
Property-test default case counts are in brackets; override with `BUSLINK_PBT_CASES`.

- [x] 1. Workspace and crate skeleton
  - Root `Cargo.toml` (members `crates/*`, edition 2024, exact dependency pins), `clippy.toml`, `crates/buslink-core` with `clock`, `rng`, `testkit`, `time`; `THIRD_PARTY_NOTICES.md`.
  - Tests: SplitMix64 reference values, ISO-8601 formatting (epoch, leap day, negative years rejected).
  - _Requirements: 11.2; design §1, §12_

- [x] 2. CRC-16/MODBUS and RTU ADU encoding
  - `protocol/crc.rs`, `rtu::encode_adu`, `SerialSettings`, `RtuTiming`.
  - Tests: P1 known vectors, CRC residue property [2000]; t3.5 at 9600 8N1 / 8E1 and fixed values above 19 200.
  - _Requirements: 1.1–1.4_

- [x] 3. Read PDUs, exceptions and limits
  - `protocol/pdu.rs`: `Table`, `ReadFunction`, `ReadRequest` (limits), `ReadData`, response encode/decode, `ExceptionCode` names, `ExpectedReply`.
  - Tests: limit edges (0, 125/126, 2000/2001, address + quantity = 65 536/65 537), exception names, P2 read round trip [2000 per FC].
  - _Requirements: 3.1, 3.2, 5.1–5.3_

- [x] 4. Length-based RTU response framer
  - `rtu::ResponseFramer`: header shape, CRC first, unit/function/byte-count checks, trailing bytes.
  - Tests: fixtures for good frame, exception, bad CRC, wrong unit, unexpected function; P4 fragmentation at every split point [500]; P5 no panic [20000].
  - _Requirements: 2.1, 2.3, 2.4, 2.6_

- [x] 5. MBAP framing
  - `protocol/tcp.rs`: encode, header validation, `MbapReceiver` with partial and coalesced input, sticky error.
  - Tests: length 0/1/255 rejected, protocol ID ≠ 0 rejected, coalesced frames, P2 through MBAP, P5 no panic.
  - _Requirements: 4.1–4.3, 4.5_

- [x] 6. Codec
  - `codec.rs`: types, word orders, strings, `RawValue`, decode pipeline (sentinel, scale/offset, finite, range), `to_raw`.
  - Tests: modbus.md table for f32 123.456 in all four orders, i16 sign, u64 exactness, P6 round trip for every type × order [1000 each], to_raw rejects non-representable values.
  - _Requirements: 6.1–6.4, 7.1–7.3_

- [x] 7. Profile schema, strict JSON, validator, typed model
  - `profiles/schema/buslink-profile.schema.json`, `profile/json.rs`, `profile/validate.rs`, `profile/model.rs`, `load_profile`.
  - Update `tools/hooks/check_profile.py` (design §4 fixes, `--json` output, skip `profiles/schema/`) and its tests.
  - Fixtures `crates/buslink-core/tests/fixtures/profiles/{valid,invalid}/`.
  - Tests: duplicate keys, 1 MB limit, every rule family, P12 agreement with Python on 5 bundled profiles + 7 valid + 70 invalid fixtures and on mutated documents [400], schema key sets equal validator key sets, P5 validator no panic on mutated documents [2000].
  - _Requirements: 8.1–8.5_

- [x] 8. Device instance, link config, quality model
  - `device.rs`, `quality.rs` (Quality, Value, Sample, TagState, OPC UA class).
  - Tests: unit ID 0 and > 247 on RTU rejected, profile ID/version mismatch rejected, P11 last-good kept with original timestamp, STALE threshold default.
  - _Requirements: 9.1–9.3, 10.1–10.4_

- [x] 9. Transport trait, FakeTransport, TCP client, link registry, exchanger
  - `transport/{mod,fake,tcp,exchange}.rs`.
  - Tests: drain before request, inter-frame gap honoured, timeout via manual clock, late RTU reply discarded on the next exchange, MBAP stale transaction ID discarded, link error mapping, second claim of a link key refused, TCP client against a loopback echo.
  - _Requirements: 2.2, 2.5, 4.4, 4.6, 11.1–11.4_

- [x] 10. Block merging
  - `poll/plan.rs`.
  - Tests: BGT-SMPS 8 registers → one FC03 block; gap handling; device limit; P7 [1000].
  - _Requirements: 13.1, 13.2_

- [x] 11. Scheduler and worker
  - `poll/scheduler.rs`, `poll/worker.rs`, `bus.rs`.
  - Tests: priority command > alarm > poll, P9 command sent once even on timeout [200], backoff growth and reset, retry bound, link loss → DISCONNECTED, STALE emission, counters, P8 no starvation with dead devices [300], worker thread end-to-end with FakeTransport, bus lag reported.
  - _Requirements: 12.1–12.4, 14.1–14.4, 15.1, 15.2_

- [x] 12. Command service
  - `command/{wire,audit,mod}.rs`.
  - Tests (in `tests/`): P3 wire round trip for FC05/06/15/16 [1000], write decoder no panic [10000], echo mismatch [1000], arming expiry and disarm reasons, role check, bounds reject (never clamp), unit 0 rejected, simulator-only rule, timeout → effect unknown + read-back + no retry, audit failure blocks send, P10 random operation sequences [500; about 1 100 frames sent per run].
  - _Requirements: 16.1–16.9_

- [x] 13. Historian
  - `historian/{mod,batch}.rs`.
  - Tests: WAL on file databases, non-GOOD stored with NULL value, append-only audit (UPDATE/DELETE fail), lossless u64 raw, decimator keeps quality changes, minute rollups from GOOD only, rollups equal a naive computation [60], retention keeps audit.
  - _Requirements: 17.1–17.5_

- [x] 14. Export
  - `export/{mod,csv,json,xlsx}.rs`.
  - Tests: CSV header and quoting, P13 formula guard [5000], ISO-8601 UTC, JSON 64-bit strings, XLSX workbook has the five sheets and numeric cells (zip inspection), no secret-like fields in the export types.
  - _Requirements: 18.1–18.4_

- [x] 15. Final checks
  - `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` (`rustfmt.toml`: `max_width = 120`), hook self-tests, profile checks, write-guard scan of every crate source file.
  - _Requirements: all_
