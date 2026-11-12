// Analog bar scale: profile ranges only, hidden without one, out-of-scale flag.
import assert from "node:assert/strict";
import { test } from "node:test";
import { barPct, barRange, outOfScale } from "./barscale.ts";

test("bar uses valid_range", () => {
  assert.deepEqual(barRange({ valid_range: [-30, 70] }), { min: -30, max: 70 });
});

test("valid_range wins over display_range", () => {
  assert.deepEqual(barRange({ valid_range: [0, 50], display_range: [0, 100] }), { min: 0, max: 50 });
});

test("bar uses display_range when there is no valid_range", () => {
  assert.deepEqual(barRange({ display_range: [0, 100] }), { min: 0, max: 100 });
});

test("no declared range hides the bar", () => {
  assert.equal(barRange({}), null);
});

test("degenerate ranges hide the bar", () => {
  assert.equal(barRange({ display_range: [5, 5] }), null);
  assert.equal(barRange({ display_range: [10, 0] }), null);
});

test("value outside the range is flagged and clamped", () => {
  const r = { min: 0, max: 100 };
  assert.equal(outOfScale(31.9, r), null);
  assert.equal(outOfScale(0, r), null);
  assert.equal(outOfScale(100, r), null);
  assert.equal(outOfScale(100.1, r), "high");
  assert.equal(outOfScale(-0.1, r), "low");
  assert.equal(outOfScale(null, r), null);
  assert.equal(barPct(150, r), 100);
  assert.equal(barPct(-5, r), 0);
  assert.equal(barPct(25, r), 25);
});
