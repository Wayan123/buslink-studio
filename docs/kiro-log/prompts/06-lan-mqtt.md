Create the spec and implement epic E7 `lan-mqtt` (docs/bmad/epics-and-sprint.md), as the buslink-builder agent.

Read: docs/bmad/prd.md (R22, R23, N2), docs/bmad/architecture.md (D4, D6, D7), research/02 §4 (MQTT/Sparkplug verdict) and §5 (IEC 62443), research/03 rows on axum and rumqttc, .kiro/steering/safety.md, and the app shell and sample bus from E3.

## Spec
`.kiro/specs/lan-mqtt/` with EARS requirements, correctness properties and tasks.

## Implement
1. **LAN dashboard (R22).** An axum server on the Tauri tokio runtime, **off by default**.
   - The user picks an interface (list private IPv4 addresses only) and a port (default 8443).
   - Self-signed certificate per session (rcgen) and a random 128-bit token.
   - The URL (`https://<ip>:<port>/?t=<token>`) is shown as text and as a QR code.
   - The same built frontend is served in a read-only mode: no connect, arm, write or export buttons.
   - Routes: `GET /` (static), `GET /api/snapshot`, `GET /api/trend?tag&from&to` (decimated server-side), `WS /api/stream`. All require the token (constant-time compare). There are **no write routes**.
   - Rate limiting and a maximum number of clients. Stop closes everything, and a restart issues a new token and certificate.
   - Works on Windows and Android. On Android, keep targetSdk 36 (research/03 notes on the Android 17 local-network permission).
2. **MQTT telemetry (R23).** Off by default. MQTT 5 with TLS via rumqttc; certificate verification uses rustls-platform-verifier on Android and native roots on Windows.
   - Topics `buslink/{site}/{gateway}/{device}/{tag}`, payload `{ts, seq, value, unit, quality, profile_version}`.
   - A retained `status` topic, with a will message setting `offline`.
   - QoS 1, message expiry, a bounded SQLite outbox replayed in order with the original ts/seq when the connection returns.
   - Credentials go in the OS keychain (Windows Credential Manager via the keyring crate) or the Android Keystore through the plugin. **They are never stored in project files or logs.**
   - No subscriptions to command topics.
3. **UI:** a Share panel (LAN on/off, URL + QR, connected clients) and an MQTT panel (broker URL, client ID, credentials, test connection, outbox depth, last error).
4. **Tests:**
   - The token is required on every route; there are no write methods (a test enumerates the router).
   - The WebSocket stream sends the same samples as the bus.
   - An integration test with an in-process or embedded broker (rumqttd is acceptable as a dev-dependency): disconnect, outbox fills, replay without duplicates, and the `seq` order is preserved.
   - A headless browser test of the LAN view against the running app (journey 3), with screenshots.

Run `cargo test --workspace`, the UI build, and `cargo.exe check` for src-tauri. Mark tasks `[x]`. Report honestly. Do not commit.
