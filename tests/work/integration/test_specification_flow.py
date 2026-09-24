from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_task_collection as fixtures
from worklib.business_services.task import load_task_collection
from worklib.services.attempt import build_initial_execution_index, render_execution_index
from worklib.business_services.plan import render_plan_contract
from worklib.business_services.task.index import render_task_index_contract
from worklib.technical.foundation.fingerprint import raw_sha256
from worklib.technical.infrastructure.json_contract import parse_json_contract


class SpecificationFlowTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        requests = tempfile.TemporaryDirectory(prefix="specification-flow-")
        self.addCleanup(requests.cleanup)
        self.requests = Path(requests.name)
        self.plan_path = self.root / self.fixture.fixture.artifacts["plan"]
        self.task_path = self.fixture.index_path
        self.constraint = {
            "id": "CONSTRAINT-001", "statement": "Use the original boundary.",
            "applies_to": ["GOAL-001"],
        }
        plan = parse_json_contract(self.plan_path.read_bytes(), source="integration Plan")
        plan["constraints"] = [self.constraint]
        self.plan_path.write_bytes(render_plan_contract(plan))
        index = copy.deepcopy(self.fixture.index)
        index["source_plan"]["canonical_sha256"] = raw_sha256(self.plan_path.read_bytes())
        (self.root / self.task_path).write_bytes(render_task_index_contract(index))
        validation = load_task_collection(self.root, str(self.root), self.task_path)
        self.artifacts = validation["collection_contract"]["artifacts"]
        execution_path = self.root / self.artifacts["execution"] / "index.json"
        execution_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.write_bytes(render_execution_index(build_initial_execution_index(validation["collection_contract"], validation)))

    def request_file(self, name, value):
        path = self.requests / name
        path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8", newline="\n")
        return path

    def run_cli(self, command, request_path, *extra):
        result = subprocess.run(
            [sys.executable, "-B", str(SCRIPT_ROOT / "work.py"),
             "--project-root", str(self.root), "--verbose", "task", command,
             "--input-file", str(request_path), "--user-config-root", str(self.root), *extra],
            stdin=subprocess.DEVNULL, capture_output=True, text=True, encoding="utf-8",
            shell=False, timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(result.stderr, "")
        response = json.loads(result.stdout)
        self.assertEqual(response["schema"], "work-cli-result/v1")
        return response["data"]

    def test_constraints_prepare_summary_update_and_verify_across_processes(self):
        changed = copy.deepcopy(self.constraint)
        changed["statement"] = "Use the confirmed boundary."
        edits = self.request_file("edits.json", {
            "schema": "work-spec-prepare-request/v1",
            "requirement_id": "example",
            "reason": "Confirm the constraint wording.",
            "edits": [{
                "target": {"artifact": "plan"},
                "field": "constraints", "semantic_after": [{
                    "key": "boundary", "existing_position": 1,
                    "statement": changed["statement"],
                    "applies_to": [{"collection": "goals", "position": 1}],
                }],
            }],
        })
        prepared_path = self.requests / "prepared.json"
        prepared = self.run_cli(
            "spec-prepare", edits, "--output-file", str(prepared_path), "--summary",
        )
        self.assertEqual(prepared["schema"], "work-specification-summary/v1")
        self.assertEqual(prepared["changed_fields"], ["/plan/constraints", "/task_index/source_plan"])
        self.assertEqual(prepared["next_step"]["command"], "task spec-validate")
        for omitted in ("request", "candidate", "preview"):
            self.assertNotIn(omitted, prepared)
        request = json.loads(prepared_path.read_text(encoding="utf-8"))
        self.assertEqual(request["schema"], "work-spec-update-request/v1")

        validated = self.run_cli("spec-validate", prepared_path, "--summary")
        self.assertEqual(validated["next_step"]["command"], "task spec-update")
        approval = validated["approved_sha256"]
        self.assertEqual(validated["next_step"]["approved_sha256"], approval)

        published = self.run_cli(
            "spec-update", prepared_path, "--approved-sha256", approval, "--summary",
        )
        self.assertEqual(published["status"], "updated")
        self.assertEqual(published["next_step"]["command"], "task spec-verify")
        installed = json.loads(self.plan_path.read_text(encoding="utf-8"))
        self.assertEqual(installed["constraints"], [changed])

        verification_path = self.request_file("verification.json", published["verification_request"])
        verified = self.run_cli("spec-verify", verification_path)
        self.assertTrue(verified["verified"])
        self.assertEqual(verified["verification_scope"], "exact_specification_result")
        self.assertFalse(verified["execution_authorized"])
        self.assertEqual(verified["next_step"], "normal_execute_preflight")


if __name__ == "__main__":
    unittest.main()
