//! Profile schema v1 validator: a rule-for-rule port of
//! `tools/hooks/check_profile.py`. Both produce the same error and warning
//! paths; `tests/profile_agreement.rs` checks that on every bundled profile
//! and fixture. Change both files together.

use super::json::J;
use std::fmt;

pub const LEVELS: [&str; 4] = ["generic", "documented", "simulated", "hardware_verified"];
pub const CAP_LEVELS: [&str; 5] = ["generic", "documented", "simulated", "hardware_verified", "unsupported"];
pub const TABLES: [&str; 4] = ["coil", "discrete_input", "input_register", "holding_register"];
pub const TYPES: [(&str, Option<i128>); 11] = [
    ("bool", Some(1)),
    ("u16", Some(1)),
    ("i16", Some(1)),
    ("bitfield", Some(1)),
    ("u32", Some(2)),
    ("i32", Some(2)),
    ("f32", Some(2)),
    ("u64", Some(4)),
    ("i64", Some(4)),
    ("f64", Some(4)),
    ("string", None),
];
pub const WORD_ORDERS: [&str; 4] = ["ABCD", "CDAB", "BADC", "DCBA"];
pub const STD_BAUD: [i128; 14] =
    [300, 600, 1200, 2400, 4800, 9600, 14400, 19200, 38400, 57600, 115_200, 230_400, 460_800, 921_600];
pub const ROLES: [&str; 2] = ["operator", "commissioner"];
pub const RISKS: [&str; 3] = ["low", "medium", "high"];
pub const COMMAND_KINDS: [&str; 5] = ["run", "stop", "setpoint", "reset_fault", "mode"];
pub const APPLY: [&str; 3] = ["immediate", "after_reboot", "unknown"];
const MAX_REGS_PER_READ: i128 = 125;

pub const TOP_KEYS: [&str; 20] = [
    "$schema",
    "$comment",
    "schema_version",
    "profile_id",
    "profile_version",
    "vendor",
    "model",
    "category",
    "firmware",
    "description",
    "support_level",
    "capabilities",
    "simulated_only",
    "source",
    "evidence",
    "transport",
    "registers",
    "commands",
    "comm_config",
    "notes",
];
const REQUIRED_TOP: [&str; 8] =
    ["schema_version", "profile_id", "profile_version", "vendor", "model", "support_level", "transport", "registers"];
pub const REG_KEYS: [&str; 20] = [
    "id",
    "name",
    "description",
    "table",
    "address",
    "doc_address",
    "type",
    "length",
    "word_order",
    "scale",
    "offset",
    "unit",
    "decimals",
    "valid_range",
    "invalid_sentinels",
    "bits",
    "access",
    "write",
    "poll",
    "source_page",
];
pub const WRITE_KEYS: [&str; 8] = ["min", "max", "step", "allowed_values", "risk", "role", "confirm", "readback"];
pub const SOURCE_KEYS: [&str; 6] = ["document", "revision", "url", "pages", "retrieved", "publisher"];
pub const HW_KEYS: [&str; 8] = ["date", "adapter", "platform", "firmware", "unit_id", "record", "result", "notes"];
pub const RTU_KEYS: [&str; 7] =
    ["unit_id", "baud", "data_bits", "parity", "stop_bits", "response_timeout_ms", "inter_frame_ms"];
pub const TCP_KEYS: [&str; 3] = ["unit_id", "port", "response_timeout_ms"];
pub const CMD_KEYS: [&str; 8] = ["id", "kind", "label", "register", "value", "readback_register", "expected", "timeout_ms"];
pub const CAP_KEYS: [&str; 4] = ["read", "write", "comm_config", "control"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

impl Report {
    fn err(&mut self, path: impl Into<String>, msg: impl Into<String>) {
        self.errors.push(Issue { path: path.into(), message: msg.into() });
    }
    fn warn(&mut self, path: impl Into<String>, msg: impl Into<String>) {
        self.warnings.push(Issue { path: path.into(), message: msg.into() });
    }
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

// ---- matchers (ASCII, whole string; Python uses re.fullmatch with re.ASCII) ----

fn is_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 48
        && b[0].is_ascii_lowercase()
        && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

fn is_profile_id(s: &str) -> bool {
    let mut prev_sep = true;
    for c in s.bytes() {
        let alnum = c.is_ascii_lowercase() || c.is_ascii_digit();
        let sep = matches!(c, b'.' | b'_' | b'-');
        if alnum {
            prev_sep = false;
        } else if sep && !prev_sep {
            prev_sep = true;
        } else {
            return false;
        }
    }
    !s.is_empty() && !prev_sep
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| digits(p))
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn is_bit_key(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    let ok = match b {
        [b'0'] => true,
        [a] => (b'1'..=b'9').contains(a),
        [a, c] => (b'1'..=b'9').contains(a) && c.is_ascii_digit(),
        _ => false,
    };
    if ok { s.parse().ok() } else { None }
}

fn is_url(s: &str) -> bool {
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.chars().any(char::is_whitespace))
}

fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}')
}

