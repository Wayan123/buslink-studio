Fix two dashboard issues found in docs/verification/02-dashboard-timeout.png, as the buslink-builder agent.

1. **Analog bar range excludes the value.** `barRange` in `src/components/tile.ts` falls back to the alarm limits padded by 10 % when a register has no `valid_range`. Soil moisture (value 31.9 %) gets a 13.5–16.5 bar with its marker pinned at the right edge. VFD motor current (0.0 A) gets a 7–9 bar. Both mislead. Fix:
   - Prefer the register's `valid_range`.
   - Otherwise use a profile display range if the schema has one; add an optional `display_range` to schema v1 only if none exists, and update docs/profiles/schema-v1.md, the JSON schema, the validator and the check_profile hook together.
   - Hide the bar when there is no declared range. Never derive the scale from alarm limits alone.
   - When the value lies outside the range, clamp the marker and show an explicit "out of scale" cue (a text label plus the marker at the end, not color alone).
   - Add unit tests in `src/*.test.ts` for: valid_range, display_range, no range → hidden, value outside → out-of-scale flag.
   - Give the shipped profiles realistic ranges where the device manual or simulator defines them: soil moisture 0–100 %, motor current 0 to the simulated VFD's rated current.
2. Re-run `npm test`, `npm run build`, `cargo test --workspace`, and `scripts/verify-journey.mjs`. Re-check screenshots 01, 02 and 07 by eye.

Do not commit. Report the changed files and the check results.
