// Store: frames update tags and trend buffers; non-GOOD points are stored as null.
import assert from "node:assert/strict";
import { test } from "node:test";
import { MAX_POINTS_PER_TAG, Store } from "./store.ts";
import type { LiveFrame } from "./types.ts";

function frame(points: LiveFrame["points"], ts = 10_000): LiveFrame {
  return { ts_ms: ts, tags: [], points, dropped: 0, links: null, alarms: null, historian_error: null };
}

test("points append in order, bad quality stored as null, late points sorted in", () => {
  const s = new Store();
  s.applyFrame(
    frame([
      { k: "d/t", t: 1, v: 1, q: "GOOD" },
      { k: "d/t", t: 3, v: 5, q: "TIMEOUT" },
      { k: "d/t", t: 2, v: 2, q: "GOOD" },
    ]),
  );
  const b = s.buffers.get("d/t");
  assert.deepEqual(b?.t, [1, 2, 3]);
  assert.deepEqual(b?.v, [1, 2, null]);
});

test("buffers are bounded", () => {
  const s = new Store();
  const pts = Array.from({ length: MAX_POINTS_PER_TAG + 50 }, (_, i) => ({ k: "d/t", t: 9_000 + i * 0.01, v: i, q: "GOOD" as const }));
  s.applyFrame(frame(pts));
  assert.equal(s.buffers.get("d/t")?.t.length, MAX_POINTS_PER_TAG);
});

test("dropped points are accumulated, never hidden", () => {
  const s = new Store();
  s.applyFrame({ ...frame([]), dropped: 3 });
  s.applyFrame({ ...frame([]), dropped: 2 });
  assert.equal(s.droppedTotal, 5);
});
