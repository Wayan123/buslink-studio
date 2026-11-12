#!/usr/bin/env python3
"""Validate BusLink Studio device profiles (schema v1), stdlib only.

Usage:
  python3 tools/hooks/check_profile.py profiles/soil/bgt-smps-8in1.json [more.json ...]
  echo '{"file_path": "profiles/x.json"}' | python3 tools/hooks/check_profile.py   # hook stdin form

The schema is described in docs/profiles/schema-v1.md. Keys are strict
(unknown keys are errors), so a profile cannot smuggle in scripts or
expressions: profiles are data, never code.

Exit codes: 0 = all valid (warnings allowed), 1 = at least one invalid file.
A path outside profiles/ or a non-.json file is skipped with exit 0.
"""
from __future__ import annotations

import copy
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
MAX_BYTES = 1_000_000

LEVELS = ("generic", "documented", "simulated", "hardware_verified")
CAP_LEVELS = LEVELS + ("unsupported",)
TABLES = ("coil", "discrete_input", "input_register", "holding_register")
TYPE_WIDTH = {"bool": 1, "u16": 1, "i16": 1, "bitfield": 1, "u32": 2, "i32": 2, "f32": 2,
              "u64": 4, "i64": 4, "f64": 4, "string": None}
WORD_ORDERS = ("ABCD", "CDAB", "BADC", "DCBA")
STD_BAUD = (300, 600, 1200, 2400, 4800, 9600, 14400, 19200, 38400, 57600, 115200, 230400, 460800, 921600)
ROLES = ("operator", "commissioner")
RISKS = ("low", "medium", "high")
COMMAND_KINDS = ("run", "stop", "setpoint", "reset_fault", "mode")
APPLY = ("immediate", "after_reboot", "unknown")
MAX_REGS_PER_READ = 125     # FC03/04 limit
MAX_BITS_PER_READ = 2000    # FC01/02 limit

# All patterns are ASCII-only and used with fullmatch(), so a trailing newline
# or a non-ASCII digit never passes (the Rust validator in
# crates/buslink-core/src/profile/validate.rs implements the same rules).
ID_RX = re.compile(r"[a-z][a-z0-9_]{0,47}", re.ASCII)
PROFILE_ID_RX = re.compile(r"[a-z0-9]+(?:[._-][a-z0-9]+)*", re.ASCII)
SEMVER_RX = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+", re.ASCII)
DATE_RX = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", re.ASCII)
BIT_RX = re.compile(r"0|[1-9][0-9]?", re.ASCII)
DIGITS_RX = re.compile(r"[0-9]+", re.ASCII)
URL_RX = re.compile(r"https?://\S+")
CONTROL_RX = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")
INT_MIN, INT_MAX = -(2 ** 63), 2 ** 64 - 1  # JSON integers outside this range are rejected

TOP_KEYS = {"$schema", "$comment", "schema_version", "profile_id", "profile_version", "vendor", "model",
            "category", "firmware", "description", "support_level", "capabilities", "simulated_only",
            "source", "evidence", "transport", "registers", "commands", "comm_config", "notes"}
REQUIRED_TOP = ("schema_version", "profile_id", "profile_version", "vendor", "model", "support_level",
                "transport", "registers")
REG_KEYS = {"id", "name", "description", "table", "address", "doc_address", "type", "length", "word_order",
            "scale", "offset", "unit", "decimals", "valid_range", "display_range", "invalid_sentinels", "bits", "access",
            "write", "poll", "source_page"}
WRITE_KEYS = {"min", "max", "step", "allowed_values", "risk", "role", "confirm", "readback"}
SOURCE_KEYS = {"document", "revision", "url", "pages", "retrieved", "publisher"}
HW_KEYS = {"date", "adapter", "platform", "firmware", "unit_id", "record", "result", "notes"}
RTU_KEYS = {"unit_id", "baud", "data_bits", "parity", "stop_bits", "response_timeout_ms", "inter_frame_ms"}
TCP_KEYS = {"unit_id", "port", "response_timeout_ms"}
CMD_KEYS = {"id", "kind", "label", "register", "value", "readback_register", "expected", "timeout_ms", "feedback"}
FEEDBACK_KEYS = {"register", "tolerance", "timeout_ms", "value", "bit"}
CAP_KEYS = {"read", "write", "comm_config", "control"}


