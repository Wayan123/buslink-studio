# core-protocol: design

Implements `requirements.md` in `crates/buslink-core`. Architecture decisions D1–D4, D8, D9 (`docs/bmad/architecture.md`).

## 1. Crate layout

```
crates/buslink-core/src/
  lib.rs
  clock.rs          Clock trait (monotonic ms, UTC ms, sleep); SystemClock, ManualClock
  rng.rs            SplitMix64, deterministic; used by property tests and the simulator
  testkit.rs        property-test runner (seeded, case count from env)
  time.rs           ISO-8601 UTC formatting (no chrono)
  protocol/
    crc.rs          CRC-16/MODBUS
    pdu.rs          Table, ReadFunction, ReadRequest, ReadData, ExceptionCode, ExpectedReply
    rtu.rs          RTU ADU encode, ResponseFramer (length-based), SerialSettings, RtuTiming
    tcp.rs          MBAP encode, header parse, MbapReceiver (stream framing)
  codec.rs          DataType, WordOrder, RawValue, decode/encode, scale/offset/range/sentinel
  profile/
    json.rs         strict JSON parse (duplicate keys rejected, 1 MB limit)
    validate.rs     port of tools/hooks/check_profile.py (same rules, same error paths)
    model.rs        typed Profile structs (serde, deny_unknown_fields)
    mod.rs          load_profile(): size check → strict parse → validate → typed
  device.rs         LinkConfig, LinkKind, DeviceInstance, validation against a profile
  quality.rs        Quality, Value, Sample, TagState (last good + age, STALE)
  transport/
    mod.rs          Transport trait, TransportError, LinkRegistry (one owner per link key)
    fake.rs         FakeTransport + FakeHandle (scripted, manual clock)
    tcp.rs          TcpTransport (std::net)
    exchange.rs     Exchanger: one request/response over a Transport (RTU, MBAP, RTU-over-TCP)
  poll/
    plan.rs         block merging
    scheduler.rs    LinkScheduler: deterministic, clock-driven, step() executes one job
    worker.rs       LinkWorker: thread that owns the transport, mpsc queue, LinkHandle
  bus.rs            SampleBus (tokio::sync::broadcast, `sync` feature only, no runtime)
  command/          THE write path
    wire.rs         write PDU encode/decode, echo verification
    mod.rs          CommandService state machine, PreparedFrame, CommandLink trait
    audit.rs        AuditRecord, AuditSink, MemoryAudit
  historian/
    mod.rs          Historian (rusqlite, WAL), schema, inserts, queries, rollups, retention
    batch.rs        LogDecimator (logging interval), SampleBatcher (1 s batches)
  export/
    mod.rs          ExportDoc, formula guard
    csv.rs  json.rs  xlsx.rs
crates/buslink-core/tests/   integration and property tests (write-related tests live here)
```

Dependencies (exact pins, licenses in `THIRD_PARTY_NOTICES.md`): `serde 1.0.229`, `serde_json 1.0.154`, `thiserror 2.0.21`, `tokio 1.54.1` (feature `sync` only), `rusqlite 0.42.0` (`bundled`), `rust_xlsxwriter 0.102.0` (default features off). No Modbus crate (D1): the codec is small, and owning it lets the guard hook see every write encoder.

## 2. Protocol

### 2.1 RTU

`rtu::encode_adu(unit, pdu)` appends the CRC; it is generic and never looks at the function code.
Responses are received by a `ResponseFramer { unit, function, expect, buf }`:

```
push(bytes) -> NeedMore(n) | Complete(Result<Vec<u8> pdu, FrameError>)
```

- Two header bytes decide the shape: `fc == function | 0x80` → 5 bytes total; `fc != function` → `UnexpectedFunction` (DECODE_ERROR); otherwise `ExpectedReply::ByteCount` needs a third byte (total `5 + n`) and `ExpectedReply::Fixed(n)` gives `4 + n`.
- On completion the CRC is checked first (CRC_ERROR), then the unit ID (DECODE_ERROR), then the byte count against the request (DECODE_ERROR in `decode_read_response`).
- Bytes past the expected length belong to nobody; the framer reports them as trailing and the next exchange drains them.

`ExpectedReply::Fixed(n)` lets the command service describe echo replies without `protocol/` knowing any write function code.

Timing (`RtuTiming::new(&SerialSettings)`): bits per char = 1 + data + parity + stop; `t1.5 = 1.5 char`, `t3.5 = 3.5 char`, fixed 750 µs / 1750 µs above 19 200 baud. Inter-frame gap = `max(ceil(t3.5), 5 ms, link floor)`; default response timeout = `max(500 ms, char time × (8 + 256) + 300 ms)` unless the profile or instance sets one.

