# 02 · Standards and dashboard design for BusLink Studio

Written 2026-11-12. Two research agents read the sources listed at the end (transcript: `history://StandardsAndDashboardDesign`) but failed before writing. This brief condenses those sources and standard practice.
- Contrast ratios are **computed** with the WCAG 2.x relative-luminance formula.
- Figures from paywalled standards come from public previews or vendor/ISA summaries, and are marked as such.

---

## 1. High-performance HMI: ANSI/ISA-101.01-2015 and ASM Consortium

**What the standard and guidance say:**
- ISA-101 covers the HMI life cycle: philosophy, style guide, display hierarchy, and use of color. Its "high-performance" approach, popularised by the ASM Consortium and ISA's overview paper, has these traits:
  - **Grey, low-saturation background.** The screen is quiet when everything is normal.
  - **Color reserved for abnormal states.** Red and orange mean alarms; they are never decoration, and color is never the only cue.
  - **Analog indicators** show the value against its normal operating range and its alarm limits (a moving bar with shaded bands), instead of bare numbers.
  - **A display hierarchy:**
    - Level 1: overview, KPIs and trends.
    - Level 2: unit control.
    - Level 3: detail / equipment.
    - Level 4: diagnostics.
  - **Trends embedded** where a decision is made. Operators judge direction and rate of change faster from a sparkline than from a number.
- Rockwell, Honeywell/ASM and Inductive Automation publish similar rules ("Design like a pro"). [INFERENCE] There is no conflict between those sources.

**Palette.** The ratios below are computed (WCAG 2.2: text needs 4.5:1, non-text and large text need 3:1).

| Use | Foreground on background | Ratio | OK for |
|---|---|---|---|
| Base text (light) | `#1F1F1F` on `#D9D9D9` | 11.68:1 | all text |
| Secondary text (light) | `#4D4D4D` on `#D9D9D9` | 5.99:1 | all text |
| Panel text (light) | `#1F1F1F` on `#ECECEC` | 13.95:1 | all text |
| Normal trend line | `#6B6B6B` on `#D9D9D9` | 3.78:1 | non-text (lines) |
| **P1 alarm** fill, white text | `#FFFFFF` on `#CC3311` | 5.19:1 | text in badge |
| P1 alarm border/icon on base | `#CC3311` on `#D9D9D9` | 3.67:1 | non-text only |
| **P2 alarm** fill, black text | `#000000` on `#EE7733` | 7.32:1 | text in badge |
| **P3 alarm** fill, black text | `#000000` on `#F0E442` | 15.88:1 | text in badge |
| Selected / info fill, white text | `#FFFFFF` on `#0077BB` | 4.82:1 | text in badge |
| Info icon on base | `#0077BB` on `#D9D9D9` | 3.42:1 | non-text only |
| **Bad quality** fill, white text | `#FFFFFF` on `#AA3377` | 6.09:1 | text in badge |
| Bad quality icon on base | `#AA3377` on `#D9D9D9` | 4.31:1 | non-text / large text |
| Base text (dark) | `#E6E6E6` on `#2B2B2B` | 11.34:1 | all text |
| Secondary (dark) | `#B3B3B3` on `#2B2B2B` | 6.75:1 | all text |
| P1 alarm text (dark) | `#EE6677` on `#2B2B2B` | 4.59:1 | text |
| P3 alarm text (dark) | `#CCBB44` on `#2B2B2B` | 7.25:1 | text |
| Info text (dark) | `#66CCEE` on `#2B2B2B` | 7.71:1 | text |
| Bad quality text (dark) | `#CC79A7` on `#2B2B2B` | 4.63:1 | text |

The hues come from Paul Tol's colour-blind-safe schemes (`#CC3311`, `#EE7733`, `#0077BB`, `#EE3377`/`#AA3377`) and Okabe–Ito (`#F0E442`, `#CC79A7`).

