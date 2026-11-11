//! P12: the Rust validator and tools/hooks/check_profile.py agree (verdict,
//! error paths, warning paths) on every bundled profile and shared fixture.
//! Also: the JSON Schema lists the same keys as the validators, every valid
//! document loads into the typed model, and mutated documents never panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::type_complexity)]

use buslink_core::profile::json::{self, J};
use buslink_core::profile::{self, ProfileError, validate};
use buslink_core::rng::SplitMix64;
use buslink_core::testkit::check;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    v.sort();
    v
}

fn fixtures() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let base = root().join("crates/buslink-core/tests/fixtures/profiles");
    let mut valid = json_files(&root().join("profiles"));
    valid.extend(json_files(&base.join("valid")));
    (valid, json_files(&base.join("invalid")))
}

#[derive(Debug, PartialEq)]
struct Verdict {
    valid: bool,
    errors: Vec<String>,
    warnings: Vec<String>,
}

fn path_of(issue: &str) -> String {
    issue.split_once(": ").map(|(p, _)| p).unwrap_or(issue).to_owned()
}

fn rust_verdict(file: &Path) -> Verdict {
    let bytes = std::fs::read(file).unwrap();
    match profile::check(&bytes) {
        Err(ProfileError::Json(_)) => Verdict {
            valid: false,
            errors: vec!["$".into()],
            warnings: vec![],
        },
        Err(e) => panic!("{}: {e}", file.display()),
        Ok(r) => {
            let mut errors: Vec<String> = r.errors.iter().map(|i| i.path.clone()).collect();
            let mut warnings: Vec<String> = r.warnings.iter().map(|i| i.path.clone()).collect();
            errors.sort();
            warnings.sort();
            Verdict {
                valid: errors.is_empty(),
                errors,
                warnings,
            }
        }
    }
}

fn python_verdicts(files: &[PathBuf]) -> Option<BTreeMap<String, Verdict>> {
    let script = root().join("tools/hooks/check_profile.py");
    let out = Command::new("python3")
        .arg(script)
        .arg("--any-path")
        .arg("--json")
        .args(files)
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).unwrap();
    let mut map = BTreeMap::new();
    for line in text.lines().filter(|l| l.starts_with('{')) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let list = |k: &str| -> Vec<String> {
            let mut l: Vec<String> = v[k]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| path_of(s.as_str().unwrap()))
                .collect();
            l.sort();
            l
        };
        let errors = list("errors");
        map.insert(
            v["file"].as_str().unwrap().to_owned(),
            Verdict {
                valid: v["valid"].as_bool().unwrap(),
                errors,
                warnings: list("warnings"),
            },
        );
    }
    Some(map)
}

#[test]
fn verdicts_match_expectations() {
    let (valid, invalid) = fixtures();
    assert!(
        valid.len() >= 12 && invalid.len() >= 60,
        "{} valid, {} invalid",
        valid.len(),
        invalid.len()
    );
    for f in &valid {
        let v = rust_verdict(f);
        assert!(v.valid, "{} should be valid: {:?}", f.display(), v.errors);
        let loaded = profile::load(&std::fs::read(f).unwrap());
        assert!(loaded.is_ok(), "{} should load: {:?}", f.display(), loaded.err());
    }
    for f in &invalid {
        let v = rust_verdict(f);
        assert!(!v.valid, "{} should be invalid", f.display());
        assert!(profile::load(&std::fs::read(f).unwrap()).is_err());
    }
}

#[test]
fn rust_and_python_agree() {
    let (mut files, invalid) = fixtures();
    files.extend(invalid);
    let Some(py) = python_verdicts(&files) else {
        eprintln!("SKIPPED: python3 not available; agreement not checked");
        return;
    };
    assert_eq!(py.len(), files.len(), "python output for every file");
    let mut mismatches = Vec::new();
    for f in &files {
        let key = f.to_string_lossy().to_string();
        let p = &py[&key];
        let r = rust_verdict(f);
        if *p != r {
            mismatches.push(format!("{}\n  python: {p:?}\n  rust:   {r:?}", f.display()));
        }
    }
    assert!(mismatches.is_empty(), "validators disagree:\n{}", mismatches.join("\n"));
}

fn schema() -> J {
    json::parse(&std::fs::read(root().join("profiles/schema/buslink-profile.schema.json")).unwrap()).unwrap()
}

fn prop_keys(obj: &J) -> Vec<String> {
    let mut k: Vec<String> = obj
        .get("properties")
        .unwrap()
        .as_obj()
        .unwrap()
        .iter()
        .map(|(k, _)| k.clone())
        .collect();
    k.sort();
    k
}

