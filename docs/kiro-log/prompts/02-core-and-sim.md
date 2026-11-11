Create Kiro feature specs for the first two epics and then implement them, as the buslink-builder agent.

Read first: docs/bmad/prd.md, docs/bmad/architecture.md, docs/bmad/epics-and-sprint.md, research/*.md, all .kiro/steering files. Reference (read only, do not copy): /mnt/d/KERJA/PT AGS/Proyek 2026/Soil-Sensor-Android/crates/soil-core/src (protocol.rs, reader.rs, model.rs) for verified facts about the BGT-SMPS sensor and USB fragmentation.

## Step 1: specs
Create `.kiro/specs/core-protocol/` and `.kiro/specs/simulator/` (requirements.md with EARS criteria citing PRD IDs, design.md with explicit correctness properties, tasks.md numbered with requirement references).

core-protocol covers PRD R4–R9 and R7 (D1, D3, D4, D8, D9 in architecture.md), as a Rust workspace crate `crates/buslink-core`. No Tauri dependency.
- RTU framing + CRC-16/MODBUS; length-based response parsing by FC/byte count with timeout; exception frames; TCP MBAP with transaction IDs.
- Codec for all types and the four word orders; scale/offset/range/sentinel.
- Profile JSON schema (`profiles/schema/buslink-profile.schema.json`) + Rust types + validation, with support levels, and a stdlib validator compatible with the existing hook `tools/hooks/check_profile.py`. Update that script if needed so both agree.
- Device instance vs profile.
- Quality model with age.
- A scheduler with one worker per link, a priority queue (command > alarm > poll), contiguous block merging within the FC limits (125 registers / 2000 coils), per-device exponential backoff, and fairness.
- A `Transport` trait + `FakeTransport` for tests.
- The command service state machine (Idle→Armed→Previewed→Confirmed→Sent→ReadBack→Done/Failed, single attempt, audit records). This is the ONLY module allowed to build write FCs (the guard hook enforces this).
- Historian schema + insert/query + minute rollups (rusqlite bundled); CSV/XLSX/JSON export with a formula guard.

simulator covers R10 and R24 as crate `crates/buslink-sim`:
- In-process devices implementing the server side of RTU and TCP over an in-memory or loopback transport.
- BGT-SMPS soil sensor values (realistic drift), VFD (run/stop/frequency setpoint/ramp/fault), temperature controller (PV/SP/output with first-order lag), energy meter.
- Fault injection (timeout, CRC error, exception, stale, wrong order).
- A deterministic seed.

Create bundled profiles in `profiles/`:
- `bgt-smps-8in1.json` (support level `documented`, source = the manual in the reference repo: register addresses and scaling exactly as that manual/reference code states; do not invent).
- `sim-vfd.json`, `sim-temp-controller.json`, `sim-energy-meter.json` (`simulated`).
- `generic-holding.json` (`generic`).

## Step 2: implement all tasks of both specs
Property tests are mandatory (dependency-free seeded generator or `proptest` if you judge it worth one dev-dependency; justify the choice in design.md):
- CRC against known vectors.
- Frame encode→decode round trip for every FC.
- Decoder never panics on random bytes.
- Codec round trip for every type × word order.
- Scheduler never starves a healthy device when another one is dead.
- The command state machine never sends without Confirmed and never sends twice.

Run `cargo test --workspace` after each change; hooks will also run it. Mark tasks `[x]` as they pass. Pin dependency versions. Don't create the Tauri app yet. Report: tasks done, test counts, PBT case counts, and anything left. Do not commit.