class Report:
    def __init__(self) -> None:
        self.errors: list[str] = []
        self.warnings: list[str] = []

    def err(self, path: str, msg: str) -> None:
        self.errors.append(f"{path}: {msg}")

    def warn(self, path: str, msg: str) -> None:
        self.warnings.append(f"{path}: {msg}")


def _no_dupes(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ValueError(f"duplicate key '{k}'")
        out[k] = v
    return out


def is_int(v) -> bool:
    return isinstance(v, int) and not isinstance(v, bool) and INT_MIN <= v <= INT_MAX


def is_num(v) -> bool:
    if isinstance(v, float):
        return v == v and v not in (float("inf"), float("-inf"))
    return is_int(v)


def same(a, b) -> bool:
    """Equal and of the same JSON type: 8.0 is not 8 and true is not 1."""
    return type(a) is type(b) and a == b


def matches(rx: re.Pattern, v) -> bool:
    return isinstance(v, str) and rx.fullmatch(v) is not None


def keys(r: Report, path: str, obj: dict, allowed: set[str]) -> None:
    for k in obj:
        if k not in allowed:
            r.err(f"{path}.{k}", "unknown key (profiles are strict data; no scripts or expressions)")


def text(r: Report, path: str, v, *, max_len: int = 200, required: bool = True, bilingual: bool = True) -> None:
    if v is None:
        if required:
            r.err(path, "required")
        return
    if isinstance(v, dict) and bilingual:  # {"en": ..., "id": ...}; the values are plain strings
        keys(r, path, v, {"en", "id"})
        for lang in ("en", "id"):
            if lang not in v:
                r.err(f"{path}.{lang}", "required when the text is bilingual")
        for lang, s in v.items():
            text(r, f"{path}.{lang}", s, max_len=max_len, bilingual=False)
        return
    if not isinstance(v, str) or not v.strip():
        r.err(path, "must be a non-empty string or {\"en\", \"id\"}")
        return
    if len(v) > max_len:
        r.err(path, f"longer than {max_len} characters")
    if CONTROL_RX.search(v):
        r.err(path, "contains control characters")
    if v[0] in "=+@\t\r":
        r.warn(path, "starts with a spreadsheet formula character; exports must escape it")


def enum(r: Report, path: str, v, allowed, required: bool = True) -> bool:
    if v is None:
        if required:
            r.err(path, f"required, one of {list(allowed)}")
        return False
    if not any(same(v, a) for a in allowed):
        r.err(path, f"'{v}' is not one of {list(allowed)}")
        return False
    return True


def int_range(r: Report, path: str, v, lo: int, hi: int, required: bool = True) -> bool:
    if v is None:
        if required:
            r.err(path, "required")
        return False
    if not is_int(v) or not lo <= v <= hi:
        r.err(path, f"must be an integer {lo}..{hi}")
        return False
    return True


def check_source(r: Report, path: str, src) -> None:
    if not isinstance(src, dict):
        r.err(path, "must be an object")
        return
    keys(r, path, src, SOURCE_KEYS)
    text(r, f"{path}.document", src.get("document"))
    text(r, f"{path}.revision", src.get("revision"), max_len=80)
    url = src.get("url")
    if url is not None and not matches(URL_RX, url):
        r.err(f"{path}.url", "must be an http(s) URL")
    if src.get("pages") is not None:
        text(r, f"{path}.pages", src.get("pages"), max_len=80)
    if src.get("publisher") is not None:
        text(r, f"{path}.publisher", src.get("publisher"), max_len=120)
    if src.get("retrieved") is not None and not matches(DATE_RX, src["retrieved"]):
        r.err(f"{path}.retrieved", "must be YYYY-MM-DD")


def check_transport(r: Report, path: str, t) -> None:
    if not isinstance(t, dict) or not t:
        r.err(path, "must be an object with 'rtu' and/or 'tcp'")
        return
    keys(r, path, t, {"rtu", "tcp", "simulator"})
    if not ({"rtu", "tcp", "simulator"} & set(t)):
        r.err(path, "needs at least one of rtu, tcp, simulator")
    rtu = t.get("rtu")
    if rtu is not None:
        p = f"{path}.rtu"
        if not isinstance(rtu, dict):
            r.err(p, "must be an object")
        else:
            keys(r, p, rtu, RTU_KEYS)
            int_range(r, f"{p}.unit_id", rtu.get("unit_id"), 1, 247)  # 0 is broadcast: never a device default
            baud = rtu.get("baud")
            if int_range(r, f"{p}.baud", baud, 300, 3_000_000) and baud not in STD_BAUD:
                r.warn(f"{p}.baud", f"{baud} is non-standard; Android CDC drivers may reject it")
            enum(r, f"{p}.data_bits", rtu.get("data_bits"), (7, 8))
            enum(r, f"{p}.parity", rtu.get("parity"), ("none", "even", "odd"))
            enum(r, f"{p}.stop_bits", rtu.get("stop_bits"), (1, 2))
            int_range(r, f"{p}.response_timeout_ms", rtu.get("response_timeout_ms"), 50, 10_000, required=False)
            int_range(r, f"{p}.inter_frame_ms", rtu.get("inter_frame_ms"), 0, 1_000, required=False)
    tcp = t.get("tcp")
    if tcp is not None:
        p = f"{path}.tcp"
        if not isinstance(tcp, dict):
            r.err(p, "must be an object")
        else:
            keys(r, p, tcp, TCP_KEYS)
            int_range(r, f"{p}.unit_id", tcp.get("unit_id"), 1, 255)
            int_range(r, f"{p}.port", tcp.get("port"), 1, 65535)
            int_range(r, f"{p}.response_timeout_ms", tcp.get("response_timeout_ms"), 50, 10_000, required=False)
    sim = t.get("simulator")
    if sim is not None and not (isinstance(sim, dict) and set(sim) <= {"model"} and matches(ID_RX, sim.get("model"))):
        r.err(f"{path}.simulator", "must be {\"model\": \"<simulator id>\"}")


def check_register(r: Report, path: str, reg, ids: set[str]) -> dict | None:
    if not isinstance(reg, dict):
        r.err(path, "must be an object")
        return None
    keys(r, path, reg, REG_KEYS)
    rid = reg.get("id")
    if not matches(ID_RX, rid):
        r.err(f"{path}.id", "must match ^[a-z][a-z0-9_]{0,47}$")
    elif rid in ids:
        r.err(f"{path}.id", f"duplicate register id '{rid}'")
    else:
        ids.add(rid)
    text(r, f"{path}.name", reg.get("name"), max_len=80)
    if reg.get("description") is not None:
        text(r, f"{path}.description", reg.get("description"), max_len=500)
    table_ok = enum(r, f"{path}.table", reg.get("table"), TABLES)
    addr_ok = int_range(r, f"{path}.address", reg.get("address"), 0, 65535)
    da = reg.get("doc_address")
    if da is not None and not (is_int(da) or (isinstance(da, str) and 0 < len(da) <= 16)):
        r.err(f"{path}.doc_address", "must be the manual's label as a short string or integer")
    typ = reg.get("type")
    type_ok = enum(r, f"{path}.type", typ, tuple(TYPE_WIDTH))
    table = reg.get("table")
    width = None
    if type_ok:
        width = TYPE_WIDTH[typ]
        if typ == "string":
            if int_range(r, f"{path}.length", reg.get("length"), 1, MAX_REGS_PER_READ):
                width = reg["length"]
        elif reg.get("length") is not None:
            r.err(f"{path}.length", "only allowed for type 'string'")
        if table_ok:
            bit_table = table in ("coil", "discrete_input")
            if bit_table and typ != "bool":
                r.err(f"{path}.type", f"table '{table}' holds single bits; type must be 'bool'")
            if not bit_table and typ == "bool":
                r.err(f"{path}.type", "a register is 16 bits; use 'bitfield' with 'bits' for flags")
        wo = reg.get("word_order")
        if typ in ("u32", "i32", "f32", "u64", "i64", "f64"):
            enum(r, f"{path}.word_order", wo, WORD_ORDERS)
        elif typ == "string":
            if wo is not None:
                enum(r, f"{path}.word_order", wo, ("ABCD", "BADC"))
        elif wo is not None:
            r.err(f"{path}.word_order", f"not meaningful for single-register type '{typ}'")
        if typ != "bitfield" and reg.get("bits") is not None:
            r.err(f"{path}.bits", "only allowed for type 'bitfield'")
    bits = reg.get("bits")
    if bits is not None and isinstance(bits, dict):
        for b, label in bits.items():
            if not (matches(BIT_RX, b) and int(b) <= 15):
                r.err(f"{path}.bits.{b}", "bit index must be \"0\"..\"15\"")
            text(r, f"{path}.bits.{b}", label, max_len=80)
    elif bits is not None:
        r.err(f"{path}.bits", "must be an object of bit index -> label")
    if addr_ok and width is not None and reg["address"] + width - 1 > 65535:
        r.err(f"{path}.address", f"address {reg['address']} + {width} registers runs past 65535")

    scale = reg.get("scale", 1)
    if not is_num(scale) or scale == 0:
        r.err(f"{path}.scale", "must be a non-zero finite number")
    if reg.get("offset") is not None and not is_num(reg["offset"]):
        r.err(f"{path}.offset", "must be a finite number")
    if reg.get("unit") is not None and (not isinstance(reg["unit"], str) or len(reg["unit"]) > 16):
        r.err(f"{path}.unit", "must be a string of at most 16 characters ('' for dimensionless)")
    int_range(r, f"{path}.decimals", reg.get("decimals"), 0, 9, required=False)
    vr = reg.get("valid_range")
    if vr is not None and not (isinstance(vr, list) and len(vr) == 2 and all(is_num(x) for x in vr) and vr[0] < vr[1]):
        r.err(f"{path}.valid_range", "must be [min, max] with min < max (engineering units)")
    dr = reg.get("display_range")
    if dr is not None and not (isinstance(dr, list) and len(dr) == 2 and all(is_num(x) for x in dr) and dr[0] < dr[1]):
        r.err(f"{path}.display_range", "must be [min, max] with min < max (engineering units)")
    sen = reg.get("invalid_sentinels")
    if sen is not None and not (isinstance(sen, list) and all(is_int(x) for x in sen)):
        r.err(f"{path}.invalid_sentinels", "must be a list of raw integers")
    if reg.get("poll") is not None:
        enum(r, f"{path}.poll", reg.get("poll"), ("fast", "normal", "slow"))
    if reg.get("source_page") is not None:
        text(r, f"{path}.source_page", reg.get("source_page"), max_len=40)

    access = reg.get("access", "read")
    enum(r, f"{path}.access", access, ("read", "read_write"))
    w = reg.get("write")
    if access == "read_write":
        if table_ok and table not in ("coil", "holding_register"):
            r.err(f"{path}.access", f"table '{table}' is read-only in Modbus")
        if not isinstance(w, dict):
            r.err(f"{path}.write", "required for read_write registers (bounds, risk, role)")
        else:
            keys(r, f"{path}.write", w, WRITE_KEYS)
            enum(r, f"{path}.write.risk", w.get("risk"), RISKS)
            enum(r, f"{path}.write.role", w.get("role"), ROLES)
            if w.get("confirm", True) is not True:
                r.err(f"{path}.write.confirm", "explicit confirmation cannot be disabled")
            if w.get("readback", True) is not True:
                r.err(f"{path}.write.readback", "read-back verification cannot be disabled")
            av = w.get("allowed_values")
            has_bounds = "min" in w or "max" in w
            if av is not None:
                if not (isinstance(av, list) and av and all(is_num(x) for x in av)):
                    r.err(f"{path}.write.allowed_values", "must be a non-empty list of numbers")
            elif typ == "bool":
                pass
            elif not has_bounds:
                r.err(f"{path}.write", "numeric writes need min and max, or allowed_values")
            if has_bounds:
                lo, hi = w.get("min"), w.get("max")
                if not (is_num(lo) and is_num(hi) and lo <= hi):
                    r.err(f"{path}.write", "min and max must both be numbers with min <= max")
                elif vr is not None and isinstance(vr, list) and len(vr) == 2 and all(is_num(x) for x in vr):
                    if lo < vr[0] or hi > vr[1]:
                        r.err(f"{path}.write", "write bounds must lie inside valid_range")
            if w.get("step") is not None and not (is_num(w["step"]) and w["step"] > 0):
                r.err(f"{path}.write.step", "must be a positive number")
    elif w is not None:
        r.err(f"{path}.write", "only allowed when access is 'read_write'")
    if width is None or not table_ok or not addr_ok:
        return None
    return {"id": rid if matches(ID_RX, rid) else None, "table": table, "start": reg["address"],
            "end": reg["address"] + width - 1,
            "writable": access == "read_write"}


def check_capabilities(r: Report, caps) -> dict:
    if caps is None:
        return {}
    if not isinstance(caps, dict):
        r.err("capabilities", "must be an object")
        return {}
    keys(r, "capabilities", caps, CAP_KEYS)
    for k, v in caps.items():
        if k in CAP_KEYS:
            enum(r, f"capabilities.{k}", v, CAP_LEVELS)
    return caps


def check_hardware_evidence(r: Report, ev) -> int:
    """Return the number of passing hardware test records."""
    if ev is None:
        return 0
    if not isinstance(ev, dict):
        r.err("evidence", "must be an object")
        return 0
    keys(r, "evidence", ev, {"hardware"})
    hw = ev.get("hardware", [])
    if not isinstance(hw, list):
        r.err("evidence.hardware", "must be a list")
        return 0
    passing = 0
    for i, e in enumerate(hw):
        p = f"evidence.hardware[{i}]"
        if not isinstance(e, dict):
            r.err(p, "must be an object")
            continue
        keys(r, p, e, HW_KEYS)
        if not matches(DATE_RX, e.get("date")):
            r.err(f"{p}.date", "required, YYYY-MM-DD")
        for k in ("adapter", "platform", "record"):
            text(r, f"{p}.{k}", e.get(k), max_len=200)
        if e.get("firmware") is not None:
            text(r, f"{p}.firmware", e.get("firmware"), max_len=80)
        if e.get("notes") is not None:
            text(r, f"{p}.notes", e.get("notes"), max_len=2000)
        if e.get("unit_id") is not None:
            int_range(r, f"{p}.unit_id", e.get("unit_id"), 1, 255)
        if enum(r, f"{p}.result", e.get("result"), ("pass", "fail", "partial")) and e["result"] == "pass":
            passing += 1
    return passing


def strip_nulls(r: Report, obj, path: str = "") -> None:
    """null is never a value in a profile: report it and treat the key as absent."""
    if isinstance(obj, dict):
        for k in list(obj):
            p = f"{path}.{k}" if path else k
            if obj[k] is None:
                r.err(p, "null is not allowed; omit the key")
                del obj[k]
            else:
                strip_nulls(r, obj[k], p)
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            strip_nulls(r, v, f"{path}[{i}]")


def validate(doc) -> Report:
    r = Report()
    if not isinstance(doc, dict):
        r.err("$", "profile must be a JSON object")
        return r
    doc = copy.deepcopy(doc)
    strip_nulls(r, doc)
    keys(r, "$", doc, TOP_KEYS)
    for k in ("$schema", "$comment"):
        if k in doc and not (isinstance(doc[k], str) and len(doc[k]) <= 2000):
            r.err(k, "must be a string")
    for k in REQUIRED_TOP:
        if k not in doc:
            r.err(k, "required")
    if "schema_version" in doc and not same(doc["schema_version"], 1):
        r.err("schema_version", "this validator understands schema_version 1 only")
    pid = doc.get("profile_id")
    if pid is not None and not (matches(PROFILE_ID_RX, pid) and 3 <= len(pid) <= 64):
        r.err("profile_id", "must be 3-64 chars of a-z, 0-9 separated by . _ -")
    pv = doc.get("profile_version")
    if pv is not None and not matches(SEMVER_RX, pv):
        r.err("profile_version", "must be MAJOR.MINOR.PATCH")
    for k in ("vendor", "model"):
        if k in doc:
            text(r, k, doc[k], max_len=80)
    for k, n in (("category", 40), ("firmware", 80)):
        if doc.get(k) is not None:
            text(r, k, doc[k], max_len=n)
    if doc.get("description") is not None:
        text(r, "description", doc["description"], max_len=1000)
    if doc.get("notes") is not None:
        notes = doc["notes"]
        items = notes if isinstance(notes, list) else [notes]
        for i, n in enumerate(items):
            text(r, f"notes[{i}]", n, max_len=2000)

    level = doc.get("support_level")
    enum(r, "support_level", level, LEVELS, required=False)
    caps = check_capabilities(r, doc.get("capabilities"))
    sim_only = doc.get("simulated_only", False)
    if not isinstance(sim_only, bool):
        r.err("simulated_only", "must be true or false")
        sim_only = False
    if doc.get("source") is not None:
        check_source(r, "source", doc["source"])
    passing = check_hardware_evidence(r, doc.get("evidence"))

    claimed = [level] + list(caps.values())
    if "documented" in claimed or "hardware_verified" in claimed:
        if doc.get("source") is None:
            r.err("source", "documented / hardware_verified profiles must cite the manual (document, revision, pages)")
    if "hardware_verified" in claimed:
        if sim_only:
            r.err("support_level", "a simulated_only profile can never be hardware_verified")
        elif passing == 0:
            r.err("evidence.hardware", "hardware_verified needs at least one passing hardware test record "
                                       "(date, adapter, platform, record, result: pass)")
    if sim_only and level not in (None, "simulated", "generic"):
        r.err("support_level", "simulated_only profiles must be 'simulated' (or 'generic')")
    if level == "generic" and doc.get("source") is None:
        r.warn("source", "generic profile without a source; label it clearly in the UI")

    transport = doc.get("transport")
    if transport is not None:
        check_transport(r, "transport", transport)
        if isinstance(transport, dict) and "simulator" in transport and not sim_only and level == "simulated" \
                and not ({"rtu", "tcp"} & set(transport)):
            r.warn("simulated_only", "simulator-only transport; set simulated_only: true")

    regs = doc.get("registers")
    spans: list[dict] = []
    ids: set[str] = set()
    if regs is not None:
        if not isinstance(regs, list) or not regs:
            r.err("registers", "must be a non-empty list")
        elif len(regs) > 2000:
            r.err("registers", "more than 2000 registers")
        else:
            for i, reg in enumerate(regs):
                span = check_register(r, f"registers[{i}]", reg, ids)
                if span:
                    spans.append(span)
    by_id = {s["id"]: s for s in spans if s["id"] is not None}

    def lookup(name):
        return by_id.get(name) if isinstance(name, str) else None

    spans.sort(key=lambda s: (s["table"], s["start"]))
    for a, b in zip(spans, spans[1:]):
        if a["table"] == b["table"] and b["start"] <= a["end"]:
            r.warn(f"registers.{b['id']}", f"overlaps '{a['id']}' in {a['table']} (alias? make sure it is intended)")
    writable = [s for s in spans if s["writable"]]
    if writable and caps.get("write") in (None, "unsupported"):
        r.err("capabilities.write", "profile has writable registers; declare the write support level")

    cmds = doc.get("commands")
    if cmds is not None:
        if not isinstance(cmds, list):
            r.err("commands", "must be a list")
        else:
            if not sim_only:
                r.err("commands", "control commands are only allowed on simulated_only profiles in schema v1 "
                                  "(no real VFD/heater control)")
            if caps.get("control") in (None, "unsupported"):
                r.err("capabilities.control", "profile has commands; declare the control support level")
            seen: set[str] = set()
            for i, c in enumerate(cmds):
                p = f"commands[{i}]"
                if not isinstance(c, dict):
                    r.err(p, "must be an object")
                    continue
                keys(r, p, c, CMD_KEYS)
                cid = c.get("id")
                if not matches(ID_RX, cid) or cid in seen:
                    r.err(f"{p}.id", "must be a unique id matching ^[a-z][a-z0-9_]{0,47}$")
                else:
                    seen.add(cid)
                kind = c.get("kind")
                enum(r, f"{p}.kind", kind, COMMAND_KINDS)
                text(r, f"{p}.label", c.get("label"), max_len=80)
                reg = lookup(c.get("register"))
                if reg is None or not reg["writable"]:
                    r.err(f"{p}.register", "must name a read_write register of this profile")
                rb = c.get("readback_register")
                if lookup(rb) is None:
                    r.err(f"{p}.readback_register", "required; must name a register of this profile")
                if kind == "setpoint":
                    if c.get("value") is not None:
                        r.err(f"{p}.value", "setpoint values come from the operator, bounded by the register's write limits")
                elif kind in COMMAND_KINDS and not is_num(c.get("value")):
                    r.err(f"{p}.value", "fixed commands need a numeric value")
                if c.get("expected") is not None and not is_num(c.get("expected")):
                    r.err(f"{p}.expected", "must be a number (engineering units of the readback register)")
                int_range(r, f"{p}.timeout_ms", c.get("timeout_ms"), 100, 30_000, required=False)
                fb = c.get("feedback")
                if fb is not None:
                    fp = f"{p}.feedback"
                    if not isinstance(fb, dict):
                        r.err(fp, "must be an object")
                    else:
                        keys(r, fp, fb, FEEDBACK_KEYS)
                        if lookup(fb.get("register")) is None:
                            r.err(f"{fp}.register", "required; must name a register of this profile")
                        tol = fb.get("tolerance")
                        if not (is_num(tol) and tol >= 0):
                            r.err(f"{fp}.tolerance", "required; a number >= 0 in engineering units")
                        int_range(r, f"{fp}.timeout_ms", fb.get("timeout_ms"), 100, 3_600_000)
                        if fb.get("value") is not None and not is_num(fb.get("value")):
                            r.err(f"{fp}.value", "must be a number (engineering units of the feedback register)")
                        if fb.get("bit") is not None:
                            int_range(r, f"{fp}.bit", fb.get("bit"), 0, 15)
                            if not (is_int(fb.get("value")) and fb.get("value") in (0, 1)):
                                r.err(f"{fp}.value", "a bit feedback needs value 0 or 1")
                        elif fb.get("value") is None and kind != "setpoint":
                            r.err(f"{fp}.value", "required unless the command is a setpoint (target = entered value)")

    cc = doc.get("comm_config")
    if cc is not None:
        if not isinstance(cc, dict):
            r.err("comm_config", "must be an object")
        else:
            keys(r, "comm_config", cc, {"unit_id", "baud"})
            if caps.get("comm_config") in (None, "unsupported"):
                r.err("capabilities.comm_config", "profile has comm_config; declare its support level")
            for k, v in cc.items():
                p = f"comm_config.{k}"
                if not isinstance(v, dict):
                    r.err(p, "must be an object")
                    continue
                keys(r, p, v, {"register", "apply", "codes"})
                reg = lookup(v.get("register"))
                if reg is None or not reg["writable"]:
                    r.err(f"{p}.register", "must name a read_write register of this profile")
                enum(r, f"{p}.apply", v.get("apply"), APPLY)
                if k == "baud":
                    codes = v.get("codes")
                    if not (isinstance(codes, dict) and codes and all(
                            matches(DIGITS_RX, code) and is_int(b) and 300 <= b <= 3_000_000
                            for code, b in codes.items())):
                        r.err(f"{p}.codes", "required: {\"<raw code>\": <baud>, ...}")
                elif v.get("codes") is not None:
                    r.err(f"{p}.codes", "only allowed for baud")
    return r


def _no_constants(name):
    raise ValueError(f"'{name}' is not valid JSON")


def load(path: pathlib.Path):
    data = path.read_bytes()
    if len(data) > MAX_BYTES:
        raise ValueError(f"file is larger than {MAX_BYTES} bytes")
    return json.loads(data.decode("utf-8"), object_pairs_hook=_no_dupes, parse_constant=_no_constants)


def path_from_stdin() -> str | None:
    if sys.stdin is None or sys.stdin.isatty():
        return None
    raw = sys.stdin.read()
    if not raw.strip():
        return None
    try:
        ev = json.loads(raw)
    except json.JSONDecodeError:
        return None
    if not isinstance(ev, dict):
        return None
    for k in ("file_path", "filePath", "path", "file"):
        if isinstance(ev.get(k), str):
            return ev[k]
    ti = ev.get("tool_input")
    if isinstance(ti, dict):
        for k in ("path", "file_path"):
            if isinstance(ti.get(k), str):
                return ti[k]
    return None


def in_profiles(p: pathlib.Path) -> bool:
    try:
        rel = p.resolve().relative_to(ROOT)
    except ValueError:
        rel = p
    parts = pathlib.PurePath(rel).parts
    # profiles/schema/ holds the JSON Schema itself, not a profile.
    return p.suffix.lower() == ".json" and "profiles" in parts[:1] and parts[1:2] != ("schema",)


def main(argv: list[str]) -> int:
    any_path = "--any-path" in argv  # validate files outside profiles/ (self-tests, drafts)
    as_json = "--json" in argv      # one JSON object per file (used by the Rust agreement test)
    args = [a for a in argv if a and a not in ("--any-path", "--json") and "{{" not in a]  # {{filePath}} -> stdin
    if not args:
        p = path_from_stdin()
        if p is None:
            print("check_profile: no file given (pass a path or hook JSON on stdin)", file=sys.stderr)
            return 0
        args = [p]
    status = 0
    for a in args:
        p = pathlib.Path(a)
        if not p.is_absolute():
            p = (pathlib.Path.cwd() / p)
        if not any_path and not in_profiles(p):
            continue
        shown = a
        try:
            doc = load(p)
        except FileNotFoundError:
            continue  # deleted between save and hook run
        except (ValueError, UnicodeDecodeError, RecursionError) as e:
            if as_json:
                print(json.dumps({"file": shown, "valid": False, "errors": [f"$: not valid JSON ({e})"],
                                  "warnings": []}))
            else:
                print(f"INVALID {shown}: not valid JSON ({e})")
            status = 1
            continue
        rep = validate(doc)
        if as_json:
            print(json.dumps({"file": shown, "valid": not rep.errors, "errors": rep.errors,
                              "warnings": rep.warnings}, ensure_ascii=False))
            status |= 1 if rep.errors else 0
            continue
        for w in rep.warnings:
            print(f"warning {shown}: {w}")
        if rep.errors:
            status = 1
            print(f"INVALID {shown}: {len(rep.errors)} error(s)")
            for e in rep.errors:
                print(f"  - {e}")
        else:
            n = len(doc.get("registers", []))
            print(f"OK {shown}: {n} registers, support_level={doc.get('support_level')}")
    if status:
        print("check_profile: profile schema v1 violations (see docs/profiles/schema-v1.md)", file=sys.stderr)
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
