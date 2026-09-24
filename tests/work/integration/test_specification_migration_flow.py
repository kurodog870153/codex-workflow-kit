from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from contracts import test_task_collection as fixtures
from worklib.services.attempt import build_initial_execution_index, render_execution_index
from worklib.technical.foundation.fingerprint import raw_sha256
from worklib.technical.infrastructure.json_contract import parse_json_contract
from worklib.orchestration.task import prepare_specification_migration, preview_specification_migration, publish_specification_migration
from worklib.business_services.task import load_task_collection


class SpecificationMigrationFlowTests(unittest.TestCase):
    def test_semantic_reconstruction_prepares_invalid_source_set(self):
        fixture = fixtures.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        root = fixture.root
        artifacts = fixture.index["artifacts"]
        plan = parse_json_contract((root / artifacts["plan"]).read_bytes(), source="Plan")
        semantic_plan = {"requirement_id": "example", "title": plan["title"], "summary": plan["summary"],
                         "goals": [row["statement"] for row in plan["goals"]],
                         "scope": [row["statement"] for row in plan["scope"]],
                         "deliverables": [row["statement"] for row in plan["deliverables"]],
                         "acceptance_criteria": [row["statement"] for row in plan["acceptance_criteria"]],
                         "hierarchy_selection_request": {"decision": "general_only", "selections": []},
                         "skill_selection_request": {"decision": "base_only", "skills": []},
                         "references": []}
        task = {"title": fixture.item["title"], "goal": fixture.item["goal"], "skill_id": None,
                "selected_paths": [], "references": ["task.general.task-records"], "dependency_positions": [],
                "candidate": {"files": [{"key": "source", "action": "modify", "path": "src.txt"}],
                              "commands": [{"key": "check", "mode": "argv", "argv": ["python", "--version"]}],
                              "validations": [{"key": "pass", "kind": "automated", "command_keys": ["check"],
                                               "pass_condition": "Exit code is zero.", "acceptance_positions": [1]}],
                              "steps": [{"key": "modify", "action": "Modify the source.", "references": [{"kind": "files", "key": "source"}]},
                                        {"key": "verify", "action": "Run validation.", "references": [{"kind": "commands", "key": "check"}, {"kind": "validations", "key": "pass"}]}]}}
        for path in (artifacts["plan"], fixture.index_path,
                     fixture.index_path.rsplit("/", 1)[0] + "/tasks/TASK-001.json"):
            (root / path).write_bytes(b'{"schema":"incompatible"}\n')
        request = {"schema": "work-spec-migration-prepare-request/v1", "mode": "reconstruction",
                   "plan": semantic_plan, "task_title": fixture.index["title"],
                   "task_summary": fixture.index["summary"],
                   "execution_defaults": fixture.index["execution_defaults"], "tasks": [task],
                   "semantic_decisions": []}
        result = prepare_specification_migration(json.dumps(request).encode(), project_root=root, user_config_root=str(root))
        self.assertEqual(result["preview"]["status"], "ready")
        self.assertEqual(len(result["request"]["candidates"]), 4)
        self.assertEqual(result["request"]["candidates"][3]["content"]["steps"][0]["id"], "STEP-001")
        self.assertEqual((root / artifacts["plan"]).read_bytes(), b'{"schema":"incompatible"}\n')

    def test_current_candidates_publish_as_one_recoverable_set(self):
        fixture = fixtures.TaskCollectionTests("test_loads_complete_collection_and_rejects_single_file_artifact")
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        root = fixture.root
        validation = load_task_collection(root, str(root), fixture.index_path)
        artifacts = validation["collection_contract"]["artifacts"]
        execution_path = root / artifacts["execution"] / "index.json"
        execution_path.parent.mkdir(parents=True, exist_ok=True)
        execution_path.write_bytes(render_execution_index(build_initial_execution_index(validation["collection_contract"], validation)))
        paths = [(artifacts["plan"], "plan", None), (artifacts["task"], "task_index", None)]
        index = parse_json_contract((root / artifacts["task"]).read_bytes(), source="index")
        base = artifacts["task"].rsplit("/", 1)[0]
        paths.extend((base + "/" + row["path"], "task_item", row["id"]) for row in index["tasks"])
        paths.append((artifacts["execution"] + "/index.json", "execution_index", None))
        request = {"schema": "work-spec-migration-preview-request/v1", "sources": [], "candidates": [], "semantic_decisions": []}
        for path, kind, task_id in paths:
            raw = (root / path).read_bytes()
            request["sources"].append({"path": path, "raw_sha256": raw_sha256(raw)})
            candidate = {"path": path, "kind": kind, "content": json.loads(raw)}
            if task_id is not None:
                candidate["task_id"] = task_id
            request["candidates"].append(candidate)
        raw_request = json.dumps(request).encode()
        preview = preview_specification_migration(raw_request, project_root=root, user_config_root=str(root))
        result = publish_specification_migration(raw_request, project_root=root, user_config_root=str(root),
                                                 operation="apply", approved_sha256=preview["fingerprint"])
        self.assertEqual(result["status"], "updated")
        self.assertTrue((root / result["completion_marker"]).is_file())


if __name__ == "__main__":
    unittest.main()
