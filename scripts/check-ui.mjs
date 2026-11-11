// UI safety check (ui.md "Security in the UI", safety rule 17): no HTML-string sinks in src/.
// Profile, device and user strings are rendered as text only. Exit 1 on failure.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../src/", import.meta.url));
const banned = [/\.innerHTML\b/, /\.outerHTML\b/, /insertAdjacentHTML/, /document\.write\b/, /\beval\s*\(/, /new\s+Function\s*\(/];

function files(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : /\.(ts|js|html)$/.test(p) ? [p] : [];
  });
}

const problems = [];
for (const f of files(root)) {
  readFileSync(f, "utf8")
    .split("\n")
    .forEach((line, i) => {
      for (const rx of banned) if (rx.test(line)) problems.push(`${f.slice(root.length)}:${i + 1}: ${rx}`);
    });
}
if (problems.length) {
  console.error(`UI check failed:\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log("UI check ok: no HTML-string sinks in src/");
