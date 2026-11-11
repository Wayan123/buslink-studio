// i18n check (ui.md): en.json and id.json have the same keys, no empty strings, every
// key used in src/ exists, and the dynamic key families are complete. Exit 1 on failure.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../src/", import.meta.url));
const en = JSON.parse(readFileSync(join(root, "i18n/en.json"), "utf8"));
const id = JSON.parse(readFileSync(join(root, "i18n/id.json"), "utf8"));
const problems = [];

for (const k of Object.keys(en)) if (!(k in id)) problems.push(`id.json is missing "${k}"`);
for (const k of Object.keys(id)) if (!(k in en)) problems.push(`en.json is missing "${k}"`);
for (const [name, d] of [["en", en], ["id", id]]) {
  for (const [k, v] of Object.entries(d)) {
    if (typeof v !== "string" || v.trim() === "") problems.push(`${name}.json "${k}" is empty`);
  }
}
// Placeholders must match between languages.
const ph = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
for (const k of Object.keys(en)) if (k in id && ph(en[k]) !== ph(id[k])) problems.push(`placeholders differ for "${k}"`);

function files(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : p.endsWith(".ts") && !p.endsWith(".test.ts") ? [p] : [];
  });
}
for (const f of files(root)) {
  const src = readFileSync(f, "utf8");
  for (const m of src.matchAll(/\bt\("([A-Za-z0-9_.]+)"/g)) {
    if (!(m[1] in en)) problems.push(`${f.slice(root.length)}: unknown key "${m[1]}"`);
  }
}

const families = {
  "quality.": ["GOOD", "TIMEOUT", "CRC_ERROR", "EXCEPTION", "STALE", "DECODE_ERROR", "DISCONNECTED"],
  "link.state.": ["connected", "polling", "down", "disconnected"],
  "link.kind.": ["rtu", "tcp", "rtu_over_tcp", "simulator"],
  "support.": ["generic", "documented", "simulated", "hardware_verified"],
  "alarm.priority.": ["1", "2", "3"],
  "alarm.state.": ["NORMAL", "UNACKED", "ACKED", "RTN_UNACKED", "SHELVED", "SUPPRESSED", "OUT_OF_SERVICE"],
  "alarm.kind.": ["HI", "HI_HI", "LO", "LO_LO"],
  "export.": ["csv", "xlsx", "json"],
  "trend.range.": ["5m", "15m", "30m", "1h", "8h", "session"],
  "order.": ["ABCD", "CDAB", "BADC", "DCBA"],
  "table.": ["coil", "discrete_input", "input_register", "holding_register"],
  "exc.hint.": ["01", "02", "03", "04", "05", "06", "08", "0A", "0B", "other"],
  "train.fault.": ["timeout", "crc_error", "exception02", "stale", "wrong_order"],
  "train.fault_name.": ["timeout", "crc_error", "exception02", "stale", "wrong_order"],
  "train.unit.": ["1", "2", "3", "4", "other"],
};
for (const [prefix, items] of Object.entries(families)) {
  for (const i of items) if (!(`${prefix}${i}` in en)) problems.push(`missing dynamic key "${prefix}${i}"`);
}
for (const lvl of families["support."]) if (!(`support.${lvl}.help` in en)) problems.push(`missing "support.${lvl}.help"`);
for (const x of ["read", "timeout", "order", "stale", "alarm", "export"]) {
  for (const part of ["title", "steps", "learn"]) if (!(`train.ex.${x}.${part}` in en)) problems.push(`missing "train.ex.${x}.${part}"`);
}

if (problems.length) {
  console.error(`i18n check failed (${problems.length}):\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log(`i18n check ok: ${Object.keys(en).length} keys in en and id`);