### 2.2 MBAP

`tcp::encode_adu(tid, unit, pdu)`; `MbapReceiver` buffers stream bytes and yields frames by the length field. A bad protocol ID or length makes the stream unframeable: the receiver returns a sticky error and the exchanger reports a link error (Req 4.5), so the app reconnects instead of resynchronising on a guess.

### 2.3 Exchanger

```rust
enum Outgoing<'a> { Read(&'a ReadRequest), Command(&'a PreparedFrame) }
Exchanger::transact(&mut self, t: &mut dyn Transport, unit, Outgoing, timeout_ms) -> ExchangeRecord
```

There is no "send arbitrary PDU" entry point: the only sources of bytes are a validated `ReadRequest` or a `PreparedFrame`, which only `command/` can construct. `ExchangeRecord` keeps the request ADU, every received byte, timings and `Result<reply_pdu, ExchangeError>` for the register inspector and the audit log.

Late RTU replies: RTU has no transaction ID. After a timeout, the next request first drains the line until it has been quiet for `DEFAULT_LATE_GUARD_MS` (200 ms), so a reply that arrives after the timeout is discarded and never matched to the next request. A reply later than that guard can only be caught when its unit, function or length differ; this limit is accepted and documented in `exchange.rs`. `ExchangeError` = `Timeout | Crc | Exception{function, code} | Decode(FrameError) | Link(TransportError)` and maps 1:1 to quality.

## 3. Codec

Canonical form: registers in big-endian value order (A = most significant byte). `WordOrder::apply(regs)` reverses registers for CDAB/DCBA and swaps bytes inside each register for BADC/DCBA. The transform is an involution, so the same function encodes and decodes.

Decode pipeline (modbus.md): registers → `RawValue` (type + order) → sentinel check (integer types compare the integer, float types compare the value) → `raw × scale + offset` → finite check → `valid_range` check. Failures return `DecodeFailure { raw, reason }` so the raw value is kept.

`to_raw(type, engineering, scale, offset)` inverts the transform for commands and rejects values whose raw form is not an exact integer of the target type (tolerance 1e-9 relative) or out of the type range.

## 4. Profiles

- `profiles/schema/buslink-profile.schema.json` (JSON Schema 2020-12) describes structure, enums and ranges for editors. Cross-field rules (support level vs evidence, write bounds inside `valid_range`, overlap warnings, command references) cannot be expressed cleanly in JSON Schema and live in the two validators.
- `profile::validate` is a line-by-line port of `check_profile.py`. Both produce `path: message` errors and warnings. The agreement test (`tests/profile_agreement.rs`) runs the Python script with `--json` over every bundled profile and every fixture in `tests/fixtures/profiles/` and compares verdicts and the sorted error and warning paths. The test is skipped with a message when `python3` is missing.
- Python fixes made so both agree: strict typing in enums (`8.0` and `true` are not `8` and `1`), `fullmatch` with ASCII classes for IDs/semver/dates/digit keys (Python `$` matched before a trailing newline, `\d` and `isdigit()` accepted non-ASCII digits and could crash on `int('²')`), `NaN`/`Infinity` rejected at parse time, integers limited to the 64-bit range, `$schema`/`$comment` must be strings, `expected` must be a number, evidence `firmware`/`notes` and `source.publisher` are checked as text, and files under `profiles/schema/` are skipped by the hook.
- `load_profile(bytes)` = size check (1 MB) → strict parse (`profile::json`, duplicate keys rejected) → validate → `serde` into typed structs with `deny_unknown_fields`. Typed parsing after validation cannot fail for valid documents; if it does, that is a bug and is reported as an error, never a panic.

## 5. Device instances and links

`LinkConfig { id, kind: Rtu{serial, port_key} | Tcp{host, port} | RtuOverTcp{host, port} | Simulator{name}, min_inter_frame_ms }`.
`DeviceInstance { id, name, link_id, unit_id, profile_id, profile_version, poll_interval_ms, stale_after_ms, response_timeout_ms, read_retries, max_gap, max_registers_per_read, max_bits_per_read }`.
`DeviceInstance::check(&profile, &link)` enforces Req 9. `LinkRegistry::claim(key)` returns a guard that releases on drop; a second claim of the same key fails (Req 11.4).

## 6. Quality model

