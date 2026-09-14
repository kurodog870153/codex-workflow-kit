from __future__ import annotations

import io
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase
from worklib.cli import main


class InvocationCommandTests(FileInputTestCase):
    def test_cli_reads_plain_text_without_writing_project_or_executing_request(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            raw = '$work execute -- $(touch should-not-exist) -- "literal"\n'
            args = self.input_arguments(["--project-root", str(root), "invocation", "parse",
                                        "--input-file", "invocation.txt"], raw)
            out = io.StringIO()
            self.assertEqual(main(args, stdout=out), 0, out.getvalue())
            self.assertEqual(json.loads(out.getvalue())["data"]["request"], raw.split("--", 1)[1])
            self.assertEqual(list(root.iterdir()), [])

    def test_cli_invalid_invocation_returns_structured_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = self.input_arguments(["--project-root", temporary, "invocation", "parse",
                                        "--input-file", "invocation.txt"], "$work -- request")
            out = io.StringIO()
            self.assertNotEqual(main(args, stdout=out), 0)
            self.assertEqual(json.loads(out.getvalue())["reason_code"], "work_invocation_mode_missing")
