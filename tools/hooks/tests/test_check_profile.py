"""Self-tests for tools/hooks/check_profile.py (schema v1, docs/profiles/schema-v1.md).

The sample profiles below are test fixtures, not published profiles. Real
profiles live in profiles/ and are written by the profile-author agent.
"""
import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

HOOKS = pathlib.Path(__file__).resolve().parents[1]
ROOT = HOOKS.parents[1]
SCRIPT = HOOKS / "check_profile.py"
sys.path.insert(0, str(HOOKS))
import check_profile  # noqa: E402

SENSOR = {
    "schema_version": 1,
    "profile_id": "test.soil-8in1",
    "profile_version": "1.0.0",
    "vendor": "Test vendor",
    "model": "Soil 8-in-1",
    "support_level": "documented",
    "capabilities": {"read": "documented", "write": "unsupported", "comm_config": "unsupported",
                     "control": "unsupported"},
    "source": {"document": "Sensor user manual", "revision": "v1.0", "pages": "4-5"},
    "transport": {"rtu": {"unit_id": 1, "baud": 9600, "data_bits": 8, "parity": "none", "stop_bits": 1}},
    "registers": [
        {"id": "temperature", "name": {"en": "Temperature", "id": "Suhu"}, "table": "holding_register",
         "address": 0, "doc_address": "40001", "type": "i16", "scale": 0.1, "unit": "°C", "decimals": 1,
         "source_page": "p. 5"},
        {"id": "moisture", "name": {"en": "Moisture", "id": "Kelembapan"}, "table": "holding_register",
         "address": 1, "type": "u16", "scale": 0.1, "unit": "%", "valid_range": [0, 100]},
        {"id": "energy", "name": "Energy", "table": "input_register", "address": 10, "type": "f32",
         "word_order": "CDAB", "unit": "kWh"},
        {"id": "status", "name": "Status", "table": "input_register", "address": 12, "type": "bitfield",
         "bits": {"0": "Fault", "3": {"en": "Ready", "id": "Siap"}}},
    ],
}

VFD = {
    "schema_version": 1,
    "profile_id": "sim.vfd-basic",
    "profile_version": "0.1.0",
    "vendor": "BusLink simulator",
    "model": "Simulated VFD",
    "support_level": "simulated",
    "simulated_only": True,
    "capabilities": {"read": "simulated", "write": "simulated", "control": "simulated"},
    "transport": {"simulator": {"model": "vfd_basic"}, "tcp": {"unit_id": 1, "port": 1502}},
    "registers": [
        {"id": "control_word", "name": "Control word", "table": "holding_register", "address": 0, "type": "u16",
         "access": "read_write", "write": {"allowed_values": [0, 1], "risk": "high", "role": "operator"}},
        {"id": "freq_ref", "name": "Frequency reference", "table": "holding_register", "address": 1, "type": "u16",
         "scale": 0.1, "unit": "Hz", "valid_range": [0, 60],
         "access": "read_write", "write": {"min": 0, "max": 50, "step": 0.1, "risk": "medium", "role": "operator"}},
        {"id": "status_word", "name": "Status word", "table": "input_register", "address": 0, "type": "bitfield"},
        {"id": "freq_out", "name": "Output frequency", "table": "input_register", "address": 1, "type": "u16",
         "scale": 0.1, "unit": "Hz"},
    ],
    "commands": [
        {"id": "run", "kind": "run", "label": {"en": "Run", "id": "Jalan"}, "register": "control_word", "value": 1,
         "readback_register": "status_word", "timeout_ms": 2000},
        {"id": "stop", "kind": "stop", "label": "Software stop (not an E-stop)", "register": "control_word",
         "value": 0, "readback_register": "status_word"},
        {"id": "set_freq", "kind": "setpoint", "label": "Apply frequency", "register": "freq_ref",
         "readback_register": "freq_ref"},
    ],
}


def errors(doc):
    return check_profile.validate(doc).errors


def mutate(base, fn):
    d = copy.deepcopy(base)
    fn(d)
    return d