```rust
enum Quality { Good, Timeout, CrcError, Exception(u8), Stale, DecodeError, Disconnected }
struct Value  { raw: RawValue, engineering: Option<f64>, text: Option<String> }
struct Sample { seq, link_id, device_id, tag_id, profile_id, profile_version, ts_ms,
                quality, value: Option<Value>, value_ts_ms, age_ms, ts_source_ms, raw, unit, error }
```

`value` is the fresh value when GOOD and the last good value otherwise; `value_ts_ms` is always the time that value was received, so a failure never refreshes it. `TagState` holds the last good value with its monotonic receive time and the last emitted quality; `age = now_mono − received_mono`. Quality serialises as `{"code": "EXCEPTION", "exception": 2}`; 64-bit raw integers serialise as strings.

## 7. Scheduler

`LinkScheduler` is synchronous and clock-driven so tests are deterministic; `LinkWorker` wraps it in a thread.

- Queues: `command` and `alarm` (FIFO each) before polling (Req 12.2). Command jobs carry a `PreparedFrame` and are executed once; there is no retry path for them in the code.
- Polling: each device has a cycle (its block list) with a due time. Among devices whose cycle is due and whose backoff has expired, the scheduler picks the earliest due time; ties go round-robin. A device mid-cycle keeps its due time, so a cycle normally finishes before later cycles start.
- On timeout: one immediate retry if retries remain and the device is healthy; otherwise the cycle aborts, the device's tags get TIMEOUT (tags in blocks not attempted say so in `error`), `failures += 1`, and the next cycle is due after `min(interval × 2^(failures−1), backoff_cap)`.
- CRC error, exception and decode error affect only the block's tags; the cycle continues.
- Link error: all devices DISCONNECTED, queued jobs fail with `Link`, `step()` returns `LinkDown`.
- STALE: before each step, tags whose last emitted quality is GOOD and whose age exceeds the stale threshold get one STALE sample.
- `step()` returns `Worked | Idle { until_mono_ms } | LinkDown`; the worker sleeps on its channel until `until` or a new request.

## 8. Command service

States: `Idle → Armed → Previewed → Confirmed → Sent → ReadBack → Done | Failed`. `Sent` and `ReadBack` are entered inside `execute()`; the service is single-threaded (`&mut self`), so no other call can observe them half-way.

- `arm(target, operator)`: role ≥ Operator; records device and expiry; audit `arm`.
- `preview(link, request)`: checks arming (not expired, same device), role vs `write.role`, `access == read_write`, bounds, unit ID (≠ 0, ≤ 247 on RTU), simulator-only rule for named commands and `simulated_only` profiles; reads the current value through the link (failure → old value unknown); encodes the PDU with `wire`, builds the display frame, assigns a fresh preview ID; audit `preview`.
- `confirm(preview_id)`: must match the current preview; moves the plan into `Confirmed`; audit `confirm` (an audit failure returns to Armed and nothing is sent).
- `execute(link)`: takes the plan out of `Confirmed` (state becomes `Sent` before any I/O), audits `sent` (failure → Failed, no send), sends the `PreparedFrame` once, verifies the echo, reads back, audits `readback`, ends in `Done` (acknowledged) or `Failed` (exception, timeout, bad echo, CRC, link lost). Link lost also disarms.
- `PreparedFrame` has private fields, no `Clone`, and is consumed by `CommandLink::send`, so the type system prevents sending the same confirmation twice.
- A frame that the worker could not transmit (link already down) comes back with an empty request and is reported as `Ack::NotSent`; any other link loss after sending is `Ack::LinkLost` (effect unknown). Both disarm.
- Old values and read-back values are decoded without the `valid_range` check, so an out-of-range state is still shown instead of hidden.
- `CommandOutcome { ack, readback: Verified | Mismatch | Unknown, readback_value, request_hex, response_hex }` keeps acknowledgement and read-back apart (Req 16.9).

## 9. Historian

SQLite with `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`. Tables: `sessions`, `devices`, `tags`, `samples(tag, ts_ms, value_ts_ms, ts_src_ms, seq, raw BLOB, raw_value TEXT, value REAL, text, quality INT, exception INT, age_ms)` (`raw` = registers as big-endian bytes; `raw_value` = the exact decoded raw value as digits, so 64-bit integers survive; both NULL-safe), `events`, `command_audit` (triggers `RAISE(ABORT)` on UPDATE and DELETE), `outbox` (table only; logic in E7), `samples_1m(tag, minute, n, min, max, sum)`. Quality codes: GOOD 0, TIMEOUT 1, CRC_ERROR 2, EXCEPTION 3, STALE 4, DECODE_ERROR 5, DISCONNECTED 6. `LogDecimator` stores a sample when its quality differs from the last stored one or when `ts − last_stored ≥ logging interval`. `rollup_until(ts)` recomputes complete minutes from GOOD rows.