**BusLink rules**
- Default theme is light ISA grey `#D9D9D9`, with text `#1F1F1F`. Dark mode is optional (`#2B2B2B`).
- Normal values render in neutral greys. Only abnormal states (alarm, bad quality, stale) get hue.
- Every live tile has: tag name, value, unit, a quality text label, data age, and a horizontal analog bar with a shaded normal band and alarm limit ticks. A sparkline of the last 5 minutes is optional.
- Alarm priority is shown by **shape + number + color**: P1 red triangle "1", P2 orange diamond "2", P3 yellow square "3". The shape and number carry the meaning; color is redundant.
- **Navigation follows the four display levels:**
  - L1: Overview (all devices, KPIs, active alarms).
  - L2: Device.
  - L3: Tag detail and trend.
  - L4: Register inspector and link diagnostics.
- No 3D gauges, no animated decoration, no gradients on data elements.

---

## 2. Alarm management: ANSI/ISA-18.2-2016 / IEC 62682:2022 / EEMUA 191 (4th ed.)

**Alarm states.** The ISA-18.2 state model, as summarised by ICONICS and the IEC 62682 preview:
- Normal
- **Unacknowledged** (active, not acked)
- **Acknowledged** (active, acked)
- **Return-to-normal unacknowledged** (cleared but not acked)
- **Shelved** (manually suppressed by the operator, with an expiry)
- **Suppressed by design** (by logic or state)
- **Out of service** (maintenance)

Latching alarms stay active until acknowledged.

**Performance targets.** Figures are from ISA and EEMUA summaries (ISA ie KPI paper, ProcessVue, Siemens white paper):
- Average alarm rate per operator: about 1 per 10 minutes is "very likely acceptable"; about 2 per 10 minutes is "maximum manageable".
- **Flood:** more than 10 alarms per 10 minutes.
- Priority distribution: about **80 % low / 15 % medium / 5 % high**.
- Standing alarms: under 5. Chattering alarms (≥ 3 in 1 minute) are a defect to fix.

**Deadband and delay starting points** (ISA-18.2 practice tables, cited in vendor guides):

| Process variable | Deadband | On/off delay |
|---|---|---|
| Flow, level | 5 % of range | 15 s |
| Pressure | 2 % | 15 s |
| Temperature | 1 % | 60 s |

**BusLink rules**
- **Alarm definition:** tag, type (HI/HIHI/LO/LOLO/ROC/quality), limit, deadband, on-delay, off-delay, priority (1–3), message, latch flag.
- **Engine:** implements the seven states above, including shelve-with-expiry and out-of-service. Alarms stay active while acknowledged.
- **Communication alarms** (timeout, CRC, disconnected) are a separate class from **process alarms**, so a dead link does not raise a flood of process alarms.
- **Alarm banner:**
  - Shows the count per priority and the oldest unacknowledged alarm.
  - The alarm list is sortable and filterable.
  - Acknowledge and shelve are logged events.
- A **flood indicator** appears when more than 10 alarms occur in 10 minutes. A chattering detector flags any alarm that fires 3 or more times in 1 minute.
- Default deadbands follow the table above, by unit type in the profile.

---

## 3. Data quality, timestamps and historians

- **OPC UA StatusCode** (Part 4 §7.38): the top two bits give the severity, Good `00`, Uncertain `01`, Bad `10`. Sub-codes include `Uncertain_LastUsableValue`, `Bad_NoCommunication`, `Bad_CommunicationError`, `Bad_Timeout`, `Bad_DecodingError` and `Bad_OutOfService`. The full list is in OPCFoundation `StatusCode.csv`.
- **Timestamps:** a DataValue carries a **SourceTimestamp** (when the value was measured or sampled at the source) and a **ServerTimestamp** (when the server received it). For Modbus, the source timestamp is when the response arrived, because devices carry no time.
- **Ignition** shows quality overlays on bad or stale tags (`Bad_Stale`, `Uncertain_LastKnownValue`, `Bad_NotConnected`).
- **PI** stores data with exception filtering plus swinging-door compression, and records digital states such as "I/O Timeout" or "Comm Fail" so trends show gaps.
- **Grafana and InfluxDB** render nulls as **gaps** unless "connect null values" is enabled. `aggregateWindow(createEmpty: true)` keeps empty windows as nulls.

