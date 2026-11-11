// Pure helpers for the control panel and the wizard (no DOM, unit-tested with node --test).
// The Rust command service is the authority; these checks only give early, field-level feedback.
import type { Lang } from "./format.ts";
import { fmtNumber } from "./format.ts";
import type { Ack, AuditRow, ProfileCommand, Register } from "./types.ts";

export type BoundsError =
  | { key: "ctl.err.not_number" }
  | { key: "ctl.err.range"; min: number; max: number }
  | { key: "ctl.err.step"; step: number }
  | { key: "ctl.err.allowed"; values: string }
  | { key: "ctl.err.not_writable" };

/** Same rules as the command service: allowed_values, else min/max/step, then valid_range. Never clamps. */
export function checkBounds(reg: Register, value: number): BoundsError | null {
  if (!Number.isFinite(value)) return { key: "ctl.err.not_number" };
  const w = reg.write;
  if (!w || reg.access !== "read_write") return { key: "ctl.err.not_writable" };
  const same = (a: number, b: number) => Math.abs(a - b) <= 1e-9 * Math.max(Math.abs(a), Math.abs(b), 1);
  if (w.allowed_values) {
    if (!w.allowed_values.some((a) => same(a, value))) return { key: "ctl.err.allowed", values: w.allowed_values.join(", ") };
  } else if (w.min !== undefined && w.max !== undefined) {
    if (value < w.min || value > w.max) return { key: "ctl.err.range", min: w.min, max: w.max };
    if (w.step !== undefined) {
      const k = (value - w.min) / w.step;
      if (Math.abs(k - Math.round(k)) > 1e-6) return { key: "ctl.err.step", step: w.step };
    }
  } else {
    return { key: "ctl.err.not_writable" };
  }
  if (reg.valid_range && (value < reg.valid_range[0] || value > reg.valid_range[1])) {
    return { key: "ctl.err.range", min: reg.valid_range[0], max: reg.valid_range[1] };
  }
  return null;
}

/** Engineering value with the register's decimals and unit: "40.0 °C", "30,00 Hz" (id). */
export function valueText(v: number, reg: Register | undefined, lang: Lang): string {
  const d = reg?.decimals;
  const s = fmtNumber(v, lang, d);
  return reg?.unit ? `${s} ${reg.unit}` : s;
}

/** Button and confirm label: the profile label plus the value for setpoints ("Apply temperature setpoint 40.0 °C"). */
export function actionLabel(label: string, cmd: ProfileCommand, value: number | undefined, reg: Register | undefined, lang: Lang): string {
  if (cmd.kind === "setpoint" && value !== undefined && Number.isFinite(value)) return `${label} ${valueText(value, reg, lang)}`;
  return label;
}

/** i18n key for an acknowledgement; a timeout or bad reply means "effect unknown". */
export function ackKey(a: Ack | null | undefined): string {
  if (!a) return "ctl.ack.none";
  return `ctl.ack.${a.kind}`;
}

/** Whole seconds left, never negative: "Armed: control enabled for 52 s". */
export function secondsLeft(ms: number | null | undefined): number {
  if (ms === null || ms === undefined || ms <= 0) return 0;
  return Math.ceil(ms / 1000);
}

/** Commands shown in the panel body; stop-kind commands get their own always-visible place. */
export function splitCommands(cmds: ProfileCommand[]): { stops: ProfileCommand[]; others: ProfileCommand[] } {
  return { stops: cmds.filter((c) => c.kind === "stop"), others: cmds.filter((c) => c.kind !== "stop") };
}

export interface AuditFilter {
  device: string;
  event: string;
  text: string;
}

/** Rows matching every set filter; text matches device, command, register, note, hex and user. */
export function filterAudit(rows: AuditRow[], f: AuditFilter): AuditRow[] {
  const needle = f.text.trim().toLowerCase();
  return rows.filter((r) => {
    if (f.device && r.device_id !== f.device) return false;
    if (f.event && r.event !== f.event) return false;
    if (!needle) return true;
    return [r.device_id, r.command_id, r.register_id, r.note, r.request_hex, r.response_hex, r.user, r.ack, r.readback]
      .some((x) => (x ?? "").toLowerCase().includes(needle));
  });
}

/** Commands with their final result row and their process feedback row, by preview ID. */
export function commandRuns(audit: AuditRow[]): { result: AuditRow; feedback: AuditRow | undefined }[] {
  const results = audit.filter((a) => a.event === "result");
  return results.map((result) => ({
    result,
    feedback: audit.find(
      (a) => a.event === "feedback" && a.device_id === result.device_id && a.command_id === result.command_id && a.id > result.id,
    ),
  }));
}

