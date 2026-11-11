// Control panel helpers (commissioning-control Req 2.3, 2.4). Run with `node --test`.
import assert from "node:assert/strict";
import { test } from "node:test";
import { actionLabel, ackKey, checkBounds, commandRuns, filterAudit, secondsLeft, splitCommands } from "./control.ts";
import type { AuditRow, ProfileCommand, Register } from "./types.ts";

const sp: Register = {
  id: "sp",
  name: "Setpoint",
  table: "holding_register",
  address: 1,
  type: "i16",
  scale: 0.1,
  unit: "°C",
  decimals: 1,
  valid_range: [0, 120],
  access: "read_write",
  write: { min: 0, max: 120, step: 0.1, risk: "medium", role: "operator" },
};
const cw: Register = {
  id: "control_word",
  name: "Control word",
  table: "holding_register",
  address: 0,
  type: "u16",
  access: "read_write",
  write: { allowed_values: [0, 1, 4], risk: "high", role: "operator" },
};
const setT: ProfileCommand = { id: "set_temperature", kind: "setpoint", label: "Apply temperature setpoint", register: "sp", readback_register: "sp" };
const stop: ProfileCommand = { id: "stop", kind: "stop", label: "Software stop. Not an emergency stop.", register: "control_word", readback_register: "control_word", value: 0 };

test("bounds: rejected, never clamped", () => {
  assert.equal(checkBounds(sp, 40), null);
  assert.equal(checkBounds(sp, 120), null);
  assert.deepEqual(checkBounds(sp, 120.1), { key: "ctl.err.range", min: 0, max: 120 });
  assert.deepEqual(checkBounds(sp, -0.1), { key: "ctl.err.range", min: 0, max: 120 });
  assert.deepEqual(checkBounds(sp, 40.05), { key: "ctl.err.step", step: 0.1 });
  assert.deepEqual(checkBounds(sp, Number.NaN), { key: "ctl.err.not_number" });
  assert.equal(checkBounds(cw, 4), null);
  assert.deepEqual(checkBounds(cw, 2), { key: "ctl.err.allowed", values: "0, 1, 4" });
  assert.deepEqual(checkBounds({ ...sp, access: "read" }, 1), { key: "ctl.err.not_writable" });
});

test("labels name the action and the value in engineering units", () => {
  assert.equal(actionLabel("Apply temperature setpoint", setT, 40, sp, "en"), "Apply temperature setpoint 40.0 °C");
  assert.equal(actionLabel("Terapkan setpoint suhu", setT, 40, sp, "id"), "Terapkan setpoint suhu 40,0 °C");
  assert.equal(actionLabel("Software stop. Not an emergency stop.", stop, undefined, cw, "en"), "Software stop. Not an emergency stop.");
});

test("ack, countdown and stop placement", () => {
  assert.equal(ackKey({ kind: "timeout" }), "ctl.ack.timeout");
  assert.equal(ackKey(null), "ctl.ack.none");
  assert.equal(secondsLeft(51_200), 52);
  assert.equal(secondsLeft(-5), 0);
  const { stops, others } = splitCommands([setT, stop]);
  assert.deepEqual(stops.map((c) => c.id), ["stop"]);
  assert.deepEqual(others.map((c) => c.id), ["set_temperature"]);
});

function row(id: number, event: string, extra: Partial<AuditRow> = {}): AuditRow {
  return {
    id, ts_ms: id, session_id: "s", user: "u", role: "operator", event, device_id: "vfd", profile_id: "sim.vfd-basic",
    profile_version: "1.1.0", unit_id: 2, function: null, address: null, register_id: null, command_id: null,
    old_value: null, requested_value: null, raw_value_hex: null, request_hex: null, response_hex: null, exception: null,
    ack: null, readback: null, readback_value: null, note: null, ...extra,
  };
}

test("audit filter and command runs for the report", () => {
  const rows = [
    row(1, "arm"),
    row(2, "result", { command_id: "run", readback: "verified" }),
    row(3, "result", { command_id: "set_frequency", readback: "verified", request_hex: "02 06 00 01 0B B8 DB AD" }),
    row(4, "feedback", { command_id: "set_frequency", readback: "reached" }),
    row(5, "arm", { device_id: "temp" }),
  ];
  assert.equal(filterAudit(rows, { device: "vfd", event: "", text: "" }).length, 4);
  assert.equal(filterAudit(rows, { device: "", event: "result", text: "" }).length, 2);
  assert.deepEqual(filterAudit(rows, { device: "", event: "", text: "0b b8" }).map((r) => r.id), [3]);
  const runs = commandRuns(rows);
  assert.deepEqual(runs.map((r) => [r.result.command_id, r.feedback?.readback ?? null]), [["run", null], ["set_frequency", "reached"]]);
});
