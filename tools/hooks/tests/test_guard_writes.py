"""Self-tests for tools/hooks/guard_writes.py (run: python3 -m unittest discover -s tools/hooks/tests -v).

Each case pipes a PreToolUse event into the real script and checks the exit code:
0 = allowed, 2 = blocked, 1 = fail-open warning. Secret-looking samples are built
by concatenation so this file itself never contains a matching literal.
"""
import json
import pathlib
import subprocess
import sys
import unittest

HOOKS = pathlib.Path(__file__).resolve().parents[1]
ROOT = HOOKS.parents[1]
SCRIPT = HOOKS / "guard_writes.py"


def run(event, raw=None):
    data = raw if raw is not None else json.dumps(event)
    p = subprocess.run([sys.executable, str(SCRIPT)], input=data, capture_output=True, text=True, cwd=ROOT)
    return p.returncode, p.stderr


def write(path, text, tool="fs_write"):
    return {"hook_event_name": "PreToolUse", "cwd": str(ROOT), "tool_name": tool,
            "tool_input": {"path": path, "text": text}}


def replace(path, old, new):
    return {"hook_event_name": "PreToolUse", "cwd": str(ROOT), "tool_name": "str_replace",
            "tool_input": {"path": path, "oldStr": old, "newStr": new}}


class Blocked(unittest.TestCase):
    def assertBlocked(self, event, needle=None):
        code, err = run(event)
        self.assertEqual(code, 2, f"expected block, got {code}: {err}")
        if needle:
            self.assertIn(needle, err)

    def test_write_api_outside_command(self):
        self.assertBlocked(write("crates/buslink-core/src/poll/engine.rs",
                                 "ctx.write_single_register(0x0001, 42).await?;"), "outside the command service")

    def test_camel_case_in_ui(self):
        self.assertBlocked(write("src/views/control.ts", "await invoke('x'); client.writeMultipleRegisters(0, [1]);"))

    def test_fc_assignment_in_tauri_shell(self):
        self.assertBlocked(write("src-tauri/src/commands.rs", "let req = Request { function_code: 0x06, addr: 1 };"))

    def test_fc_token_in_ts(self):
        self.assertBlocked(write("src/inspector.ts", "const op = 'FC16';"))

    def test_raw_frame_array(self):
        self.assertBlocked(write("crates/buslink-core/src/protocol/frames.rs",
                                 "const F: [u8; 6] = [0x01, 0x06, 0x00, 0x01, 0x00, 0x03];"))

    def test_raw_hex_string(self):
        self.assertBlocked(write("src/demo.ts", "const frame = '01 10 00 00 00 02 04 00 0A 01 02';"))

    def test_rmodbus_builder(self):
        self.assertBlocked(write("crates/buslink-core/src/transport/rtu.rs",
                                 "req.generate_set_holding(0, 7, &mut buf)?;"))

    def test_str_replace_adds_write(self):
        self.assertBlocked(replace("crates/buslink-core/src/poll/engine.rs",
                                   "let v = read(ctx).await?;",
                                   "let v = read(ctx).await?;\nctx.write_single_coil(3, true).await?;"))

    def test_kotlin_plugin(self):
        self.assertBlocked(write("plugins/tauri-plugin-buslink-usb/android/src/main/java/x/Usb.kt",
                                 "val frame = byteArrayOf(1, 6); port.writeSingleRegister(frame)"))

    def test_broadcast_in_command_service(self):
        self.assertBlocked(write("crates/buslink-core/src/command/service.rs",
                                 "let slave = Slave(0);\nctx.set_slave(slave);\nctx.write_single_register(1, 5).await?;"),
                           "broadcast")

    def test_broadcast_unit_field_in_sim(self):
        self.assertBlocked(write("crates/buslink-sim/src/vfd.rs",
                                 "let cmd = WriteCmd { unit_id: 0, fc: 6, addr: 1, value: 2 };"), "broadcast")

    def test_broadcast_helper(self):
        self.assertBlocked(write("crates/buslink-core/src/command/encode.rs",
                                 "ctx.set_slave(Slave::broadcast());\nctx.write_multiple_registers(0, &v).await?;"))

    def test_broadcast_raw_frame(self):
        self.assertBlocked(write("crates/buslink-core/src/command/encode.rs",
                                 "const B: [u8; 4] = [0x00, 0x06, 0x00, 0x01];"), "broadcast")

    def test_aws_key(self):
        self.assertBlocked(write("src-tauri/tauri.conf.json", '{"k": "' + "AKIA" + "ABCDEFGHIJKLMNOP" + '"}'),
                           "AWS access key")

    def test_mqtt_password_assignment(self):
        self.assertBlocked(write("crates/buslink-core/src/mqtt/config.rs",
                                 'let mqtt_pass' + 'word = "S3cr3t-Broker-99";'), "hard-coded credential")

    def test_mqtt_url_with_credentials(self):
        self.assertBlocked(write("docs/setup.md", "Broker: mqtts://plant:" + "Hunter2Hunter2" + "@broker.local:8883"),
                           "embedded credentials")

    def test_private_key_even_in_example(self):
        self.assertBlocked(write("config/tls.example.pem", "-----BEGIN " + "PRIVATE KEY-----\nMIIB..."))

    def test_json_api_key(self):
        self.assertBlocked(write("profiles/cloud.json", '{"api_' + 'key": "a8f5f167f44f4964e6c998dee827110c"}'))


