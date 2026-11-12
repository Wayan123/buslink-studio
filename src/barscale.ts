// Analog bar scale: pure logic, no DOM. The scale comes only from the profile
// (`valid_range`, else `display_range`). Alarm limits never define a scale.
import type { Register } from "./types.ts";

export interface Range {
  min: number;
  max: number;
}

export type ScaleSource = "valid_range" | "display_range";
export type OutOfScale = "low" | "high" | null;

function toRange(r: [number, number] | undefined): Range | null {
  if (!r || !Number.isFinite(r[0]) || !Number.isFinite(r[1]) || r[1] <= r[0]) return null;
  return { min: r[0], max: r[1] };
}

/** Bar range from the profile, or null when the register declares none (bar hidden). */
export function barRange(reg: Pick<Register, "valid_range" | "display_range">): Range | null {
  return toRange(reg.valid_range) ?? toRange(reg.display_range);
}

/** Whether a value lies outside the bar range (marker is clamped to the end). */
export function outOfScale(value: number | null | undefined, r: Range): OutOfScale {
  if (value === null || value === undefined || !Number.isFinite(value)) return null;
  if (value < r.min) return "low";
  if (value > r.max) return "high";
  return null;
}

/** Position in percent, clamped to the track. */
export function barPct(v: number, r: Range): number {
  const p = ((v - r.min) / (r.max - r.min)) * 100;
  return Math.max(0, Math.min(100, p));
}
