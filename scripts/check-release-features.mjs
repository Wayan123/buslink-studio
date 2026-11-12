// Release guard (security review 07, info item): the `e2e` Cargo feature must never reach an
// installer or an APK. It is a verification-only feature: it exports into $BUSLINK_E2E_EXPORT_DIR
// instead of the native save dialog and allows loopback LAN sharing.
//
// Two checks, both fail with exit code 1:
//   config    nothing selects the feature: no `--features e2e` / `-F e2e` among the build arguments,
//             not in Cargo.toml's `default` features, not in tauri.conf.json `build.features`,
//             no CARGO_FEATURE_E2E / CARGO_ENCODED... style environment.
//   artifact  the compiled binary does not contain the e2e-only string BUSLINK_E2E_EXPORT_DIR
//             (UTF-8 or UTF-16LE). The string exists only under #[cfg(feature = "e2e")].
//
// Usage:
//   node scripts/check-release-features.mjs [--args <tauri build arguments...>]   config check
//   node scripts/check-release-features.mjs --artifact [--optional] <file...>     binary check
// `scripts/release.mjs` runs both around `tauri build`; tauri.conf.json runs the binary check as
// `beforeBundleCommand`, so a plain `npm run tauri -- build` is covered too.
//
// Limits: the binary check reads the unbundled executable or shared library. A compressed
// installer or APK cannot be scanned reliably, so it is checked before bundling.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const FEATURE = "e2e";
export const MARKER = "BUSLINK_E2E_EXPORT_DIR";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** Features named in cargo/tauri arguments: `--features a,b`, `--features=a b`, `-F a`, `-Fa`, `--features "x y"`. */
export function featuresInArgs(args) {
  const out = [];
  const split = (v) => v.split(/[\s,]+/).filter(Boolean);
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--features" || a === "-F" || a === "-f") {
      if (i + 1 < args.length) out.push(...split(args[++i]));
    } else if (a.startsWith("--features=")) out.push(...split(a.slice("--features=".length)));
    else if (a.startsWith("-F") && a.length > 2) out.push(...split(a.slice(2)));
  }
  // `crate/feature` form
  return out.map((f) => f.split("/").pop());
}

/** Feature names listed in `default = [...]` of the [features] table. */
export function defaultFeatures(cargoToml) {
  const m = /^\[features\][^\[]*?^default\s*=\s*\[([^\]]*)\]/ms.exec(cargoToml);
  if (!m) return [];
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** Everything in the repo and the environment that would turn the feature on. Empty list = clean. */
export function configProblems({ args = [], cargoToml = "", tauriConf = {}, env = {} } = {}) {
  const problems = [];
  if (featuresInArgs(args).includes(FEATURE)) problems.push(`build arguments enable the "${FEATURE}" feature`);
  if (defaultFeatures(cargoToml).some((f) => f === FEATURE || f.endsWith(`/${FEATURE}`))) {
    problems.push(`src-tauri/Cargo.toml lists "${FEATURE}" in its default features`);
  }
  const confFeatures = tauriConf?.build?.features;
  if (Array.isArray(confFeatures) && confFeatures.includes(FEATURE)) problems.push(`tauri.conf.json build.features contains "${FEATURE}"`);
  if (env.CARGO_FEATURE_E2E !== undefined && env.CARGO_FEATURE_E2E !== "") problems.push("CARGO_FEATURE_E2E is set in the environment");
  for (const k of ["CARGO_BUILD_FEATURES", "CARGO_ARGS", "TAURI_FEATURES"]) {
    if (featuresInArgs(["--features", env[k] ?? ""]).includes(FEATURE)) problems.push(`${k} enables "${FEATURE}"`);
  }
  return problems;
}

/** True when the bytes contain the e2e-only string, as UTF-8 or UTF-16LE. */
export function hasMarker(bytes) {
  const buf = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  return buf.includes(Buffer.from(MARKER, "utf8")) || buf.includes(Buffer.from(MARKER, "utf16le"));
}

export function artifactProblems(files, { optional = false } = {}) {
  const problems = [];
  for (const f of files) {
    if (!existsSync(f)) {
      if (!optional) problems.push(`${f}: not found, cannot verify that it is a release build`);
      continue;
    }
    if (hasMarker(readFileSync(f))) problems.push(`${f}: contains ${MARKER}, so it was built with --features ${FEATURE}`);
  }
  return problems;
}

export function checkConfig(args = []) {
  const read = (p) => (existsSync(p) ? readFileSync(p, "utf8") : "");
  let tauriConf = {};
  try {
    tauriConf = JSON.parse(read(join(ROOT, "src-tauri", "tauri.conf.json")) || "{}");
  } catch {
    tauriConf = {};
  }
  return configProblems({ args, cargoToml: read(join(ROOT, "src-tauri", "Cargo.toml")), tauriConf, env: process.env });
}

function main(argv) {
  const i = argv.indexOf("--artifact");
  let problems;
  if (i >= 0) {
    // --optional: skip files that do not exist (used by tauri.conf.json beforeBundleCommand, which
    // must not break builds that produce a different binary path, such as the Android build).
    const optional = argv.includes("--optional");
    const files = argv
      .slice(i + 1)
      .filter((f) => f !== "--optional")
      .map((f) => resolve(ROOT, f));
    if (files.length === 0) problems = ["--artifact needs at least one file"];
    else problems = artifactProblems(files, { optional });
  } else {
    const j = argv.indexOf("--args");
    problems = checkConfig(j >= 0 ? argv.slice(j + 1) : []);
  }
  if (problems.length) {
    console.error(`release guard FAILED:\n${problems.map((p) => `  - ${p}`).join("\n")}`);
    console.error(`The "${FEATURE}" feature is for verification builds only and must not ship in an installer or APK.`);
    return 1;
  }
  console.log(`release guard ok (${i >= 0 ? "binary has no e2e code" : "no e2e feature selected"})`);
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));
