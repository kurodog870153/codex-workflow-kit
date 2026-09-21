from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts.test_task import TaskInstructionContractTests
from worklib.business_services.task import (
    load_task_collection,
    load_task_closure,
    load_task_execution_context,
)
from worklib.business_services.plan import render_plan_contract, validate_plan_contract
from worklib.business_services.task.index import render_task_index_contract
from worklib.business_services.task.item import render_task_item_contract, validate_task_item_contract
from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.json_contract import parse_json_contract


class TaskCollectionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = TaskInstructionContractTests("test_valid_task_returns_instruction_fingerprints")
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.project_root
        self.legacy_path = self.fixture.artifacts["task"]
        self.index_path = "outputs/work/tasks/example/index.json"

        plan_path = self.root / self.fixture.artifacts["plan"]
        plan = parse_json_contract(plan_path.read_bytes(), source="test Plan")
        plan["artifacts"]["task"] = self.index_path
        plan_path.write_bytes(render_plan_contract(plan))
        plan_validation = validate_plan_contract(
            plan_path.read_bytes(),
            source="test Plan",
            actual_plan_path=self.fixture.artifacts["plan"],
            project_root=self.root,
            user_config_root=str(self.root),
            _allow_task_index=True,
        )

        logical = copy.deepcopy(self.fixture.contract)
        logical["artifacts"]["task"] = self.index_path  # type: ignore[index]
        logical["source_plan"]["canonical_sha256"] = plan_validation["plan_sha256"]  # type: ignore[index]
        raw_task = logical.pop("tasks")[0]  # type: ignore[union-attr,index]
        self.item = {"schema": "work-task-item/v1", **raw_task}
        self.item_raw = render_task_item_contract(self.item)
        item_validation = validate_task_item_contract(
            self.item_raw, source="test item", expected_task_id="TASK-001"
        )
        self.index = {
            **logical,
            "schema": "work-task-index/v1",
            "tasks": [
                {
                    "id": "TASK-001",
                    "path": "tasks/TASK-001.json",
                    "canonical_sha256": item_validation["task_item_sha256"],
                }
            ],
        }
        self.index_raw = render_task_index_contract(self.index)
        index_file = self.root / self.index_path
        item_file = index_file.parent / "tasks" / "TASK-001.json"
        item_file.parent.mkdir(parents=True)
        index_file.write_bytes(self.index_raw)
        item_file.write_bytes(self.item_raw)

    def test_loads_complete_collection_and_rejects_single_file_artifact(self) -> None:
        collection = load_task_collection(
            self.root, str(self.root), self.index_path
        )
        self.assertEqual(collection["task_ids"], ["TASK-001"])
        self.assertEqual(collection["task_count"], 1)
        self.assertEqual(len(collection["task_collection_sha256"]), 64)

        legacy_root = self.fixture.project_root
        legacy_plan_path = legacy_root / self.fixture.artifacts["plan"]
        plan = parse_json_contract(legacy_plan_path.read_bytes(), source="test Plan")
        plan["artifacts"]["task"] = self.legacy_path
        legacy_plan_path.write_bytes(render_plan_contract(plan))
        plan_validation = validate_plan_contract(
            legacy_plan_path.read_bytes(),
            source="test Plan",
            actual_plan_path=self.fixture.artifacts["plan"],
            project_root=legacy_root,
            user_config_root=str(legacy_root),
        )
        contract = copy.deepcopy(self.fixture.contract)
        contract["source_plan"]["canonical_sha256"] = plan_validation["plan_sha256"]  # type: ignore[index]
        from worklib.business_services.task.document import render_task_contract
        legacy_file = legacy_root / self.legacy_path
        legacy_file.parent.mkdir(parents=True, exist_ok=True)
        legacy_file.write_bytes(render_task_contract(contract))
        with self.assertRaises(WorkError):
            load_task_collection(legacy_root, str(legacy_root), self.legacy_path)

    def test_rejects_missing_or_orphan_items(self) -> None:
        item_file = self.root / self.index_path
        item_file = item_file.parent / "tasks" / "TASK-001.json"
        item_file.unlink()
        with self.assertRaises(WorkError) as context:
            load_task_collection(self.root, str(self.root), self.index_path)
        self.assertEqual(context.exception.code, "file_not_found")

        item_file.write_bytes(self.item_raw)
        (item_file.parent / "TASK-999.json").write_bytes(self.item_raw)
        with self.assertRaises(WorkError) as context:
            load_task_collection(self.root, str(self.root), self.index_path)
        self.assertEqual(context.exception.code, "task_collection_directory_mismatch")
        self.assertEqual(context.exception.details["orphan"], ["TASK-999.json"])

    def test_rejects_item_fingerprint_mismatch(self) -> None:
        index = copy.deepcopy(self.index)
        index["tasks"][0]["canonical_sha256"] = "0" * 64  # type: ignore[index]
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))

        with self.assertRaises(WorkError) as context:
            load_task_collection(self.root, str(self.root), self.index_path)

        self.assertEqual(context.exception.code, "task_item_fingerprint_mismatch")

    def test_loads_target_dependency_closure_in_index_order(self) -> None:
        second = copy.deepcopy(self.item)
        second["id"] = "TASK-002"
        second["dependencies"] = ["TASK-001"]
        second_raw = render_task_item_contract(second)
        second_validation = validate_task_item_contract(
            second_raw, source="test item", expected_task_id="TASK-002"
        )
        index = copy.deepcopy(self.index)
        index["tasks"].append(  # type: ignore[union-attr]
            {
                "id": "TASK-002",
                "path": "tasks/TASK-002.json",
                "canonical_sha256": second_validation["task_item_sha256"],
            }
        )
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))
        (self.root / self.index_path).parent.joinpath("tasks/TASK-002.json").write_bytes(second_raw)

        closure = load_task_closure(self.root, self.index_path, "TASK-002")

        self.assertEqual(list(closure), ["TASK-001", "TASK-002"])

    def test_execution_context_validates_all_items_but_exposes_target_closure(self) -> None:
        second = copy.deepcopy(self.item)
        second["id"] = "TASK-002"
        second_raw = render_task_item_contract(second)
        second_validation = validate_task_item_contract(
            second_raw, source="test item", expected_task_id="TASK-002"
        )
        index = copy.deepcopy(self.index)
        index["tasks"].append(  # type: ignore[union-attr]
            {
                "id": "TASK-002",
                "path": "tasks/TASK-002.json",
                "canonical_sha256": second_validation["task_item_sha256"],
            }
        )
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))
        (self.root / self.index_path).parent.joinpath(
            "tasks/TASK-002.json"
        ).write_bytes(second_raw)

        context = load_task_execution_context(
            self.root,
            str(self.root),
            self.index_path,
            "TASK-001",
        )

        contract = context["contract"]
        validation = context["validation"]
        self.assertEqual(
            [task["id"] for task in contract["tasks"]],  # type: ignore[index]
            ["TASK-001"],
        )
        self.assertEqual(validation["task_ids"], ["TASK-001", "TASK-002"])  # type: ignore[index]
        self.assertEqual(
            set(validation["task_item_sha256"]),  # type: ignore[index]
            {"TASK-001", "TASK-002"},
        )
        self.assertEqual(
            set(validation["task_instructions_sha256"]),  # type: ignore[index]
            {"TASK-001", "TASK-002"},
        )

    def test_closure_does_not_read_unrelated_item(self) -> None:
        second = copy.deepcopy(self.item)
        second["id"] = "TASK-002"
        second_raw = render_task_item_contract(second)
        second_validation = validate_task_item_contract(
            second_raw, source="test item", expected_task_id="TASK-002"
        )
        index = copy.deepcopy(self.index)
        index["tasks"].append(  # type: ignore[union-attr]
            {
                "id": "TASK-002",
                "path": "tasks/TASK-002.json",
                "canonical_sha256": second_validation["task_item_sha256"],
            }
        )
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))
        (self.root / self.index_path).parent.joinpath(
            "tasks/TASK-002.json"
        ).write_bytes(b"broken unrelated item")

        closure = load_task_closure(self.root, self.index_path, "TASK-001")

        self.assertEqual(list(closure), ["TASK-001"])

    def test_collection_reuses_cross_task_validation(self) -> None:
        invalid = copy.deepcopy(self.item)
        invalid["dependencies"] = ["TASK-999"]
        invalid_raw = render_task_item_contract(invalid)
        invalid_validation = validate_task_item_contract(
            invalid_raw, source="test item", expected_task_id="TASK-001"
        )
        index = copy.deepcopy(self.index)
        index["tasks"][0]["canonical_sha256"] = invalid_validation["task_item_sha256"]  # type: ignore[index]
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))
        (self.root / self.index_path).parent.joinpath("tasks/TASK-001.json").write_bytes(invalid_raw)

        with self.assertRaises(WorkError) as context:
            load_task_collection(self.root, str(self.root), self.index_path)

        self.assertEqual(context.exception.code, "invalid_task_dependency")


if __name__ == "__main__":
    unittest.main()
