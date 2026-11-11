# app-shell-desktop: design

Implements `requirements.md`. Depends on the finished `core-protocol` and `simulator` specs.

## 1. Where the code lives and why

| Part | Location | Reason |
|---|---|---|
| Session service (links, workers, devices, hub thread, live store, frame log, historian writer, export, inspector) | `crates/buslink-core/src/session/` | Pure Rust, no Tauri. Tested in WSL with `cargo test --workspace` against the simulator, so the shell stays a thin IPC layer. Android (E6) and the LAN view (E7) reuse it unchanged. |
| Min-max trend decimation | `crates/buslink-core/src/trend.rs` | Pure function with property tests (research/02 §7: decimate on the Rust side). |
| USB VID:PID classification | `crates/buslink-core/src/transport/usb_ids.rs` | The same table serves Windows now and Android later. |
| Frame tap | `crates/buslink-core/src/poll/scheduler.rs` (`set_frame_tap`) | Additive hook: the scheduler reports each `ExchangeRecord` it already has, so the inspector shows real frames without a second poll. |
| Training bus + fault buttons | `crates/buslink-sim/src/training.rs` | The simulator owns faults; core never depends on the simulator. |
| Windows serial transport, port listing, IPC commands, event emitter, save dialog | `src-tauri/src/` | Platform glue only. |

The session takes a **transport factory** (`Arc<dyn Fn(&LinkConfig) -> Result<Box<dyn Transport>, TransportError>>`), so core never links `serialport`, Tauri or the simulator. The shell's factory maps `LinkKind::Rtu` → `SerialTransport` (Windows), `Tcp`/`RtuOverTcp` → core `TcpTransport`, `Simulator` → `Training::transport()`.

## 2. Session service

```text
Session
 ├─ links: Mutex<BTreeMap<link_id, LinkEntry>>
 │    LinkEntry { config, simulated, transport: Arc<Mutex<Box<dyn Transport>>>,
 │                devices: Vec<(DeviceInstance, Arc<Profile>)>, worker: Option<LinkWorker>,
 │                state: Connected | Polling | Down | Disconnected, last_counters, error }
 ├─ bus: SampleBus  ← every worker's sink (re-sequenced by one AtomicU64: Req 4.8)
 ├─ hub thread: subscribes to the bus every 50 ms
 │    live.apply(sample) → alarms.observe(sample) → LogDecimator → SampleBatcher (≤ 1 s) → Historian
 │    alarms.tick(now); alarm events → historian events; worker Down detection → link event
 ├─ live: Mutex<LiveStore>   (latest TagView per tag, pending trend points, dropped counter)
 ├─ alarms: Mutex<AlarmEngine> (dashboard-ui spec)
 ├─ frames: Mutex<FrameLog>  (32 exchanges per (link, unit))
 └─ historian: Option<Mutex<Historian>> + batcher: Mutex<(LogDecimator, SampleBatcher)>
```

Lock order is fixed: `links` → `live` → `alarms` → `batcher` → `historian`. Worker threads take no session lock (their sink only publishes to the bus; the frame tap takes only `frames`).

- **Ownership (Req 2.6, 4.3).** `connect` refuses a link whose `owner_key()` matches an existing entry or is owned in the `LinkRegistry`. The transport is opened once at connect and wrapped in `SharedTransport` (delegating, `close()` is a no-op) so a worker restart (device added or removed, Req 4.4) reuses the open port; `disconnect` stops the worker and closes the real transport. Each worker claims the owner key from the registry when it spawns and releases it when its thread ends.
- **No write path (Req 4.5).** The session has no method that builds or submits a write. The shell exposes none. Control arrives with E5 through the command service.
- **Down, never auto-restart (Req 4.7).** The hub sees `LinkWorker::is_finished()` with state `Down`, marks the link Down, records an event and drops the worker. Only a user `disconnect` + `connect` + `start_polling` polls again.
- **User disconnect (Req 4.6).** For every tag of the link the session republishes its last sample with quality `DISCONNECTED`, `value` and `value_ts_ms` unchanged, `age_ms` = now − value time, `raw` none.

## 3. Live frames (Req 5)

`LiveStore::apply` keeps per tag the last `Arc<Sample>`, the last non-empty raw registers (for the inspector) and a `TagView` (serialisable: key `device/tag`, quality text, exception code, engineering value, text, exact raw value string, value time, sample time, age, raw words, unit, error). Every applied sample also becomes a `TrendPoint {k, t, v, q}` with `v = None` unless GOOD.

`take_frame()` returns the changed `TagView`s, the pending points (at most 20 000; the rest is counted in `dropped`), bus lag notices added to `dropped`, the link views and the alarm snapshot if it changed. The shell's emitter thread calls it every 250 ms (≤ 4 Hz, Req 5.1) and emits `buslink://frame` when the frame is non-empty, plus a heartbeat once per second. `snapshot()` returns every tag, link and the alarm state (Req 5.4).

## 4. Trend decimation (Req 7)

`trend::decimate(samples, from_ms, to_ms, buckets, max_gap_ms) -> TrendSeries { t, v, gaps, decimated, raw_count }`:

1. Walk samples in time order. A GOOD sample with a value extends the current **run**; anything else closes the run and opens (or extends) a gap whose reason is the first bad quality. Two GOOD samples further apart than `max_gap_ms` close the run with a `NO_DATA` gap.
2. Each gap emits exactly one `null` point (at the first bad sample, or at the midpoint of a no-data interval) and one `Gap {from_ms, to_ms, reason}`.
3. If the GOOD samples in the range fit in `2 × buckets`, runs are emitted raw. Otherwise each run is cut by bucket (`(ts − from) × buckets / (to − from)`) and each (run, bucket) emits its minimum and maximum sample in time order (one point if they coincide).

