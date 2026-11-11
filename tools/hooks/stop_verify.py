#!/usr/bin/env python3
"""Stop hook: make sure the Rust workspace still compiles before Kiro finishes.

Runs `cargo check --workspace --quiet` at the repository root.
  - Compiles (or there is no Cargo.toml / no cargo yet): exit 0, no output,
    so the agent stops normally.
  - Fails: prints {"decision": "block", "reason": "<errors>"} on stdout and
    exits 0. For Stop hooks "block" means "do not stop yet": Kiro starts
    another turn with the reason, i.e. the compiler errors.
  - Loop guard: after 3 consecutive failures with the same errors in one
    session the hook stops forcing new turns and only warns on stderr.

Linux cargo is preferred (pure-Rust core crates); cargo.exe is the fallback,
with its own target dir so the two toolchains never share artifacts.
Overrides for self-tests: BUSLINK_ROOT, BUSLINK_STOP_VERIFY_CMD, BUSLINK_STATE_DIR.
Stdlib only.
"""
from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(os.environ.get("BUSLINK_ROOT") or pathlib.Path(__file__).resolve().parents[2])
MAX_REPEATS = 3
TIMEOUT_S = 540  # the hook itself has a 600 s timeout
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def command() -> list[str] | None:
    override = os.environ.get("BUSLINK_STOP_VERIFY_CMD")
    if override:
        return shlex.split(override)
    if shutil.which("cargo"):
        return ["cargo", "check", "--workspace", "--quiet"]
    if shutil.which("cargo.exe"):
        return ["cargo.exe", "check", "--workspace", "--quiet", "--target-dir", "target/windows"]
    return None


def errors_only(output: str, limit: int = 6000) -> str:
    """Keep compiler errors (with their context), drop warnings."""
    out = ANSI.sub("", output)
    blocks, cur, keep = [], [], False
    for line in out.splitlines():
        if re.match(r"^(error|warning)(\[|:)", line):
            if cur and keep:
                blocks.append("\n".join(cur))
            cur, keep = [line], line.startswith("error")
        else:
            cur.append(line)
    if cur and keep:
        blocks.append("\n".join(cur))
    text = "\n\n".join(blocks).strip() or out.strip()
    return text if len(text) <= limit else "...\n" + text[-limit:]


def state_file(session: str) -> pathlib.Path:
    base = pathlib.Path(os.environ.get("BUSLINK_STATE_DIR") or tempfile.gettempdir())
    safe = re.sub(r"[^A-Za-z0-9_.-]", "_", session or "nosession")[:80]
    return base / f"buslink-stop-verify-{safe}.json"


def main() -> int:
    session = ""
    try:
        raw = sys.stdin.read() if sys.stdin and not sys.stdin.isatty() else ""
        if raw.strip():
            ev = json.loads(raw)
            if isinstance(ev, dict):
                session = str(ev.get("session_id") or "")
    except (json.JSONDecodeError, OSError):
        pass

    if not (ROOT / "Cargo.toml").is_file() and not os.environ.get("BUSLINK_STOP_VERIFY_CMD"):
        return 0
    cmd = command()
    if cmd is None:
        print("stop-verify: cargo not found; skipped compile check", file=sys.stderr)
        return 0
    try:
        proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                              errors="replace", timeout=TIMEOUT_S)
    except subprocess.TimeoutExpired:
        print(f"stop-verify: '{' '.join(cmd)}' timed out after {TIMEOUT_S}s; not blocking", file=sys.stderr)
        return 0
    except OSError as e:
        print(f"stop-verify: could not run cargo ({e}); not blocking", file=sys.stderr)
        return 0

    sf = state_file(session)
    if proc.returncode == 0:
        try:
            sf.unlink()
        except OSError:
            pass
        return 0

    errors = errors_only((proc.stdout or "") + "\n" + (proc.stderr or ""))
    digest = hashlib.sha256(errors.encode()).hexdigest()[:16]
    try:
        st = json.loads(sf.read_text()) if sf.is_file() else {}
    except (OSError, json.JSONDecodeError):
        st = {}
    count = st.get("count", 0) + 1 if st.get("digest") == digest else 1
    try:
        sf.write_text(json.dumps({"digest": digest, "count": count}))
    except OSError:
        pass
    if count > MAX_REPEATS:
        print(f"stop-verify: cargo check still fails with the same errors after {MAX_REPEATS} attempts; "
              "not forcing another turn. Report the failure to the user.\n" + errors, file=sys.stderr)
        return 0
    reason = ("`cargo check --workspace` fails, so the work is not finished. Fix these compile errors "
              f"(attempt {count}/{MAX_REPEATS}), then stop again:\n\n{errors}")
    print(json.dumps({"decision": "block", "reason": reason}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