fn str_of(v: Option<&J>) -> Option<&str> {
    v.and_then(J::as_str)
}

fn matches_id(v: Option<&J>) -> bool {
    str_of(v).is_some_and(is_id)
}

fn int_of(v: Option<&J>) -> Option<i128> {
    v.and_then(J::as_int)
}

fn num_of(v: Option<&J>) -> Option<f64> {
    v.and_then(J::as_num)
}

fn is_str(v: Option<&J>, s: &str) -> bool {
    str_of(v) == Some(s)
}

/// Python-style rendering of a value inside a path (`None` for missing or non-string).
fn py_key(v: Option<&str>) -> String {
    v.map(str::to_owned).unwrap_or_else(|| "None".to_owned())
}

struct Obj<'a>(&'a [(String, J)]);

impl<'a> Obj<'a> {
    fn get(&self, k: &str) -> Option<&'a J> {
        self.0.iter().find(|(key, _)| key == k).map(|(_, v)| v)
    }
    fn has(&self, k: &str) -> bool {
        self.get(k).is_some()
    }
    fn keys(&self) -> impl Iterator<Item = &'a str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }
}

fn keys(r: &mut Report, path: &str, obj: &Obj<'_>, allowed: &[&str]) {
    for k in obj.keys() {
        if !allowed.contains(&k) {
            r.err(format!("{path}.{k}"), "unknown key (profiles are strict data; no scripts or expressions)");
        }
    }
}

fn text(r: &mut Report, path: &str, v: Option<&J>, max_len: usize, required: bool, bilingual: bool) {
    let Some(v) = v else {
        if required {
            r.err(path, "required");
        }
        return;
    };
    if let (J::Obj(m), true) = (v, bilingual) {
        let o = Obj(m);
        keys(r, path, &o, &["en", "id"]);
        for lang in ["en", "id"] {
            if !o.has(lang) {
                r.err(format!("{path}.{lang}"), "required when the text is bilingual");
            }
        }
        for (lang, s) in m {
            text(r, &format!("{path}.{lang}"), Some(s), max_len, true, false);
        }
        return;
    }
    let Some(s) = v.as_str().filter(|s| !s.trim().is_empty()) else {
        r.err(path, "must be a non-empty string or {\"en\", \"id\"}");
        return;
    };
    if s.chars().count() > max_len {
        r.err(path, format!("longer than {max_len} characters"));
    }
    if s.chars().any(is_control) {
        r.err(path, "contains control characters");
    }
    if s.starts_with(['=', '+', '@', '\t', '\r']) {
        r.warn(path, "starts with a spreadsheet formula character; exports must escape it");
    }
}

fn text_req(r: &mut Report, path: &str, v: Option<&J>, max_len: usize) {
    text(r, path, v, max_len, true, true);
}

fn enum_str(r: &mut Report, path: &str, v: Option<&J>, allowed: &[&str], required: bool) -> bool {
    let Some(v) = v else {
        if required {
            r.err(path, format!("required, one of {allowed:?}"));
        }
        return false;
    };
    if v.as_str().is_some_and(|s| allowed.contains(&s)) {
        true
    } else {
        r.err(path, format!("not one of {allowed:?}"));
        false
    }
}

fn enum_int(r: &mut Report, path: &str, v: Option<&J>, allowed: &[i128]) -> bool {
    let Some(v) = v else {
        r.err(path, format!("required, one of {allowed:?}"));
        return false;
    };
    if v.as_int().is_some_and(|i| allowed.contains(&i)) {
        true
    } else {
        r.err(path, format!("not one of {allowed:?}"));
        false
    }
}

