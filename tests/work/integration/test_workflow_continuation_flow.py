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

from artifacts import test_specification as fixtures
from worklib.contracts.execution_index import build_initial_execution_index, render_execution_index
from worklib.contracts.plan import render_plan_contract
from worklib.contracts.task import render_task_contract, validate_task_contract
from worklib.foundation.fingerprint import raw_sha256


class WorkflowContinuationFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.requests = self.root / (
            "outputs/work/transactions/example/specification/"
            "20260915T103000Z-a1b2c3d4"
        )
        self.requests.mkdir(parents=True)
        self.constraint = {
            "id": "CONSTRAINT-001",
            "statement": "Use the original boundary.",
            "applies_to": ["GOAL-001"],
        }
        self.fixture.plan["constraints"] = [self.constraint]
        self.fixture.plan_path.write_bytes(render_plan_contract(self.fixture.plan))
        self.fixture.task["source_plan"]["canonical_sha256"] = raw_sha256(
            self.fixture.plan_path.read_bytes()
        )
        self.fixture.task_path.write_bytes(render_task_contract(self.fixture.task))
        validation = validate_task_contract(
            self.fixture.task_path.read_bytes(),
            source="continuation integration fixture",
            actual_task_path=self.fixture.artifacts["task"],
            project_root=self.root,
            user_config_root=str(self.root),
        )
        self.fixture.index = build_initial_execution_index(self.fixture.task, validation)
        self.fixture.index_path.write_bytes(render_execution_index(self.fixture.index))

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
                "plan_path": self.fixture.artifacts["plan"],
                "reason": "Confirm the constraint wording and save progress.",
                "edits": [
                    {
                        "artifact": "plan",
                        "field": "constraints",
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
        installed = json.loads(self.fixture.plan_path.read_text(encoding="utf-8"))
        self.assertEqual(
            installed["constraints"][0]["statement"],
            "Use the confirmed boundary.",
        )

    def test_failed_verification_stops_before_checkpoint_save(self) -> None:
        specification, _progress, approval = self.prepare_continuation()
        verification = self.publish_and_verification_request(
            specification, approval["specification"]
        )
        changed = json.loads(self.fixture.plan_path.read_text(encoding="utf-8"))
        changed["summary"] = "Unexpected drift after publication."
        self.fixture.plan_path.write_bytes(render_plan_contract(changed))

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
        self.assertEqual(rejected["reason_code"], "specification_verification_failed")
        self.assertFalse((self.root / "outputs/work/progress/example/plan/progress.json").exists())


if __name__ == "__main__":
    unittest.main()
