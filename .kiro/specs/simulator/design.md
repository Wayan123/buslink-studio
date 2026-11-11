# simulator: design

Implements `requirements.md` in `crates/buslink-sim`, which depends on `buslink-core` (never the other way round).

## 1. Layout

```
crates/buslink-sim/src/
  lib.rs
  world.rs        SimWorld: devices by unit ID, faults, seed, fixed-step clock; handle_pdu / handle_rtu
  server.rs       request dispatch: read FCs via core ReadRequest, write FCs via core command::wire decoder
  image.rs        RegisterImage: sparse tables + writable sets; ImageBuilder with word-order fault
  fault.rs        FaultKind, Trigger, FaultRule, FaultTable
  transport.rs    SimTransport (in-memory, RTU or MBAP framing, seeded chunking)
  tcp.rs          SimTcpServer (std::net, loopback)
  devices/
    mod.rs        SimModel trait, model registry by simulator model ID
    soil.rs       BGT-SMPS 8-in-1
    vfd.rs        VFD
    temp.rs       temperature controller
    energy.rs     energy meter
crates/buslink-sim/tests/   integration tests that combine core + sim
```

## 2. Device model contract

```rust
trait SimModel: Send {
    fn model_id(&self) -> &'static str;
    fn step(&mut self, dt_ms: u64, rng: &mut SplitMix64);   // fixed step, 100 ms
    fn image(&self, wrong_order: bool) -> RegisterImage;    // registers as served now
    fn apply(&mut self, write: &WriteRequest) -> Result<(), ExceptionCode>;
}
```

`RegisterImage` holds four sparse tables and the set of writable addresses. The server answers reads from the image, so the register map is defined once per model. `ImageBuilder` encodes values with `buslink_core::codec`; with the wrong-order fault it serves 32/64-bit values word-swapped (`ABCD → CDAB`) and 16-bit values byte-swapped.

Writes: the server checks that every target address exists and is writable (exception 02 otherwise), then calls `apply` once with the whole request; models validate values (exception 03) before changing state, so a rejected request changes nothing.

## 3. World and time

`SimWorld::new(seed)` or `SimWorld::training(seed)` (soil 1, VFD 2, temperature 3, meter 4); `add_device(unit, model)`; each device gets its own `SplitMix64` seeded with `seed ^ (unit × golden ratio constant)`. `advance_to(now_ms)` runs `step(100 ms)` as many times as needed since the last step, so results do not depend on poll timing (Req 8.2). A device under the `stale` fault is not stepped (values frozen) but still answers.

`handle_pdu(unit, pdu, now_ms) -> Reply { pdu: Option<Vec<u8>>, delay_ms, corrupt: bool }`:
1. Advance the world.
2. Unit 0: apply writes to all devices, no reply. Unknown unit: no reply.
3. Evaluate the device's fault rules (first matching rule wins): timeout → no reply; exception → exception PDU; CRC → reply marked corrupt; late reply → `delay_ms`; stale and wrong order change how the image is produced.
4. Dispatch through `server.rs`.

## 4. Transports

`SimTransport::rtu(world, clock, chunking)` and `SimTransport::mbap(world, clock)` implement core `Transport`:
- `write_all` collects request bytes, frames RTU requests by function code (FC01–06: 8 bytes; FC15/16: `9 + byte count`; unknown FC: whole buffer), drops frames with bad CRC (silence), and queues the reply with `ready_at = now + turnaround + delay`. A corrupt reply flips CRC bits (RTU) or the byte count (MBAP).
- `read(buf, timeout)` returns queued bytes that are ready, waiting through `clock.sleep_ms` (a manual clock advances instantly); with nothing ready before the timeout it returns 0.
- Chunking: `Chunking::Whole` or `Chunking::Random { max, seed }` splits replies into 1..=max byte pieces.

`SimTcpServer::start(world, addr)` binds a `std::net::TcpListener` (loopback only unless the caller passes another address explicitly), accepts with a non-blocking poll loop, switches accepted sockets back to blocking (Windows inherits non-blocking mode from the listener), and serves one connection per thread with MBAP framing, a connection cap (8) and a 260-byte frame limit. `stop()` joins the threads.

## 5. Device models

