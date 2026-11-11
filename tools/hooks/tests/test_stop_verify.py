"""Self-tests for tools/hooks/stop_verify.py and tools/hooks/typecheck_ui.sh.

cargo is replaced by a tiny Python command through BUSLINK_STOP_VERIFY_CMD, so
these tests run without a Rust workspace.
"""
import json
import os
import pathlib
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

HOOKS = pathlib.Path(__file__).resolve().parents[1]
ROOT = HOOKS.parents[1]
STOP = HOOKS / "stop_verify.py"
TYPECHECK = HOOKS / "typecheck_ui.sh"

FAIL_CMD = (f"{shlex.quote(sys.executable)} -c \"import sys; "
            "sys.stderr.write('warning: unused variable x\\n  --> a.rs:1:1\\n\\n"
            "error[E0425]: cannot find value y in this scope\\n  --> crates/buslink-core/src/lib.rs:3:5\\n'); "
            "sys.exit(101)\"")
PASS_CMD = f"{shlex.quote(sys.executable)} -c \"print('ok')\""


def run_stop(cmd, state_dir, session="s1", root=None):
    env = {**os.environ, "BUSLINK_STOP_VERIFY_CMD": cmd, "BUSLINK_STATE_DIR": state_dir}
    if root:
        env["BUSLINK_ROOT"] = root
    return subprocess.run([sys.executable, str(STOP)], input=json.dumps({"hook_event_name": "Stop",
                                                                         "session_id": session}),
                          capture_output=True, text=True, env=env, cwd=ROOT)


class StopVerify(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def test_pass_is_silent(self):
        p = run_stop(PASS_CMD, self.tmp.name)
        self.assertEqual((p.returncode, p.stdout), (0, ""))

    def test_failure_returns_continue_decision_with_errors_only(self):
        p = run_stop(FAIL_CMD, self.tmp.name)
        self.assertEqual(p.returncode, 0, p.stderr)
        out = json.loads(p.stdout)
        self.assertEqual(out["decision"], "block")
        self.assertIn("error[E0425]", out["reason"])
        self.assertIn("lib.rs:3:5", out["reason"])
        self.assertNotIn("unused variable", out["reason"])

    def test_loop_guard_gives_up_after_three_identical_failures(self):
        for _ in range(3):
            self.assertIn("decision", run_stop(FAIL_CMD, self.tmp.name).stdout)
        p = run_stop(FAIL_CMD, self.tmp.name)
        self.assertEqual((p.returncode, p.stdout), (0, ""))
        self.assertIn("not forcing another turn", p.stderr)
        # a passing check resets the counter
        run_stop(PASS_CMD, self.tmp.name)
        self.assertIn("decision", run_stop(FAIL_CMD, self.tmp.name).stdout)

    def test_no_cargo_workspace_yet(self):
        env = {k: v for k, v in os.environ.items() if k != "BUSLINK_STOP_VERIFY_CMD"}
        env["BUSLINK_ROOT"] = self.tmp.name  # empty dir: no Cargo.toml
        p = subprocess.run([sys.executable, str(STOP)], input="{}", capture_output=True, text=True, env=env)
        self.assertEqual((p.returncode, p.stdout), (0, ""))

    def test_bad_stdin_is_tolerated(self):
        env = {**os.environ, "BUSLINK_STOP_VERIFY_CMD": PASS_CMD, "BUSLINK_STATE_DIR": self.tmp.name}
        p = subprocess.run([sys.executable, str(STOP)], input="garbage", capture_output=True, text=True, env=env)
        self.assertEqual(p.returncode, 0)


@unittest.skipUnless(shutil.which("sh"), "needs sh")
class TypecheckUi(unittest.TestCase):
    def test_missing_node_modules_prints_hint_and_exits_0(self):
        with tempfile.TemporaryDirectory() as tmp:
            hooks = pathlib.Path(tmp, "tools/hooks")
            hooks.mkdir(parents=True)
            shutil.copy(TYPECHECK, hooks / "typecheck_ui.sh")
            pathlib.Path(tmp, "tsconfig.json").write_text("{}", encoding="utf-8")
            p = subprocess.run(["sh", str(hooks / "typecheck_ui.sh")], capture_output=True, text=True, cwd=tmp)
            self.assertEqual(p.returncode, 0)
            self.assertIn("node_modules/typescript is missing", p.stdout)

    def test_no_tsconfig_yet(self):
        with tempfile.TemporaryDirectory() as tmp:
            hooks = pathlib.Path(tmp, "tools/hooks")
            hooks.mkdir(parents=True)
            shutil.copy(TYPECHECK, hooks / "typecheck_ui.sh")
            p = subprocess.run(["sh", str(hooks / "typecheck_ui.sh")], capture_output=True, text=True, cwd=tmp)
            self.assertEqual(p.returncode, 0)
            self.assertIn("no tsconfig.json", p.stdout)


if __name__ == "__main__":
    unittest.main()
