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

from worklib.services.plan import create_plan_file, prepare_initial_plan
from contracts import test_plan as fixtures
from worklib.foundation.errors import ExitCode, WorkError


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
                "worklib.services.plan.prepare_plan_json_contract",
                return_value=(validation, rendered),
            ), patch(
                "worklib.services.plan.validate_plan_file",
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
        self.request = {
            "requirement_id": contract["requirement_id"],
            "content": {key: copy.deepcopy(contract[key]) for key in (
                "title", "summary", "goals", "scope", "deliverables", "acceptance_criteria")},
            "hierarchy_selection": copy.deepcopy(contract["hierarchy_selection"]),
            "skill_selection": copy.deepcopy(contract["skill_selection"]), "references": [],
        }

    def prepare(self):
        return prepare_initial_plan(json.dumps(self.request).encode(), source="test",
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

    def test_custom_paths_require_all_three_and_preserve_routing(self):
        self.request["artifacts"] = {"plan": "custom/plans/example.json",
            "task": "custom/tasks/example/task.json", "execution": "custom/executions/example"}
        self.assertEqual(self.prepare()["plan"]["artifacts"], self.request["artifacts"])
        del self.request["artifacts"]["execution"]
        with self.assertRaises(WorkError):
            self.prepare()
        self.assertEqual(list(self.root.iterdir()), [])

    def test_missing_content_and_machine_field_override_are_rejected(self):
        original = copy.deepcopy(self.request)
        for change in ("missing", "status", "changes"):
            with self.subTest(change=change):
                self.request = copy.deepcopy(original)
                if change == "missing":
                    del self.request["content"]["goals"]
                else:
                    self.request["content"][change] = []
                with self.assertRaises(WorkError):
                    self.prepare()
                self.assertEqual(list(self.root.iterdir()), [])

    def test_stale_selection_and_invalid_references_are_rejected(self):
        original = copy.deepcopy(self.request)
        for change in ("hierarchy", "skill", "references"):
            with self.subTest(change=change):
                self.request = copy.deepcopy(original)
                if change == "references":
                    self.request["references"] = ["unknown.reference"]
                else:
                    self.request[change + "_selection"]["selection_sha256"] = "0" * 64
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
        from worklib.services.instruction_work_selection import build_work_instruction_selection

        def stale(**kwargs):
            result = build_work_instruction_selection(**kwargs)
            result["instructions_sha256"] = "0" * 64
            return result

        with patch("worklib.services.plan.build_work_instruction_selection", side_effect=stale):
            with self.assertRaises(WorkError):
                self.prepare()
        self.assertEqual(list(self.root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
