from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from tests.work.contracts import test_task_collection
from worklib.services.specification import prepare_specification, update_specification
from worklib.services.task_collection import (
    load_task_collection,
    load_task_execution_context,
)
from worklib.contracts.execution_index import (
    build_initial_execution_index,
    render_execution_index,
)
from worklib.contracts.task_index import render_task_index_contract
from worklib.contracts.task_item import (
    render_task_item_contract,
    validate_task_item_contract,
)
from worklib.foundation import spec_transactions
from worklib.foundation.fingerprint import read_raw


TASK_COUNT = 100


class LargeTaskCollectionIOTests(unittest.TestCase):
    def setUp(self) -> None:
        fixture = test_task_collection.TaskCollectionTests(
            "test_loads_complete_collection_and_rejects_single_file_artifact"
        )
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.root = fixture.root
        self.index_path = fixture.index_path
        self.plan_path = fixture.fixture.artifacts["plan"]
        self.collection = (self.root / self.index_path).parent

        references = []
        for number in range(1, TASK_COUNT + 1):
            task_id = f"TASK-{number:03d}"
            item = copy.deepcopy(fixture.item)
            item["id"] = task_id
            item["title"] = f"Large fixture task {number}"
            item["goal"] = f"Validate large fixture task {number}."
            if number > 1:
                item["files"] = [
                    {
                        "id": "FILE-001",
                        "action": "create",
                        "path": f"generated/{task_id}.txt",
                    }
                ]
            if number == TASK_COUNT:
                item["dependencies"] = ["TASK-001"]
            raw = render_task_item_contract(item)
            validation = validate_task_item_contract(
                raw, source=task_id, expected_task_id=task_id
            )
            relative = f"tasks/{task_id}.json"
            (self.collection / relative).write_bytes(raw)
            references.append(
                {
                    "id": task_id,
                    "path": relative,
                    "canonical_sha256": validation["task_item_sha256"],
                }
            )

        index = copy.deepcopy(fixture.index)
        index["tasks"] = references
        (self.root / self.index_path).write_bytes(render_task_index_contract(index))
        validation = load_task_collection(
            self.root, str(self.root), self.index_path
        )
        self.execution_dir = validation["collection_contract"]["artifacts"][
            "execution"
        ]
        execution_path = self.root / self.execution_dir / "index.json"
        execution_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.write_bytes(
            render_execution_index(
                build_initial_execution_index(
                    validation["collection_contract"], validation
                )
            )
        )

    def _measure_collection_reads(self, operation):
        paths: list[Path] = []
        byte_count = 0

        def measured(path: Path) -> bytes:
            nonlocal byte_count
            raw = read_raw(path)
            paths.append(Path(path))
            byte_count += len(raw)
            return raw

        with patch(
            "worklib.infrastructure.task_collection.read_raw", side_effect=measured
        ):
            result = operation()
        return result, paths, byte_count

    def test_execute_validates_all_items_but_exposes_target_dependency_closure(
        self,
    ) -> None:
        target = f"TASK-{TASK_COUNT:03d}"
        context, execute_paths, execute_bytes = self._measure_collection_reads(
            lambda: load_task_execution_context(
                self.root,
                str(self.root),
                self.index_path,
                target,
            )
        )
        execute_names = [path.name for path in execute_paths]
        self.assertEqual(
            execute_names,
            ["index.json", f"{target}.json", "TASK-001.json"]
            + [f"TASK-{number:03d}.json" for number in range(2, TASK_COUNT)],
        )
        self.assertEqual(
            [task["id"] for task in context["contract"]["tasks"]],
            ["TASK-001", target],
        )

        _, full_paths, full_bytes = self._measure_collection_reads(
            lambda: load_task_collection(
                self.root, str(self.root), self.index_path
            )
        )
        self.assertEqual(len(full_paths), TASK_COUNT + 1)
        self.assertEqual(full_bytes, execute_bytes)
        self.assertIn("TASK-050.json", execute_names)

    def test_single_item_update_preserves_unrelated_item_bytes(self) -> None:
        target = f"TASK-{TASK_COUNT:03d}"
        item_paths = {
            f"TASK-{number:03d}": self.collection
            / "tasks"
            / f"TASK-{number:03d}.json"
            for number in range(1, TASK_COUNT + 1)
        }
        before = {task_id: path.read_bytes() for task_id, path in item_paths.items()}
        current = json.loads(before[target])
        request = {
            "schema": "work-spec-prepare-request/v1",
            "plan_path": self.plan_path,
            "reason": "Measure one-item publication",
            "edits": [
                {
                    "artifact": "task_item",
                    "task_id": target,
                    "operation": "replace",
                    "path": "/goal",
                    "before": current["goal"],
                    "after": current["goal"] + " Confirmed.",
                }
            ],
        }
        prepared = prepare_specification(
            json.dumps(request).encode(),
            project_root=self.root,
            user_config_root=str(self.root),
        )
        transaction = prepared["preview"]["transaction"]
        item_rows = [
            row
            for row in transaction["files"]
            if "/tasks/TASK-" in row["path"]
        ]
        self.assertEqual(
            [row["path"] for row in item_rows],
            [f"outputs/work/tasks/example/tasks/{target}.json"],
        )

        published: list[tuple[str, int]] = []
        original_replace = spec_transactions.replace_checked

        def measured_replace(path, expected, target_raw, temporary, **kwargs):
            published.append((path.as_posix(), len(target_raw)))
            return original_replace(
                path, expected, target_raw, temporary, **kwargs
            )

        raw_request = json.dumps(prepared["request"]).encode()
        with patch.object(
            spec_transactions, "replace_checked", side_effect=measured_replace
        ):
            update_specification(
                raw_request,
                operation="apply",
                approved_sha256=prepared["preview"]["approved_sha256"],
                project_root=self.root,
                user_config_root=str(self.root),
            )

        published_items = [
            (path, size) for path, size in published if "/tasks/TASK-" in path
        ]
        self.assertEqual(len(published_items), 1)
        self.assertTrue(published_items[0][0].endswith(f"/tasks/{target}.json"))
        self.assertGreater(published_items[0][1], 0)
        for task_id, path in item_paths.items():
            if task_id != target:
                self.assertEqual(path.read_bytes(), before[task_id])


if __name__ == "__main__":
    unittest.main()
