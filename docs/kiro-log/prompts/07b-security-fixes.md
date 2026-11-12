Fix the findings of the read-only security review in docs/security/review-07.md, as the buslink-builder agent. Read the review first, plus .kiro/steering/safety.md.

## Must fix
1. **M1, self-asserted role** (`src-tauri/src/control.rs`, `ArmReq` / `control_arm`).
   - The webview must not be able to claim `Commissioner` by sending it in IPC.
   - Keep the role in host state (Rust). It starts as `Operator`.
   - Raising it to `Commissioner` needs a separate deliberate host action: a `commissioner_unlock` command that is audited and expires with the arm. Disconnect, reconnect, profile change and app background drop it back to `Operator`.
   - The UI role selector calls that command. It no longer sends the role inside `ArmReq`.
   - Cap `user` at 64 characters, trim it, and reject control characters. Do this both server-side and in the UI.
   - Document in docs/ (and in the UI hint) that this release is a single-user local app with no login, so the role is a safety interlock, not authentication.
   - Tests: a forged `role: Commissioner` in the arm request is ignored or rejected; comm-config writes fail without the unlock; the unlock expires and is dropped on disconnect.
2. **M2, loopback exemption for `simulated_only` profiles** (`crates/buslink-core/src/device.rs:69`, `src-tauri/src/commands.rs` `host_of`).
   - A `simulated_only` profile is controllable only on a link the session itself created for the in-process training simulator, identified by a session-generated marker. "Any loopback TCP address" no longer counts.
   - A user-entered `127.0.0.1:<port>` link with a `simulated_only` profile is monitor-only, and the UI says why.
   - Tests: training link allowed; manual loopback link refused for control; manual non-loopback refused.

## Low items to fix now
3. **L1, LAN token in URLs.** Accept `?t=` only on `GET /`. Set the existing cookie and answer 303 to `/` without the token. API and WebSocket routes accept only the cookie. Update `verify-lan.mjs` and the router tests.
4. **L2, LAN snapshot leaks profile metadata.** Send a display subset per profile (id, version, register id/name/unit/scale/display_range/valid_range, alarm limits). Leave out `source.url` and `evidence`. Add a test asserting those keys are absent.
5. **L3, connection starvation.** Add a per-IP cap (for example 4) and a separate cap on unauthenticated connections. Add a test.
6. **Info, e2e feature guard.** Make the release build script or CI fail if `--features e2e` reaches an installer or APK build.

## Leave as documented
- `style-src 'unsafe-inline'`.
- The native confirm dialog for high-risk writes.
- The Android Keystore.

Record these three in docs/security/review-07-fixes.md as accepted with reasons, together with a table of every finding: id, status (fixed / accepted), commit-ready file list, test name.

## Checks
Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `npm test`, `npm run build`, `cargo.exe check` for src-tauri, the hook self-tests, and then rebuild the e2e binary and rerun `verify-control.mjs` and `verify-lan.mjs`. Mark nothing as fixed unless its test passes. Do not commit.