fn int_range(r: &mut Report, path: &str, v: Option<&J>, lo: i128, hi: i128, required: bool) -> bool {
    let Some(v) = v else {
        if required {
            r.err(path, "required");
        }
        return false;
    };
    match v.as_int() {
        Some(i) if (lo..=hi).contains(&i) => true,
        _ => {
            r.err(path, format!("must be an integer {lo}..{hi}"));
            false
        }
    }
}

fn check_source(r: &mut Report, path: &str, src: &J) {
    let Some(m) = src.as_obj() else {
        r.err(path, "must be an object");
        return;
    };
    let o = Obj(m);
    keys(r, path, &o, &SOURCE_KEYS);
    text_req(r, &format!("{path}.document"), o.get("document"), 200);
    text_req(r, &format!("{path}.revision"), o.get("revision"), 80);
    if let Some(url) = o.get("url")
        && !url.as_str().is_some_and(is_url)
    {
        r.err(format!("{path}.url"), "must be an http(s) URL");
    }
    if o.has("pages") {
        text_req(r, &format!("{path}.pages"), o.get("pages"), 80);
    }
    if o.has("publisher") {
        text_req(r, &format!("{path}.publisher"), o.get("publisher"), 120);
    }
    if let Some(d) = o.get("retrieved")
        && !d.as_str().is_some_and(is_date)
    {
        r.err(format!("{path}.retrieved"), "must be YYYY-MM-DD");
    }
}

fn check_transport(r: &mut Report, path: &str, t: &J) {
    let Some(m) = t.as_obj().filter(|m| !m.is_empty()) else {
        r.err(path, "must be an object with 'rtu' and/or 'tcp'");
        return;
    };
    let o = Obj(m);
    keys(r, path, &o, &["rtu", "tcp", "simulator"]);
    if !["rtu", "tcp", "simulator"].iter().any(|k| o.has(k)) {
        r.err(path, "needs at least one of rtu, tcp, simulator");
    }
    if let Some(rtu) = o.get("rtu") {
        let p = format!("{path}.rtu");
        match rtu.as_obj() {
            None => r.err(&p, "must be an object"),
            Some(m) => {
                let o = Obj(m);
                keys(r, &p, &o, &RTU_KEYS);
                int_range(r, &format!("{p}.unit_id"), o.get("unit_id"), 1, 247, true);
                let baud = o.get("baud");
                if int_range(r, &format!("{p}.baud"), baud, 300, 3_000_000, true)
                    && let Some(b) = int_of(baud)
                    && !STD_BAUD.contains(&b)
                {
                    r.warn(format!("{p}.baud"), format!("{b} is non-standard; Android CDC drivers may reject it"));
                }
                enum_int(r, &format!("{p}.data_bits"), o.get("data_bits"), &[7, 8]);
                enum_str(r, &format!("{p}.parity"), o.get("parity"), &["none", "even", "odd"], true);
                enum_int(r, &format!("{p}.stop_bits"), o.get("stop_bits"), &[1, 2]);
                int_range(r, &format!("{p}.response_timeout_ms"), o.get("response_timeout_ms"), 50, 10_000, false);
                int_range(r, &format!("{p}.inter_frame_ms"), o.get("inter_frame_ms"), 0, 1_000, false);
            }
        }
    }
    if let Some(tcp) = o.get("tcp") {
        let p = format!("{path}.tcp");
        match tcp.as_obj() {
            None => r.err(&p, "must be an object"),
            Some(m) => {
                let o = Obj(m);
                keys(r, &p, &o, &TCP_KEYS);
                int_range(r, &format!("{p}.unit_id"), o.get("unit_id"), 1, 255, true);
                int_range(r, &format!("{p}.port"), o.get("port"), 1, 65_535, true);
                int_range(r, &format!("{p}.response_timeout_ms"), o.get("response_timeout_ms"), 50, 10_000, false);
            }
        }
    }
    if let Some(sim) = o.get("simulator") {
        let ok = sim.as_obj().is_some_and(|m| {
            let o = Obj(m);
            o.keys().all(|k| k == "model") && matches_id(o.get("model"))
        });
        if !ok {
            r.err(format!("{path}.simulator"), "must be {\"model\": \"<simulator id>\"}");
        }
    }
}

#[derive(Clone, Debug)]
struct Span {
    id: Option<String>,
    table: String,
    start: i128,
    end: i128,
    writable: bool,
}