The session calls it with the historian samples of one tag (after flushing the batcher) and `max_gap_ms = device stale threshold`.

### Correctness properties (property tests, `crates/buslink-core/tests/trend_props.rs`)

- **T1 values are real:** every output point with a value equals a GOOD input sample with the same time and value.
- **T2 gaps are gaps:** between two consecutive output values there is no non-GOOD input sample and no input interval longer than `max_gap_ms`.
- **T3 extremes kept:** for every (run, bucket), its minimum and maximum input values appear in the output.
- **T4 bounded:** when decimated, output values ≤ 2 × (buckets + runs).
- **T5 sorted:** output times are non-decreasing; nulls count equals gaps count.

## 5. Inspector backend (Req 9)

`set_frame_tap` on the scheduler calls a closure with every `ExchangeRecord` (poll, alarm and command jobs). The session's tap stores `FrameEntry {ts_ms, unit, request_hex, response_hex, outcome, error, elapsed_ms}` in a ring of 32 per (link, unit). `inspect(device)` combines the profile register (table, doc label via `Register::doc_label`, PDU offset, read FC), the last raw words, `codec::decode_all_orders` for 32/64-bit types, the live quality and the exception name, plus the frame ring and the device counters.

## 6. Historian and export (Req 6, 8)

- Database: `<app_data_dir>/buslink-history.sqlite3` (WAL). Session ID `s<utc_ms>`; app version from Cargo; platform `windows`. Logging interval default 1000 ms through `LogDecimator` (quality changes always stored).
- If opening fails, `historian = None` and `historian_error` is set; live values still flow (Req 6.5).
- `export(format)`: flush the batcher, `ExportDoc::from_historian`, then `export::csv::measurements`, `export::xlsx::workbook` or `export::json::to_string`. The shell asks the native save dialog (`tauri-plugin-dialog`, Rust side, blocking call on a worker thread) for a path with filter `csv`/`xlsx`/`json` and a suggested name `buslink-<session>.<ext>`, writes the bytes, and returns `{saved: true, path}` or `{saved: false}` (Req 8.2–8.3). The webview never sends a path.

## 7. Windows serial transport (Req 2)

`SerialTransport` wraps `serialport::SerialPort` (4.10.1): `open(name, SerialSettings)` sets baud, data bits, parity, stop bits, `FlowControl::None`. `read(buf, timeout)` sets the port timeout and maps `TimedOut` to `Ok(0)`; other errors map to `Disconnected`. Open errors: `NoDevice` → "port not found", access denied → "port busy or access denied" (`PermissionDenied`). `close()` drops the port. Listing uses `serialport::available_ports()`; `usb_ids::classify(vid, pid)` adds the chip label. On non-Windows desktop builds the serial functions return a clear "not available on this platform" error.

## 8. Training (Req 10)

`Training::new(seed, clock)` builds `SimWorld::training(seed)` (units 1–4). Its link is `LinkKind::Simulator { name: "training", 9600 8N1 }` with ID `training`; `transport()` returns `SimTransport::rtu` with seeded random chunking (USB-like fragmentation). The shell connects it, adds the BGT-SMPS (unit 1), VFD (2) and temperature controller (3) instances, adds training alarm presets (dashboard-ui) and starts polling. `inject(unit, fault, duration_ms)` adds a `FaultRule` with `Trigger::Window { now, now + duration }` on the simulator's monotonic clock; `clear(unit)` removes them; `active(now)` lists them for the UI. Fault IPC refuses any link that is not the training link (Req 10.3). Profiles are never modified (Req 10.4).

## 9. IPC surface (`src-tauri/src/commands.rs`)

`app_info`, `list_ports`, `list_profiles`, `connect_link`, `disconnect_link`, `add_device`, `remove_device`, `start_polling`, `stop_polling`, `live_snapshot`, `trend_query`, `historian_sessions`, `export_session`, `inspect_device`, `alarm_*` (dashboard-ui), `training_start`, `training_fault`, `training_clear`, `training_status`. Every command returns `Result<T, String>` with a human-readable error. Arguments are validated in core (`DeviceInstance::check`, `SerialSettings::validate`, host/port parsing). No command accepts a file path, a raw frame or a write request.

Capabilities: `core:default` only on the main window. CSP: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`. (`'unsafe-inline'` for styles is required by uPlot, which sets element styles; scripts stay `'self'`.)

## 10. Session correctness properties (`crates/buslink-sim/tests/session.rs`, real clock, short intervals)

- **S1 training journey:** start training → three devices GOOD within 3 s → timeout fault on unit 3 for 1.5 s → its tags go TIMEOUT then STALE/GOOD again → trend query for one of its tags contains a gap with reason TIMEOUT → CSV export contains TIMEOUT rows for unit 3 and GOOD rows for units 1 and 2.
- **S2 one master:** a second `connect` with the same owner key is refused; after `disconnect` it succeeds.
- **S3 unit 0:** `add_device` with unit 0 is refused with the core reason.
- **S4 disconnect:** after `disconnect`, every tag of the link reports DISCONNECTED with its old value time; no new GOOD sample appears.
- **S5 sequence:** sample sequence numbers seen on the bus are strictly increasing across two links.
- **S6 no silent drop:** points delivered by frames + `dropped` = samples published.