fn sorted(keys: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn schema_keys_match_validator() {
    let s = schema();
    let defs = s.get("$defs").unwrap();
    let props = s.get("properties").unwrap();
    assert_eq!(prop_keys(&s), sorted(&validate::TOP_KEYS));
    assert_eq!(prop_keys(defs.get("register").unwrap()), sorted(&validate::REG_KEYS));
    assert_eq!(prop_keys(defs.get("write").unwrap()), sorted(&validate::WRITE_KEYS));
    assert_eq!(prop_keys(defs.get("command").unwrap()), sorted(&validate::CMD_KEYS));
    assert_eq!(prop_keys(props.get("source").unwrap()), sorted(&validate::SOURCE_KEYS));
    assert_eq!(
        prop_keys(props.get("capabilities").unwrap()),
        sorted(&validate::CAP_KEYS)
    );
    let transport = props.get("transport").unwrap().get("properties").unwrap();
    assert_eq!(prop_keys(transport.get("rtu").unwrap()), sorted(&validate::RTU_KEYS));
    assert_eq!(prop_keys(transport.get("tcp").unwrap()), sorted(&validate::TCP_KEYS));
    let hw = props
        .get("evidence")
        .unwrap()
        .get("properties")
        .unwrap()
        .get("hardware")
        .unwrap()
        .get("items")
        .unwrap();
    assert_eq!(prop_keys(hw), sorted(&validate::HW_KEYS));
    // RTU unit ID 0 is never allowed as a default.
    assert_eq!(
        transport
            .get("rtu")
            .unwrap()
            .get("properties")
            .unwrap()
            .get("unit_id")
            .unwrap()
            .get("minimum"),
        Some(&J::Int(1))
    );
}

fn random_leaf(r: &mut SplitMix64) -> J {
    match r.below(9) {
        0 => J::Null,
        1 => J::Bool(r.bool()),
        2 => J::Int(i128::from(r.next_u16()) - 100),
        3 => J::Int(i128::from(r.below(10))),
        4 => J::Float(r.unit_f64() * 200.0 - 100.0),
        5 => J::Str(String::new()),
        6 => J::Str(
            [
                "read_write",
                "holding_register",
                "ABCD",
                "u32",
                "x",
                "=1",
                "simulated",
                "1.0.0",
            ][r.below(8) as usize]
                .into(),
        ),
        7 => J::Arr(vec![J::Int(1), J::Float(0.5)]),
        _ => J::Obj(vec![("en".into(), J::Str("a".into()))]),
    }
}

fn mutate(r: &mut SplitMix64, v: &mut J, depth: u32) {
    match v {
        J::Obj(m) if !m.is_empty() => {
            let i = r.below(m.len() as u64) as usize;
            match r.below(4) {
                0 if depth > 0 => m[i].1 = random_leaf(r),
                1 => {
                    m.remove(i);
                }
                2 => {
                    let key: String = ["value", "write", "access", "word_order", "zzz"][r.below(5) as usize].into();
                    m.retain(|(k, _)| *k != key); // a real document cannot hold duplicate keys
                    m.push((key, random_leaf(r)));
                }
                _ => mutate(r, &mut m[i].1, depth + 1),
            }
        }
        J::Arr(a) if !a.is_empty() => {
            let i = r.below(a.len() as u64) as usize;
            if r.chance(200) {
                a[i] = random_leaf(r);
            } else {
                mutate(r, &mut a[i], depth + 1);
            }
        }
        other => *other = random_leaf(r),
    }
}

#[test]
fn mutated_documents_never_panic_and_valid_ones_load() {
    let (valid, _) = fixtures();
    let docs: Vec<J> = valid
        .iter()
        .map(|f| json::parse(&std::fs::read(f).unwrap()).unwrap())
        .collect();
    let mut still_valid = 0u32;
    check("profile_mutation", 2000, |r| {
        let mut d = r.pick(&docs).unwrap().clone();
        for _ in 0..r.range_u64(1, 4) {
            mutate(r, &mut d, 0);
        }
        let report = validate::validate(&d);
        let text = serde_json::to_vec(&d.to_value()).unwrap();
        let loaded = profile::load(&text);
        if report.is_valid() {
            still_valid += 1;
            assert!(
                loaded.is_ok(),
                "valid document must load into the typed model: {:?}",
                loaded.err()
            );
        }
    });
    assert!(still_valid > 0);
}

#[test]
fn rust_and_python_agree_on_mutated_documents() {
    let (valid, _) = fixtures();
    let docs: Vec<J> = valid
        .iter()
        .map(|f| json::parse(&std::fs::read(f).unwrap()).unwrap())
        .collect();
    let dir = std::env::temp_dir().join(format!("buslink-agree-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut files = Vec::new();
    let mut n = 0u32;
    check("profile_agreement_mutated", 400, |r| {
        let mut d = r.pick(&docs).unwrap().clone();
        for _ in 0..r.range_u64(1, 3) {
            mutate(r, &mut d, 0);
        }
        let f = dir.join(format!("m{n:05}.json"));
        n += 1;
        std::fs::write(&f, serde_json::to_vec(&d.to_value()).unwrap()).unwrap();
        files.push(f);
    });
    let py = python_verdicts(&files);
    let result = py.map(|py| {
        files
            .iter()
            .filter_map(|f| {
                let p = &py[&f.to_string_lossy().to_string()];
                let r = rust_verdict(f);
                (*p != r).then(|| {
                    format!(
                        "{}\n  python: {p:?}\n  rust:   {r:?}",
                        std::fs::read_to_string(f).unwrap()
                    )
                })
            })
            .collect::<Vec<_>>()
    });
    std::fs::remove_dir_all(&dir).unwrap();
    match result {
        None => eprintln!("SKIPPED: python3 not available"),
        Some(m) => assert!(m.is_empty(), "{} disagreements, first:\n{}", m.len(), m[0]),
    }
}
