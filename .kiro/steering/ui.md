---
inclusion: fileMatch
fileMatchPattern: "src/**"
---

# UI rules (webview + LAN dashboard)

Sources: `research/02-standards-and-dashboard-design.md` (ISA-101 / ASM high-performance HMI, ISA-18.2 / IEC 62682 / EEMUA 191, OPC UA quality, uPlot choice, WCAG 2.2 AA) and the owner's brief. Contrast ratios below are computed with the WCAG 2.x relative-luminance formula (research/02 §1).

## ISA-101 high-performance style

- **Grey, calm normal state.** Default theme is light ISA grey `#D9D9D9` with text `#1F1F1F`. No 3D gauges, gradients on data elements, photos or decorative animation.
- **Color means abnormal.** Normal values render in neutral greys. Only abnormal states (process alarm, bad quality, stale) get hue, and every colored state also has a text label and a shape. Never use green for "running" or "OK"; normal needs no color.
- **Every live tile** shows: tag name, value, unit, a quality text label, data age, and a horizontal analog bar with a shaded normal band and alarm-limit ticks. A 5-minute sparkline is optional.
- **Display hierarchy (four levels)**, also the navigation:
  - L1 Overview: all devices (worst state first), KPIs, active alarms.
  - L2 Device: its tags.
  - L3 Tag detail and trend.
  - L4 Register inspector and link diagnostics.
- Process alarms (value out of limits) and communication problems (quality) look different and are listed separately.

### Palette (project tokens)

| Token | Light | Dark | Use | Ratio (computed) |
|---|---|---|---|---|
| `--bg` | `#D9D9D9` | `#1F1F1F` | page background | |
| `--surface` | `#ECECEC` | `#2B2B2B` | tiles, tables | dark text colors are rated on `#2B2B2B`; a darker `--bg` only raises them |
| `--text` | `#1F1F1F` | `#E6E6E6` | values, labels | 11.68:1 on bg, 13.95:1 on surface (light); 11.34:1 (dark) |
| `--text-muted` | `#4D4D4D` | `#B3B3B3` | secondary text | 5.99:1 on bg, 7.16:1 on surface (light); 6.75:1 (dark) |
| `--line` | `#6B6B6B` | `#9A9A9A` | normal trend line, outlines | 3.78:1 on bg, 4.51:1 on surface; dark 5.03:1; non-text only |
| `--alarm-1` (P1) | `#CC3311` fill, white text | text `#EE6677` | critical | 5.19:1 badge; 3.67:1 as border/icon (non-text only); dark text 4.59:1 |
| `--alarm-2` (P2) | `#EE7733` fill, black text | same fill | high | 7.32:1 |
| `--alarm-3` (P3) | `#F0E442` fill, black text | text `#CCBB44` | low/advisory | 15.88:1; dark 7.25:1. Never yellow text on grey in light mode |
| `--info` | `#0077BB` fill, white text | text `#66CCEE` | selection, info | 4.82:1; icon 3.42:1; dark 7.71:1 |
| `--quality-bad` | `#AA3377` fill, white text | text `#CC79A7` | comm/quality problems | 6.09:1; icon 4.31:1; dark 4.63:1 |
| `--focus` | `#0077BB` | `#66CCEE` | focus ring | ≥ 3:1 against the base |

Hues are Paul Tol / Okabe–Ito colour-blind-safe. Any new pair must be checked with the same formula before use.

### Alarm priority shapes

Priority is shown by **shape + number + color**; shape and number carry the meaning, color is redundant:
- P1: red triangle with "1".
- P2: orange diamond with "2".
- P3: yellow square with "3".

## Quality display

Every value shows its quality. Mapping (research/02 §3):

| BusLink quality | OPC UA | Tile | Trend |
|---|---|---|---|
| GOOD | Good | neutral; age in grey | line |
| STALE | Uncertain_LastUsableValue | value greyed + "STALE · 12 s" + hatched tile edge | gap after the last good point |
| TIMEOUT | Bad_Timeout | magenta `TIMEOUT` badge; last value dimmed and struck through | gap |
| CRC_ERROR | Bad_CommunicationError | magenta `CRC` badge | gap |
| EXCEPTION(n) | Bad_DeviceFailure / Bad_NotSupported | magenta `EXC 02 Illegal data address` | gap |
| DECODE_ERROR | Bad_DecodingError | magenta `DECODE` | gap |
| DISCONNECTED | Bad_NoCommunication | whole device card greyed + `DISCONNECTED` | gap |

- Failed or stale values show the **last good value dimmed with its age**, never as a current reading, never as 0 and never blank without explanation. A non-color cue (hatch, strike-through, outline) always accompanies the badge.
- Tiles show data age and the poll interval actually achieved. A device header shows link state separately from value quality.
- The register inspector shows, per row: table, doc address (manual label), PDU offset, FC, raw hex, decoded value in the profile's word order, all four word orders side by side, quality and exception text with a likely-cause hint, plus the last request/response frames.

