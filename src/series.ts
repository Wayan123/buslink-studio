// Store buffers → uPlot data. Gaps are gaps: non-GOOD points are `null` (uPlot breaks the line),
// holes longer than the stale threshold get a `null` too, and every gap is returned with its reason.
// `undefined` is only used for alignment between series (uPlot spans it; it never stands for lost data).
import type { Gap } from "./types.ts";
import type { TrendBuffer } from "./store.ts";

export const NO_DATA = "NO_DATA";

export interface Series {
  t: number[];
  v: (number | null)[];
  gaps: Gap[];
}

export function bufferToSeries(b: TrendBuffer | undefined, from: number, to: number, staleMs: number): Series {
  const out: Series = { t: [], v: [], gaps: [] };
  if (!b) return out;
  let lastGood: number | null = null;
  let open: Gap | null = null;
  for (let i = 0; i < b.t.length; i++) {
    const t = b.t[i] ?? 0;
    if (t < from || t > to) continue;
    const q = b.q[i];
    const v = b.v[i];
    const good = q === "GOOD" && v !== null && v !== undefined && Number.isFinite(v);
    if (good) {
      if (open) {
        open.to_ms = t;
        open = null;
      } else if (lastGood !== null && t - lastGood > staleMs) {
        out.t.push(lastGood + (t - lastGood) / 2);
        out.v.push(null);
        out.gaps.push({ from_ms: lastGood, to_ms: t, reason: NO_DATA });
      }
      out.t.push(t);
      out.v.push(v);
      lastGood = t;
    } else {
      if (!open) {
        open = { from_ms: lastGood ?? t, to_ms: t, reason: q === "EXCEPTION" ? "EXCEPTION" : (q ?? NO_DATA) };
        out.gaps.push(open);
        out.t.push(t);
        out.v.push(null);
      } else {
        open.to_ms = t;
      }
      lastGood = null;
    }
  }
  // Nothing new for longer than the stale threshold: the tail is a gap, not a flat line.
  if (open) {
    open.to_ms = Math.max(open.to_ms, to);
  } else if (lastGood !== null && to - lastGood > staleMs) {
    out.gaps.push({ from_ms: lastGood, to_ms: to, reason: NO_DATA });
  }
  return out;
}

/** Shortest live x span, so a chart opened in the first seconds of a session is not a single column. */
export const MIN_LIVE_SPAN_MS = 30_000;

/**
 * Live x range: from the first buffered point (the session's data start) to now, capped at the
 * chosen window. Without data the full window is shown. Only the axis fits; no point is invented.
 */
export function liveBounds(now: number, windowMs: number, firstMs: number | undefined): { from: number; to: number } {
  const capped = now - windowMs;
  if (firstMs === undefined || !Number.isFinite(firstMs)) return { from: capped, to: now };
  const from = Math.max(capped, Math.min(firstMs, now - MIN_LIVE_SPAN_MS));
  return { from, to: now };
}

/** Earliest timestamp over the given buffers (any quality: a gap at the start is still shown). */
export function firstPointMs(buffers: (TrendBuffer | undefined)[]): number | undefined {
  let first: number | undefined;
  for (const b of buffers) {
    const t = b?.t[0];
    if (t !== undefined && (first === undefined || t < first)) first = t;
  }
  return first;
}

/** Merge several series onto one x array for uPlot. Own nulls stay null; other series' x get undefined. */
export function align(series: Series[]): { x: number[]; ys: (number | null | undefined)[][] } {
  const xs = [...new Set(series.flatMap((s) => s.t))].sort((a, b) => a - b);
  const index = new Map(xs.map((x, i) => [x, i]));
  const ys = series.map((s) => {
    const y: (number | null | undefined)[] = new Array(xs.length).fill(undefined);
    s.t.forEach((t, i) => {
      const at = index.get(t);
      if (at === undefined) return;
      const v = s.v[i];
      // A null always wins over a value at the same instant: never hide a gap.
      if (v === null || y[at] === undefined) y[at] = v ?? null;
    });
    return y;
  });
  return { x: xs, ys };
}
