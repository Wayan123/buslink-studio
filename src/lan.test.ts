// LAN mode (lan-mqtt Req 2.5): the browser build has no write-capable view and a strict message parser.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { backoffMs, LAN_NAV, LAN_ROUTES, parseMessage, query } from "./lan.ts";
import { clockOffsetMs, nowMs, syncServerTime } from "./clock.ts";

test("the browser nav and routes are the read-only views only", () => {
  assert.deepEqual(
    LAN_NAV.map(([href]) => href),
    ["#/dashboard", "#/trends"],
  );
  assert.deepEqual([...LAN_ROUTES].sort(), ["dashboard", "device", "trends"]);
  for (const forbidden of ["connect", "control", "commission", "audit", "report", "alarms", "inspector", "training", "library", "share", "mqtt"]) {
    assert.ok(!LAN_ROUTES.includes(forbidden), forbidden);
  }
});

test("messages: only snapshot, frame and lagged with the right shape are accepted", () => {
  const snap = parseMessage(JSON.stringify({ type: "snapshot", data: { tags: [], links: [], profiles: [] } }));
  assert.equal(snap?.type, "snapshot");
  const frame = parseMessage(JSON.stringify({ type: "frame", data: { tags: [], points: [] } }));
  assert.equal(frame?.type, "frame");
  assert.deepEqual(parseMessage('{"type":"lagged","data":7}'), { type: "lagged", data: 7 });
  for (const bad of ["", "nope", "null", "[]", '{"type":"frame"}', '{"type":"frame","data":{}}', '{"type":"x","data":{"tags":[]}}', '{"type":"lagged","data":"7"}']) {
    assert.equal(parseMessage(bad), null, bad);
  }
});

test("requests never carry the token in the URL (the session cookie authenticates)", () => {
  assert.equal(query({ tag: "dev/t", from: 1 }), "?tag=dev%2Ft&from=1");
  assert.equal(query(), "");
  assert.ok(!/[?&]t=/.test(query({ tag: "dev/t" })));
});

test("reconnect backoff doubles up to 15 s", () => {
  assert.deepEqual([0, 1, 2, 3, 4, 5, 20].map(backoffMs), [1000, 2000, 4000, 8000, 15000, 15000, 15000]);
});

test("host time: ages use the host clock, however wrong the browser clock is", () => {
  syncServerTime(1_000_000, 4_000_000);
  assert.equal(clockOffsetMs(), -3_000_000);
  assert.ok(Math.abs(nowMs() - (Date.now() - 3_000_000)) < 50);
  syncServerTime(5_000, 5_000);
  assert.equal(clockOffsetMs(), 0);
});

test("the LAN transport has no way to send a body or use a method other than GET", () => {
  const src = readFileSync(new URL("./lan.ts", import.meta.url), "utf8");
  assert.ok(!/method:\s*["'](?!GET)/.test(src), "only GET");
  assert.ok(!/\bbody\s*:/.test(src), "no request body");
  assert.ok(!/\.send\(/.test(src), "the socket never sends");
  assert.ok(!/XMLHttpRequest|sendBeacon/.test(src));
  // Views reachable in LAN mode never call a write-capable api function.
  for (const f of ["views/dashboard.ts", "views/trends.ts"]) {
    const v = readFileSync(new URL(`./${f}`, import.meta.url), "utf8");
    const calls = [...v.matchAll(/\bapi\.(\w+)\(/g)].map((m) => m[1]);
    for (const c of calls) assert.ok(["trend", "exportSession"].includes(c ?? ""), `${f} uses api.${c}`);
  }
  // The one write-capable call in the dashboard (export) sits behind the LAN switch.
  const dash = readFileSync(new URL("./views/dashboard.ts", import.meta.url), "utf8");
  assert.ok(dash.includes("lan ? null : exportBar()"));
  assert.ok(readdirSync(new URL("./views/", import.meta.url)).includes("share.ts"));
});
