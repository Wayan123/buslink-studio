// U1/U2: store buffers → uPlot series. Run with `node --test` (type stripping).
import assert from "node:assert/strict";
import { test } from "node:test";
import { align, bufferToSeries, NO_DATA } from "./series.ts";
import type { TrendBuffer } from "./store.ts";
import type { QualityCode } from "./types.ts";

function buf(points: [number, number | null, QualityCode][]): TrendBuffer {
  return { t: points.map((p) => p[0]), v: points.map((p) => p[1]), q: points.map((p) => p[2]) };
}

test("U1 non-GOOD points become null, never a number", () => {
  const s = bufferToSeries(
    buf([
      [0, 1, "GOOD"],
      [1000, 2, "GOOD"],
      [2000, 999, "TIMEOUT"],
      [3000, 999, "TIMEOUT"],
      [4000, 3, "GOOD"],
    ]),
    0,
    4000,
    5000,
  );
  assert.deepEqual(s.v, [1, 2, null, 3]);
  assert.deepEqual(s.gaps, [{ from_ms: 1000, to_ms: 4000, reason: "TIMEOUT" }]);
});

test("U1 random buffers: every value comes from a GOOD point", () => {
  let seed = 7;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
  const qs: QualityCode[] = ["GOOD", "GOOD", "GOOD", "TIMEOUT", "STALE", "CRC_ERROR", "EXCEPTION", "DISCONNECTED"];
  for (let c = 0; c < 200; c++) {
    let t = 0;
    const pts: [number, number | null, QualityCode][] = [];
    for (let i = 0; i < 100; i++) {
      t += 1 + Math.floor(rnd() * 3000);
      pts.push([t, Math.round(rnd() * 100), qs[Math.floor(rnd() * qs.length)] ?? "GOOD"]);
    }
    const b = buf(pts);
    const s = bufferToSeries(b, 0, t, 2000);
    s.t.forEach((x, i) => {
      const v = s.v[i];
      if (v === null || v === undefined) return;
      const j = b.t.indexOf(x);
      assert.ok(j >= 0 && b.q[j] === "GOOD" && b.v[j] === v, `value ${v} at ${x} is not GOOD`);
    });
    // U2: two consecutive drawn values never straddle a bad point or a hole longer than the threshold.
    for (let i = 1; i < s.t.length; i++) {
      const [a, bb] = [s.t[i - 1] ?? 0, s.t[i] ?? 0];
      if (s.v[i - 1] === null || s.v[i] === null) continue;
      assert.ok(bb - a <= 2000, `hole ${a}..${bb} drawn over`);
      for (let j = 0; j < b.t.length; j++) {
        const x = b.t[j] ?? 0;
        if (x > a && x < bb) assert.equal(b.q[j], "GOOD");
      }
    }
  }
});

test("U2 holes and a silent tail are NO_DATA gaps", () => {
  const s = bufferToSeries(
    buf([
      [0, 1, "GOOD"],
      [10_000, 2, "GOOD"],
    ]),
    0,
    30_000,
    3000,
  );
  assert.deepEqual(s.v, [1, null, 2]);
  assert.equal(s.gaps.length, 2);
  assert.equal(s.gaps[0]?.reason, NO_DATA);
  assert.deepEqual([s.gaps[1]?.from_ms, s.gaps[1]?.to_ms], [10_000, 30_000]);
});

test("align keeps own nulls and uses undefined only for alignment", () => {
  const a = { t: [1, 2, 3], v: [1, null, 3], gaps: [] };
  const b = { t: [2, 4], v: [5, 6], gaps: [] };
  const { x, ys } = align([a, b]);
  assert.deepEqual(x, [1, 2, 3, 4]);
  assert.deepEqual(ys[0], [1, null, 3, undefined]);
  assert.deepEqual(ys[1], [undefined, 5, undefined, 6]);
});
