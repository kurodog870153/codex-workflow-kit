from __future__ import annotations

import io
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase
from contracts import test_delegation as fixtures
from worklib.cli import main


class DelegationCommandTests(FileInputTestCase):
    def test_build_derives_transport_and_plan_context_from_formal_plan(self):
        fixture = fixtures.DelegationTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        request = {"schema": "work-delegation-build-request/v1", "role": "task-coordinator",
                   "request": "Coordinate the confirmed TASK work.",
                   "source_plan_path": fixture.fixture.artifacts["plan"]}
        args = self.input_arguments(["--project-root", str(fixture.root), "delegation", "build",
                                     "--input-file", "build.json"], json.dumps(request))
        out = io.StringIO()
        self.assertEqual(main(args, stdout=out), 0, out.getvalue())
        data = json.loads(out.getvalue())["data"]
        self.assertEqual(data["sender"], "parent")
        self.assertEqual(data["marker"], "WORK_DELEGATION_V1")
        self.assertEqual(data["context"]["source_plan"], fixture.plan)
        self.assertNotIn("authorized", data)

    def test_cli_uses_independent_role_and_json_input_without_writes(self):
        fixture = fixtures.DelegationTests()
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        before = {str(path): path.read_bytes() for path in fixture.root.rglob("*") if path.is_file()}
        args = self.input_arguments(["--project-root", str(fixture.root), "delegation", "validate", "--role", "plan",
            "--sender", "parent", "--input-file", "envelope.json"], json.dumps(fixture.envelope()))
        out = io.StringIO()
        self.assertEqual(main(args, stdout=out), 0, out.getvalue())
        self.assertFalse(json.loads(out.getvalue())["data"]["grants_authorization"])
        args[args.index("--role") + 1] = "execute"
        out = io.StringIO()
        self.assertNotEqual(main(args, stdout=out), 0)
        self.assertEqual(json.loads(out.getvalue())["reason_code"], "delegation_boundary_mismatch")
        self.assertEqual(before, {str(path): path.read_bytes() for path in fixture.root.rglob("*") if path.is_file()})
