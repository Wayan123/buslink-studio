# lan-mqtt: design

## 1. Module layout

| Path | Contents |
|---|---|
| `crates/buslink-core/src/lan/mod.rs` | `LanServer` (start/stop/status), `LanConfig`, `LanStatus`, `LanSource` and `AssetSource` traits. |
| `lan/net.rs` | `private_interfaces()` (if-addrs), `check_bind(ip)`, QR matrix. |
| `lan/token.rs` | Token generation (ring `SystemRandom`, 16 bytes → 32 hex), `ct_eq`. |
| `lan/tls.rs` | Self-signed certificate (rcgen, ECDSA P-256, 30 days, IP SAN), rustls `ServerConfig`, SHA-256 fingerprint. |
| `lan/limits.rs` | `Limiter`: per-address token bucket and failure lockout; pure, time passed in. |
| `lan/hub.rs` | Subscribes to the sample bus, keeps a `LiveStore`, makes ≤ 4 Hz frames, offers `snapshot_and_subscribe()`. |
| `lan/routes.rs` | The route table (`ROUTES`), axum router, auth/limit middleware, handlers, WebSocket. |
| `lan/serve.rs` | Accept loop: TLS accept, hyper HTTP/1 with upgrades, connection semaphore, shutdown. |
| `crates/buslink-core/src/mqtt/` | `config.rs` (broker URL, validation), `topic.rs`, `payload.rs`, `outbox.rs` (SQLite), `tls.rs`, `service.rs` (feeder thread + async sender), `secrets.rs` (`SecretStore`, `Secret`, memory store). |
| `crates/buslink-core/src/session/live.rs` | `LiveStore::seed` (hub starts with the current tags). |
| `crates/buslink-core/tests/lan_*.rs`, `mqtt_*.rs`, `tests/common/broker.rs` | Integration tests; the in-process MQTT 5 broker. |
| `src-tauri/src/share.rs` | IPC for LAN and MQTT (`lan_*`, `mqtt_*`), `AssetSource` over the Tauri asset resolver, `SecretStore` (Windows Credential Manager), config files. |
| `src/views/share.ts`, `src/views/mqtt.ts`, `src/lan.ts` | Panels; LAN-mode transport (fetch + WebSocket) for the browser build. |
| `scripts/verify-lan.mjs` | Headless browser journey 3. |

The core keeps no Tauri dependency: the server takes a tokio `Handle` implicitly (it is `async fn start`), a `LanSource` (snapshot, bus, trend, profiles) and an `AssetSource` (static files). `Session` implements `LanSource`.

## 2. LAN server

```
accept → permit (≤ 32) → TLS (5 s) → hyper http1 (+upgrades, header timeout 10 s)
       → Extension(ClientAddr) → middleware: limiter.check → no-body check → token check → handler
```

- **Route table.** `ROUTES: [(&str, Kind)]` is the single source for the router; every entry is `GET`. The router has a `fallback` that is also behind the middleware, so an unknown path without a token is 401 (paths are not enumerable), with a token 404. Non-GET with a valid token: 405.
- **Token.** Query parameter `t` or cookie `bl_t`. `GET /` with a valid query token answers with `Set-Cookie: bl_t=<token>; Secure; HttpOnly; SameSite=Strict; Path=/`. `ct_eq` compares the full length of both strings with XOR accumulation and no early exit. The token never appears in logs.
- **Limiter.** Bucket capacity 60, refill 30/s per address; 5 auth failures in 60 s → 60 s lockout (429 + `Retry-After`). Table capped at 1024 addresses (oldest evicted). Time is a parameter (`now_ms`) so the unit tests are deterministic.
- **Static files.** `AssetSource::get(path)`; `/` → `index.html` with `<meta name="buslink-mode" content="lan">` inserted after `<head>`. Paths containing `..`, `\\` or NUL are 404. No directory listing, no SPA fallback.
- **Snapshot.** `{ …LiveSnapshot, profiles: [{key, profile}] }` from the hub's consistent cut.
- **Trend.** `tag=<device_id>/<tag_id>` (the `TagView.key`), `from`, `to` epoch ms, `buckets` default 600. Runs on `spawn_blocking`; `Session::trend` already decimates min/max per bucket and marks gaps.
- **Stream.** `{"type":"snapshot","data":…}` first, then `{"type":"frame","data":LiveFrame}`; on `Lagged` → `{"type":"lagged","missed":n}` followed by a fresh snapshot. Client ping/pong handled by axum; a close or error removes the client from the table. A 5 s send timeout drops slow clients.
- **Hub.** One std thread per server: drain bus → skip samples older than the tag's seeded `seq` → `LiveStore::apply`; every 250 ms `take_changes` → one frame JSON (`Arc<str>`) → `broadcast`; every 1 s attach links and alarms from `source.snapshot()`. `snapshot_and_subscribe` takes the hub lock, subscribes to the broadcast, builds the snapshot from the store: no frame can be missed or arrive older than the snapshot.
- **Stop.** A `watch` flag ends the accept loop; dropping its `JoinSet` aborts every connection task (sockets closed). `stop()` awaits the accept task, so the port is free when it returns. Token and certificate live inside the server value and die with it.
- **Bind check.** `check_bind(ip, port)`: IPv4, private (RFC 1918) or loopback (tests and the e2e build only; never listed), never unspecified, public, link-local, multicast. Port 1024–65535 (default 8443).
- **Android.** Same code; the address comes from the same `private_interfaces()` (getifaddrs). `INTERNET` is already declared; targetSdk 36 means no local-network runtime permission. The Android service rule (server shown only while the foreground service runs) is not enforced here; see "Not verified" in the run report.