fn check_register(r: &mut Report, path: &str, reg: &J, ids: &mut Vec<String>) -> Option<Span> {
    let Some(m) = reg.as_obj() else {
        r.err(path, "must be an object");
        return None;
    };
    let o = Obj(m);
    keys(r, path, &o, &REG_KEYS);
    let rid = str_of(o.get("id")).filter(|s| is_id(s));
    match rid {
        None => r.err(format!("{path}.id"), "must match ^[a-z][a-z0-9_]{0,47}$"),
        Some(id) if ids.iter().any(|e| e == id) => r.err(format!("{path}.id"), format!("duplicate register id '{id}'")),
        Some(id) => ids.push(id.to_owned()),
    }
    text_req(r, &format!("{path}.name"), o.get("name"), 80);
    if o.has("description") {
        text_req(r, &format!("{path}.description"), o.get("description"), 500);
    }
    let table_ok = enum_str(r, &format!("{path}.table"), o.get("table"), &TABLES, true);
    let addr_ok = int_range(r, &format!("{path}.address"), o.get("address"), 0, 65_535, true);
    if let Some(da) = o.get("doc_address") {
        let ok = da.as_int().is_some() || da.as_str().is_some_and(|s| (1..=16).contains(&s.chars().count()));
        if !ok {
            r.err(format!("{path}.doc_address"), "must be the manual's label as a short string or integer");
        }
    }
    let type_names: Vec<&str> = TYPES.iter().map(|(n, _)| *n).collect();
    let type_ok = enum_str(r, &format!("{path}.type"), o.get("type"), &type_names, true);
    let typ = str_of(o.get("type"));
    let table = str_of(o.get("table"));
    let mut width: Option<i128> = None;
    if type_ok && let Some(typ) = typ {
        width = TYPES.iter().find(|(n, _)| *n == typ).and_then(|(_, w)| *w);
        if typ == "string" {
            if int_range(r, &format!("{path}.length"), o.get("length"), 1, MAX_REGS_PER_READ, true) {
                width = int_of(o.get("length"));
            }
        } else if o.has("length") {
            r.err(format!("{path}.length"), "only allowed for type 'string'");
        }
        if table_ok && let Some(table) = table {
            let bit_table = matches!(table, "coil" | "discrete_input");
            if bit_table && typ != "bool" {
                r.err(format!("{path}.type"), format!("table '{table}' holds single bits; type must be 'bool'"));
            }
            if !bit_table && typ == "bool" {
                r.err(format!("{path}.type"), "a register is 16 bits; use 'bitfield' with 'bits' for flags");
            }
        }
        let wo = o.get("word_order");
        if matches!(typ, "u32" | "i32" | "f32" | "u64" | "i64" | "f64") {
            enum_str(r, &format!("{path}.word_order"), wo, &WORD_ORDERS, true);
        } else if typ == "string" {
            if wo.is_some() {
                enum_str(r, &format!("{path}.word_order"), wo, &["ABCD", "BADC"], true);
            }
        } else if wo.is_some() {
            r.err(format!("{path}.word_order"), format!("not meaningful for single-register type '{typ}'"));
        }
        if typ != "bitfield" && o.has("bits") {
            r.err(format!("{path}.bits"), "only allowed for type 'bitfield'");
        }
    }
    match o.get("bits") {
        Some(J::Obj(bits)) => {
            for (b, label) in bits {
                if !is_bit_key(b).is_some_and(|n| n <= 15) {
                    r.err(format!("{path}.bits.{b}"), "bit index must be \"0\"..\"15\"");
                }
                text_req(r, &format!("{path}.bits.{b}"), Some(label), 80);
            }
        }
        Some(_) => r.err(format!("{path}.bits"), "must be an object of bit index -> label"),
        None => {}
    }
    let address = int_of(o.get("address"));
    if addr_ok
        && let (Some(a), Some(w)) = (address, width)
        && a + w - 1 > 65_535
    {
        r.err(format!("{path}.address"), format!("address {a} + {w} registers runs past 65535"));
    }

    match o.get("scale") {
        None => {}
        Some(s) => {
            if !s.as_num().is_some_and(|v| v != 0.0) {
                r.err(format!("{path}.scale"), "must be a non-zero finite number");
            }
        }
    }
    if let Some(off) = o.get("offset")
        && off.as_num().is_none()
    {
        r.err(format!("{path}.offset"), "must be a finite number");
    }
    if let Some(u) = o.get("unit")
        && !u.as_str().is_some_and(|s| s.chars().count() <= 16)
    {
        r.err(format!("{path}.unit"), "must be a string of at most 16 characters ('' for dimensionless)");
    }
    int_range(r, &format!("{path}.decimals"), o.get("decimals"), 0, 9, false);
    let vr = o.get("valid_range");
    let vr_nums: Option<(f64, f64)> = vr.and_then(J::as_arr).and_then(|a| match a {
        [lo, hi] => Some((lo.as_num()?, hi.as_num()?)),
        _ => None,
    });
    if vr.is_some() && !vr_nums.is_some_and(|(lo, hi)| lo < hi) {
        r.err(format!("{path}.valid_range"), "must be [min, max] with min < max (engineering units)");
    }
    if let Some(sen) = o.get("invalid_sentinels")
        && !sen.as_arr().is_some_and(|a| a.iter().all(|x| x.as_int().is_some()))
    {
        r.err(format!("{path}.invalid_sentinels"), "must be a list of raw integers");
    }
    if o.has("poll") {
        enum_str(r, &format!("{path}.poll"), o.get("poll"), &["fast", "normal", "slow"], true);
    }
    if o.has("source_page") {
        text_req(r, &format!("{path}.source_page"), o.get("source_page"), 40);
    }

    let read_default = J::Str("read".into());
    let access = o.get("access").unwrap_or(&read_default);
    enum_str(r, &format!("{path}.access"), Some(access), &["read", "read_write"], true);
    let w = o.get("write");
    let read_write = access.as_str() == Some("read_write");
    if read_write {
        if table_ok && let Some(t) = table
            && !matches!(t, "coil" | "holding_register")
        {
            r.err(format!("{path}.access"), format!("table '{t}' is read-only in Modbus"));
        }
        match w.and_then(J::as_obj) {
            None => r.err(format!("{path}.write"), "required for read_write registers (bounds, risk, role)"),
            Some(m) => {
                let wo = Obj(m);
                let p = format!("{path}.write");
                keys(r, &p, &wo, &WRITE_KEYS);
                enum_str(r, &format!("{p}.risk"), wo.get("risk"), &RISKS, true);
                enum_str(r, &format!("{p}.role"), wo.get("role"), &ROLES, true);
                if wo.get("confirm").is_some_and(|v| *v != J::Bool(true)) {
                    r.err(format!("{p}.confirm"), "explicit confirmation cannot be disabled");
                }
                if wo.get("readback").is_some_and(|v| *v != J::Bool(true)) {
                    r.err(format!("{p}.readback"), "read-back verification cannot be disabled");
                }
                let has_bounds = wo.has("min") || wo.has("max");
                if let Some(av) = wo.get("allowed_values") {
                    let ok = av.as_arr().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.as_num().is_some()));
                    if !ok {
                        r.err(format!("{p}.allowed_values"), "must be a non-empty list of numbers");
                    }
                } else if typ == Some("bool") {
                } else if !has_bounds {
                    r.err(&p, "numeric writes need min and max, or allowed_values");
                }
                if has_bounds {
                    match (num_of(wo.get("min")), num_of(wo.get("max"))) {
                        (Some(lo), Some(hi)) if lo <= hi => {
                            if let Some((vlo, vhi)) = vr_nums
                                && (lo < vlo || hi > vhi)
                            {
                                r.err(&p, "write bounds must lie inside valid_range");
                            }
                        }
                        _ => r.err(&p, "min and max must both be numbers with min <= max"),
                    }
                }
                if let Some(step) = wo.get("step")
                    && !step.as_num().is_some_and(|s| s > 0.0)
                {
                    r.err(format!("{p}.step"), "must be a positive number");
                }
            }
        }
    } else if w.is_some() {
        r.err(format!("{path}.write"), "only allowed when access is 'read_write'");
    }
    let (Some(w), true, true, Some(a), Some(t)) = (width, table_ok, addr_ok, address, table) else {
        return None;
    };
    Some(Span { id: rid.map(str::to_owned), table: t.to_owned(), start: a, end: a + w - 1, writable: read_write })
}

