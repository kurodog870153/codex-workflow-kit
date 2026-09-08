from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import ExitCode


class TaskCheckpointFlowTests(unittest.TestCase):
    def setUp(self):
        from worklib.hierarchy.selection import build_hierarchy_selection
        from worklib.instructions.selection import build_instruction_selection
        from worklib.instructions.work_selection import build_work_instruction_selection
        from worklib.skills.selection import selection_sha256

        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        work_root = SCRIPT_ROOT.parent
        hierarchy = build_hierarchy_selection({"decision": "general_only", "selections": []}, skill_root=work_root)
        self.plan = {
            "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed", "title": "Plan", "summary": "Result",
            "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/task.json", "execution": "outputs/work/executions/example"},
            "hierarchy_selection": hierarchy,
            "work_instruction_selection": build_work_instruction_selection(skill_root=work_root, mode="plan", selected_paths=[]),
            "skill_selection": {"schema": "work-skill-selection/v1", "decision": "base_only", "skills": [], "selection_sha256": selection_sha256("base_only", [])},
            "goals": [{"id": "GOAL-001", "statement": "Result"}],
            "scope": [{"id": "SCOPE-001", "kind": "in_scope", "statement": "Source", "goal_ids": ["GOAL-001"]}],
            "deliverables": [{"id": "DELIVERABLE-001", "statement": "Result", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}],
            "acceptance_criteria": [{"id": "ACCEPTANCE-001", "statement": "Observable", "deliverable_ids": ["DELIVERABLE-001"]}],
        }
        self.cli("plan", "create", "--stdin", "--plan-path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root), payload=self.plan)
        validation = self.cli("plan", "validate", "--path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root))
        selection = build_instruction_selection(skill_root=work_root, mode="task", selected_paths=[], reference_names=["task.general.task-records"])
        source = {key: validation[key] for key in ("plan_sha256", "hierarchy_selection_sha256", "skill_selection_sha256")}
        index = {"schema": "work-task-planning-index/v1", "requirement_id": "example", "revision": 1, "current_task_id": "TASK-001", "source": source,
                 "tasks": [{"id": "TASK-001", "title": "Task", "goal": "Result", "scope": ["Source"], "skill_id": None, "dependencies": [], "status": "planned", "boundary_revision": 1, "instructions_sha256": selection["instructions_sha256"]}]}
        second = copy.deepcopy(index["tasks"][0])
        second.update(id="TASK-002", title="Second task", dependencies=["TASK-001"])
        index["tasks"].append(second)
        self.cli("task", "draft-init", "--stdin", payload=index)
        candidate = {"id": "TASK-001", "title": "Task", "goal": "Result", "skill_id": None, "instruction_selection": selection,
                     "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
                     "steps": [{"id": "STEP-001", "action": "Review result.", "references": ["VAL-001"]}],
                     "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "User", "criteria": "Result is observable.", "acceptance_ids": ["ACCEPTANCE-001"]}]}
        self.draft = {"schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001", "revision": 1, "boundary_revision": 1,
                      "source": source, "instructions_sha256": selection["instructions_sha256"], "status": "refined", "notes": [], "confirmed_decisions": [],
                      "tentative": [], "open_questions": [], "next_discussion_point": None, "task_candidate": candidate}
        self.metadata = {"title": "Formal TASK", "summary": "Result"}
        self.options = {"expected_revision": 2, "plan_path": self.plan["artifacts"]["plan"], "user_config_root": str(self.root)}

    def cli(self, *arguments, payload=None, expected_code=0):
        import os
        import subprocess

        result = subprocess.run(
            [sys.executable, "-B", str(SCRIPT_ROOT / "work.py"), "--project-root", str(self.root), *arguments],
            input=json.dumps(payload, ensure_ascii=False) if payload is not None else "",
            capture_output=True, text=True, encoding="utf-8", timeout=60,
            env={**os.environ, "PYTHONIOENCODING": "utf-8"},
        )
        self.assertEqual(result.returncode, expected_code, result.stderr)
        self.assertEqual(result.stdout if expected_code else result.stderr, "")
        return json.loads(result.stderr if expected_code else result.stdout)

    def index(self):
        return self.cli("task", "draft-read", "--requirement-id", "example")

    def read_draft(self, task_id):
        return self.cli("task", "draft-read", "--requirement-id", "example", "--task-id", task_id)

    def save_discussion(self, discussion):
        index = self.index()
        revision = index["revision"]
        index["revision"] += 1
        index["current_task_id"] = discussion["task_id"]
        for entry in index["tasks"]:
            if entry["id"] == discussion["task_id"]:
                entry["status"] = discussion["status"]
        return self.cli("task", "draft-save", "--stdin", "--expected-revision", str(revision),
                        payload={"index": index, "draft": discussion})

    def formal_arguments(self):
        return ["--stdin", "--requirement-id", "example", "--expected-revision", str(self.index()["revision"]),
                "--plan-path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root)]

    def test_checkpoint_changes_and_approval_across_fresh_processes(self):
        from worklib.contracts.plan import render_plan_contract

        # First session ends mid-discussion; a new process retrieves the question.
        discussion = copy.deepcopy(self.draft)
        discussion.update(status="in_progress", open_questions=["Which review criteria?"],
                          next_discussion_point="Confirm criteria.", notes=["TASK-001 private discussion evidence"])
        self.save_discussion(discussion)
        index = self.index()
        self.assertNotIn("TASK-001 private discussion evidence", json.dumps(index))
        restored = self.read_draft("TASK-001")
        self.assertEqual(restored["open_questions"], ["Which review criteria?"])
        self.assertEqual(restored["notes"], discussion["notes"])
        self.assertEqual(restored["next_discussion_point"], "Confirm criteria.")
        restored.update(revision=2, status="refined", open_questions=[], next_discussion_point=None)
        self.save_discussion(restored)

        second = copy.deepcopy(self.draft)
        second["task_id"] = "TASK-002"
        second["task_candidate"].update(id="TASK-002", title="Second task", dependencies=["TASK-001"])
        second["notes"] = ["TASK-002 independent discussion evidence"]
        self.save_discussion(second)
        only_first = self.read_draft("TASK-001")
        self.assertNotIn("TASK-002 independent discussion evidence", json.dumps(only_first))
        reviewed = self.cli("task", "draft-assemble", *self.formal_arguments(), payload=self.metadata)
        self.assertEqual(len(reviewed["contract"]["tasks"]), 2)

        # An approved list boundary change invalidates both it and its dependent.
        index = self.index()
        revision = index["revision"]
        index["revision"] += 1
        index["tasks"][0]["goal"] = "Updated result"
        result = self.cli("task", "draft-list-update", "--stdin", "--expected-revision", str(revision),
                          payload={"index": index, "reason": "Confirmed result clarification"})
        self.assertEqual(result["affected_task_ids"], ["TASK-001", "TASK-002"])
        for task_id in ("TASK-001", "TASK-002"):
            self.assertEqual(self.read_draft(task_id)["status"], "needs_review")

        # Simulate a separately approved Plan change, then refresh its snapshot.
        self.plan["summary"] = "Clarified expected result"
        (self.root / self.plan["artifacts"]["plan"]).write_bytes(render_plan_contract(self.plan))
        index = self.index()
        source_args = ["--requirement-id", "example", "--expected-revision", str(index["revision"]),
                       "--plan-path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root)]
        drift = self.cli("task", "draft-check", *source_args, "--task-id", "TASK-001", "--general-only",
                         "--reference", "task.general.task-records", expected_code=ExitCode.ARTIFACT_INTEGRITY)
        self.assertEqual(drift["code"], "draft_source_drift")
        result = self.cli("task", "draft-source-update", *source_args, "--stdin", payload={
            "reason": "Confirmed Plan clarification",
            "selections": {task_id: {"selected_paths": [], "references": ["task.general.task-records"]}
                           for task_id in ("TASK-001", "TASK-002")},
        })
        self.assertEqual(result["affected_task_ids"], ["TASK-001", "TASK-002"])

        for task_id in ("TASK-001", "TASK-002"):
            index = self.index()
            checked = self.cli("task", "draft-check", "--requirement-id", "example", "--task-id", task_id,
                               "--expected-revision", str(index["revision"]), "--plan-path", self.plan["artifacts"]["plan"],
                               "--user-config-root", str(self.root), "--general-only", "--reference", "task.general.task-records")
            self.assertEqual(checked["status"], "valid")
            discussion = self.read_draft(task_id)
            entry = next(entry for entry in index["tasks"] if entry["id"] == task_id)
            discussion["task_candidate"]["goal"] = entry["goal"]
            discussion.update(revision=discussion["revision"] + 1, status="refined", next_discussion_point=None)
            self.save_discussion(discussion)

        formal_args = self.formal_arguments()
        rejected = self.cli("task", "draft-create", *formal_args, "--approved-sha256", reviewed["approval_sha256"],
                            payload=self.metadata, expected_code=ExitCode.ARTIFACT_INTEGRITY)
        self.assertEqual(rejected["code"], "draft_approval_mismatch")
        self.assertFalse((self.root / self.plan["artifacts"]["task"]).exists())
        self.assertFalse((self.root / self.plan["artifacts"]["execution"]).exists())
        final = self.cli("task", "draft-assemble", *formal_args, payload=self.metadata)
        created = self.cli("task", "draft-create", *formal_args, "--approved-sha256", final["approval_sha256"], payload=self.metadata)
        self.assertEqual(created["task_sha256"], final["task_sha256"])
        validated = self.cli("task", "validate", "--path", self.plan["artifacts"]["task"], "--user-config-root", str(self.root))
        self.assertEqual(validated["task_count"], 2)
        self.assertEqual(validated["task_sha256"], final["task_sha256"])
        self.assertTrue((self.root / self.plan["artifacts"]["execution"] / "index.json").is_file())
        self.assertIn("TASK-001 private discussion evidence", self.read_draft("TASK-001")["notes"])


if __name__ == "__main__":
    unittest.main()
