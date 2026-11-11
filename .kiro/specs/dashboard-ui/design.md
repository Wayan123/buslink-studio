# dashboard-ui: design

Implements `requirements.md`. The backend is the session service from `app-shell-desktop`.

## 1. Technology choices

- **Plain TypeScript + DOM**, no framework (tech.md). One small `h()` helper builds elements with `textContent` only; there is no `innerHTML` anywhere in `src/` (checked by `scripts/check-ui.mjs`).
- **Charts: uPlot 1.6.32 (MIT).** research/02 §7 compares uPlot, ECharts, Chart.js, Plotly and lightweight-charts and recommends uPlot: ~20 KB for the Android WebView, canvas speed for 100k+ points, native `null` gaps (`spanGaps` off), real timestamps. ECharts has built-in min-max sampling but costs ~330 KB; lightweight-charts spaces points by index (breaks irregular polling); LTTB (Chart.js, ECharts) can drop spikes. Min-max decimation is done in Rust (`trend.rs`), so uPlot's lack of built-in decimation does not matter. Decision: uPlot.
- **Build:** Vite 8.3.8, TypeScript 5.9.3 (`erasableSyntaxOnly`, so `node --test` can run the pure modules with type stripping), `@tauri-apps/api` 2.12.4.

## 2. Module layout

```
src/
  main.ts            boot: i18n, theme, router, store, event subscription
  api.ts             typed invoke() wrappers + event listener; the only file that calls Tauri
  types.ts           wire types mirrored from Rust (TagView, LinkView, AlarmView, TrendSeries, ...)
  store.ts           live state: tags, links, alarms, trend buffers (pure, testable)
  dom.ts             h(), clear(), formatting helpers (locale numbers, ages, local/UTC times)
  quality.ts         quality → badge text, class, OPC UA label, exception hints (pure)
  i18n.ts            t(key, vars), language detect/persist, <html lang>
  theme.ts           system / light / dark
  views/
    connect.ts       connection center
    library.ts       device library + add device
    dashboard.ts     L1 overview + L2 device tiles (tiles updated in place)
    trends.ts        uPlot charts, gap bands, alarm bands, zoom
    inspector.ts     register inspector + frames + counters
    alarms.ts        banner, process/comm lists, actions, add alarm, log
    training.ts      exercises + fault buttons
  components/
    tile.ts          one tag tile with analog bar
    badges.ts        quality badge, alarm priority shape, support-level badge
  styles.css         tokens (ui.md palette), layout, components
  i18n/en.json, i18n/id.json
  *.test.ts          node --test unit tests for store, quality, trend helpers, i18n parity
```

Router: hash routes `#/connect`, `#/library`, `#/dashboard`, `#/device/<id>`, `#/trends`, `#/inspector/<id>`, `#/alarms`, `#/training` mapped onto the four display levels (L1 overview = dashboard + alarm banner; L2 device; L3 trends; L4 inspector + connection diagnostics).

## 3. Live data flow

1. On start `live_snapshot` fills the store; then `buslink://frame` events (≤ 4 Hz from Rust) are applied: changed `TagView`s replace entries, `points` append to per-tag ring buffers (30 min, max 7 200 points per tag), `dropped > 0` shows a visible "N points not delivered" notice.
2. Views subscribe to the store; the dashboard updates only the tiles whose tag changed (Req 10.1). Ages tick once per second from `value_ts_ms`.
3. Pause: the store keeps receiving; views stop rendering and the "Paused: values are not live" banner shows (Req 3.5).

## 4. Tile (ISA-101)

```
┌ Temperature ───────────── [TIMEOUT] ┐
│ 25,3 °C  (dimmed + struck when bad)  │
│ ▕░░░░▓▓▓▓▓▓▓▓▓░░░│░▏ normal band, limit ticks, value marker │
│ age 12 s · cycle 1,0 s               │
└──────────────────────────────────────┘
```

- Range for the bar: the register's `valid_range`, else the alarm limits; no bar when neither exists.
- Normal band: between the LO and HI limits when defined; ticks for every limit.
- Quality: GOOD → no badge, grey age; STALE → greyed value, "STALE · 12 s", hatched edge; TIMEOUT/CRC/EXC/DECODE → magenta badge, value struck through; DISCONNECTED → device card greyed with "DISCONNECTED".
- Active process alarm → priority shape (triangle 1 / diamond 2 / square 3) + border in the priority color.

## 5. Trends

- One uPlot per unit pair. Data from the store buffer for "Live 5 min / 15 min / 30 min"; "1 h / 8 h / session" calls `trend_query(device, tag, from, to, buckets = plot width px)`.
- Store → series: every point with `v = null` stays `null` (uPlot breaks the line); a gap interval is computed for each run of null points (reason = quality) and for spaces longer than the stale threshold (`NO_DATA`). Rust supplies gaps for historian ranges.
- Plugins: `gapBands` (hatched canvas pattern in `drawClear`, label text when wide enough), `alarmBands` (horizontal bands at alarm limits), tooltip in the legend (local time + UTC + quality).
- Zoom: uPlot drag-x; "Reset zoom" button; keyboard: the chart has a labeled "Reset zoom" button and range buttons (canvas itself is not focusable; the same data is available in the inspector table).

