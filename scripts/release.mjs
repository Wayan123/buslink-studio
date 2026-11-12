// Release builds with the e2e guard around them (security review 07).
//   node scripts/release.mjs windows [extra tauri args]   -> tauri build --bundles nsis
//   node scripts/release.mjs android [extra tauri args]   -> tauri android build --apk --target aarch64
// Fails before building if `--features e2e` (or anything else that selects it) is present, and after
// building if the compiled library contains e2e code. Run it with Windows node (tauri needs the
// Windows toolchain, see .kiro/steering/tech.md).
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { artifactProblems, checkConfig } from "./check-release-features.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const [target, ...extra] = process.argv.slice(2);

const PLANS = {
  windows: {
    args: ["build", "--bundles", "nsis"],
    // The exe is also checked by tauri.conf.json beforeBundleCommand, before the installer exists.
    artifacts: [join("src-tauri", "target", "release", "buslink-studio.exe")],
  },
  android: {
    args: ["android", "build", "--apk", "--target", "aarch64"],
    artifacts: [join("src-tauri", "gen", "android", "app", "src", "main", "jniLibs", "arm64-v8a", "libbuslink_studio_lib.so")],
  },
};

const plan = PLANS[target];
if (!plan) {
  console.error("usage: node scripts/release.mjs <windows|android> [extra tauri args]");
  process.exit(2);
}
const fail = (problems) => {
  console.error(`release guard FAILED:\n${problems.map((p) => `  - ${p}`).join("\n")}`);
  process.exit(1);
};

const args = [...plan.args, ...extra];
const before = checkConfig(args);
if (before.length) fail(before);

const tauri = join(ROOT, "node_modules", "@tauri-apps", "cli", "tauri.js");
const run = spawnSync(process.execPath, [tauri, ...args], { cwd: ROOT, stdio: "inherit" });
if (run.status !== 0) process.exit(run.status ?? 1);

const after = artifactProblems(plan.artifacts.map((a) => join(ROOT, a)));
if (after.length) fail(after);
console.log("release build finished; the guard found no e2e code in the compiled binary");