fn check_capabilities<'a>(r: &mut Report, caps: Option<&'a J>) -> Vec<(&'a str, &'a J)> {
    let Some(caps) = caps else { return Vec::new() };
    let Some(m) = caps.as_obj() else {
        r.err("capabilities", "must be an object");
        return Vec::new();
    };
    let o = Obj(m);
    keys(r, "capabilities", &o, &CAP_KEYS);
    for (k, v) in m {
        if CAP_KEYS.contains(&k.as_str()) {
            enum_str(r, &format!("capabilities.{k}"), Some(v), &CAP_LEVELS, true);
        }
    }
    m.iter().map(|(k, v)| (k.as_str(), v)).collect()
}

fn check_hardware_evidence(r: &mut Report, ev: Option<&J>) -> usize {
    let Some(ev) = ev else { return 0 };
    let Some(m) = ev.as_obj() else {
        r.err("evidence", "must be an object");
        return 0;
    };
    let o = Obj(m);
    keys(r, "evidence", &o, &["hardware"]);
    let empty = J::Arr(Vec::new());
    let Some(hw) = o.get("hardware").unwrap_or(&empty).as_arr() else {
        r.err("evidence.hardware", "must be a list");
        return 0;
    };
    let mut passing = 0;
    for (i, e) in hw.iter().enumerate() {
        let p = format!("evidence.hardware[{i}]");
        let Some(m) = e.as_obj() else {
            r.err(&p, "must be an object");
            continue;
        };
        let o = Obj(m);
        keys(r, &p, &o, &HW_KEYS);
        if !str_of(o.get("date")).is_some_and(is_date) {
            r.err(format!("{p}.date"), "required, YYYY-MM-DD");
        }
        for k in ["adapter", "platform", "record"] {
            text_req(r, &format!("{p}.{k}"), o.get(k), 200);
        }
        if o.has("firmware") {
            text_req(r, &format!("{p}.firmware"), o.get("firmware"), 80);
        }
        if o.has("notes") {
            text_req(r, &format!("{p}.notes"), o.get("notes"), 2000);
        }
        if o.has("unit_id") {
            int_range(r, &format!("{p}.unit_id"), o.get("unit_id"), 1, 255, true);
        }
        if enum_str(r, &format!("{p}.result"), o.get("result"), &["pass", "fail", "partial"], true)
            && is_str(o.get("result"), "pass")
        {
            passing += 1;
        }
    }
    passing
}