class Valid(unittest.TestCase):
    def test_documented_sensor(self):
        self.assertEqual(errors(SENSOR), [])

    def test_simulated_vfd_with_commands(self):
        self.assertEqual(errors(VFD), [])

    def test_hardware_verified_with_evidence(self):
        d = mutate(SENSOR, lambda d: d.update(
            support_level="hardware_verified",
            evidence={"hardware": [{"date": "2026-11-20", "adapter": "USB TO RS485 (B), CH343G 1A86:55D3",
                                    "platform": "Windows 11", "unit_id": 1,
                                    "record": "docs/evidence/soil-windows.md", "result": "pass"}]}))
        self.assertEqual(errors(d), [])


class Invalid(unittest.TestCase):
    def assertError(self, doc, needle):
        errs = errors(doc)
        self.assertTrue(any(needle in e for e in errs), f"no error containing {needle!r}: {errs}")

    def test_missing_required(self):
        self.assertError(mutate(SENSOR, lambda d: d.pop("profile_id")), "profile_id: required")

    def test_bad_support_level(self):
        self.assertError(mutate(SENSOR, lambda d: d.update(support_level="verified")), "support_level")

    def test_hardware_verified_without_evidence(self):
        self.assertError(mutate(SENSOR, lambda d: d.update(support_level="hardware_verified")), "evidence.hardware")

    def test_failed_evidence_is_not_evidence(self):
        d = mutate(SENSOR, lambda d: d.update(support_level="hardware_verified", evidence={"hardware": [
            {"date": "2026-11-20", "adapter": "CH340", "platform": "Android", "record": "x", "result": "fail"}]}))
        self.assertError(d, "evidence.hardware")

    def test_documented_without_source(self):
        self.assertError(mutate(SENSOR, lambda d: d.pop("source")), "must cite the manual")

    def test_simulated_only_cannot_be_hardware_verified(self):
        self.assertError(mutate(VFD, lambda d: d.update(support_level="hardware_verified")), "never be hardware_verified")

    def test_unknown_key_blocks_script(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(script="fetch('x')")), "unknown key")

    def test_unknown_top_level_key(self):
        self.assertError(mutate(SENSOR, lambda d: d.update(transform="value*2")), "unknown key")

    def test_address_out_of_range(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(address=70000)), "0..65535")

    def test_multi_register_runs_past_end(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][2].update(address=65535)), "runs past 65535")

    def test_doc_label_as_address_rejected(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(address="40001")), "integer")

    def test_bad_type(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(type="float")), "type")

    def test_word_order_required_for_32_bit(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][2].pop("word_order")), "word_order")

    def test_bad_word_order(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][2].update(word_order="little")), "word_order")

    def test_word_order_on_16_bit(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(word_order="ABCD")), "not meaningful")

    def test_bool_in_register_table(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(type="bool")), "bitfield")

    def test_string_too_long_for_one_read(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(type="string", length=126)), "length")

    def test_broadcast_unit_id(self):
        self.assertError(mutate(SENSOR, lambda d: d["transport"]["rtu"].update(unit_id=0)), "1..247")

    def test_write_on_input_register(self):
        def f(d):
            d["registers"][2].update(access="read_write", write={"min": 0, "max": 1, "risk": "low", "role": "operator"})
        self.assertError(mutate(SENSOR, f), "read-only in Modbus")

    def test_write_without_bounds(self):
        self.assertError(mutate(VFD, lambda d: d["registers"][1]["write"].pop("min")), "min")

    def test_write_bounds_outside_valid_range(self):
        self.assertError(mutate(VFD, lambda d: d["registers"][1]["write"].update(max=80)), "inside valid_range")

    def test_readback_cannot_be_disabled(self):
        self.assertError(mutate(VFD, lambda d: d["registers"][1]["write"].update(readback=False)), "read-back")

    def test_writable_needs_write_capability(self):
        self.assertError(mutate(VFD, lambda d: d["capabilities"].pop("write")), "capabilities.write")

    def test_commands_only_on_simulated_profiles(self):
        self.assertError(mutate(VFD, lambda d: d.update(simulated_only=False, support_level="documented",
                                                        source={"document": "x", "revision": "1"})),
                         "only allowed on simulated_only")

    def test_command_register_must_be_writable(self):
        self.assertError(mutate(VFD, lambda d: d["commands"][0].update(register="freq_out")), "read_write register")

    def test_command_needs_readback(self):
        self.assertError(mutate(VFD, lambda d: d["commands"][0].pop("readback_register")), "readback_register")

    def test_duplicate_register_id(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][1].update(id="temperature")), "duplicate")

    def test_bilingual_name_needs_both(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(name={"en": "Temperature"})), ".id")

    # Rules aligned with crates/buslink-core/src/profile/validate.rs
    def test_enum_is_type_strict(self):
        self.assertError(mutate(SENSOR, lambda d: d["transport"]["rtu"].update(data_bits=8.0)), "data_bits")
        self.assertError(mutate(SENSOR, lambda d: d["transport"]["rtu"].update(stop_bits=True)), "stop_bits")
        self.assertError(mutate(SENSOR, lambda d: d.update(schema_version=True)), "schema_version")

    def test_patterns_are_full_ascii_matches(self):
        self.assertError(mutate(SENSOR, lambda d: d.update(profile_version="1.0.0\n")), "profile_version")
        self.assertError(mutate(SENSOR, lambda d: d.update(profile_version="\u0661.0.0")), "profile_version")
        self.assertError(mutate(SENSOR, lambda d: d["registers"][3].update(bits={"\u00b2": "x"})), "bits")

    def test_null_is_rejected(self):
        self.assertError(mutate(SENSOR, lambda d: d["registers"][0].update(scale=None)), "null is not allowed")

    def test_non_string_references_do_not_crash(self):
        d = mutate(VFD, lambda d: d["commands"][0].update(register=["control_word"], readback_register={"a": 1}))
        self.assertError(d, "commands[0].register")
        self.assertError(mutate(VFD, lambda d: d["registers"][0].update(id=["x"])), "registers[0].id")

    def test_expected_must_be_number(self):
        self.assertError(mutate(VFD, lambda d: d["commands"][0].update(expected="1")), "expected")