**Quality mapping:**

| BusLink quality | OPC UA equivalent | UI treatment | Trend |
|---|---|---|---|
| GOOD | Good | neutral; age shown in grey | line |
| STALE (last good older than 3× the poll interval) | Uncertain_LastUsableValue | value greyed + "STALE · 12 s" + hatched tile edge | gap after the last good point |
| TIMEOUT | Bad_Timeout | magenta "TIMEOUT" badge; keep the last value struck through | gap |
| CRC_ERROR | Bad_CommunicationError | magenta "CRC" badge | gap |
| EXCEPTION(n) | Bad_DeviceFailure / Bad_NotSupported (by code) | magenta "EXC 02 Illegal address" | gap |
| DECODE_ERROR | Bad_DecodingError | magenta "DECODE" | gap |
| DISCONNECTED | Bad_NoCommunication | whole device card greyed + "DISCONNECTED" | gap |

**BusLink rules**
- Every sample stores both timestamps (`ts_src` = response time, `ts_rx` = time stored) plus quality and `age_ms`.
- Trends **never interpolate across non-GOOD samples**: draw `null` and show the gap. The tooltip on a gap states the reason.
- Exports include `quality`, `quality_opcua` (numeric code) and `age_ms` columns.
- Logging can apply a configurable deadband per tag (a value is recorded when |Δ| > deadband or the heartbeat interval passes). Raw polling still feeds the live view.

---

## 4. MQTT 5 and Sparkplug B 3.0 (ISO/IEC 20237:2023)

- **MQTT 5** (OASIS) adds message expiry, session expiry, will delay, topic aliases and reason codes. These help for intermittent links such as phones.
- **Sparkplug B 3.0:**
  - **Namespace:** `spBv1.0/{group_id}/{NBIRTH|NDEATH|NDATA|NCMD|DBIRTH|DDEATH|DDATA|DCMD}/{edge_node_id}[/{device_id}]`, plus `spBv1.0/STATE/{host_id}`.
  - **Payload:** protobuf metrics with aliases.
  - **Sequencing:** `seq` cycles 0–255 per edge node, and `bdSeq` ties BIRTH to DEATH.
  - **Store-and-forward:** uses a "historical" flag.
  - Published as ISO/IEC 20237:2023.
- **Fit for a phone or laptop gateway.** Sparkplug gives state awareness: the broker's will message publishes DEATH when the gateway drops. The costs are protobuf, a BIRTH/rebirth flow, and alias management. For a hackathon MVP with ad-hoc brokers, plain JSON topics are easier to test with any MQTT client. [INFERENCE]

**BusLink rules**
- **MVP** publishes MQTT 5 plain JSON on `buslink/{site}/{gateway}/{device}/{tag}`, with payload `{ts, seq, value, unit, quality, profile_version}`. The gateway has a retained `buslink/{site}/{gateway}/status` = `online|offline`, and the will message sets `offline`.
- Messages with expiry use QoS 1. The offline outbox is bounded and replayed with the original `ts` and `seq`; consumers deduplicate on `(gateway, seq)`.
- **No commands over MQTT** in the competition build (telemetry only).
- **Sparkplug B is post-MVP:** generate NBIRTH/DBIRTH from the profile and send DDATA by alias.

---

## 5. Security: IEC 62443 and Modbus/TCP Security

- **IEC 62443 security levels:**
  - SL1: casual or coincidental violation.
  - SL2: intentional, simple means.
  - SL3: sophisticated means.
  - SL4: extended resources.
- **Foundational requirements:**
  - FR1: identification and authentication.
  - FR2: use control.
  - FR3: system integrity.
  - FR4: data confidentiality.
  - FR5: restricted data flow.
  - FR6: timely response to events.
  - FR7: resource availability.
- **Relevant 62443-4-2 component requirements:**
  - CR 1.1: human user identification and authentication.
  - CR 2.1: authorization enforcement.
  - CR 2.8: auditable events.
  - CR 3.1: communication integrity.
  - CR 4.1: information confidentiality.
