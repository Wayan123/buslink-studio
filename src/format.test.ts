// U4/U5: i18n key parity and locale number formatting (display only; exports come from Rust).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decimalsFor, fmtAge, fmtNumber } from "./format.ts";

test("U4 en.json and id.json have identical keys", () => {
  const en = JSON.parse(readFileSync(new URL("./i18n/en.json", import.meta.url), "utf8")) as Record<string, string>;
  const id = JSON.parse(readFileSync(new URL("./i18n/id.json", import.meta.url), "utf8")) as Record<string, string>;
  assert.deepEqual(Object.keys(en).sort(), Object.keys(id).sort());
});

test("U5 Indonesian uses a decimal comma, English a dot", () => {
  assert.equal(fmtNumber(25.3, "id", 1), "25,3");
  assert.equal(fmtNumber(25.3, "en", 1), "25.3");
  assert.equal(fmtNumber(null, "en"), "—");
  assert.equal(fmtNumber(Number.NaN, "id"), "—");
});

test("decimals follow the profile scale", () => {
  assert.equal(decimalsFor(0.1, undefined), 1);
  assert.equal(decimalsFor(0.01, undefined), 2);
  assert.equal(decimalsFor(undefined, undefined), 0);
  assert.equal(decimalsFor(0.1, 3), 3);
});

test("ages are compact", () => {
  assert.equal(fmtAge(800, "en"), "800 ms");
  assert.equal(fmtAge(12_400, "en"), "12 s");
  assert.equal(fmtAge(180_000, "en"), "3 min");
  assert.equal(fmtAge(null, "en"), "—");
});