class Cli(unittest.TestCase):
    def run_cli(self, *args, stdin=""):
        return subprocess.run([sys.executable, str(SCRIPT), *args], input=stdin, capture_output=True, text=True,
                              cwd=ROOT)

    def test_json_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = pathlib.Path(tmp, "x.json")
            f.write_text(json.dumps(mutate(SENSOR, lambda d: d.pop("profile_id"))), encoding="utf-8")
            p = self.run_cli("--any-path", "--json", str(f))
            self.assertEqual(p.returncode, 1)
            out = json.loads(p.stdout.strip())
            self.assertFalse(out["valid"])
            self.assertIn("profile_id: required", out["errors"])

    def test_nan_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = pathlib.Path(tmp, "nan.json")
            f.write_text('{"schema_version": NaN}', encoding="utf-8")
            p = self.run_cli("--any-path", str(f))
            self.assertEqual(p.returncode, 1)
            self.assertIn("not valid JSON", p.stdout)

    def test_schema_dir_is_skipped(self):
        p = self.run_cli(str(ROOT / "profiles" / "schema" / "buslink-profile.schema.json"))
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout, "")

    def test_bundled_profiles_are_valid(self):
        files = sorted(str(p) for p in (ROOT / "profiles").glob("*.json"))
        if files:
            p = self.run_cli(*files)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)

    def test_cli_valid_and_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            good = pathlib.Path(tmp, "good.json")
            good.write_text(json.dumps(SENSOR), encoding="utf-8")
            bad = pathlib.Path(tmp, "bad.json")
            bad.write_text(json.dumps(mutate(SENSOR, lambda d: d.update(support_level="hardware_verified"))),
                           encoding="utf-8")
            p = self.run_cli("--any-path", str(good))
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertIn("OK", p.stdout)
            p = self.run_cli("--any-path", str(bad))
            self.assertEqual(p.returncode, 1)
            self.assertIn("INVALID", p.stdout)

    def test_duplicate_json_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            f = pathlib.Path(tmp, "dup.json")
            f.write_text('{"schema_version": 1, "schema_version": 2}', encoding="utf-8")
            p = self.run_cli("--any-path", str(f))
            self.assertEqual(p.returncode, 1)
            self.assertIn("duplicate key", p.stdout)

    def test_outside_profiles_is_skipped(self):
        p = self.run_cli("package.json")
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout, "")

    def test_hook_stdin_and_unsubstituted_placeholder(self):
        p = self.run_cli("{{filePath}}", stdin=json.dumps({"file_path": "src/main.ts"}))
        self.assertEqual(p.returncode, 0)


if __name__ == "__main__":
    unittest.main()
