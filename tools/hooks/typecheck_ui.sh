#!/usr/bin/env sh
# PostFileSave hook (src/**/*.ts|css|html): type-check the web UI with `npx tsc --noEmit`.
# Always exits 0: a missing toolchain prints a hint instead of failing the save.
cd "$(dirname "$0")/../.." || exit 0

if [ ! -f tsconfig.json ]; then
  echo "typecheck-ui: no tsconfig.json yet; skipped."
  exit 0
fi
if [ ! -f node_modules/typescript/bin/tsc ]; then
  echo "typecheck-ui: node_modules/typescript is missing. Run 'npm ci' (WSL) or 'cmd.exe /c npm.cmd ci' (Windows toolchain), then save again."
  exit 0
fi
if ! command -v npx >/dev/null 2>&1; then
  echo "typecheck-ui: npx is not on PATH; skipped."
  exit 0
fi

npx tsc --noEmit 2>&1 | tail -n 40
exit 0