/// `null` is never a value in a profile: report it and drop the key.
fn strip_nulls(r: &mut Report, v: &J, path: &str) -> J {
    match v {
        J::Obj(m) => {
            let mut out = Vec::with_capacity(m.len());
            for (k, val) in m {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                if val.is_null() {
                    r.err(p, "null is not allowed; omit the key");
                } else {
                    out.push((k.clone(), strip_nulls(r, val, &p)));
                }
            }
            J::Obj(out)
        }
        J::Arr(a) => J::Arr(a.iter().enumerate().map(|(i, x)| strip_nulls(r, x, &format!("{path}[{i}]"))).collect()),
        other => other.clone(),
    }
}

/// Validate a parsed profile document.
pub fn validate(doc: &J) -> Report {
    let mut r = Report::default();
    if doc.as_obj().is_none() {
        r.err("$", "profile must be a JSON object");
        return r;
    }
    let doc = strip_nulls(&mut r, doc, "");
    let Some(m) = doc.as_obj() else { return r };
    let o = Obj(m);
    keys(&mut r, "$", &o, &TOP_KEYS);
    for k in ["$schema", "$comment"] {
        if let Some(v) = o.get(k)
            && !v.as_str().is_some_and(|s| s.chars().count() <= 2000)
        {
            r.err(k, "must be a string");
        }
    }
    for k in REQUIRED_TOP {
        if !o.has(k) {
            r.err(k, "required");
        }
    }
    if let Some(v) = o.get("schema_version")
        && v.as_int() != Some(1)
    {
        r.err("schema_version", "this validator understands schema_version 1 only");
    }
    if let Some(pid) = o.get("profile_id")
        && !pid.as_str().is_some_and(|s| is_profile_id(s) && (3..=64).contains(&s.chars().count()))
    {
        r.err("profile_id", "must be 3-64 chars of a-z, 0-9 separated by . _ -");
    }
    if let Some(pv) = o.get("profile_version")
        && !pv.as_str().is_some_and(is_semver)
    {
        r.err("profile_version", "must be MAJOR.MINOR.PATCH");
    }
    for k in ["vendor", "model"] {
        if o.has(k) {
            text_req(&mut r, k, o.get(k), 80);
        }
    }
    for (k, n) in [("category", 40), ("firmware", 80)] {
        if o.has(k) {
            text_req(&mut r, k, o.get(k), n);
        }
    }
    if o.has("description") {
        text_req(&mut r, "description", o.get("description"), 1000);
    }
    if let Some(notes) = o.get("notes") {
        let items: Vec<&J> = match notes {
            J::Arr(a) => a.iter().collect(),
            other => vec![other],
        };
        for (i, n) in items.into_iter().enumerate() {
            text_req(&mut r, &format!("notes[{i}]"), Some(n), 2000);
        }
    }

    let level = o.get("support_level");
    enum_str(&mut r, "support_level", level, &LEVELS, false);
    let caps = check_capabilities(&mut r, o.get("capabilities"));
    let cap = |k: &str| caps.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
    let cap_unset = |k: &str| cap(k).is_none_or(|v| v.as_str() == Some("unsupported"));
    let sim_only = match o.get("simulated_only") {
        None => false,
        Some(J::Bool(b)) => *b,
        Some(_) => {
            r.err("simulated_only", "must be true or false");
            false
        }
    };
    if let Some(src) = o.get("source") {
        check_source(&mut r, "source", src);
    }
    let passing = check_hardware_evidence(&mut r, o.get("evidence"));

    let claims = |name: &str| is_str(level, name) || caps.iter().any(|(_, v)| v.as_str() == Some(name));
    if (claims("documented") || claims("hardware_verified")) && !o.has("source") {
        r.err("source", "documented / hardware_verified profiles must cite the manual (document, revision, pages)");
    }
    if claims("hardware_verified") {
        if sim_only {
            r.err("support_level", "a simulated_only profile can never be hardware_verified");
        } else if passing == 0 {
            r.err(
                "evidence.hardware",
                "hardware_verified needs at least one passing hardware test record (date, adapter, platform, record, result: pass)",
            );
        }
    }
    if sim_only && !(level.is_none() || is_str(level, "simulated") || is_str(level, "generic")) {
        r.err("support_level", "simulated_only profiles must be 'simulated' (or 'generic')");
    }
    if is_str(level, "generic") && !o.has("source") {
        r.warn("source", "generic profile without a source; label it clearly in the UI");
    }

    if let Some(transport) = o.get("transport") {
        check_transport(&mut r, "transport", transport);
        if let Some(t) = transport.as_obj() {
            let t = Obj(t);
            if t.has("simulator") && !sim_only && is_str(level, "simulated") && !t.has("rtu") && !t.has("tcp") {
                r.warn("simulated_only", "simulator-only transport; set simulated_only: true");
            }
        }
    }

    let mut spans: Vec<Span> = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    if let Some(regs) = o.get("registers") {
        match regs.as_arr() {
            Some(a) if a.is_empty() => r.err("registers", "must be a non-empty list"),
            None => r.err("registers", "must be a non-empty list"),
            Some(a) if a.len() > 2000 => r.err("registers", "more than 2000 registers"),
            Some(a) => {
                for (i, reg) in a.iter().enumerate() {
                    if let Some(s) = check_register(&mut r, &format!("registers[{i}]"), reg, &mut ids) {
                        spans.push(s);
                    }
                }
            }
        }
    }
    // Python dict semantics: the last span with a given id wins.
    let lookup = |name: Option<&str>| -> Option<&Span> {
        let name = name?;
        spans.iter().rev().find(|s| s.id.as_deref() == Some(name))
    };
    let mut sorted = spans.clone();
    sorted.sort_by(|a, b| (a.table.as_str(), a.start).cmp(&(b.table.as_str(), b.start)));
    for pair in sorted.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.table == b.table && b.start <= a.end {
            r.warn(
                format!("registers.{}", py_key(b.id.as_deref())),
                format!("overlaps '{}' in {} (alias? make sure it is intended)", py_key(a.id.as_deref()), a.table),
            );
        }
    }
    if spans.iter().any(|s| s.writable) && cap_unset("write") {
        r.err("capabilities.write", "profile has writable registers; declare the write support level");
    }

    if let Some(cmds) = o.get("commands") {
        match cmds.as_arr() {
            None => r.err("commands", "must be a list"),
            Some(list) => {
                if !sim_only {
                    r.err(
                        "commands",
                        "control commands are only allowed on simulated_only profiles in schema v1 (no real VFD/heater control)",
                    );
                }
                if cap_unset("control") {
                    r.err("capabilities.control", "profile has commands; declare the control support level");
                }
                let mut seen: Vec<String> = Vec::new();
                for (i, c) in list.iter().enumerate() {
                    let p = format!("commands[{i}]");
                    let Some(m) = c.as_obj() else {
                        r.err(&p, "must be an object");
                        continue;
                    };
                    let co = Obj(m);
                    keys(&mut r, &p, &co, &CMD_KEYS);
                    match str_of(co.get("id")).filter(|s| is_id(s)) {
                        Some(id) if !seen.iter().any(|e| e == id) => seen.push(id.to_owned()),
                        _ => r.err(format!("{p}.id"), "must be a unique id matching ^[a-z][a-z0-9_]{0,47}$"),
                    }
                    let kind = co.get("kind");
                    enum_str(&mut r, &format!("{p}.kind"), kind, &COMMAND_KINDS, true);
                    text_req(&mut r, &format!("{p}.label"), co.get("label"), 80);
                    if !lookup(str_of(co.get("register"))).is_some_and(|s| s.writable) {
                        r.err(format!("{p}.register"), "must name a read_write register of this profile");
                    }
                    if lookup(str_of(co.get("readback_register"))).is_none() {
                        r.err(format!("{p}.readback_register"), "required; must name a register of this profile");
                    }
                    if is_str(kind, "setpoint") {
                        if co.has("value") {
                            r.err(
                                format!("{p}.value"),
                                "setpoint values come from the operator, bounded by the register's write limits",
                            );
                        }
                    } else if str_of(kind).is_some_and(|k| COMMAND_KINDS.contains(&k)) && num_of(co.get("value")).is_none() {
                        r.err(format!("{p}.value"), "fixed commands need a numeric value");
                    }
                    if co.get("expected").is_some_and(|v| v.as_num().is_none()) {
                        r.err(format!("{p}.expected"), "must be a number (engineering units of the readback register)");
                    }
                    int_range(&mut r, &format!("{p}.timeout_ms"), co.get("timeout_ms"), 100, 30_000, false);
                }
            }
        }
    }

    if let Some(cc) = o.get("comm_config") {
        match cc.as_obj() {
            None => r.err("comm_config", "must be an object"),
            Some(m) => {
                keys(&mut r, "comm_config", &Obj(m), &["unit_id", "baud"]);
                if cap_unset("comm_config") {
                    r.err("capabilities.comm_config", "profile has comm_config; declare its support level");
                }
                for (k, v) in m {
                    let p = format!("comm_config.{k}");
                    let Some(vm) = v.as_obj() else {
                        r.err(&p, "must be an object");
                        continue;
                    };
                    let vo = Obj(vm);
                    keys(&mut r, &p, &vo, &["register", "apply", "codes"]);
                    if !lookup(str_of(vo.get("register"))).is_some_and(|s| s.writable) {
                        r.err(format!("{p}.register"), "must name a read_write register of this profile");
                    }
                    enum_str(&mut r, &format!("{p}.apply"), vo.get("apply"), &APPLY, true);
                    if k == "baud" {
                        let ok = vo.get("codes").and_then(J::as_obj).is_some_and(|codes| {
                            !codes.is_empty()
                                && codes.iter().all(|(code, b)| {
                                    digits(code) && b.as_int().is_some_and(|b| (300..=3_000_000).contains(&b))
                                })
                        });
                        if !ok {
                            r.err(format!("{p}.codes"), "required: {\"<raw code>\": <baud>, ...}");
                        }
                    } else if vo.has("codes") {
                        r.err(format!("{p}.codes"), "only allowed for baud");
                    }
                }
            }
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matchers() {
        assert!(is_id("temperature_1"));
        assert!(!is_id("Temperature"));
        assert!(!is_id("1abc"));
        assert!(!is_id(&"a".repeat(49)));
        assert!(is_profile_id("bgt.smps-8in1"));
        assert!(!is_profile_id("bgt..smps"));
        assert!(!is_profile_id("bgt."));
        assert!(!is_profile_id(".bgt"));
        assert!(is_semver("1.0.0"));
        assert!(!is_semver("1.0.0\n"));
        assert!(!is_semver("1.0"));
        assert!(!is_semver("١.0.0"));
        assert!(is_date("2026-11-12"));
        assert!(!is_date("2026-1-12"));
        assert_eq!(is_bit_key("15"), Some(15));
        assert_eq!(is_bit_key("0"), Some(0));
        assert_eq!(is_bit_key("01"), None);
        assert_eq!(is_bit_key("²"), None);
        assert!(is_url("https://example.org/x"));
        assert!(!is_url("https://"));
        assert!(!is_url("ftp://x"));
        assert!(!is_url("https://a b"));
    }
}
