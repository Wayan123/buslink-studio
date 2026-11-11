---
inclusion: fileMatch
fileMatchPattern: "src/**"
---

# UI rules (webview + LAN dashboard)

TODO: `research/02-standards-and-dashboard-design.md` was not available when this file was written (2026-11-11). These rules come from ISA-101 / WCAG 2.2 basics and the owner's brief. Revise this file when that research lands.

## ISA-101 high-performance style

- **Gray, calm normal state.** Neutral light-gray background, dark-gray text and outlines, no saturated color when everything is normal. No 3D, gradients, photos or decorative animation.
- **Color means abnormal.** Saturated colors are reserved for alarms and abnormal quality, and every colored state also has a text label and a shape or icon. Never use green for "running" or "OK"; normal needs no color.
- Values show number + unit + quality. Use analog indicators (bar with normal range band, setpoint marker) where position matters more than digits.
- Hierarchy: overview (all devices, worst state first) → device page (tags) → detail (trend, register inspector).
- Project color tokens (ISA-101 does not mandate exact RGB values; check every pair with a contrast checker):

| Token | Use | Rule |
|---|---|---|
| `--bg` | page background | light neutral gray |
| `--surface` | tiles, tables | slightly lighter/darker gray than `--bg` |
| `--text`, `--text-muted` | values, labels | ≥ 4.5:1 on `--surface`; muted only for secondary text, still ≥ 4.5:1 |
| `--alarm-1` (critical) | red | badge with white or dark text ≥ 4.5:1 |
| `--alarm-2` (high) | orange/amber | dark text on fill |
| `--alarm-3` (low/advisory) | yellow | dark text on fill; never yellow text on gray |
| `--quality-bad` | comm/quality problems | distinct from process alarm colors (e.g. magenta or cyan outline + hatch) |
| `--focus` | focus ring | ≥ 3:1 against adjacent colors |

- Process alarms (value out of limits) and communication problems (quality) look different and are listed separately.

## Quality display

- Every value shows its quality. GOOD shows the value plainly. Any other quality shows a text badge (`TIMEOUT`, `CRC`, `EXC 02`, `STALE`, `DECODE`) and the age ("12 s ago"), plus a non-color cue (hatched background or outlined value).
- STALE / failed values show the **last good value dimmed with its age**, never as a current reading, never as 0 and never blank without explanation.
- Tiles show data age and the poll interval actually achieved. A device header shows link state separately from value quality.
- The register inspector shows, per row: table, doc address (manual label), PDU offset, FC, raw hex, decoded value in the profile's word order, all four word orders on demand, quality and exception text.

## Trends

- **Gaps are gaps.** Any sample that is not GOOD breaks the line (pass `null`); never interpolate, draw connecting lines or carry the last value forward across lost communication. Shade comm-loss intervals with a hatched band and label it ("No data: TIMEOUT").
- Keep raw samples separate from display resampling. When downsampling, draw min/max bands, not just averages, and say which one is shown.
- Show alarm limits as bands, setpoint as a line, time axis in local time with UTC in tooltips/exports.
- Throttle rendering (UI refresh independent of poll rate, ≤ 4 fps for trends on Android) to save battery.

## Control panel (simulated devices only)

- Banner "Simulated device" on every control view. Armed state is obvious (border + text "Armed: control enabled for 52 s").
- Buttons name the action and value: "Apply setpoint 40 °C". The confirm dialog shows device, unit ID, register (doc address + offset), old → new, raw value and encoded frame.
- Separate fields for command sent, acknowledged, and measured feedback; show pending/unknown honestly.
- Every stop control carries the label "Software stop. Not an emergency stop." / "Stop perangkat lunak. Bukan tombol darurat."
- The LAN dashboard build renders no control panel, no write UI and no arming controls at all.

## Accessibility (WCAG 2.2 AA)

- Contrast: text ≥ 4.5:1 (large text ≥ 3:1); UI components, chart lines and focus indicators ≥ 3:1 (1.4.3, 1.4.11).
- Never color alone (1.4.1): text label + shape/pattern for every state; trends distinguish series by dash pattern or markers too.
- Fully keyboard operable, visible focus that is not hidden by sticky headers (2.1.1, 2.4.7, 2.4.11). Logical heading and tab order.
- Pointer targets ≥ 24×24 CSS px (2.5.8). **Android: ≥ 48×48 dp** with ≥ 8 dp spacing; destructive or control actions never sit next to each other.
- Live value updates must not flood screen readers: use `aria-live="polite"` only for alarms and connection changes, not for every sample.
- `prefers-reduced-motion`: no blinking or animated transitions; alarm flashing becomes a static marker. Nothing flashes more than 3 times per second (2.3.1).
- Reflow at 320 CSS px wide and 200% zoom without losing content (1.4.10, 1.4.4). Support system dark mode only if both themes pass contrast.
- Form errors are announced in text next to the field (3.3.1); confirmation dialogs trap and restore focus.

## i18n (id / en)

- All strings in `src/i18n/en.json` and `src/i18n/id.json` with the same keys; a missing key is a test failure. Default language follows the system, switchable at runtime, persisted.
- Units and technical symbols are identical in both languages (°C, %RH, µS/cm, FC03, CRC). Terms like "register", "holding register", "unit ID" may stay English in Indonesian text when that is what technicians use.
- Display numbers in the active locale (id uses decimal comma), but exports always use `.` decimals and ISO-8601 UTC.
- Set `lang` on `<html>`; text may grow ~30% in Indonesian, so no fixed-width labels.

## Security in the UI

- Render profile, device and user strings with `textContent` or escaped templates; never `innerHTML` with data.
- Never show secrets (MQTT password, LAN token) after entry except an explicit "show" toggle; QR/URL for LAN sharing is shown only on the host device.
