# simulator: requirements

Epic E2 (`docs/bmad/epics-and-sprint.md`). Crate: `crates/buslink-sim`. Covers PRD R10 and R24, and supports N6 (simulator evidence is separate from hardware evidence).
Rules that always apply: `.kiro/steering/safety.md` (rule 12: simulator results never upgrade a profile), `.kiro/steering/modbus.md`.

## Requirement 1: in-process Modbus server (R24)

**User story:** As a judge or trainee without hardware, I want simulated devices that behave like Modbus slaves on a bus, so that every BusLink feature can be tried end to end.

1. THE simulator SHALL implement the server side of FC01, FC02, FC03, FC04, FC05, FC06, FC15 and FC16 for every simulated device.
2. WHEN a request addresses an unmapped register, THE simulator SHALL answer exception 02; WHEN a function is not supported by the device, exception 01; WHEN a quantity is outside the spec limits or a value is invalid for the device, exception 03.
3. WHEN a multi-register or multi-coil write touches any address that is not writable, THE simulator SHALL reject the whole request and change nothing.
4. WHEN an RTU request has a bad CRC or addresses a unit ID that no device has, THE simulator SHALL stay silent (as a real slave does).
5. WHEN a request uses unit ID 0 (broadcast), THE simulator SHALL apply writes to every device and SHALL NOT reply, matching the serial line spec. BusLink itself never sends it (safety rule 4).

## Requirement 2: transports (R24, R3)

1. THE simulator SHALL provide an in-memory transport implementing `buslink_core::transport::Transport` for RTU framing and for MBAP framing.
2. THE in-memory RTU transport SHALL be able to deliver responses in seeded random chunk sizes, so that USB-style fragmentation is exercised.
3. THE simulator SHALL provide a Modbus TCP server bound to a loopback address (default `127.0.0.1`, port chosen by the caller or the OS) that serves all simulated devices by unit ID, can be stopped, and never binds to `0.0.0.0` by default.
4. Response timing SHALL follow the shared clock, so tests with a manual clock never sleep.

## Requirement 3: BGT-SMPS soil sensor (R10)

1. THE simulator SHALL serve the BGT-SMPS 8-in-1 register map exactly as documented (holding registers 0–7: temperature i16 ÷ 10 °C, moisture u16 ÷ 10 %, EC u16 µS/cm, salinity u16, N/P/K u16 mg/kg, pH u16 ÷ 100), FC03 only, read-only.
2. Values SHALL drift realistically (bounded mean-reverting random walk) and deterministically for a given seed.
3. Registers outside 0–7 SHALL answer exception 02 (registers 8–9 are undocumented and not served).

## Requirement 4: simulated VFD (R10, R18)

1. THE VFD SHALL expose a control word (stop 0, run 1, reset fault 4), a frequency setpoint (0.00–50.00 Hz, ×100), output frequency, status word (running, at speed, fault bits), fault code, motor current and a ramp time.
2. WHEN running, the output frequency SHALL ramp toward the setpoint at the configured ramp rate; WHEN stopped, it SHALL ramp to zero.
3. WHEN a fault is active, run commands SHALL be acknowledged but the drive SHALL stay stopped until a reset while stopped clears the fault (acknowledged ≠ reached target).
4. Out-of-range setpoints and unknown control words SHALL answer exception 03.

## Requirement 5: simulated temperature controller (R10, R18)

1. THE controller SHALL expose PV, SP, output %, mode (auto/manual), manual output and heater enable.
2. PV SHALL follow a first-order lag toward `ambient + gain × output` with a fixed time constant.
3. In auto mode a PI loop SHALL drive the output toward SP; in manual mode the manual output applies; with the heater disabled the output is 0 %.
4. Out-of-range writes SHALL answer exception 03.

## Requirement 6: simulated energy meter (R10)

1. THE meter SHALL expose voltage, current, active power, power factor and frequency as f32, and energy as u32 and u64 counters, in input registers.
2. Energy SHALL integrate active power over simulated time and never decrease.

## Requirement 7: fault injection (R24)

1. THE simulator SHALL support per-device faults: timeout (no reply), CRC error (corrupted CRC; on MBAP a corrupted byte count), exception (given code), stale (values frozen while replies continue), wrong word order (multi-register values served word-swapped, 16-bit values byte-swapped), and late reply (reply after a delay).
2. Each fault SHALL have a trigger: always, a seeded probability, a time window, or a request-count window.
3. Faults SHALL be switchable at run time and SHALL never change the device's register map.

## Requirement 8: determinism (R24)

1. WHEN two simulator worlds are created with the same seed and receive the same requests at the same simulated times, THEY SHALL produce byte-identical responses.
2. Process models SHALL advance in fixed simulated time steps, independent of how often they are polled.

## Requirement 9: bundled profiles (R10, safety rule 12)

1. THE repository SHALL ship `profiles/bgt-smps-8in1.json` (`documented`, source = the BGT-SMPS manual, register map exactly as documented), `profiles/sim-vfd.json`, `profiles/sim-temp-controller.json`, `profiles/sim-energy-meter.json` (`simulated`, `simulated_only`) and `profiles/generic-holding.json` (`generic`).
2. Every bundled profile SHALL pass both validators.
3. THE simulated device register maps SHALL match their profiles (checked by a test that reads every profile register from the simulator and decodes it without DECODE_ERROR).
4. No simulator result SHALL change a profile's support level.

## Requirement 10: drives the E1 tests (epic done-when)

1. THE simulator SHALL drive core integration tests: multi-device polling on one RTU link with the real scheduler, fault injection visible as qualities and gaps, the command service against the VFD and temperature controller with read-back, and a TCP round trip through the loopback server.
