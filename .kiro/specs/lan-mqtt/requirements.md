# lan-mqtt: requirements

Epic E7 (`docs/bmad/epics-and-sprint.md`). Covers PRD R22, R23 and N2; architecture D4 (sample bus), D6 (LAN server), D7 (plain JSON topics); research/02 §4 (MQTT 5, plain JSON topics, outbox, no commands) and §5 (IEC 62443 FR4/FR5: off by default, one interface, TLS + token, read-only, rate limited).
Rules that always apply: `.kiro/steering/safety.md` (rules 11, 14, 15, 16 in particular), `.kiro/steering/ui.md` (security in the UI, LAN build renders no control UI), `.kiro/steering/android.md` (network).

Out of scope: any write, command or arming route on the LAN server; command subscriptions over MQTT; Sparkplug B; MQTT over WebSocket; Modbus/TCP Security (port 802); user accounts on the LAN view; Android Keystore secret storage verified on a phone (see design §7); `ACCESS_LOCAL_NETWORK` (targetSdk stays 36).

Decisions recorded while writing this spec:
- The LAN view serves the L1 overview and L3 trends only. The alarm list has acknowledge and shelve buttons, and the register inspector needs a device query, so neither is in the LAN build. The alarm banner (counts) is shown because it is display only.
- The four routes of the request are the only routes. Profile ranges for the analog bars travel inside the `/api/snapshot` response (`profiles` key), not on a fifth route.
- `/api/stream` carries the same `TagView`/`TrendPoint` records the desktop webview gets, built from the same sample bus.
- Static assets are protected by the token too: `GET /?t=…` (the only place a URL token is accepted) sets a `Secure; HttpOnly; SameSite=Strict` cookie and answers 303 to `/`, so the token does not stay in the address bar; every other route, including the WebSocket, accepts the cookie only (security review 07, L1).
- The embedded test broker is written on rumqttc's MQTT 5 packet codec instead of rumqttd: the tests must cut the connection, withhold acknowledgements and inspect every packet the client sends (no SUBSCRIBE), which a stock broker does not allow.

## Requirement 1: LAN dashboard server (R22, N2)

**User story:** As a technician, I want to show the live view to a colleague's browser on the same Wi-Fi, so that they can watch without touching the bus.

1. THE LAN server SHALL be off at every app start and SHALL start only after the user presses Start sharing.
2. THE interface list SHALL contain private IPv4 addresses only (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16); the server SHALL refuse to bind 0.0.0.0, a public address, a multicast address or an IPv6 address. The default port SHALL be 8443.
3. WHEN the server starts, THE system SHALL generate a new self-signed certificate (subject alternative name = the chosen IP) and a new random 128-bit token from the OS random source; THE URL `https://<ip>:<port>/?t=<token>` SHALL be shown as text and as a QR code on the host device only, together with the certificate SHA-256 fingerprint.
4. THE server SHALL expose exactly these routes, all GET: `/` (and the static files of the built frontend), `/api/snapshot`, `/api/trend?tag&from&to[&buckets]`, `/api/stream` (WebSocket). No other method or path SHALL return anything but 401, 404, 405 or 429.
5. EVERY request, including unknown paths and the WebSocket upgrade, SHALL carry the token (query `t` or the cookie set by the first page load) and SHALL be compared in constant time; WHEN the token is missing or wrong, THE server SHALL answer 401 with no detail and count the failure for rate limiting.
6. THE server SHALL reject any request with a body (Content-Length > 0 or Transfer-Encoding) with 400 and SHALL reject a WebSocket upgrade whose Origin does not match the Host.
7. THE server SHALL rate-limit per client address (token bucket) and lock an address out for 60 s after 5 failed token checks in 60 s (429 with Retry-After); SHALL limit open connections (default 32) and WebSocket clients (default 8; the next one gets 503); SHALL time out TLS handshakes (5 s) and request headers (10 s).
8. THE server SHALL send `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff` and a Content-Security-Policy that allows only same-origin resources.
9. WHEN the user presses Stop sharing, THE server SHALL close the listener and every open connection and WebSocket, and the token and certificate SHALL be discarded; WHEN sharing starts again, THE system SHALL issue a new token and certificate, so the old URL stays invalid.
10. THE LAN server SHALL run on the Tauri async runtime on Windows and Android and SHALL read the same session and sample bus as the webview; it SHALL NOT open a bus, poll, or call the command service.

## Requirement 2: live data to browsers (R22, R12, safety rule 11)

**User story:** As a viewer, I want the same tiles, quality and age as the operator sees, so that I am never shown a stale value as current.

