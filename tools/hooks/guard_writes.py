#!/usr/bin/env python3
"""PreToolUse guard for BusLink Studio file writes (stdlib only).

Kiro pipes the hook event as JSON on stdin:
  {"hook_event_name": "PreToolUse", "cwd": "...", "tool_name": "fs_write",
   "tool_input": {"path": "...", "text": "..."}}

Only the lines a write would ADD are inspected. The guard blocks (exit 2,
reason on stderr) when an edit:
  1. adds Modbus write function codes (FC05/06/15/16, FC22/23) or write API
     names (write_single_register, ...) in source code outside
     crates/buslink-core/src/command/**, crates/buslink-sim/** and tests;
  2. adds a write addressed to the broadcast unit ID 0 (anywhere except tests);
  3. adds something that looks like a secret (API key, token, MQTT password,
     private key). Generic `password = "..."` assignments are allowed in
     example/sample files; high-confidence key formats are blocked everywhere.

Parse problems fail open: a warning on stderr and exit 1 (Kiro shows the
warning and the tool still runs). See .kiro/steering/safety.md.
"""
from __future__ import annotations

import json
import os
import pathlib
import re
import sys

ROOT = pathlib.Path(os.environ.get("BUSLINK_ROOT") or pathlib.Path(__file__).resolve().parents[2])

WRITE_TOOLS = {"fs_write", "fs_append", "str_replace", "write", "fsWrite", "fsAppend", "strReplace", "create_file"}

