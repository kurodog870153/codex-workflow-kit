from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.foundation.invocation import parse_invocation
from worklib.foundation.errors import WorkError


class InvocationTests(unittest.TestCase):
    def parse(self, text):
        return parse_invocation(text.encode("utf-8"), source="invocation")

    def test_modes_preserve_opaque_request_exactly(self):
        tail = '  建立 e\u0301\r\n"quoted" -- $HOME $(command)\n$work execute -- embedded\n'
        for mode in ("plan", "task", "execute"):
            with self.subTest(mode=mode):
                result = self.parse("$work " + mode + " --" + tail)
                self.assertEqual(result, {"schema": "work-invocation/v1", "mode": mode,
                                          "request": tail, "entry": {"kind": "workflow"}})

    def test_bom_and_header_whitespace(self):
        result = self.parse("\ufeff \t$work\tplan\n--\n需求")
        self.assertEqual(result["request"], "\n需求")
        self.assertEqual(result["mode"], "plan")

    def test_rejects_missing_fields_extra_headers_and_private_modes(self):
        cases = {
            "請使用 $work plan -- example": "work_invocation_not_explicit",
            "`$work plan -- example`": "work_invocation_not_explicit",
            "$workflow plan -- example": "work_invocation_not_explicit",
            "$work": "work_invocation_mode_missing",
            "$work -- example": "work_invocation_mode_missing",
            "$work Plan -- example": "work_invocation_mode_invalid",
            "$work artifact-editor -- example": "work_invocation_mode_invalid",
            "$work progress-saver -- example": "work_invocation_mode_invalid",
            "$work task web/backend -- example": "work_invocation_delimiter",
            "$work plan --request": "work_invocation_delimiter",
            "$work execute": "work_invocation_delimiter",
            "$work plan -- \n\t": "work_invocation_request_missing",
        }
        for value, code in cases.items():
            with self.subTest(value=value), self.assertRaises(WorkError) as error:
                self.parse(value)
            self.assertEqual(error.exception.code, code)

    def test_explicit_resume_and_task_planning_routes(self):
        for mode in ("plan", "task"):
            result = self.parse(f"$work {mode} -- resume example-12\n")
            self.assertEqual(result["entry"], {"kind": "progress_resume", "requirement_id": "example-12"})
        self.assertEqual(self.parse("$work task -- example-12")["entry"],
                         {"kind": "task_planning", "requirement_id": "example-12"})
        for value in ("$work execute -- resume example", "$work plan -- example", "$work task -- 規劃功能"):
            self.assertEqual(self.parse(value)["entry"], {"kind": "workflow"})

    def test_invalid_resume_identity_is_not_silently_reinterpreted(self):
        for request in ("resume", "resume example more", "resume ../example", "resume EXAMPLE", "resume .."):
            with self.subTest(request=request), self.assertRaises(WorkError):
                self.parse("$work plan -- " + request)

    def test_invalid_utf8_is_rejected(self):
        with self.assertRaises(WorkError) as error:
            parse_invocation(b"$work plan -- \xff", source="invocation")
        self.assertEqual(error.exception.code, "invalid_utf8")