1. WHEN a WebSocket client connects, THE server SHALL first send a consistent snapshot (tags, links, alarms, profiles for the analog bars) and then the frames produced after that snapshot, at most 4 per second, independent of the poll rate.
2. THE frames SHALL contain, for every sample published on the sample bus, the same sequence number, value, unit, quality, exception code and age as the bus sample; failed reads SHALL carry the last good value and its original timestamp, never a zero or a fresh time.
3. WHEN a client falls behind, THE server SHALL tell it ("lagged") and send a fresh snapshot instead of silently skipping samples; slow clients SHALL be dropped after a 5 s send timeout.
4. `/api/trend` SHALL return the historian series decimated on the server (min/max per pixel column, gaps preserved) with `buckets` clamped to 10–4000; `from` and `to` SHALL be validated (`from < to`) and an unknown tag SHALL give 404.
5. THE frontend served to browsers SHALL be the same build in a read-only mode, selected by the server (a `buslink-mode=lan` meta tag in `index.html`): no connect, arm, control, commissioning, audit, report, alarm acknowledge/shelve or export controls SHALL be rendered, and the UI SHALL show a "Read-only view" notice. The browser build SHALL contain no call path to a write route (there is none).
6. THE view SHALL show the link state of the stream ("Live", "Reconnecting…") and SHALL mark all values STALE-looking via the normal age display when the stream is down.

## Requirement 3: MQTT 5 telemetry (R23, N2, safety rules 14 and 16)

**User story:** As an integrator, I want selected tags published to my broker without losing data during a Wi-Fi drop, so that my historian has no silent gaps.

1. THE MQTT publisher SHALL be off at every app start and SHALL start only after the user enables it.
2. THE publisher SHALL use MQTT 5 over TLS (rustls). Plain `mqtt://` SHALL be refused unless the user ticks "Allow unencrypted connection" for a local test broker. Certificate verification SHALL use the platform roots (native roots on Windows, rustls-platform-verifier on Android) plus an optional user-supplied CA PEM.
3. THE topic SHALL be `buslink/{site}/{gateway}/{device}/{tag}`; every level SHALL be sanitised so it contains no `/`, `#`, `+`, `$`, NUL, control character or empty string.
4. THE payload SHALL be JSON with `ts` (UTC epoch ms of the result), `seq`, `value`, `unit`, `quality`, `profile_version`, and also `age_ms`, `exception` (when quality is EXCEPTION), `session` (so `(gateway, session, seq)` is a unique key). A non-GOOD quality SHALL carry the last good value with its growing age, or `null`; it SHALL never carry 0 or a fresh timestamp.
5. THE gateway SHALL publish a retained `buslink/{site}/{gateway}/status` = `online` after each connect, and SHALL register a will message (retained, QoS 1) with `offline`; a clean stop SHALL publish `offline` before disconnecting.
6. THE publisher SHALL use QoS 1, set a message expiry interval on every message (default 3600 s, remaining lifetime when replayed), and SHALL drop expired messages from the outbox instead of sending them (counted).
7. EVERY sample to be published SHALL first be stored in a bounded SQLite outbox (default 10 000 rows); a sender SHALL publish in insertion order and delete a row only after its PUBACK. WHEN the connection returns, unacknowledged and queued rows SHALL be replayed in order with their original `ts` and `seq`. WHEN the outbox is full, THE oldest rows SHALL be dropped and counted (a sequence gap is visible to consumers).
8. A message SHALL be lost only through the bounded outbox or message expiry, both counted and shown; a message MAY be delivered twice only when its PUBACK was lost at a disconnect, and consumers SHALL deduplicate on `(gateway, session, seq)`.
9. THE publisher SHALL publish per-tag at most once per second unless the quality changes (configurable); throttled samples are counted.
10. THE publisher SHALL NOT subscribe to any topic, and SHALL NOT accept, queue or act on any inbound application message. Commands SHALL never enter the outbox.
11. CREDENTIALS: the broker password SHALL be stored only in the OS secret store (Windows Credential Manager) or in memory; THE project files, logs, status, errors, exports and the outbox SHALL NOT contain it. The UI SHALL clear the password field after saving and SHALL never show it again.
12. THE system SHALL offer Test connection (connect, report the CONNACK result or the error, disconnect) using a distinct client ID so it never evicts a running publisher.
13. THE reconnect SHALL use exponential backoff (1 s up to 30 s) and SHALL never block the poll workers or the UI.

## Requirement 4: Share and MQTT panels

**User story:** As a user, I want to see what is shared and what is queued, so that I never think data left the device when it did not.

1. THE Share panel SHALL show: interface and port inputs (interfaces from the private list), Start/Stop sharing, the URL as text with a copy button, the QR code, the certificate fingerprint, a "browser will warn about the certificate" hint, connected WebSocket clients (address, since) and counters (rejected, rate-limited).
2. THE MQTT panel SHALL show: broker URL, site, gateway, client ID, username, password (write-only), optional CA PEM, allow-unencrypted tick, Test connection, Enable/Disable, state, outbox depth, published, dropped, expired, throttled, last acknowledged time and the last error text.
3. EVERY string SHALL exist in `en.json` and `id.json`; the panels SHALL work at 360 px and be keyboard operable; errors SHALL appear as text next to the field.
4. WHILE the LAN server or MQTT is on, THE app header SHALL show a visible "Sharing" / "MQTT on" indicator so it is never on unnoticed.

## Requirement 5: verification

1. THE test suite SHALL include the properties L1–L7 and M1–M7 of the design, the broker integration test (disconnect, outbox fills, replay without duplicates, `seq` order), and a headless-browser run of PRD journey 3 against the running app with screenshots in `docs/verification/`.
2. THE report SHALL state what was not verified (Android, real broker, real second device).