class Allowed(unittest.TestCase):
    def assertAllowed(self, event):
        code, err = run(event)
        self.assertEqual(code, 0, f"expected allow, got {code}: {err}")

    def test_command_service_write(self):
        self.assertAllowed(write("crates/buslink-core/src/command/encode.rs",
                                 "pub fn write_single_register(unit: UnitId, addr: u16, v: u16) -> Pdu {\n"
                                 "    Pdu::new(0x06, addr, v)\n}"))

    def test_command_service_rejects_broadcast(self):
        self.assertAllowed(write("crates/buslink-core/src/command/guard.rs",
                                 "if unit_id == 0 {\n    return Err(CommandError::Broadcast);\n}\n"
                                 "ctx.write_single_register(addr, v).await"))

    def test_simulator_handles_writes(self):
        self.assertAllowed(write("crates/buslink-sim/src/server.rs",
                                 "match fc { 0x06 => self.write_single_register(a, v), _ => exc(1) }"))

    def test_integration_test(self):
        self.assertAllowed(write("crates/buslink-core/tests/command_roundtrip.rs",
                                 "let f = [0x01, 0x06, 0x00, 0x01, 0x00, 0x03];\n"
                                 "assert!(svc.write_single_register(Slave(0), 1, 3).is_err());"))

    def test_ui_unit_test(self):
        self.assertAllowed(write("src/control/panel.test.ts", "expect(label('FC06')).toBe('Write single register');"))

    def test_read_code(self):
        self.assertAllowed(write("crates/buslink-core/src/poll/engine.rs",
                                 "let regs = ctx.read_holding_registers(0, 8).await?;\nlet fc = 3;"))

    def test_frame_length_match_arm(self):
        self.assertAllowed(write("crates/buslink-core/src/transport/rtu.rs",
                                 "let len = match pdu[0] { 0x05 | 0x06 | 0x0F | 0x10 => 8, 0x03 | 0x04 => 5 + n, _ => 5 };"))

    def test_comment_mentions_write(self):
        self.assertAllowed(write("crates/buslink-core/src/poll/engine.rs",
                                 "// Never call write_single_register here; use the command service.\nlet x = 1;"))

    def test_docs_and_steering(self):
        self.assertAllowed(write(".kiro/steering/modbus.md", "FC06 write_single_register; FC16 limit 123."))

    def test_unchanged_existing_lines(self):
        # Rewriting a file only checks lines that are new; existing lines are not "added".
        import os, tempfile
        with tempfile.TemporaryDirectory() as tmp:
            f = pathlib.Path(tmp, "crates/buslink-core/src/poll/engine.rs")
            f.parent.mkdir(parents=True)
            old = "ctx.write_single_register(1, 2).await?; // legacy line already in the file"
            f.write_text(old + "\n", encoding="utf-8")
            ev = write(str(f), old + "\nlet extra = 1;\n")
            p = subprocess.run([sys.executable, str(SCRIPT)], input=json.dumps(ev), capture_output=True, text=True,
                               cwd=tmp, env={**os.environ, "BUSLINK_ROOT": tmp})
            self.assertEqual(p.returncode, 0, p.stderr)
            ev = write(str(f), old + "\nctx.write_single_coil(1, true).await?;\n")
            p = subprocess.run([sys.executable, str(SCRIPT)], input=json.dumps(ev), capture_output=True, text=True,
                               cwd=tmp, env={**os.environ, "BUSLINK_ROOT": tmp})
            self.assertEqual(p.returncode, 2, p.stderr)

    def test_example_env(self):
        self.assertAllowed(write(".env.example", 'MQTT_PASSWORD="your-broker-password"\nmqtt_password = "replace_me_1"'))

    def test_env_lookup(self):
        self.assertAllowed(write("crates/buslink-core/src/mqtt/config.rs",
                                 'let password = std::env::var("BUSLINK_MQTT_PASSWORD")?;'))

    def test_placeholder_and_i18n_key(self):
        self.assertAllowed(write("src/i18n/en.json",
                                 '{"mqtt.password": "Password", "token_hint": "lan.token-hint", "api_key": "<your key>"}'))

    def test_absolute_path(self):
        self.assertAllowed(write(str(ROOT / "crates/buslink-core/src/command/mod.rs"), "pub mod encode; // FC06"))

    def test_non_write_tool(self):
        self.assertAllowed({"hook_event_name": "PreToolUse", "tool_name": "read_file",
                            "tool_input": {"path": "x.rs"}})

    def test_append(self):
        self.assertAllowed(write("docs/kiro-log/LEDGER.md", "- note", tool="fs_append"))


class FailOpen(unittest.TestCase):
    def test_garbage_stdin(self):
        code, err = run(None, raw="not json")
        self.assertEqual(code, 1)
        self.assertIn("fail-open", err)

    def test_empty_stdin(self):
        code, _ = run(None, raw="")
        self.assertEqual(code, 1)

    def test_missing_path(self):
        code, err = run({"tool_name": "fs_write", "tool_input": {"text": "x"}})
        self.assertEqual(code, 1)
        self.assertIn("no file path", err)


if __name__ == "__main__":
    unittest.main()
