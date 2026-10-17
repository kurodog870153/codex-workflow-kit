import json
import io
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
import urllib.request
from unittest.mock import patch

import ci_perf


class PerformanceReportTests(unittest.TestCase):
    def test_measure_preserves_unicode_log_when_console_is_cp1252(self):
        with tempfile.TemporaryDirectory() as directory:
            console_bytes = io.BytesIO()
            console = io.TextIOWrapper(console_bytes, encoding="cp1252")
            args = SimpleNamespace(directory=directory, phase="test", command=[sys.executable, "-c", "print('測試通過')"])
            with patch.object(sys, "stdout", console):
                self.assertEqual(ci_perf.measure(args), 0)
            self.assertIn("測試通過", (Path(directory) / "test.log").read_text(encoding="utf-8"))
            self.assertIn(b"\\u6e2c", console_bytes.getvalue())
            self.assertEqual(json.loads((Path(directory) / "test.json").read_text(encoding="utf-8"))["exit_code"], 0)

    def test_complete_report_requires_nextest_and_doc_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for phase in ci_perf.PHASES:
                (root / f"{phase}.json").write_text(json.dumps({"phase": phase, "duration_seconds": 2.5, "exit_code": 0}), encoding="utf-8")
            (root / "doc.log").write_text("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n", encoding="utf-8")
            (root / "junit.xml").write_text('<testsuites><testsuite name="binary"><testcase classname="suite" name="case" time="1.25" /></testsuite></testsuites>', encoding="utf-8")
            report = ci_perf.build_report(root, root / "junit.xml", {"platform": "macos", "commit": "abc", "toolchain": "1.85.0"})
            self.assertEqual(report["status"], "passed")
            self.assertEqual(report["test_execution_seconds"], 5.0)
            self.assertEqual(report["doc_tests"]["passed"], 1)

    def test_report_preserves_failure_and_missing_doc_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for phase, code in (("compile", 0), ("test", 1), ("doc", 0)):
                (root / f"{phase}.json").write_text(json.dumps({"phase": phase, "duration_seconds": 2.5, "exit_code": code}), encoding="utf-8")
            (root / "junit.xml").write_text('<testsuites><testsuite name="binary"><testcase classname="suite" name="case" time="1.25"><failure /></testcase></testsuite></testsuites>', encoding="utf-8")
            report = ci_perf.build_report(root, root / "junit.xml", {"platform": "macos", "commit": "abc", "toolchain": "1.85.0"})
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["tests"][0]["outcome"], "failed")
            self.assertIn("Doc test results missing", report["diagnostics"])

    def test_invalid_junit_duration_cannot_be_reported_as_success(self):
        with tempfile.TemporaryDirectory() as directory:
            junit = Path(directory) / "junit.xml"
            junit.write_text('<testsuites><testsuite name="binary"><testcase name="case" time="NaN" /></testsuite></testsuites>', encoding="utf-8")
            tests, error = ci_perf.parse_junit(junit)
            self.assertEqual(tests, [])
            self.assertIn("invalid", error)

    def test_enforcement_uses_test_evidence_not_uploaded_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for phase, code in (("compile", 0), ("test", 1), ("doc", 0)):
                (root / f"{phase}.json").write_text(json.dumps({"phase": phase, "duration_seconds": 1, "exit_code": code}), encoding="utf-8")
            (root / "doc.log").write_text("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n", encoding="utf-8")
            (root / "junit.xml").write_text('<testsuites><testsuite name="binary"><testcase name="case" time="1" /></testsuite></testsuites>', encoding="utf-8")
            (root / "performance.json").write_text('{"status":"passed"}', encoding="utf-8")
            self.assertEqual(ci_perf.enforce_command(SimpleNamespace(directory=root, junit=root / "junit.xml")), 1)

    def test_five_matching_baselines_use_median_and_strict_threshold(self):
        base = {"schema_version": 1, "config_id": ci_perf.CONFIG_ID, "platform": "macos", "target": "native", "runner_os": "macOS", "runner_arch": "ARM64", "runner_image": "macos15", "toolchain": "1.85.0", "test_ids_sha256": "same", "status": "passed"}
        history = [{**base, "test_execution_seconds": seconds} for seconds in (8, 9, 10, 11, 50)]
        self.assertEqual(ci_perf.compare_reports({**base, "test_execution_seconds": 13}, history)["status"], "normal")
        self.assertEqual(ci_perf.compare_reports({**base, "test_execution_seconds": 13.01}, history)["status"], "regression")
        self.assertEqual(ci_perf.compare_reports({**base, "test_execution_seconds": 15}, history[:4])["status"], "insufficient")
        history[0] = {**history[0], "test_ids_sha256": "changed"}
        self.assertEqual(ci_perf.compare_reports({**base, "test_execution_seconds": 15}, history)["status"], "incomparable")
        history[0] = {**history[0], "test_ids_sha256": "same", "status": "incomplete"}
        self.assertEqual(ci_perf.compare_reports({**base, "test_execution_seconds": 15}, history)["status"], "insufficient")

    def test_artifact_redirect_does_not_forward_token_to_storage_host(self):
        request = urllib.request.Request("https://api.github.com/artifact", headers={"Authorization": "Bearer secret"})
        redirected = ci_perf.StripCrossHostAuthorization().redirect_request(request, None, 302, "Found", {}, "https://release-assets.githubusercontent.com/file")
        self.assertIsNotNone(redirected)
        self.assertIsNone(redirected.get_header("Authorization"))

    def test_history_skips_missing_artifact_before_selecting_five(self):
        base = {"schema_version": 1, "config_id": ci_perf.CONFIG_ID, "platform": "macos", "target": "native", "runner_os": "macOS", "runner_arch": "ARM64", "runner_image": "macos15", "toolchain": "1.85.0", "test_ids_sha256": "same", "status": "passed"}
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "performance.json"
            report.write_text(json.dumps({**base, "test_execution_seconds": 13.01}), encoding="utf-8")
            runs = [{"id": index} for index in range(6)]
            reports = {index: {**base, "test_execution_seconds": seconds} for index, seconds in enumerate((8, 9, 10, 11, 50), start=1)}
            with patch.dict("os.environ", {"GITHUB_REPOSITORY": "owner/repo", "GITHUB_TOKEN": "token"}), patch.object(ci_perf, "api_json", return_value={"workflow_runs": runs}), patch.object(ci_perf, "artifact_report", side_effect=lambda _repo, run_id, _name, _token: reports.get(run_id)), patch.object(ci_perf, "append_summary") as summary:
                self.assertEqual(ci_perf.compare_command(SimpleNamespace(report=report, workflow="workflow.yml", artifact="rust-perf")), 0)
            self.assertIn("效能退化警告", summary.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