## Alarms (ISA-18.2)

- Definition: tag, type (HI/HIHI/LO/LOLO), limit, deadband, on-delay, off-delay, priority 1–3, message, latch flag.
- States: Normal, Unacknowledged, Acknowledged, Return-to-normal unacknowledged, Shelved (with expiry), Suppressed by design, Out of service. An acknowledged alarm stays active until the value returns.
- Communication alarms (timeout, CRC, exception, decode, disconnected) are a separate class from process alarms; while a tag's quality is bad, its process alarms hold their state instead of flooding.
- Banner: count per priority and the oldest unacknowledged alarm. The list is sortable and filterable; acknowledge and shelve are logged events.
- Flood indicator when more than 10 alarms activate in 10 minutes; chattering flag when one alarm activates 3 or more times in 1 minute.
- Default deadbands by unit type: flow/level 5 % of range, pressure 2 %, temperature 1 %; default on/off delay 15 s (temperature 60 s). Training presets may use shorter delays and must say so.

## Trends

- **uPlot** (MIT, ~20 KB): fast canvas, native `null` gaps, real timestamps. Not LTTB anywhere alarm-relevant.
- **Gaps are gaps.** Any sample that is not GOOD, and any interval without samples longer than the stale threshold, breaks the line (`null`); never interpolate, connect across or carry the last value forward. Shade gap intervals with a hatched band and label the reason ("No data: TIMEOUT"); the tooltip on a gap states it.
- Keep raw samples separate from display resampling. Long ranges are decimated **min-max per pixel column on the Rust side**; say "min/max per pixel" when decimated.
- Overlays: alarm limits as horizontal bands, setpoint as a line, acknowledge and write events as vertical markers.
- At most 2 y-axes (one per unit) per chart; other units go in additional charts. Series differ by dash pattern or marker as well as color.
- Time axis in local time; UTC in tooltips and exports.
- UI refresh ≤ 4 Hz, independent of the poll rate.

## Control panel (simulated devices only)

- Banner "Simulated device" on every control view. Armed state is obvious (border + text "Armed: control enabled for 52 s").
- Buttons name the action and value: "Apply setpoint 40 °C". The confirm dialog shows device, unit ID, register (doc address + offset), old → new, raw value and encoded frame.
- Separate fields for command sent, acknowledged, and measured feedback; show pending/unknown honestly.
- Every stop control carries the label "Software stop. Not an emergency stop." / "Stop perangkat lunak. Bukan tombol darurat."
- The LAN dashboard build renders no control panel, no write UI and no arming controls at all.

## Accessibility (WCAG 2.2 AA)

- Contrast: text ≥ 4.5:1 (large text ≥ 3:1: ≥ 18.66 px bold or ≥ 24 px); UI components, chart lines and focus indicators ≥ 3:1 (1.4.3, 1.4.11).
- Never color alone (1.4.1): text label ("TIMEOUT", "P1") + shape for every status; about 8 % of men have red–green colour vision deficiency.
- Fully keyboard operable, visible focus ring (`--focus`) that sticky headers never hide (2.1.1, 2.4.7, 2.4.11). Logical heading and tab order.
- Targets ≥ 24×24 CSS px everywhere (2.5.8); 44 px for primary mobile actions (connect, arm, stop). Android: ≥ 48×48 dp with ≥ 8 dp spacing; destructive or control actions never sit next to each other.
- Live regions: alarms and connection changes go to an `aria-live="polite"` region; P1 alarms use `assertive`. Never announce every sample (4.1.3).
- A "Pause live updates" control freezes dashboard values for reading or screenshots, with a visible "Paused" state (2.2.2).
- `prefers-reduced-motion`: no pulsing alarms; use a static border. Nothing flashes more than 3 times per second (2.3.1).
- Reflow at 320–360 CSS px wide and 200 % zoom without losing content (1.4.10, 1.4.4). Portrait and landscape both work; no orientation lock (1.3.4).
- Light and dark themes both use the palette above; dark follows the OS unless the user picks a theme.
- Form errors are announced in text next to the field (3.3.1); confirmation dialogs trap and restore focus.

## i18n (id / en)

- All strings in `src/i18n/en.json` and `src/i18n/id.json` with the same keys; a missing key is a test failure. Default language follows the system, switchable at runtime, persisted.
- Units and technical symbols are identical in both languages (°C, %RH, µS/cm, FC03, CRC). Terms like "register", "holding register", "unit ID" may stay English in Indonesian text when that is what technicians use.
- Display numbers in the active locale (id uses decimal comma), but exports always use `.` decimals and ISO-8601 UTC.
- Set `lang` on `<html>`; text may grow ~30 % in Indonesian, so no fixed-width labels.

## Security in the UI

- Render profile, device and user strings with `textContent` or escaped templates; never `innerHTML` with data.
- Never show secrets (MQTT password, LAN token) after entry except an explicit "show" toggle; QR/URL for LAN sharing is shown only on the host device.
