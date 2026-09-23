from __future__ import annotations

import copy
import json
import subprocess
import sys
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


class WorkflowContinuationFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = fixtures.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.requests = self.root / (
            "outputs/work/transactions/example/specification/"
            "20260915T103000Z-a1b2c3d4"
        )
        self.requests.mkdir(parents=True)
        self.plan_path = self.root / self.fixture.fixture.artifacts["plan"]
        self.task_path = self.fixture.index_path
        self.constraint = {
            "id": "CONSTRAINT-001",
            "statement": "Use the original boundary.",
            "applies_to": ["GOAL-001"],
        }
        plan = parse_json_contract(self.plan_path.read_bytes(), source="continuation Plan")
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

    def request_file(self, name: str, value: object) -> Path:
        path = self.requests / name
        path.write_text(
            json.dumps(value, ensure_ascii=False, indent=2),
            encoding="utf-8",
            newline="\n",
        )
        return path

    def run_cli(self, arguments: list[str], *, expected: int = 0) -> dict[str, object]:
        result = subprocess.run(
            [
                sys.executable,
                "-B",
                str(SCRIPT_ROOT / "work.py"),
                "--project-root",
                str(self.root),
                "--verbose",
                *arguments,
            ],
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            encoding="utf-8",
            shell=False,
            timeout=60,
        )
        self.assertEqual(result.returncode, expected, result.stdout)
        self.assertEqual(result.stderr, "")
        response = json.loads(result.stdout)
        self.assertEqual(response["schema"], "work-cli-result/v1")
        return response

    def prepare_continuation(self) -> tuple[Path, Path, dict[str, str]]:
        changed = copy.deepcopy(self.constraint)
        changed["statement"] = "Use the confirmed boundary."
        edits = self.request_file(
            "spec-prepare.json",
            {
                "schema": "work-spec-prepare-request/v1",
                "plan_path": self.artifacts["plan"],
                "reason": "Confirm the constraint wording and save progress.",
                "edits": [
                    {
                        "artifact": "plan",
                        "operation": "replace",
                        "path": "/constraints",
                        "before": [self.constraint],
                        "after": [changed],
                        "affected_ids": ["CONSTRAINT-001"],
                    }
                ],
            },
        )
        specification = self.requests / "spec-prepared-request.json"
        self.run_cli(
            [
                "task",
                "spec-prepare",
                "--input-file",
                str(edits),
                "--output-file",
                str(specification),
                "--user-config-root",
                str(self.root),
            ]
        )
        spec_preview = self.run_cli(
            [
                "task",
                "spec-validate",
                "--input-file",
                str(specification),
                "--user-config-root",
                str(self.root),
            ]
        )["data"]

        content = self.request_file(
            "progress-content.json",
            {
                "title": "Specification continuation",
                "request": "Update the confirmed constraint and preserve the checkpoint.",
                "current_task_id": None,
                "context": {"affected_ids": ["CONSTRAINT-001"]},
                "source_status": [],
                "notes": ["The specification candidate was reviewed."],
                "confirmed_decisions": [
                    {"statement": "Use the confirmed constraint wording."}
                ],
                "tentative": [],
                "open_questions": [],
                "next_discussion_point": "Continue from the verified specification.",
            },
        )
        progress_prepared = self.run_cli(
            [
                "progress",
                "prepare",
                "--input-file",
                str(content),
                "--requirement-id",
                "example",
                "--mode",
                "plan",
                "--expected-revision",
                "0",
            ]
        )["data"]
        progress = self.request_file("progress-candidate.json", progress_prepared["progress"])
        progress_preview = self.run_cli(
            [
                "progress",
                "validate",
                "--input-file",
                str(progress),
                "--expected-revision",
                "0",
            ]
        )["data"]
        continuation_approval = {
            "specification": spec_preview["approved_sha256"],
            "progress": progress_preview["approved_sha256"],
        }
        return specification, progress, continuation_approval

    def publish_and_verification_request(
        self, specification: Path, approval: str
    ) -> Path:
        published = self.run_cli(
            [
                "task",
                "spec-update",
                "--input-file",
                str(specification),
                "--user-config-root",
                str(self.root),
                "--approved-sha256",
                approval,
            ]
        )["data"]
        self.assertEqual(published["status"], "updated")
        return self.request_file("spec-verification-request.json", published["verification_request"])

    def test_one_approval_continues_through_publish_verify_and_checkpoint(self) -> None:
        specification, progress, approval = self.prepare_continuation()
        verification = self.publish_and_verification_request(
            specification, approval["specification"]
        )
        verified = self.run_cli(
            [
                "task",
                "spec-verify",
                "--input-file",
                str(verification),
                "--user-config-root",
                str(self.root),
            ]
        )["data"]
        self.assertTrue(verified["verified"])

        saved = self.run_cli(
            [
                "progress",
                "save",
                "--input-file",
                str(progress),
                "--expected-revision",
                "0",
                "--approved-sha256",
                approval["progress"],
            ]
        )["data"]
        restored = self.run_cli(
            [
                "progress",
                "read",
                "--requirement-id",
                "example",
                "--mode",
                "plan",
            ]
        )["data"]
        self.assertEqual(saved["sha256"], restored["sha256"])
        self.assertEqual(restored["progress"]["revision"], 1)
        installed = json.loads(self.plan_path.read_text(encoding="utf-8"))
        self.assertEqual(
            installed["constraints"][0]["statement"],
            "Use the confirmed boundary.",
        )

    def test_failed_verification_stops_before_checkpoint_save(self) -> None:
        specification, _progress, approval = self.prepare_continuation()
        verification = self.publish_and_verification_request(
            specification, approval["specification"]
        )
        changed = json.loads(self.plan_path.read_text(encoding="utf-8"))
        changed["summary"] = "Unexpected drift after publication."
        self.plan_path.write_bytes(render_plan_contract(changed))

        rejected = self.run_cli(
            [
                "task",
                "spec-verify",
                "--input-file",
                str(verification),
                "--user-config-root",
                str(self.root),
            ],
            expected=5,
        )
        self.assertEqual(rejected["reason_code"], "spec_verify_state_changed")
        self.assertFalse((self.root / "outputs/work/progress/example/plan/progress.json").exists())


if __name__ == "__main__":
    unittest.main()
