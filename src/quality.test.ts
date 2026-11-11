// U3: every quality has a text label and a non-color cue; age-based STALE display.
import assert from "node:assert/strict";
import { test } from "node:test";
import { displayQuality, exceptionHintKey, QUALITY_CODES, qualityInfo } from "./quality.ts";
import type { TagView } from "./types.ts";

test("U3 every non-GOOD quality has a badge text and a cue", () => {
  for (const q of QUALITY_CODES) {
    const info = qualityInfo(q, q === "EXCEPTION" ? 2 : null);
    if (q === "GOOD") {
      assert.equal(info.badge, "");
      assert.equal(info.notCurrent, false);
      continue;
    }
    assert.ok(info.badge.length > 0, `${q} has no badge`);
    assert.notEqual(info.cue, "none", `${q} relies on color only`);
    assert.ok(info.notCurrent);
    assert.ok(info.opcua.startsWith("Bad") || info.opcua.startsWith("Uncertain"));
  }
  assert.equal(qualityInfo("EXCEPTION", 2).badge, "EXC 02");
  assert.equal(qualityInfo("CRC_ERROR").badge, "CRC");
  assert.equal(exceptionHintKey(2), "exc.hint.02");
  assert.equal(exceptionHintKey(0x0b), "exc.hint.0B");
  assert.equal(exceptionHintKey(0x42), "exc.hint.other");
});

test("an old GOOD value is displayed as STALE", () => {
  const tag = { quality: "GOOD", value_ts_ms: 1000 } as TagView;
  assert.equal(displayQuality(tag, 3000, 3500), "GOOD");
  assert.equal(displayQuality(tag, 3000, 4001), "STALE");
  assert.equal(displayQuality({ ...tag, quality: "TIMEOUT" }, 3000, 9999), "TIMEOUT");
});
