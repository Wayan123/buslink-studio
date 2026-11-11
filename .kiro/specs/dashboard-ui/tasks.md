# dashboard-ui: tasks

Depends on app-shell-desktop tasks 1–5. After every UI task: `npx tsc --noEmit`, `node --test src/`, `node scripts/check-i18n.mjs`, `node scripts/check-ui.mjs`. After the alarm task: `cargo test --workspace`. A task is `[x]` only when its checks pass.

- [x] 1. Alarm engine (core)
  - `crates/buslink-core/src/alarm/mod.rs`; session integration (observe, tick, events to historian, snapshot in frames); IPC `alarm_list`, `alarm_add`, `alarm_remove`, `alarm_ack`, `alarm_shelve`, `alarm_unshelve`, `alarm_set_out_of_service`, `alarm_events`.
  - Tests: unit tests for every state transition; properties A1–A6 [300 each].
  - _Requirements: 6.1–6.9_

- [x] 2. UI foundation
  - `api.ts`, `types.ts`, `store.ts`, `dom.ts`, `quality.ts`, `i18n.ts`, `theme.ts`, `styles.css`, `main.ts` router and header (language, theme, nav, alarm banner, live regions).
  - Tests: U1–U5; `scripts/check-i18n.mjs`, `scripts/check-ui.mjs`.
  - _Requirements: 8.1–8.8_

- [x] 3. Connection center
  - `views/connect.ts`.
  - _Requirements: 1.1–1.5_

- [x] 4. Device library
  - `views/library.ts`, `components/badges.ts`.
  - _Requirements: 2.1–2.3_

- [x] 5. Live dashboard
  - `views/dashboard.ts`, `components/tile.ts`; worst-first sort, in-place tile updates, density, pause, export buttons.
  - _Requirements: 3.1–3.5, 9.1, 10.1_

- [x] 6. Trends
  - `views/trends.ts` with uPlot, gap bands, alarm bands, zoom, historian ranges.
  - _Requirements: 4.1–4.7_

- [x] 7. Register inspector
  - `views/inspector.ts`.
  - _Requirements: 5.1, 5.2_

- [x] 8. Alarms and training views
  - `views/alarms.ts`, `views/training.ts`, training alarm presets.
  - _Requirements: 6.7, 6.8, 7.1–7.3_

- [x] 9. Verification
  - `scripts/verify-journey.mjs`; run against the e2e build; screenshots and results in `docs/verification/`.
  - _Requirements: 8.7, 10.2; app-shell-desktop 10.1_
