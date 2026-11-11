# simulator: tasks

Depends on core-protocol tasks 1–12. Run `cargo test --workspace` after every task. A task is `[x]` only when its tests pass.

- [ ] 1. Crate skeleton, register image, server dispatch
  - `crates/buslink-sim` with `image.rs`, `server.rs`, `devices/mod.rs` (`SimModel`).
  - Tests: exception 01/02/03 rules, atomic multi-write (S4), quantity limits.
  - _Requirements: 1.1–1.3_

- [ ] 2. World, fixed-step time, faults
  - `world.rs`, `fault.rs`: unit routing, silence for unknown units, broadcast handling, triggers (always, probability, window, request count).
  - Tests: S1, S2, trigger windows, broadcast never replies.
  - _Requirements: 1.4, 1.5, 7.1–7.3, 8.1, 8.2_

- [ ] 3. Device models
  - `devices/soil.rs`, `vfd.rs`, `temp.rs`, `energy.rs`.
  - Tests: soil value bounds and FC03-only, VFD ramp/fault/reset, temperature lag and PI convergence (S8), energy monotonic (S9).
  - _Requirements: 3.1–3.3, 4.1–4.4, 5.1–5.4, 6.1, 6.2_

- [ ] 4. In-memory transports and loopback TCP server
  - `transport.rs` (RTU + MBAP, seeded chunking), `tcp.rs`.
  - Tests: core exchanger against the in-memory RTU transport with random chunking, core `TcpTransport` against `SimTcpServer`, connection cap, stop joins threads.
  - _Requirements: 2.1–2.4_

- [ ] 5. Bundled profiles
  - `profiles/bgt-smps-8in1.json`, `sim-vfd.json`, `sim-temp-controller.json`, `sim-energy-meter.json`, `generic-holding.json`; disclosure entry for the BGT-SMPS facts.
  - Tests: both validators pass; S3 every sim profile register decodes GOOD.
  - _Requirements: 9.1–9.4_

- [ ] 6. End-to-end tests with the core
  - Multi-device scheduler on one simulated RTU link with a dead device (S6), every fault kind as client quality (S5), command service against VFD and temperature controller (S7), write round trip (S10), historian + export of a simulated session with a gap.
  - _Requirements: 10.1, core 12–18_
