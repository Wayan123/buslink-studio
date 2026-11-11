// Quality → display mapping (ui.md "Quality display", research/02 §3). Pure module.
import type { QualityCode, TagView } from "./types.ts";

export type Cue = "none" | "hatch" | "strike" | "grey";

export interface QualityInfo {
  code: QualityCode;
  /** Short badge text; empty for GOOD (normal needs no badge). */
  badge: string;
  /** CSS class carrying color + non-color cue. */
  cls: string;
  cue: Cue;
  opcua: string;
  /** True when the value shown is not a current reading. */
  notCurrent: boolean;
}

export const QUALITY_CODES: QualityCode[] = [
  "GOOD",
  "TIMEOUT",
  "CRC_ERROR",
  "EXCEPTION",
  "STALE",
  "DECODE_ERROR",
  "DISCONNECTED",
];

export function hex2(n: number): string {
  return n.toString(16).toUpperCase().padStart(2, "0");
}

export function qualityInfo(code: QualityCode, exception: number | null = null): QualityInfo {
  switch (code) {
    case "GOOD":
      return { code, badge: "", cls: "q-good", cue: "none", opcua: "Good", notCurrent: false };
    case "STALE":
      return { code, badge: "STALE", cls: "q-stale", cue: "hatch", opcua: "Uncertain_LastUsableValue", notCurrent: true };
    case "TIMEOUT":
      return { code, badge: "TIMEOUT", cls: "q-bad", cue: "strike", opcua: "Bad_Timeout", notCurrent: true };
    case "CRC_ERROR":
      return { code, badge: "CRC", cls: "q-bad", cue: "strike", opcua: "Bad_CommunicationError", notCurrent: true };
    case "EXCEPTION":
      return {
        code,
        badge: exception === null ? "EXC" : `EXC ${hex2(exception)}`,
        cls: "q-bad",
        cue: "strike",
        opcua: exception === 1 || exception === 2 ? "Bad_NotSupported" : "Bad_DeviceFailure",
        notCurrent: true,
      };
    case "DECODE_ERROR":
      return { code, badge: "DECODE", cls: "q-bad", cue: "strike", opcua: "Bad_DecodingError", notCurrent: true };
    case "DISCONNECTED":
      return { code, badge: "DISCONNECTED", cls: "q-disc", cue: "grey", opcua: "Bad_NoCommunication", notCurrent: true };
  }
}

/**
 * Quality to display now. A GOOD value older than the stale threshold is shown as STALE
 * (for example after polling was stopped), so an old reading never looks current.
 */
export function displayQuality(tag: TagView, staleAfterMs: number, nowMs: number): QualityCode {
  if (tag.quality === "GOOD" && tag.value_ts_ms !== null && nowMs - tag.value_ts_ms > staleAfterMs) {
    return "STALE";
  }
  return tag.quality;
}

/** i18n key of the likely-cause hint for a Modbus exception code. */
export function exceptionHintKey(code: number | null): string | null {
  if (code === null) return null;
  const known = [1, 2, 3, 4, 5, 6, 8, 10, 11];
  return known.includes(code) ? `exc.hint.${hex2(code)}` : "exc.hint.other";
}

/** Rank for "worst state first" (lower = worse). */
export function qualityRank(code: QualityCode): number {
  switch (code) {
    case "DISCONNECTED":
      return 0;
    case "TIMEOUT":
      return 1;
    case "CRC_ERROR":
      return 2;
    case "EXCEPTION":
      return 3;
    case "DECODE_ERROR":
      return 4;
    case "STALE":
      return 5;
    case "GOOD":
      return 9;
  }
}