CODE_EXT = {".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".kt", ".kts", ".java", ".html", ".vue", ".svelte"}

# Paths (relative, POSIX) where Modbus write code is allowed to live.
WRITE_CODE_ALLOWED = (
    re.compile(r"^crates/buslink-core/src/command/"),
    re.compile(r"^crates/buslink-sim/"),
)
TEST_PATH = re.compile(
    r"(^|/)tests?/"                       # crates/x/tests/..., src/test/...
    r"|(^|/)tests\.rs$"                   # src/foo/tests.rs
    r"|_tests?\.rs$"                      # codec_test.rs
    r"|\.(test|spec)\.[cm]?[jt]sx?$"      # ui.test.ts, x.spec.js
    r"|(^|/)src/(test|androidTest)/"      # Android unit/instrumented tests
)
# The hook scripts and their fixtures describe the patterns they detect.
SELF_PATH = re.compile(r"^tools/hooks/")

WRITE_NAME = re.compile(
    r"(?i)\b("
    r"masked?_?write_?registers?"
    r"|read_?write_?multiple_?registers"
    r"|write_?(single_?|multiple_?)?(holding_?)?(registers?|coils?)"
    r"|generate_set_(holdings?|coils?)\w*"          # rmodbus request builders
    r")\b"
)
FC_TOKEN = re.compile(r"(?i)\bfc[-_ ]?0?(5|6|15|16|22|23)\b")
FC_ASSIGN = re.compile(
    r"(?i)\b(function_?code|func_?code|function|func|fn_?code|fc)[\"']?\s*(==|:|=)\s*"
    r"(0x0?5|0x0?6|0x0?f|0x10|0x16|0x17|5|6|15|16|22|23)\b"
)
HEX_ARRAY_WRITE = re.compile(r"(?i)\[\s*0x[0-9a-f]{1,2}\s*,\s*0x(0?5|0?6|0?f|10|16|17)\s*,")
HEX_SEQ = re.compile(r"\b[0-9A-Fa-f]{2}(?: [0-9A-Fa-f]{2}){5,}\b")
WRITE_FC_BYTES = {0x05, 0x06, 0x0F, 0x10, 0x16, 0x17}

BROADCAST = re.compile(
    r"(?i)\b(unit|slave|server|device)(_?id|_?addr(ess)?)?[\"']?\s*(:|=(?!=))\s*0+\b(?![.x])"
    r"|\b(slave|unit_?id|unitid)\s*\(\s*0+\s*\)"
    r"|\bslave\s*::\s*broadcast\b"
    r"|\.broadcast\s*\("
    r"|\[\s*0x0{1,2}\s*,\s*0x(0?5|0?6|0?f|10|16|17)\s*,"
)

HIGH_CONFIDENCE_SECRETS = [
    ("AWS access key", re.compile(r"\b(AKIA|ASIA)[0-9A-Z]{16}\b")),
    ("private key block", re.compile(r"-----BEGIN ([A-Z0-9]+ )*PRIVATE KEY-----")),
    ("GitHub token", re.compile(r"\b(gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b")),
    ("Slack token", re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b")),
    ("Google API key", re.compile(r"\bAIza[0-9A-Za-z_\-]{35}\b")),
    ("API secret key (sk-...)", re.compile(r"\bsk-[A-Za-z0-9_\-]{20,}\b")),
]
GENERIC_SECRET = re.compile(
    r"(?i)\b[\w.-]*(api[_-]?key|secret|token|passw(or)?d|passwd|pwd|client[_-]?secret|"
    r"access[_-]?key|auth[_-]?key|private[_-]?key)[\w.-]*[\"']?\s*(:|=(?!=))\s*[\"']([^\"'\s]{8,})[\"']"
)
URL_CREDENTIALS = re.compile(r"(?i)\b(mqtts?|wss?|ssl|tcp|https?)://([^\s:/@\"']+):([^\s@/\"']{3,})@")
PLACEHOLDER = re.compile(
    r"(?i)^(<[^>]*>|\$\{[^}]*\}|\{\{[^}]*\}\}|\$[A-Z_][A-Z0-9_]*|%[A-Z_]+%|\*+|x{3,}|\.{3,})$"
    r"|example|placeholder|changeme|change_me|your[_-]|redacted|dummy|not[_-]?a[_-]?secret|fake|sample"
)
# Values like "lan.token_hint", "mqtt-password" or "Password" are identifiers, i18n keys or
# UI labels, not secrets. Real secrets carry digits, symbols or mixed case.
IDENTIFIER_VALUE = re.compile(r"^[a-z]+(?:[._-][a-z]+)+$|^[A-Za-z][a-z]+$")
EXAMPLE_FILE = re.compile(r"(?i)(^|[._-])(example|sample)s?([._-]|$)")


def warn_open(msg: str) -> int:
    print(f"guard-write-paths: WARNING ({msg}); allowing the write (fail-open).", file=sys.stderr)
    return 1


def rel_path(raw: str, cwd: str | None) -> str:
    p = raw.replace("\\", "/")
    path = pathlib.PurePosixPath(p)
    root = str(ROOT).replace("\\", "/")
    if path.is_absolute():
        s = str(path)
        for base in (root, (cwd or "").replace("\\", "/")):
            if base and (s == base or s.startswith(base.rstrip("/") + "/")):
                return s[len(base.rstrip("/")) + 1:]
        return s.lstrip("/")
    return str(path)  # PurePosixPath already drops a leading "./"


def first(d: dict, *keys: str):
    for k in keys:
        v = d.get(k)
        if isinstance(v, str):
            return v
    return None


def extract(tool: str, ti: dict, cwd: str | None) -> tuple[str, list[str], list[int]] | None:
    """Return (relative path, lines of the new text, indices of added lines)."""
    raw_path = first(ti, "path", "file_path", "filePath", "targetFile", "target_file", "file")
    if raw_path is None:
        return None
    rel = rel_path(raw_path, cwd)
    command = ti.get("command") if isinstance(ti.get("command"), str) else None
    new_str = first(ti, "newStr", "new_str", "new_string", "newString")
    old_str = first(ti, "oldStr", "old_str", "old_string", "oldString")
    text = first(ti, "text", "content", "file_text", "fileText", "contents")

    if tool in ("str_replace", "strReplace") or command == "str_replace" or (new_str is not None and old_str is not None):
        new_text, old_lines = new_str or "", (old_str or "").splitlines()
    elif tool in ("fs_append", "fsAppend") or command in ("append", "insert"):
        new_text, old_lines = (text if text is not None else new_str) or "", []
    else:
        new_text = text if text is not None else (new_str or "")
        old_lines = []
        target = ROOT / rel
        try:
            if target.is_file() and target.stat().st_size < 4_000_000:
                old_lines = target.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            old_lines = []
    lines = new_text.splitlines()
    old_set = {l.strip() for l in old_lines}
    added = [i for i, l in enumerate(lines) if l.strip() and l.strip() not in old_set]
    return rel, lines, added


def strip_comment(line: str, ext: str) -> str:
    s = line.strip()
    if ext == ".html":
        return re.sub(r"<!--.*?-->", "", line)
    if s.startswith(("*", "/*")) and not s.startswith("*/") and ext != ".py":
        return ""
    line = re.sub(r"/\*.*?\*/", "", line)
    # Drop // comments, but not the // inside URLs such as https://.
    return re.sub(r"(?<![:\"'\w])//.*$", "", line)


def has_write_indicator(code: str, raw: str) -> str | None:
    for rx, label in ((WRITE_NAME, "Modbus write API"), (FC_TOKEN, "write function code"),
                      (FC_ASSIGN, "write function code"), (HEX_ARRAY_WRITE, "raw write frame")):
        m = rx.search(code)
        if m:
            return f"{label} '{m.group(0).strip()}'"
    for m in HEX_SEQ.finditer(raw):
        parts = m.group(0).split()
        if int(parts[1], 16) in WRITE_FC_BYTES:
            return f"raw write frame '{m.group(0)}'"
    return None


def broadcast_hit(raw: str) -> str | None:
    m = BROADCAST.search(raw)
    if m:
        return m.group(0)
    for h in HEX_SEQ.finditer(raw):
        parts = h.group(0).split()
        if parts[0] == "00" and int(parts[1], 16) in WRITE_FC_BYTES:
            return h.group(0)
    return None


def check(rel: str, lines: list[str], added: list[int]) -> list[str]:
    problems: list[str] = []
    ext = pathlib.PurePosixPath(rel).suffix.lower()
    name = pathlib.PurePosixPath(rel).name
    is_test = bool(TEST_PATH.search(rel))
    is_self = bool(SELF_PATH.search(rel))
    write_ok_here = is_test or any(rx.search(rel) for rx in WRITE_CODE_ALLOWED)
    is_example = bool(EXAMPLE_FILE.search(name))

    if ext in CODE_EXT and not is_self:
        code_lines = [strip_comment(l, ext) for l in lines]
        for i in added:
            hit = has_write_indicator(code_lines[i], code_lines[i])
            if hit and not write_ok_here:
                problems.append(
                    f"{rel}:{i + 1}: adds {hit} outside the command service. Modbus writes must go through "
                    "crates/buslink-core/src/command/ (arming, bounds, confirm, read-back, audit); see safety.md.")
            if is_test:
                continue
            window = range(max(0, i - 3), min(len(lines), i + 4))
            if hit:
                b = next((broadcast_hit(code_lines[j]) for j in window if broadcast_hit(code_lines[j])), None)
                if b:
                    problems.append(f"{rel}:{i + 1}: write addressed to broadcast unit ID 0 ('{b.strip()}'). "
                                    "Broadcast writes are never allowed.")
            else:
                b = broadcast_hit(code_lines[i])
                if b and any(has_write_indicator(code_lines[j], code_lines[j]) for j in window if j != i):
                    problems.append(f"{rel}:{i + 1}: broadcast unit ID 0 ('{b.strip()}') next to a write. "
                                    "Broadcast writes are never allowed.")

    if not is_self:
        for i in added:
            line = lines[i]
            for label, rx in HIGH_CONFIDENCE_SECRETS:
                if rx.search(line):
                    problems.append(f"{rel}:{i + 1}: looks like a {label}. Secrets never go in project files; "
                                    "use the OS secret store or an environment variable.")
            if is_example:
                continue
            for m in GENERIC_SECRET.finditer(line):
                if not PLACEHOLDER.search(m.group(4)) and not IDENTIFIER_VALUE.match(m.group(4)):
                    problems.append(f"{rel}:{i + 1}: hard-coded credential in '{m.group(1)}'. Use the OS secret "
                                    "store, an environment variable, or a *.example file with a placeholder.")
                    break
            u = URL_CREDENTIALS.search(line)
            if u and not PLACEHOLDER.search(u.group(3)):
                problems.append(f"{rel}:{i + 1}: URL with embedded credentials ({u.group(1)}://user:***@...). "
                                "Keep broker credentials out of files.")
    return problems


def main() -> int:
    try:
        raw = sys.stdin.read()
    except Exception as e:  # pragma: no cover
        return warn_open(f"cannot read stdin: {e}")
    if not raw.strip():
        return warn_open("empty hook input")
    try:
        event = json.loads(raw)
    except json.JSONDecodeError as e:
        return warn_open(f"hook input is not JSON: {e}")
    if not isinstance(event, dict):
        return warn_open("hook input is not a JSON object")
    tool = str(event.get("tool_name") or event.get("toolName") or "")
    if tool not in WRITE_TOOLS:
        return 0
    ti = event.get("tool_input") or event.get("toolInput") or {}
    if isinstance(ti, str):
        try:
            ti = json.loads(ti)
        except json.JSONDecodeError:
            return warn_open("tool_input is not JSON")
    if not isinstance(ti, dict):
        return warn_open("tool_input is not an object")
    try:
        got = extract(tool, ti, event.get("cwd") or os.getcwd())
    except Exception as e:  # defensive: never break editing on a guard bug
        return warn_open(f"could not inspect tool input: {e}")
    if got is None:
        return warn_open("no file path in tool_input")
    try:
        problems = check(*got)
    except Exception as e:
        return warn_open(f"check failed: {e}")
    if problems:
        print("guard-write-paths BLOCKED this write:", file=sys.stderr)
        for p in problems[:20]:
            print(f"  - {p}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
