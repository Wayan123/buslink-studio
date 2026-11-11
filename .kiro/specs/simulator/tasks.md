# simulator: tasks

Depends on core-protocol tasks 1–12. Run `cargo test --workspace` after every task. A task is `[x]` only when its tests pass.

- [x] 1. Crate skeleton, register image, server dispatch
  - `crates/buslink-sim` with `image.rs`, `server.rs`, `devices/mod.rs` (`SimModel`).
  - Tests: exception 01/02/03 rules, atomic multi-write (S4), quantity limits.
  - _Requirements: 1.1–1.3_

- [x] 2. World, fixed-step time, faults
  - `world.rs`, `fault.rs`: unit routing, silence for unknown units, broadcast handling, triggers (always, probability, window, request count).
  - Tests: S1 [100], S2 [100], trigger windows and request counts, first shaping fault wins, stale freezes values, wrong order swaps, broadcast never replies.
  - _Requirements: 1.4, 1.5, 7.1–7.3, 8.1, 8.2_

- [x] 3. Device models
  - `devices/soil.rs`, `vfd.rs`, `temp.rs`, `energy.rs`.
  - Tests: soil value bounds [8] and FC03-only, VFD ramp/fault/reset, temperature lag and PI convergence (S8) [30], energy monotonic (S9) [20].
  - _Requirements: 3.1–3.3, 4.1–4.4, 5.1–5.4, 6.1, 6.2_

- [x] 4. In-memory transports and loopback TCP server
  - `transport.rs` (RTU + MBAP, seeded chunking), `tcp.rs`.
  - Tests: core exchanger against the in-memory RTU transport with random chunking [200], MBAP in memory, core `TcpTransport` against `SimTcpServer`, connection cap, stop joins threads.
  - _Requirements: 2.1–2.4_

- [x] 5. Bundled profiles
  - `profiles/bgt-smps-8in1.json`, `sim-vfd.json`, `sim-temp-controller.json`, `sim-energy-meter.json`, `generic-holding.json`; disclosure entry for the BGT-SMPS facts.
  - Tests: both validators pass; S3 every sim profile register decodes GOOD.
  - _Requirements: 9.1–9.4_

- [x] 6. End-to-end tests with the core
  - Multi-device scheduler on one simulated RTU link with a dead device (S6) [20], every fault kind as client quality (S5), command service against VFD and temperature controller, also through the link worker with audit stored in the historian (S7), write round trip (S10) [500], historian + export of a simulated session with a gap.
  - _Requirements: 10.1, core 12–18_
