# Roles in BusLink Studio: an interlock, not authentication

This release is a **single-user local app with no login**. Whoever can run the app, and whoever controls its webview, is "the user". The role (`operator`, `commissioner`) is therefore a **safety interlock** that makes a risky action deliberate and visible in the audit log. It is **not authentication** and does not protect against someone who already controls the computer or the app process (IEC 62443 IAC/UC is not claimed).

## How the role works (security review 07, M1)

- The role lives in the Rust host (`CommandService`), not in the request. The arm request (`ArmRequest`) has only `device_id` and `user`; a request that carries `role` (or any other extra field) is refused at deserialization.
- **Arming always starts as `operator`.** A new arm, a reconnect, an app restart and a profile import all start as operator.
- `commissioner_unlock(device_id)` is a **separate host action**. It needs an active arm on that device, writes a `role_unlock` row to the audit log first (if the row cannot be written, the role does not change), and then raises the armed operator to `commissioner`.
- The unlock **lives inside the arm**. It ends when the arm ends: idle expiry (default 60 s), operator disarm, disconnect or stopped polling, reconnect or link loss, profile change, app background, or arming again. The next arm is `operator`.
- Unit ID and baud changes (communication settings) need `commissioner`; without the unlock the preview is refused with a role error.
- The UI role selector arms as operator and then calls `commissioner_unlock`; the commissioning wizard's "Arm as commissioner" button does the same two calls. The calls are separate, so the audit log shows both.
- The user name is trimmed and must be 1-64 characters without control characters. The host enforces it (`validate_user`); the UI checks the same rule for early feedback.

## What this does not do

- It does not stop a person with the app open from clicking "Commissioner". That is by design for a local training and commissioning tool; the audit log records who typed which name and when.
- The user name is self-declared text.
- The control panel and the wizard work only on simulated devices in this release (`simulated_only` profiles on the session's own Training simulator link; see `review-07-fixes.md`, M2).

## Where it is tested

`crates/buslink-core/tests/command_props.rs`: `forged_role_in_the_arm_request_is_rejected`, `user_names_are_trimmed_capped_and_free_of_control_characters`, `comm_config_needs_the_unlock_and_the_unlock_ends_with_the_arm`.
`crates/buslink-sim/tests/control_session.rs`: `commissioner_unlock_is_host_state_and_ends_with_the_arm`, `commissioner_unlock_expires_with_the_arm`.
End to end: `scripts/verify-control.mjs` (forged role over IPC refused; selector unlock; disconnect drops it).