## 3. Read-only frontend

`src/lan.ts` exports `isLan()` (meta tag), `lanSnapshot()`, `lanTrend()`, `lanStream(onFrame, onState)` (WebSocket with reconnect and backoff, token from `location.search`). `api.ts` keeps its Tauri `invoke` functions; in LAN mode `api.snapshot`/`api.trend` use `lan.ts` and every other function throws `read-only view`. `main.ts` in LAN mode: nav = overview + trends, no disarm hook, no profile IPC (profiles come from the snapshot), a persistent "Read-only view" notice, and `body.lan`. The dashboard hides export buttons and "connect" links when `isLan()`. `scripts/check-ui.mjs` is unchanged (no HTML-string sinks); a unit test asserts the LAN nav has no write view.

## 4. MQTT service

```
bus ─► feeder thread ─ throttle ─ topic+payload ─► outbox (SQLite) ─ notify ─► async sender ─► broker
                                                        ▲ delete on PUBACK        │ reconnect: new client, replay from lowest id
```

- **Config** (`MqttConfig`, serde, saved by the shell as `mqtt.json` in the app data dir; **no password field**): `broker_url`, `site`, `gateway`, `client_id`, `username`, `ca_pem`, `allow_insecure`, `expiry_s` (3600), `keep_alive_s` (30), `outbox_max` (10 000), `min_interval_ms` (1000). `broker_url` accepts `mqtts://host[:8883]`, `ssl://`, `tls://`, and `mqtt://host[:1883]` only with `allow_insecure`; user-info in the URL is rejected (credentials have their own fields).
- **Topic.** `sanitize_level`: map every char outside `[A-Za-z0-9._-]` to `_`, trim, empty → `_`, cap 64 chars. Device = `device_id`, tag = `tag_id`.
- **Payload** (`serde_json`): `{ts, seq, value, unit, quality, age_ms, exception?, profile_version, session}`. `value` = engineering value if finite, else text, else `null`.
- **Outbox** (`outbox.sqlite3`, WAL): `outbox(id INTEGER PRIMARY KEY AUTOINCREMENT, topic, payload BLOB, enq_ms, sample_seq, session)`. `push`, `peek_after(id, n)`, `delete(id)`, `depth()`, overflow → delete oldest, counted. The data model in architecture lists `tries`; replay is bounded by expiry instead, so the column is omitted.
- **Sender.** For each connection attempt a fresh rumqttc v5 `AsyncClient`/`EventLoop` is built (clean start, no session state, no rumqttc-internal resend), so replay is driven only by the outbox and ordering is the outbox order. Window of 10 in flight. `Outgoing::Publish(pkid)` events, which arrive in publish order, map the pkid to the outbox id; `PubAck(pkid)` deletes the row. Status messages ride the same queue with a sentinel entry.
- **Duplicates.** At a disconnect, rows without PUBACK stay in the outbox and are sent again (at-least-once). The payload carries `(session, seq)` so consumers deduplicate; a test with a withheld PUBACK shows the one possible duplicate and asserts order.
- **Backoff.** 1, 2, 4 … 30 s, reset on CONNACK.
- **TLS.** `mqtt/tls.rs`: rustls `ClientConfig` with the ring provider; roots = platform (see below) + optional PEM CA. Windows/Linux/macOS: `rustls-native-certs`. Android: `rustls-platform-verifier` (needs its Kotlin component and the Gradle snippet, see `docs/android-build.md`; not built here).
- **Secrets.** `Secret(String)` has a redacting `Debug` and no `Serialize`. `SecretStore` trait (`set/get/delete`); `MemorySecretStore` in core. The shell uses Windows Credential Manager (keyring) on Windows and memory only elsewhere.
- **Status** (`MqttStatus`): state (`off`, `connecting`, `online`, `backoff`), counters, `outbox_depth`, `last_error`, `last_ack_ms`. Errors are plain text and never include the password.
- **Test connection**: separate client ID `<id>-test-<4 hex>`, 8 s timeout, returns `{ok, detail, elapsed_ms}`.

