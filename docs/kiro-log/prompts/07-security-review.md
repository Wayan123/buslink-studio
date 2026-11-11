Review the code changes since the commit tagged in your task message (use `git diff <base>..HEAD` and `git show`) against .kiro/steering/safety.md, docs/bmad/architecture.md and research/02 §5 (IEC 62443).

Check each category and give a verdict for every one, saying "no findings" where clean:
1. Any code path that can emit FC05/06/15/16 (or raw write frames) outside `crates/buslink-core/src/command/**` and `crates/buslink-sim/**`.
2. Command service bypass: sending unarmed, after expiry, without confirmation, twice, or with broadcast unit ID 0.
3. Bounds enforcement on setpoints and commissioning values.
4. LAN server: token check on every route, constant-time comparison, any write or state-changing route, TLS on by default, interface binding (no 0.0.0.0 by default), rate and client limits, token leaking into logs.
5. MQTT: credential storage (no plaintext in files or logs), TLS verification not disabled, no command subscriptions, outbox bounds.
6. Profiles and imports: JSON Schema enforced at load; no expression or script evaluation; CSV/XLSX import size limits; formula-injection guard on export.
7. Android: USB permission handling, foreground-service declaration, exported components, SAF use, no broad storage permission.
8. Tauri: the capabilities/allowlist is minimal, IPC commands validate their inputs, CSP is set, and remote content is never loaded.
9. Secrets in the repo, logs or exports.
10. Quality honesty: is a stale or bad value ever shown as fresh, or interpolated across gaps?

Output a findings table: severity (critical/high/medium/low/info), file:line, issue, concrete fix. Do not edit files.