- **Zones and conduits:** the tool sits on the engineering side, and the RS485 bus is its own zone.
- **Modbus/TCP Security** (Modbus.org): TLS 1.2+ on **port 802**, mutual X.509 authentication, and an X.509 v3 **role extension** carrying a role used for authorization. Legacy Modbus TCP (port 502) has no authentication.

**BusLink rules**
- **Read-only by default (FR2).** Writes need explicit arming per device, with a session timeout, and go through the command service only.
- An audit record for every arm, write, read-back, acknowledge and shelve (CR 2.8). Each record carries the user/session, device, profile version, and request/response hex.
- **LAN dashboard (FR4/FR5):**
  - Off by default; binds to one chosen interface.
  - Uses TLS (self-signed per session) plus a random token.
  - Read-only routes, no write endpoints; rate-limited.
- **Secrets** (MQTT passwords) go in the OS keychain or Android Keystore. They never appear in profiles, projects, exports or logs.
- **Profiles are data only.** They are validated against a JSON Schema, and expressions are limited to scale/offset; no scripts.
- Modbus/TCP Security (port 802) is a later transport option and is out of scope for the MVP. The docs must say so clearly.

---

## 6. Modbus protocol and serial line facts

**Application Protocol V1.1b3 limits:**
- FC01/02: 1–2000 bits.
- FC03/04: 1–125 registers.
- FC05: single coil, FF00/0000.
- FC06: single register.
- FC15: 1–1968 bits.
- FC16: 1–123 registers.
- FC23: read 125 / write 121.
- The PDU is at most 253 bytes; the RTU ADU at most 256 bytes.

**Exception codes:**

| Code | Meaning |
|---|---|
| 01 | Illegal function |
| 02 | Illegal data address |
| 03 | Illegal data value |
| 04 | Server device failure |
| 05 | Acknowledge |
| 06 | Server busy |
| 08 | Memory parity error |
| 0A | Gateway path unavailable |
| 0B | Gateway target failed to respond |

**Serial Line Guide V1.02:**
- RTU frames are separated by ≥ 3.5 character times, and a gap > 1.5 character times inside a frame is an error. Above 19200 baud, fixed values are recommended: **t1.5 = 750 µs, t3.5 = 1.75 ms**.
- Default 8E1 (19200 even parity is the spec default); 8N2 when there is no parity. 8N1 is common in practice but non-standard.
- Unit addresses 1–247; 0 is broadcast (writes only, no reply).
- **RS-485 physical layer** (Serial Line Guide + TI SLLA272):
  - 120 Ω termination at **both ends** of the trunk only.
  - Fail-safe biasing at one point.
  - Up to 32 unit loads per segment.
  - About 1200 m at low baud rates.
  - Daisy chain with short stubs.
  - Common/ground reference conductor.

**Word order** (Modbus Poll naming): `ABCD` (big-endian), `CDAB` (word-swapped), `BADC` (byte-swapped), `DCBA` (little-endian).

**BusLink rules**
- **Scheduler:** splits reads at the FC limits above.
- **RTU parser:** uses the expected length (from the function code and byte count), with inter-frame silence as a secondary check. It applies t3.5 = max(3.5 chars, 1.75 ms).
- **Unit ID 0:** writes to it are rejected in the UI and by the guard hook.
- **Exceptions:** shown as code + name + a likely-cause hint.
- **Inspector:** shows all four word orders side by side; the user chooses one, and it is saved in the profile.
- **Wiring help page:** termination at both ends, biasing, A/B naming differences between vendors, the separate sensor power supply, and the fact that the Waveshare (B) board is non-isolated.

---

## 7. Time-series charting library

Bundle sizes are minified + gzip from the bundlephobia API, read by the earlier agent. Treat them as approximate.