## 5. Shell (`src-tauri/src/share.rs`)

`LanState`/`MqttState` hold `Option<server>` behind a `tokio::sync::Mutex`. Commands: `lan_interfaces`, `lan_start{ip,port}`, `lan_stop`, `lan_status`, `mqtt_get_config`, `mqtt_save_config`, `mqtt_set_password`, `mqtt_clear_password`, `mqtt_test`, `mqtt_start`, `mqtt_stop`, `mqtt_status`. `lan_start` accepts only an address from `private_interfaces()` (plus loopback in the `e2e` feature). Sharing and MQTT are stopped on app exit. No command carries a frame.

## 6. Correctness properties

- **L1** (`tests/lan_server.rs`): every route × method × with/without/wrong token, plus unknown paths: no token or wrong token → 401 (or 429 after lockout) and the handler never runs; with the token only the table's GET routes answer 2xx.
- **L2** (unit, 500 cases): `ct_eq(a, b) == (a == b)` for random byte strings including different lengths and prefixes.
- **L3** (`lan_server.rs`): `ROUTES` has only GET; POST/PUT/PATCH/DELETE/OPTIONS on every path with a valid token never return 2xx; a request with a body is 400.
- **L4** (unit, 300 cases): over random request schedules the limiter never admits more than `capacity + refill × elapsed` per address, a locked address stays blocked until expiry, memory stays bounded.
- **L5** (`lan_server.rs`): a WebSocket client receives, for every sample published on the bus, a tag view with the same `seq`, value, unit and quality (and a trend point with the same time/quality); the first message is a snapshot and later frames never regress a tag's `seq`.
- **L6** (`lan_server.rs`): after `stop()` the port refuses connections and open sockets are closed; a restart gives a different token and certificate fingerprint and the old URL answers 401; client limit gives 503.
- **L7** (unit, 500 cases): `check_bind` accepts exactly RFC 1918 (and loopback when allowed), never unspecified, public, link-local, multicast.
- **M1** (unit, 500 cases): any string → topic level without forbidden characters, non-empty, ≤ 64.
- **M2** (unit): payload has all required keys; a TIMEOUT sample carries the last good value and its age, never 0 or a fresh `ts`.
- **M3** (`tests/mqtt_outbox.rs`, 300 cases): outbox returns rows in insertion order; depth ≤ max; overflow drops the oldest and counts; reopen keeps the rows.
- **M4** (`tests/mqtt_broker.rs`): broker up → N samples arrive in `seq` order; broker connection cut → outbox fills (depth = M); reconnect → the M samples arrive after the N, in order, original `ts`/`seq`, no duplicates; outbox empty at the end.
- **M5** (`mqtt_broker.rs`): the broker saw CONNECT with a will (topic `…/status`, payload `offline`, retained, QoS 1), a retained `online` publish, and **no SUBSCRIBE** in the whole run; a clean stop publishes `offline`.
- **M6** (unit + `mqtt_broker.rs`): `Debug` of `Secret`, `MqttStatus` JSON, `MqttConfig` JSON and every error string never contain the password; the password reaches only the CONNECT packet.
- **M7** (unit): `mqtt://` without `allow_insecure` is refused; user-info in the URL is refused.
- **V1**: `scripts/verify-lan.mjs`: PRD journey 3 against the running e2e app with a second headless Chrome as the "LAN browser".

## 7. Risks and honest limits

- Android: the Kotlin Keystore secret store and the platform-verifier Gradle component are not built in this run; on Android the password is kept in memory for the run and the MQTT panel says so.
- The browser warns about the self-signed certificate; the user checks the fingerprint shown in the Share panel.
- Only WebSocket viewers count as "connected clients".
- A phone that sleeps stops the server unless the foreground service runs (E6 owns that service).
