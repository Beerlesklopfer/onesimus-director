"""Acceptance tests for byte evidence, separate from command execution."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("compare_command_responses.py")
spec = importlib.util.spec_from_file_location("compare_command_responses", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.left, self.right = self.root / "left.raw", self.root / "right.raw"
        self.report = self.root / "report.json"
        self.args = ["--command", "help", "--context", "director-main/api1", "--fixture-id", "fixture-01",
                     "--comparison-level", "clients", "--reference-director-version", "bareos-25.1.2",
                     "--candidate-director-version", "bareos-25.1.2", "--bconsole-version", "25.1.2",
                     "--oconsole-version", "0.1.0", "--bconsole-response", str(self.left),
                     "--oconsole-response", str(self.right), "--bconsole-status", "completed",
                     "--oconsole-status", "completed", "--bconsole-exit-code", "0",
                     "--oconsole-exit-code", "0", "--report", str(self.report)]

    def compare(self, left, right, extra=()):
        for path, data in [(self.left, left), (self.right, right)]:
            if data is not None:
                path.write_bytes(data)
        result = subprocess.run([sys.executable, str(SCRIPT), *self.args, *extra], capture_output=True)
        report = json.loads(self.report.read_bytes()) if self.report.exists() else None
        return result, report

    def test_identical_arbitrary_bytes_hashes_and_metadata(self):
        data = b" x\r\n\xff\x00\n"
        result, report = self.compare(data, data)
        self.assertEqual(result.returncode, 0)
        self.assertTrue(report["passed"])
        self.assertEqual(report["responses"]["bconsole"]["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(report["responses"]["oconsole"]["byte_count"], len(data))
        self.assertEqual(report["comparison_level"], "clients")
        self.assertEqual(report["fixture_id"], "fixture-01")
        self.assertEqual(self.left.read_bytes(), data)
        self.assertEqual(self.right.read_bytes(), data)

    def test_newline_whitespace_encoding_and_json_order_are_not_normalized(self):
        for left, right in [(b"x\n", b"x"), (b"x\r\n", b"x\n"),
                            (b"x ", b"x"), (b"\xc3\xa4", b"\xe4"),
                            (b'{"a":1,"b":2}', b'{"b":2,"a":1}')]:
            with self.subTest(left=left, right=right):
                result, report = self.compare(left, right)
                self.assertEqual(result.returncode, 1)
                self.assertFalse(report["passed"])
                self.assertIn("first_difference_offset", report)
                self.assertIn("--- bconsole", report["diff"])
                self.assertEqual(self.left.read_bytes(), left)

    def test_empty_completed_responses_pass(self):
        result, report = self.compare(b"", b"")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(report["responses"]["bconsole"]["byte_count"], 0)

    def test_missing_files_never_pass(self):
        result, report = self.compare(None, None)
        self.assertEqual(result.returncode, 1)
        self.assertFalse(report["passed"])
        self.assertIsNone(report["bytes_equal"])
        self.assertIn("read_error", report["responses"]["bconsole"])

    def test_failed_timeout_unrun_and_missing_exit_evidence_never_pass(self):
        for extra in [("--bconsole-status", "timeout"), ("--oconsole-status", "unrun"),
                      ("--bconsole-status", "error"), ("--oconsole-exit-code", "1")]:
            with self.subTest(extra=extra):
                result, report = self.compare(b"", b"", extra)
                self.assertEqual(result.returncode, 1)
                self.assertFalse(report["passed"])
        args = module.parser().parse_args(self.args)
        args.bconsole_exit_code = None
        self.assertFalse(module.compare(args)["passed"])

    def test_comparison_layers_are_separate(self):
        result, report = self.compare(b"x", b"x", ["--candidate-director-version", "onesimus"])
        self.assertEqual(result.returncode, 1)
        result, report = self.compare(b"x", b"x", ["--comparison-level", "directors", "--candidate-director-version", "onesimus"])
        self.assertEqual(result.returncode, 0)
        self.assertEqual(report["versions"]["candidate_director"], "onesimus")

    def test_diff_and_output_collision_protection(self):
        diff = self.root / "diff.txt"
        result, report = self.compare(b"x\n", b"x", ["--diff-file", str(diff)])
        self.assertEqual(diff.read_text(), report["diff"])
        result, _ = self.compare(b"untouched", b"different", ["--report", str(self.left)])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.left.read_bytes(), b"untouched")
        hardlink = self.root / "hardlink"
        os.link(self.left, hardlink)
        result = subprocess.run([sys.executable, str(SCRIPT), *self.args, "--report", str(hardlink)], capture_output=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.left.read_bytes(), b"untouched")


if __name__ == "__main__":
    unittest.main()
