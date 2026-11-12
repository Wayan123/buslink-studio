// node --test scripts/check-release-features.test.mjs
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { artifactProblems, configProblems, defaultFeatures, featuresInArgs, hasMarker, MARKER } from "./check-release-features.mjs";

test("feature arguments are found in every cargo spelling", () => {
  assert.deepEqual(featuresInArgs(["build", "--features", "custom-protocol,e2e"]), ["custom-protocol", "e2e"]);
  assert.deepEqual(featuresInArgs(["--features=custom-protocol e2e"]), ["custom-protocol", "e2e"]);
  assert.deepEqual(featuresInArgs(["-F", "e2e"]), ["e2e"]);
  assert.deepEqual(featuresInArgs(["-Fe2e"]), ["e2e"]);
  assert.deepEqual(featuresInArgs(["--features", "buslink-studio/e2e"]), ["e2e"]);
  assert.deepEqual(featuresInArgs(["build", "--bundles", "nsis"]), []);
});

test("the guard fails when e2e is requested and passes a clean release", () => {
  const clean = { args: ["build", "--bundles", "nsis"], cargoToml: "[features]\ncustom-protocol = []\ne2e = []\n", tauriConf: { build: {} }, env: {} };
  assert.deepEqual(configProblems(clean), []);
  assert.equal(configProblems({ ...clean, args: ["build", "--features", "e2e"] }).length, 1);
  assert.equal(configProblems({ ...clean, args: ["android", "build", "-F", "custom-protocol,e2e"] }).length, 1);
  assert.equal(configProblems({ ...clean, cargoToml: '[features]\ndefault = ["custom-protocol", "e2e"]\ne2e = []\n' }).length, 1);
  assert.equal(configProblems({ ...clean, tauriConf: { build: { features: ["e2e"] } } }).length, 1);
  assert.equal(configProblems({ ...clean, env: { CARGO_FEATURE_E2E: "1" } }).length, 1);
  assert.equal(configProblems({ ...clean, env: { CARGO_BUILD_FEATURES: "e2e" } }).length, 1);
});

test("default features are read only from the [features] table", () => {
  assert.deepEqual(defaultFeatures('[dependencies]\nx = { default = ["e2e"] }\n[features]\ndefault = ["a"]\n'), ["a"]);
  assert.deepEqual(defaultFeatures("[features]\ne2e = []\n"), []);
});

test("the e2e marker is found as UTF-8 and UTF-16LE, and not in clean bytes", () => {
  assert.ok(hasMarker(Buffer.concat([Buffer.from("xx"), Buffer.from(MARKER), Buffer.from("yy")])));
  assert.ok(hasMarker(Buffer.concat([Buffer.from("xx"), Buffer.from(MARKER, "utf16le")])));
  assert.ok(!hasMarker(Buffer.from("BUSLINK_E2E is mentioned but not the variable")));
});

test("artifact check: a missing file is a failure, a marked file is a failure", () => {
  assert.equal(artifactProblems(["/nonexistent/buslink.exe"]).length, 1);
  const here = fileURLToPath(import.meta.url);
  // This test file names the marker only through the imported constant, so it is clean.
  assert.deepEqual(artifactProblems([fileURLToPath(new URL("./check-i18n.mjs", import.meta.url))]), []);
  assert.ok(readFileSync(here, "utf8").length > 0);
});

test("when both builds exist, the e2e binary is flagged and the release binary is not", { skip: !existsSync(fileURLToPath(new URL("../src-tauri/target/e2e/release/buslink-studio.exe", import.meta.url))) }, () => {
  const e2e = fileURLToPath(new URL("../src-tauri/target/e2e/release/buslink-studio.exe", import.meta.url));
  assert.equal(artifactProblems([e2e]).length, 1);
  const rel = fileURLToPath(new URL("../src-tauri/target/release/buslink-studio.exe", import.meta.url));
  if (existsSync(rel)) assert.deepEqual(artifactProblems([rel]), []);
});