| Model ID | Table | Registers (PDU offset) | Behaviour |
|---|---|---|---|
| `bgt_smps_8in1` | holding | 0 temp i16 ×0.1 °C, 1 moisture u16 ×0.1 %, 2 EC u16 µS/cm, 3 salinity u16, 4 N, 5 P, 6 K u16 mg/kg, 7 pH u16 ×0.01 | Ornstein–Uhlenbeck drift around plausible field values; FC03 only; read-only (writes → exception 01) |
| `vfd_basic` | holding | 0 control word (rw: 0 stop, 1 run, 4 reset), 1 setpoint ×0.01 Hz (rw 0–5000), 2 ramp time ×0.1 s (rw 1–600), 3 output freq ×0.01 Hz, 4 status bitfield (bit0 running, bit1 at speed, bit3 fault), 5 fault code, 6 motor current ×0.1 A | ramp at `50 Hz / ramp time`; current ∝ frequency; faults latch until reset while stopped |
| `temp_controller_basic` | holding | 0 PV i16 ×0.1 °C, 1 SP i16 ×0.1 °C (rw 0–1200), 2 output u16 ×0.1 % , 3 mode (rw 0 auto, 1 manual), 4 manual output ×0.1 % (rw 0–1000), 5 heater enable (rw 0/1) | `τ dPV/dt = ambient + gain·out − PV` (τ 60 s, gain 150 °C, ambient 25 °C); PI in auto |
| `energy_meter_basic` | input | 0 voltage f32 V, 2 current f32 A, 4 active power f32 W, 6 power factor f32, 8 frequency f32 Hz, 10 energy u32 ×0.01 kWh, 12 energy u64 Wh (ABCD) | sinusoidal load profile with noise; energy integrates power |

Fault codes, ranges and register maps are simulator design choices, not vendor documentation; the profiles say so.

## 6. Fault model

```rust
enum FaultKind { Timeout, CrcError, Exception(u8), Stale, WrongOrder, LateReply { delay_ms: u64 } }
enum Trigger { Always, Probability { per_mille: u16 }, Window { from_ms, until_ms }, Requests { skip: u32, count: u32 } }
struct FaultRule { unit: u8, kind: FaultKind, trigger: Trigger }
```

Probability draws come from a dedicated fault RNG seeded from the world seed, so a fault sequence is reproducible. Every probabilistic rule draws once per request to its unit, active or not. `Requests` counts requests per device. `Stale` needs a deterministic trigger (always, window, request count): it is evaluated while the process steps, where there is no request to draw for.

"Stale" means frozen process values while communication stays GOOD (a gateway serving a cache). Client-side STALE quality comes from age, so it is produced by timeouts and backoff, not by this fault; the training material explains both.

## 7. Correctness properties

| ID | Property | Req | Test |
|---|---|---|---|
| S1 | Same seed + same request schedule → byte-identical responses | 8.1 | `tests/determinism.rs` |
| S2 | Model state after N fixed steps does not depend on how `advance_to` calls were split | 8.2 | `tests/determinism.rs` |
| S3 | Every register of every bundled sim profile reads and decodes GOOD from the simulator | 9.3 | `tests/profiles_match.rs` |
| S4 | A rejected multi-write changes no register | 1.3 | `tests/server.rs` |
| S5 | Each fault kind yields the expected client quality (TIMEOUT, CRC_ERROR, EXCEPTION(code), GOOD with frozen values, DECODE_ERROR or wrong values, TIMEOUT then the late reply discarded) | 7.1 | `tests/end_to_end.rs` |
| S6 | With the soil sensor dead, the VFD and meter on the same link keep their poll interval within the core fairness bound | 10.1 | `tests/end_to_end.rs` |
| S7 | VFD: run → ack, read-back verified, output ramps; fault → run acknowledged but not running; reset clears | 4.2, 4.3 | `tests/end_to_end.rs` |
| S8 | Temperature PV converges toward SP in auto mode and never exceeds the physical bound | 5.2, 5.3 | `tests/models.rs` |
| S9 | Energy counters never decrease | 6.2 | `tests/models.rs` |
| S10 | P3 (core): command `wire` encodings are decoded by the simulator server and echoed correctly for FC05/06/15/16 (a test-only coil/register bank model covers FC05/15, which no bundled model uses) | core 16.1 | `tests/end_to_end.rs` |

## 8. Safety notes

- The simulator is the only place outside `command/` that decodes write PDUs; it never encodes write *requests*.
- Bundled simulator profiles are `simulated_only: true` with support level `simulated`; nothing in this crate edits profiles.
- The TCP server binds loopback by default and has a connection cap; it is a training server, not a production slave.