## 10. Export

`ExportDoc { generated_ms, app_version, session, devices, measurements, events, commands }` (one session per export) is built from the historian. CSV columns: `ts_utc, ts_ms, device_id, tag_id, value, text, unit, quality, exception_code, age_ms, value_ts_utc, raw_hex, seq, profile_id, profile_version`. `guard_text` applies the formula guard to every text cell in CSV and XLSX. JSON uses strings for 64-bit integers and never emits NaN.

## 11. Correctness properties

Each property has a test; `P` tests are seeded property tests (section 12).

| ID | Property | Req | Test |
|---|---|---|---|
| P1 | CRC known vectors (`123456789` → `0x4B37`, `01 03 00 00 00 08` → `44 0C`), and for any message `crc16(msg ++ crc_le(msg)) == 0` | 1.1, 1.2 | `protocol_props::crc_vectors`, `crc_residue` |
| P2 | For every read FC and random valid request/data: request encode→decode and response encode→decode are identity, through RTU and MBAP ADUs | 5.1, 1, 4 | `protocol_props::read_round_trip` |
| P3 | For every write FC: command `wire` encode→decode is identity, and the simulator's echo passes `verify_echo` | 16.1 | `tests/command_wire.rs` |
| P4 | A response split at any byte boundary, in any chunking, reassembles to the same result as one read | 2.1 | `protocol_props::fragmentation` |
| P5 | The framer, MBAP receiver, PDU decoders, codec and profile validator never panic on random bytes | 2.6 | `protocol_props::no_panic` |
| P6 | Codec round trip for every type × word order; DCBA = CDAB∘BADC; every order is an involution | 6.1, 6.2 | `codec_props` |
| P7 | Block plans cover every tag exactly once, never split a tag, stay within limits, and (gap 0) read no uncovered address | 13 | `scheduler_props::plan_props` |
| P8 | With ≥ 1 dead device on a link, every healthy device's gap between GOOD cycles stays within the Req 14.3 bound | 14.3 | `scheduler_props::no_starvation` |
| P9 | Command frames are sent once, never retried, and before polls | 12.2, 12.3 | `scheduler_props::command_priority` |
| P10 | Command service: no send unless the state was Confirmed by the current preview; at most one send per confirm; unit 0 and out-of-bounds values are never sent; every send has a preceding `sent` audit record | 16 | `tests/command_props.rs` |
| P11 | A failed read never changes `value_ts_ms` or replaces the last good value; age grows monotonically | 10.2 | `quality` unit tests + scheduler tests |
| P12 | Rust and Python validators agree on bundled profiles and fixtures | 8.3 | `tests/profile_agreement.rs` |
| P13 | Formula guard escapes every text cell starting with `= + - @ \t \r` and nothing else | 18.2 | `export` unit tests |

## 12. Property-test approach

Choice: a dependency-free seeded generator (`rng::SplitMix64` + `testkit::check`) instead of `proptest`.

- The simulator needs a deterministic PRNG anyway (R24 "deterministic seed"); one small generator serves both.
- No extra dependency tree (proptest brings `rand`, `regex-syntax`, `rusty-fork`, `tempfile` and their dependencies), which keeps the WSL and Android builds lean and the license list short.
- Failures print the seed and case index; `BUSLINK_PBT_SEED` and `BUSLINK_PBT_CASES` reproduce or deepen a run.
- Trade-off accepted: no automatic shrinking. Generators are kept small (short frames, few devices) so failing cases stay readable.

Default case counts are listed in `tasks.md` and the run report. Each property's seed is `0xB0511AC2026 ^ fnv1a(property name) ^ (case × 0x9E3779B97F4A7C15)`, so every run is reproducible without configuration.

Integration tests compare the Rust profile validator against the Python one with `python3`; the comparison is skipped with a message if `python3` is missing. XLSX tests read the workbook with `zip` 9.5.1, already in the tree through rust_xlsxwriter, added as a dev-dependency only.

## 13. Error handling

- `thiserror` enums per module; no `unwrap()`/`expect()` outside tests (clippy `unwrap_used = deny`).
- Device and network input is length-checked before indexing; no panics on device input.
- Mutex poisoning is recovered with `into_inner()`.

## 14. Out of scope here

Windows serial and Android USB transports (E3, E6), alarms (E4), the commissioning wizard flow (E5), LAN server and MQTT (E7). The `outbox` table exists so the schema is stable.
