Create the spec and implement epic E5 `commissioning-control` (docs/bmad/epics-and-sprint.md), as the buslink-builder agent.

Read: docs/bmad/prd.md (R17, R18, R19, N1), docs/bmad/architecture.md (D8), research/02 §2 and §5, .kiro/steering/safety.md, the existing command service in crates/buslink-core/src/command/, the simulator VFD and temperature-controller models, and the app/UI code from E3/E4.

## Spec
`.kiro/specs/commissioning-control/` with EARS requirements, correctness properties and tasks.

## Implement
1. **Commissioning wizard (R17)** for one device: change unit ID or baud using the profile's documented config registers. Steps:
   - Record a journal entry (old settings).
   - Arm, preview the frame, confirm.
   - Write once.
   - Reconnect at the new settings.
   - Read back, then mark pass or fail.
   - If it fails, show recovery hints with the old settings.
   Only offer this for profiles whose `commands` section declares the config registers. In the competition build these are the simulated devices; the soil sensor profile has none unless its manual documents them, so don't invent them.
2. **Control panel (R18)** for the simulated VFD (run/stop, frequency setpoint, ramp, fault reset) and the simulated temperature controller (setpoint, output mode).
   - Arm with a session timeout (default 120 s), and show a countdown.
   - Check bounds from the profile.
   - Show an explicit confirm dialog whose label includes the engineering value and unit ("Apply setpoint 40.0 °C"), with the encoded frame preview (hex).
   - Single attempt, then read back. "Write acknowledged" is shown separately from "process reached target", which is judged against live PV feedback with a tolerance and a timeout.
   - The software STOP is always visible while armed and is labelled "Software stop, not an emergency stop".
   - Disconnect or reconnect drops the arm state. Nothing auto-resumes.
3. **Audit log view** with filters, exportable to CSV/XLSX. **Session report (R19)** as HTML and PDF (print-to-PDF through the WebView is fine): devices, profile versions, quality statistics, alarms, commands with read-back results.
4. **Tests:**
   - Property tests: a command is never sent unarmed, unconfirmed, after its arm has expired, or twice; bounds are always enforced.
   - Simulator integration tests for VFD and temperature-controller journeys: set 30.0 Hz, read back OK, reached target, stop.
   - A UI headless check of journey 4 (PRD §5), with screenshots in docs/verification/.

Run `cargo test --workspace`, the UI type-check and build, and `cargo.exe check` for src-tauri. Mark tasks `[x]`. Report honestly. Do not commit.