## 6. Alarm engine (`crates/buslink-core/src/alarm/mod.rs`)

```rust
AlarmDef { id, device_id, tag_id, kind: Hi|HiHi|Lo|LoLo, limit, deadband, on_delay_ms, off_delay_ms,
           priority: 1..=3, latch, message }
Alarm    { def | comm(device), class: Process|Comm, active, acked, pending_on_since, pending_off_since,
           shelved_until, out_of_service, held (quality not GOOD), activations: VecDeque<ts> }
```

Displayed state, in order: OutOfService → Shelved → Unacked (active ∧ ¬acked, or latched ∧ ¬acked) → Acked (active ∧ acked) → RtnUnacked (¬active ∧ ¬acked, not latched) → Suppressed (held ∧ ¬active) → Normal.

- `observe(sample, now)`: GOOD value → evaluate the raw condition with deadband (HI/HIHI: on `v ≥ limit`, off `v < limit − deadband`; LO/LOLO mirrored); start or cancel the pending timers. Non-GOOD → `held = true`, timers cancelled, state unchanged. Comm alarm per device: active while any tag of the device has a bad quality (no delay), message = worst quality.
- `tick(now)`: completes on/off delays, expires shelves (logged `shelve_expired`).
- `ack`, `shelve(until)`, `unshelve`, `set_out_of_service`: return errors for unknown IDs; each logs an event.
- Flood: activations in the last 10 min > 10. Chatter: ≥ 3 activations of one alarm in 60 s.
- `default_deadband(unit, span)` and `default_delay_ms(unit)` from research/02.

### Correctness properties (`crates/buslink-core/tests/alarm_props.rs`)

- **A1 delay:** a condition that holds for less than the on-delay never activates [300].
- **A2 deadband:** after activation, values that stay within `limit − deadband ≤ v` (HI) never clear [300].
- **A3 hold:** while a tag's quality is bad, its process alarm never changes `active` [300].
- **A4 latch:** a latched alarm never reaches Normal without an acknowledge after its last activation [300].
- **A5 shelve:** a shelved alarm is never counted as unacknowledged in the banner until its shelve ends; after expiry its state equals the unshelved twin's state [300].
- **A6 events:** every state change returned by the engine has exactly one event.

## 7. Accessibility and i18n

- Tokens from `ui.md`; light by default, dark through `prefers-color-scheme` or the theme switch; both sets meet the ratios listed there.
- Focus ring `outline: 3px solid var(--focus)` with offset; `scroll-padding-top` keeps focus out from under the sticky header.
- Live region `#announcer` (polite) for alarm and link changes; `#announcer-urgent` (assertive) for P1.
- `@media (prefers-reduced-motion: reduce)` disables all transitions; there are no animations by default.
- Layout: CSS grid with `minmax(min(100%, 16rem), 1fr)` tiles; header nav wraps; tables scroll inside their own container (`overflow-x: auto`) so the page does not.
- `scripts/check-i18n.mjs`: same keys in both files, no empty strings, every `t("...")` key used in `src/` exists. `scripts/check-ui.mjs`: no `innerHTML`/`outerHTML`/`insertAdjacentHTML`/`document.write` in `src/`.

## 8. UI correctness properties (`src/*.test.ts`, `node --test`)

- **U1:** store → uPlot series puts `null` at every non-GOOD point and never a number.
- **U2:** gap intervals from the store cover every null run and every interval longer than the stale threshold.
- **U3:** quality badge text exists for every quality code and never relies on color (each has a label and a CSS cue class).
- **U4:** `en.json` and `id.json` have identical key sets.
- **U5:** Indonesian number formatting uses a decimal comma; export paths never format numbers in the UI.

## 9. Verification (headless journey)

Tauri IPC exists only inside the app webview, so a page served by `npm run dev` in plain Chrome has no backend. The journey therefore runs against the **built app**, whose WebView2 is Chromium:

1. Build an e2e binary: `cargo.exe build --release --features e2e --target-dir src-tauri/target/e2e`. The `e2e` feature (off in the installer build) makes `export_session` write into the directory named by the host environment variable `BUSLINK_E2E_EXPORT_DIR` instead of opening the native save dialog, which CDP cannot drive. The path still never comes from the webview.
2. Start it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`.
3. `scripts/verify-journey.mjs` (puppeteer-core, Windows node) connects over CDP and clicks through: Training mode → 3 devices GOOD → inject a timeout on unit 3 → the trend shows a gap band "No data: TIMEOUT" → export CSV → check the CSV has TIMEOUT rows. It also sets the viewport to 360 px for a layout check and saves screenshots to `docs/verification/`.
4. Static checks of `dist/` (no backend: shows the "backend not available" state) run in headless Windows Chrome at 360 px.