| Library | Size | Many points / streaming | Gaps | Decimation | Multi-axis | License |
|---|---|---|---|---|---|---|
| **uPlot** | ~20 KB | Very fast canvas; 100k+ points; `setData` append | `null` → gap (spanGaps off) | none built in (do min-max yourself) | yes | MIT |
| Apache ECharts 5/6 | ~330 KB+ (tree-shaken less) | Good; `large`, `progressive`, `appendData` | `connectNulls:false` | `sampling: 'lttb' \| 'minmax' \| 'average'` | yes | Apache-2.0 |
| Chart.js 4 | ~70 KB | Fair; slower at 100k | `spanGaps` | decimation plugin (LTTB, min-max) | yes | MIT |
| Plotly.js | ~1 MB+ | Heavy; WebGL traces help | `connectgaps` | none | yes | MIT |
| lightweight-charts | ~45 KB | Fast | whitespace data | none | limited price scales | Apache-2.0 + attribution |

**Notes on the options:**
- lightweight-charts spaces bars evenly by index, not proportionally to time (issues #1342, #847). That breaks irregular polling intervals.
- **Min-max/M4 decimation** (Jugel et al., VLDB 2014) keeps extreme values pixel-accurate. LTTB (Steinarsson 2013) looks smooth but can drop spikes. Spikes matter for alarms.

**BusLink rules**
- Use **uPlot** for trends. It is small for the Android WebView, fast, handles `null` gaps natively, and works with real timestamps.
- Decimate with **min-max per pixel column** on the Rust side for long ranges; it works with the historian's minute rollups. Never use LTTB for alarm-relevant data.
- **Gaps:** non-GOOD samples become `null`, and a shaded band marks the gap with its reason.
- **Overlays:** alarm limits are horizontal bands; acknowledge and write events are vertical markers.
- **Axes:** one y-axis per unit, at most 2 per chart; tags with other units go in additional charts.
- UI refresh is throttled to ≤ 4 Hz, independent of the polling rate.

---

## 8. Accessibility and colour vision

- **WCAG 2.2 AA requirements:**
  - **1.4.3** text contrast 4.5:1 (3:1 for large text, ≥ 18.66 px bold or ≥ 24 px).
  - **1.4.11** non-text contrast 3:1 for UI components and graphics.
  - **1.4.1** colour is not the only means of conveying information.
  - **2.5.8** target size at least 24×24 CSS px (new in 2.2).
  - **4.1.3** status messages announced without moving focus.
  - **2.2.2** moving or updating content that starts automatically and lasts more than 5 s needs pause/stop/hide unless it is essential.
  - **1.3.4** no orientation lock.
- **Colour vision deficiency:** about **8 % of men** and 0.5 % of women of Northern European ancestry have red–green deficiency (MedlinePlus/NEI). That is why red-versus-green status alone fails.

**BusLink rules**
- Use the palette in §1. Every status has a **text label** ("TIMEOUT", "P1") and a **shape**.
- **Live regions:** alarms and connection changes go to an `aria-live="polite"` region; P1 alarms use `assertive`.
- **Pause:** a "Pause live updates" control freezes the dashboard values for reading or screenshots (2.2.2).
- **Touch targets:** ≥ 24 px everywhere and 44 px for primary mobile actions (connect, arm, stop).
- **Keyboard:** all controls are reachable. Focus rings are visible (`#0077BB`, 3:1 against the base).
- **Motion and orientation:** respect `prefers-reduced-motion` (no pulsing alarms; use a static border instead). Portrait and landscape both work.

---

## Sources

1. ISA, *The High Performance HMI* overview v2.01: https://www.isa.org/getmedia/06130a38-f7af-4b35-8c9c-2c34f25c1977/The-High-Performance-HMI-Overview-v2-01.pdf
2. ANSI/ISA-101.01-2015 preview: https://webstore.ansi.org/preview-pages/ISA/preview_ANSI+ISA+101.01-2015.pdf; ISA-101 standards page: https://www.isa.org/standards-and-publications/isa-standards/isa-101-standards
3. Inductive Automation, *Design like a pro*: https://inductiveautomation.com/resources/article/design-like-a-pro-optimizing-your-hmi
4. Control Global, high-performance HMI rules: https://img.controlglobal.com/files/base/ebm/controlglobal/document/2022/08/1661879878912-cg1709highperformancehmirules…
5. IEC 62682:2022 preview: https://assets.vde-verlag.de/iec-normen/preview-pdf/info_iec62682%7Bed2.0.CMV%7Den.pdf
6. ICONICS alarm state transition diagram: https://documentation.iconics.com/v10.98/Content/Alarming/Alarm%20Server/Alarm%20References/alarm-state-transition-diag…
7. Alarm System Performance Metrics (ISA Ireland): https://isa.ie/wp-content/uploads/2016/06/Alarm_System_Performance_Metrics_Kim_Van_camp.pdf
8. ProcessVue, Alarm system performance KPIs: https://www.processvue.com/downloads/Alarm_system_performance_KPIs_V1_0.pdf
9. EEMUA 191 new edition: https://www.eemua.org/news/good-practice-for-all-aspects-of-industrial-alarm-systems-%E2%80%93-new-edition-of-eemua-191…
10. OPC UA Part 4 StatusCode: https://reference.opcfoundation.org/specs/OPC-10000-4/7.38.1; DataValue timestamps: https://reference.opcfoundation.org/specs/OPC-10000-4/7.11
11. OPC UA StatusCode list: https://raw.githubusercontent.com/OPCFoundation/UA-Nodeset/latest/Schema/StatusCode.csv
12. Ignition quality codes and overlays: https://www.docs.inductiveautomation.com/docs/8.3/platform/tags/quality-codes-and-overlays
13. Grafana time series (null values): https://grafana.com/docs/grafana/latest/visualizations/panels-visualizations/visualizations/time-series/
14. InfluxDB aggregateWindow: https://docs.influxdata.com/flux/v0/stdlib/universe/aggregatewindow/
15. Sparkplug 3.0.0 specification: https://sparkplug.eclipse.org/specification/version/3.0/documents/sparkplug-specification-3.0.0.pdf
16. MQTT 5.0 (OASIS): https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html
17. Modbus/TCP Security protocol: https://www.modbus.org/file/secure/modbussecurityprotocol.pdf
18. ISAGCA, IEC 62443 quick start: https://isagca.org/ (Quick Start Guide PDF); ISA 62443-3-2 preview: https://www.isa.org/getmedia/661c718f-8e64-446d-acf9-2c6286db1b33/isa-62443-3-2-preview.pdf
19. Modbus Application Protocol V1.1b3: https://www.modbus.org/file/secure/modbusprotocolspecification.pdf
20. Modbus over Serial Line V1.02: https://www.modbus.org/file/secure/modbusoverserial.pdf
21. TI SLLA272, RS-485 design guide: https://www.ti.com/lit/an/slla272d/slla272d.pdf
22. Modbus Poll user manual (data formats): https://www.modbustools.com/mbpoll-user-manual.html
23. uPlot: https://github.com/leeoniya/uPlot; missing-data demo: https://raw.githubusercontent.com/leeoniya/uPlot/master/demos/missing-data.html
24. ECharts line series sampling: https://raw.githubusercontent.com/apache/echarts-doc/master/en/option/series/line.md
25. Chart.js decimation: https://www.chartjs.org/docs/latest/configuration/decimation.html
26. lightweight-charts time scale and issues: https://tradingview.github.io/lightweight-charts/docs/time-scale, https://github.com/tradingview/lightweight-charts/issues/1342
27. bundlephobia API (sizes): https://bundlephobia.com/api/size?package=uplot (and echarts, chart.js, lightweight-charts)
28. Jugel et al., M4 (VLDB 2014): http://www.vldb.org/pvldb/vol7/p797-jugel.pdf
29. WCAG 2.2 Understanding (target size, non-text contrast, use of color, pause/stop/hide): https://www.w3.org/WAI/WCAG22/Understanding/
30. MedlinePlus, color vision deficiency: https://medlineplus.gov/genetics/condition/color-vision-deficiency/
31. Paul Tol colour schemes: https://personal.sron.nl/~pault/data/colourschemes.pdf; Okabe–Ito: https://jfly.uni-koeln.de/color/
32. Tauri WebView versions: https://v2.tauri.app/reference/webview-versions/
