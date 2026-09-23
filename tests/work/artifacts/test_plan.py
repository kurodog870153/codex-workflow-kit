from __future__ import annotations

import sys
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from worklib.business_services.plan import create_plan_file, prepare_semantic_plan
from contracts import test_plan as fixtures
from worklib.models.common.errors import ExitCode, WorkError


class PlanArtifactTests(unittest.TestCase):
    def test_create_writes_validated_bytes_without_overwriting(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project_root = Path(temporary).resolve()
            validation = {
                "schema": "work-plan-validation/v1",
                "requirement_id": "example",
                "status": "confirmed",
                "plan_sha256": "a" * 64,
            }
            rendered = b"canonical plan"
            with patch(
                "worklib.business_services.plan.prepare_plan_json_contract",
                return_value=(validation, rendered),
            ), patch(
                "worklib.business_services.plan.validate_plan_file",
                return_value=validation,
            ):
                result = create_plan_file(
                    b"request",
                    source="test",
                    raw_plan_path="outputs/plan.md",
                    project_root=project_root,
                    user_config_root=temporary,
                )
                path = project_root / "outputs" / "plan.md"
                self.assertEqual(path.read_bytes(), rendered)
                self.assertEqual(result["schema"], "work-plan-create/v1")
                self.assertEqual(result["path"], "outputs/plan.md")

                with self.assertRaises(WorkError) as context:
                    create_plan_file(
                        b"request",
                        source="test",
                        raw_plan_path="outputs/plan.md",
                        project_root=project_root,
                        user_config_root=temporary,
                    )
                self.assertEqual(context.exception.exit_code, ExitCode.WORKFLOW_STATE)
                self.assertEqual(context.exception.code, "plan_already_exists")
                self.assertEqual(path.read_bytes(), rendered)


class InitialPlanPreparationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.PlanInstructionContractTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        contract = self.fixture.contract
        self.root = self.fixture.project_root
        self.request = {"requirement_id": contract["requirement_id"],
            "title": contract["title"], "summary": contract["summary"],
            "goals": [item["statement"] for item in contract["goals"]],
            "scope": [item["statement"] for item in contract["scope"]],
            "deliverables": [item["statement"] for item in contract["deliverables"]],
            "acceptance_criteria": [item["statement"] for item in contract["acceptance_criteria"]],
            "references": []}
        self.request["hierarchy_selection_request"] = {"decision": "general_only", "selections": []}
        self.request["skill_selection_request"] = {"decision": "base_only", "skills": []}

    def prepare(self):
        return prepare_semantic_plan(json.dumps(self.request).encode(), source="test",
                                    project_root=self.root, user_config_root=str(self.root))

    def test_default_candidate_is_read_only_and_can_use_existing_create(self):
        result = self.prepare()
        expected = copy.deepcopy(self.fixture.contract)
        expected["artifacts"]["task"] = "outputs/work/tasks/example/index.json"
        self.assertEqual(result["plan"], expected)
        self.assertEqual(list(self.root.iterdir()), [])
        created = create_plan_file(json.dumps(result["plan"]).encode(), source="candidate",
            raw_plan_path=result["path"], project_root=self.root, user_config_root=str(self.root))
        self.assertEqual(created["plan_sha256"], result["validation"]["plan_sha256"])

    def test_missing_content_and_machine_field_override_are_rejected(self):
        original = copy.deepcopy(self.request)
        for change in ("missing", "status", "content"):
            with self.subTest(change=change):
                self.request = copy.deepcopy(original)
                if change == "missing":
                    del self.request["goals"]
                else:
                    self.request[change] = []
                with self.assertRaises(WorkError):
                    self.prepare()
                self.assertEqual(list(self.root.iterdir()), [])

    def test_old_full_prepare_request_is_rejected(self):
        old = {"requirement_id": self.request["requirement_id"],
               "content": {"title": self.request["title"]},
               "hierarchy_selection": self.fixture.contract["hierarchy_selection"],
               "skill_selection": self.fixture.contract["skill_selection"], "references": []}
        with self.assertRaises(WorkError) as caught:
            prepare_semantic_plan(json.dumps(old).encode(), source="test",
                project_root=self.root, user_config_root=str(self.root))
        self.assertEqual(caught.exception.code, "invalid_object_fields")

    def test_invalid_selection_choices_and_references_are_rejected(self):
        original = copy.deepcopy(self.request)
        for change in ("hierarchy", "skill", "references"):
            with self.subTest(change=change):
                self.request = copy.deepcopy(original)
                if change == "references":
                    self.request["references"] = ["unknown.reference"]
                elif change == "hierarchy":
                    self.request["hierarchy_selection_request"]["decision"] = "instruction_paths"
                else:
                    self.request["skill_selection_request"]["decision"] = "external_skills"
                with self.assertRaises(WorkError):
                    self.prepare()
                self.assertEqual(list(self.root.iterdir()), [])

    def test_existing_plan_is_preserved(self):
        path = self.root / self.fixture.contract["artifacts"]["plan"]
        path.parent.mkdir(parents=True)
        path.write_bytes(b"preserve existing Plan")
        with self.assertRaises(WorkError) as caught:
            self.prepare()
        self.assertEqual(caught.exception.code, "plan_already_exists")
        self.assertEqual(path.read_bytes(), b"preserve existing Plan")

    def test_instruction_drift_is_rejected_by_final_validator(self):
        from worklib.services.instruction.work_selection import build_work_instruction_selection

        def stale(loaded):
            result = build_work_instruction_selection(loaded)
            result["instructions_sha256"] = "0" * 64
            return result

        with patch("worklib.business_services.plan.build_work_selection", side_effect=stale):
            with self.assertRaises(WorkError):
                self.prepare()
        self.assertEqual(list(self.root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
