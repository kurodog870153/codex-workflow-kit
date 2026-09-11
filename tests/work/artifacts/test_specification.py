from __future__ import annotations

import copy
import hashlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts import specification
from worklib.artifacts.specification import update_specification
from worklib.cli import main
from worklib.contracts.execution_index import build_initial_execution_index, render_execution_index
from worklib.contracts.plan import render_plan_contract
from worklib.contracts.task import render_task_contract, validate_task_contract
from worklib.foundation.errors import WorkError
from worklib.foundation.spec_update import require_no_spec_update, state_writer
from worklib.hierarchy.selection import build_hierarchy_selection
from worklib.instructions.selection import build_instruction_selection
from worklib.instructions.task_selection import build_task_document_instruction_selection
from worklib.instructions.work_selection import build_work_instruction_selection
from worklib.skills.selection import selection_sha256


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


class SpecificationUpdateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        work_root = SCRIPT_ROOT.parent
        self.artifacts = {
            "plan": "outputs/work/plans/example.json",
            "task": "outputs/work/tasks/example/task.json",
            "execution": "outputs/work/executions/example",
        }
        hierarchy = build_hierarchy_selection({"decision": "general_only", "selections": []}, skill_root=work_root)
        self.plan = {
            "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed",
            "title": "Plan", "summary": "Original result", "artifacts": self.artifacts,
            "hierarchy_selection": hierarchy,
            "work_instruction_selection": build_work_instruction_selection(skill_root=work_root, mode="plan", selected_paths=[]),
            "skill_selection": {"schema": "work-skill-selection/v1", "decision": "base_only", "skills": [], "selection_sha256": selection_sha256("base_only", [])},
            "goals": [{"id": "GOAL-001", "statement": "Result"}],
            "scope": [{"id": "SCOPE-001", "kind": "in_scope", "statement": "Result", "goal_ids": ["GOAL-001"]}],
            "deliverables": [{"id": "DELIVERABLE-001", "statement": "Result", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}],
            "acceptance_criteria": [{"id": "ACCEPTANCE-001", "statement": "Observable result", "deliverable_ids": ["DELIVERABLE-001"]}],
        }
        self.plan_path = self.root / self.artifacts["plan"]
        self.plan_path.parent.mkdir(parents=True)
        self.plan_path.write_bytes(render_plan_contract(self.plan))
        selection = build_instruction_selection(skill_root=work_root, mode="task", selected_paths=[], reference_names=["task.general.task-records"])
        self.task = {
            "schema": "work-task/v1", "requirement_id": "example", "spec_id": "TASK-SPEC-001", "status": "confirmed",
            "title": "Tasks", "summary": "Deliver result", "artifacts": self.artifacts,
            "source_plan": {"canonical_sha256": digest(self.plan_path.read_bytes()), "hierarchy_selection_sha256": hierarchy["selection_sha256"]},
            "instruction_selection": build_task_document_instruction_selection([selection], skill_root=work_root),
            "tasks": [{
                "id": f"TASK-{number:03d}", "title": f"Outcome {number}", "skill_id": None,
                "instruction_selection": copy.deepcopy(selection),
                "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
                "goal": "Deliver the result",
                "steps": [{"id": "STEP-001", "action": "Confirm the outcome", "references": ["VAL-001"]}],
                "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "user", "criteria": "Result is observable", "acceptance_ids": ["ACCEPTANCE-001"]}],
            } for number in (1, 2, 3)],
            "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
        }
        self.task["tasks"][1]["dependencies"] = ["TASK-001"]
        self.task_path = self.root / self.artifacts["task"]
        self.task_path.parent.mkdir(parents=True)
        self.task_path.write_bytes(render_task_contract(self.task))
        validation = validate_task_contract(self.task_path.read_bytes(), source="fixture",
            actual_task_path=self.artifacts["task"], project_root=self.root, user_config_root=str(self.root))
        self.index = build_initial_execution_index(self.task, validation)
        self.index_path = self.root / self.artifacts["execution"] / "index.json"
        self.index_path.parent.mkdir(parents=True)
        self.index_path.write_bytes(render_execution_index(self.index))

    def request(self, *, change_plan=True):
        plan = json.loads(self.plan_path.read_bytes())
        old = json.loads(self.task_path.read_bytes())
        task = copy.deepcopy(old)
        if change_plan:
            previous = plan["summary"]
            plan["summary"] += " revised"
            changes = plan.setdefault("changes", [])
            changes.append({
                "id": f"PLAN-CHANGE-{len(changes)+1:03d}", "date": "2026-09-10",
                "location": "summary", "before": previous, "after": plan["summary"],
                "reason": "Confirmed revision", "affected_ids": ["GOAL-001"],
            })
        else:
            task["tasks"][0]["goal"] += " revised"
        task["source_plan"]["canonical_sha256"] = digest(render_plan_contract(plan))
        number = int(task["spec_id"][-3:]) + 1
        task["spec_id"] = f"TASK-SPEC-{number:03d}"
        task["readiness"]["spec_id"] = task["spec_id"]
        # Evidence is constructed independently from the production helper.
        changed_keys = sorted(key for key in old if key not in {"spec_id", "readiness", "changes"} and old[key] != task[key])
        task["changes"] = [{
            "id": f"TASK-CHANGE-{number-1:03d}", "spec_id": task["spec_id"], "date": "2026-09-10",
            "reason": "Confirmed revision", "affected_ids": ["TASK-001", "TASK-002", "TASK-003"],
            "edits": [{"operation": "replace", "path": "/" + key, "before": old[key], "after": task[key]} for key in changed_keys],
        }]
        return {
            "schema": "work-spec-update-request/v1", "reason": "Confirmed revision",
            "expected": {key + "_sha256": digest(path.read_bytes()) for key, path in self.paths().items()},
            "plan": plan, "task": task,
        }

    def paths(self):
        return {"plan": self.plan_path, "task": self.task_path, "index": self.index_path}

    def run_update(self, request, operation="validate", approval=None):
        return update_specification(json.dumps(request).encode("utf-8"), project_root=self.root,
            user_config_root=str(self.root), operation=operation, approved_sha256=approval)

    def snapshot(self):
        return {p.relative_to(self.root).as_posix(): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}

    def migration_request(self):
        # Simulate a consistent historical baseline unavailable in the install.
        plan = copy.deepcopy(self.plan)
        task = copy.deepcopy(self.task)
        plan["work_instruction_selection"]["sources"][0]["canonical_sha256"] = "a" * 64
        plan["work_instruction_selection"]["instructions_sha256"] = "b" * 64
        self.plan_path.write_bytes(render_plan_contract(plan))
        task["source_plan"]["canonical_sha256"] = digest(self.plan_path.read_bytes())
        for selection in [task["instruction_selection"]] + [row["instruction_selection"] for row in task["tasks"]]:
            selection["sources"][0]["canonical_sha256"] = "c" * 64
            selection["instructions_sha256"] = "d" * 64
        self.task_path.write_bytes(render_task_contract(task))
        validation = validate_task_contract(self.task_path.read_bytes(), source="historical fixture",
            actual_task_path=self.artifacts["task"], project_root=self.root, user_config_root=str(self.root),
            _historical_work_sources=True)
        self.index = build_initial_execution_index(task, validation)
        self.index_path.write_bytes(render_execution_index(self.index))
        request = self.request()
        request["schema"] = "work-spec-migration-request/v1"
        request["instruction_review"] = {mode: "Reviewed current guidance; revised specification and validations."
                                         for mode in ("plan", "task", "execute")}
        request["plan"]["work_instruction_selection"] = copy.deepcopy(self.plan["work_instruction_selection"])
        candidate = request["task"]
        candidate["source_plan"]["canonical_sha256"] = digest(render_plan_contract(request["plan"]))
        candidate["instruction_selection"] = copy.deepcopy(self.task["instruction_selection"])
        for row, original in zip(candidate["tasks"], self.task["tasks"]):
            row["instruction_selection"] = copy.deepcopy(original["instruction_selection"])
            row["validations"][0]["criteria"] += "; reviewed against current guidance"
        candidate["changes"][0]["edits"] = [
            {"operation": "replace", "path": "/" + key, "before": task[key], "after": candidate[key]}
            for key in sorted(task) if key not in {"spec_id", "readiness", "changes"} and task[key] != candidate[key]
        ]
        return request

    def run_migration(self, request, operation="validate", approval=None):
        return update_specification(json.dumps(request).encode(), project_root=self.root,
            user_config_root=str(self.root), operation=operation, approved_sha256=approval, migration=True)

    def migration_repair_request(self):
        request = self.migration_request()
        # The Plan changed after TASK binding; the index still identifies the original TASK.
        plan = json.loads(self.plan_path.read_bytes())
        plan["title"] += " clarified"
        self.plan_path.write_bytes(render_plan_contract(plan))
        original_task = json.loads(self.task_path.read_bytes())
        request["expected"]["plan_sha256"] = digest(self.plan_path.read_bytes())
        request["source_plan_repair"] = {
            "recorded_sha256": original_task["source_plan"]["canonical_sha256"],
            "actual_sha256": digest(self.plan_path.read_bytes()),
            "review": "Reviewed the Plan title clarification and all TASKs; user accepted this baseline.",
        }
        request["plan"]["title"] = plan["title"]
        candidate = request["task"]
        candidate["source_plan"]["canonical_sha256"] = digest(render_plan_contract(request["plan"]))
        candidate["changes"][0]["edits"] = [
            {"operation": "replace", "path": "/" + key, "before": original_task[key], "after": candidate[key]}
            for key in sorted(original_task)
            if key not in {"spec_id", "readiness", "changes"} and original_task[key] != candidate[key]
        ]
        return request

    def test_migration_binding_repair_preserves_originals_and_publishes_valid_candidates(self):
        request = self.migration_repair_request()
        self.add_history()
        request["expected"]["index_sha256"] = digest(self.index_path.read_bytes())
        before = self.snapshot()
        preview = self.run_migration(request)
        self.assertEqual(before, self.snapshot())
        self.assertEqual(preview["migration"]["source_plan_repair"], request["source_plan_repair"])
        self.assertEqual(preview["candidate"]["task"]["source_plan"]["canonical_sha256"],
                         digest(render_plan_contract(request["plan"])))
        self.assertEqual(preview["file_readiness"], "requires_execute_preflight")
        self.assertEqual(self.run_migration(request, "apply", preview["approved_sha256"])["status"], "updated")
        validation = validate_task_contract(self.task_path.read_bytes(), source="repaired TASK",
            actual_task_path=self.artifacts["task"], project_root=self.root, user_config_root=str(self.root))
        index = json.loads(self.index_path.read_bytes())
        self.assertEqual(index["task_sha256"], validation["task_sha256"])
        self.assertTrue(all(row["status"] == "pending_retry" for row in index["tasks"]))
        for name, raw in before.items():
            if "/ATTEMPT-" in name:
                self.assertEqual((self.root / name).read_bytes(), raw)
        journal = next(self.index_path.parent.glob(".work-spec-update-*.json"))
        record = json.loads(journal.read_bytes())
        for key, path in self.paths().items():
            self.assertEqual(record["before"][key].encode("utf-8"), before[path.relative_to(self.root).as_posix()])
        self.assertEqual(record["migration"]["source_plan_repair"], request["source_plan_repair"])
        self.assertEqual(self.run_update(self.request())["status"], "valid")

    def test_migration_missing_repair_reports_original_binding_without_writes(self):
        request = self.migration_repair_request()
        repair = request.pop("source_plan_repair")
        before = self.snapshot()
        output, errors = io.StringIO(), io.StringIO()
        code = main(["--project-root", str(self.root), "task", "migrate-validate", "--stdin",
                     "--user-config-root", str(self.root)], stdin=io.StringIO(json.dumps(request)),
                    stdout=output, stderr=errors)
        self.assertEqual(code, 5)
        self.assertEqual(output.getvalue(), "")
        error = json.loads(errors.getvalue())
        self.assertEqual(error["code"], "source_plan_fingerprint_mismatch")
        self.assertEqual(error["details"], {
            "source": "original TASK", "task_path": self.artifacts["task"], "plan_path": self.artifacts["plan"],
            "recorded_sha256": repair["recorded_sha256"], "actual_sha256": repair["actual_sha256"],
        })
        self.assertEqual(before, self.snapshot())

    def test_migration_rejects_invalid_or_stale_repair_evidence(self):
        request = self.migration_repair_request()
        evidence = request["source_plan_repair"]
        for value, code in (
            (None, "expected_object"),
            ({key: value for key, value in evidence.items() if key != "review"}, "invalid_object_fields"),
            (dict(evidence, review=" "), "empty_text_value"),
            (dict(evidence, extra=True), "invalid_object_fields"),
            (dict(evidence, recorded_sha256="invalid"), "invalid_sha256"),
            (dict(evidence, recorded_sha256="0" * 64), "spec_migration_source_plan_repair_changed"),
            (dict(evidence, actual_sha256="0" * 64), "spec_migration_source_plan_repair_changed"),
            (dict(evidence, actual_sha256=request["task"]["source_plan"]["canonical_sha256"]),
             "spec_migration_source_plan_repair_changed"),
        ):
            with self.subTest(value=value):
                candidate = copy.deepcopy(request)
                candidate["source_plan_repair"] = value
                before = self.snapshot()
                with self.assertRaises(WorkError) as error:
                    self.run_migration(candidate)
                self.assertEqual(error.exception.code, code)
                self.assertEqual(before, self.snapshot())

    def test_binding_repair_is_rejected_for_normal_updates_and_consistent_baselines(self):
        request = self.migration_request()
        original = json.loads(self.task_path.read_bytes())["source_plan"]["canonical_sha256"]
        request["source_plan_repair"] = {"recorded_sha256": original, "actual_sha256": original, "review": "Reviewed"}
        before = self.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_migration(request)
        self.assertEqual(error.exception.code, "spec_migration_source_plan_repair_unneeded")
        request["schema"] = "work-spec-update-request/v1"
        del request["instruction_review"]
        with self.assertRaises(WorkError) as error:
            self.run_update(request)
        self.assertEqual(error.exception.code, "invalid_object_fields")
        self.assertEqual(error.exception.details["unknown"], ["source_plan_repair"])
        self.assertEqual(before, self.snapshot())

    def test_binding_repair_never_accepts_a_mismatched_candidate(self):
        request = self.migration_repair_request()
        wrong = request["source_plan_repair"]["actual_sha256"]
        request["task"]["source_plan"]["canonical_sha256"] = wrong
        before = self.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_migration(request)
        self.assertEqual(error.exception.code, "source_plan_fingerprint_mismatch")
        self.assertEqual(error.exception.details["source"], "candidate TASK")
        self.assertEqual(error.exception.details["recorded_sha256"], wrong)
        self.assertEqual(error.exception.details["actual_sha256"], digest(render_plan_contract(request["plan"])))
        self.assertEqual(before, self.snapshot())

    def test_binding_repair_preserves_other_baseline_and_candidate_checks(self):
        for defect, code in (
            ("index", "spec_update_index_identity"),
            ("hierarchy", "source_plan_hierarchy_selection_mismatch"),
            ("union", "historical_instruction_union_mismatch"),
            ("lock", "spec_update_lock_present"),
            ("active", "spec_update_active_task"),
            ("traceability", "invalid_reference"),
            ("candidate_sources", "work_instruction_selection_sources_mismatch"),
            ("edits", "spec_update_change_evidence"),
        ):
            with self.subTest(defect=defect):
                request = self.migration_repair_request()
                task = json.loads(self.task_path.read_bytes())
                index = json.loads(self.index_path.read_bytes())
                if defect == "index":
                    index["task_sha256"] = "0" * 64
                elif defect == "hierarchy":
                    task["source_plan"]["hierarchy_selection_sha256"] = "0" * 64
                elif defect == "union":
                    task["instruction_selection"]["sources"][0]["canonical_sha256"] = "e" * 64
                elif defect == "lock":
                    index["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-009"}
                elif defect == "active":
                    index["tasks"][0].update(status="in_progress", latest_attempt="ATTEMPT-001")
                    index["overall_status"] = "in_progress"
                elif defect == "traceability":
                    task["tasks"][0]["traceability"]["goal_ids"] = ["GOAL-999"]
                elif defect == "candidate_sources":
                    request["plan"]["work_instruction_selection"] = json.loads(self.plan_path.read_bytes())["work_instruction_selection"]
                else:
                    request["task"]["changes"][0]["edits"][0]["before"] = "invented"
                self.task_path.write_bytes(render_task_contract(task))
                self.index_path.write_bytes(render_execution_index(index))
                request["expected"] = {key + "_sha256": digest(path.read_bytes()) for key, path in self.paths().items()}
                before = self.snapshot()
                with self.assertRaises(WorkError) as error:
                    self.run_migration(request)
                self.assertEqual(error.exception.code, code)
                self.assertEqual(before, self.snapshot())

    def test_binding_repair_approval_binds_review_and_original_bytes(self):
        request = self.migration_repair_request()
        preview = self.run_migration(request)
        changed = copy.deepcopy(request)
        changed["source_plan_repair"]["review"] += " Additional reviewed context."
        before = self.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_migration(changed, "apply", preview["approved_sha256"])
        self.assertEqual(error.exception.code, "spec_update_approval_changed")
        self.assertEqual(before, self.snapshot())
        self.plan_path.write_bytes(self.plan_path.read_bytes().replace(b"\n", b"\r\n"))
        before = self.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_migration(request, "apply", preview["approved_sha256"])
        self.assertEqual(error.exception.code, "spec_update_source_changed")
        self.assertEqual(before, self.snapshot())

    def test_binding_repair_recovery_retains_the_reviewed_mismatched_baseline(self):
        request = self.migration_repair_request()
        approval = self.run_migration(request)["approved_sha256"]
        replace = specification._replace

        def interrupted(path, *args, **kwargs):
            if path == self.task_path:
                raise OSError("injected interruption after Plan publication")
            return replace(path, *args, **kwargs)

        with patch.object(specification, "_replace", side_effect=interrupted):
            with self.assertRaises(WorkError) as error:
                self.run_migration(request, "apply", approval)
        self.assertEqual(error.exception.code, "spec_update_interrupted")
        changed = copy.deepcopy(request)
        changed["source_plan_repair"]["review"] += " changed"
        before = self.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_migration(changed, "recover", approval)
        self.assertEqual(error.exception.code, "spec_update_recovery_request")
        self.assertEqual(before, self.snapshot())
        for expected in ("recovered", "already_completed"):
            self.assertEqual(self.run_migration(request, "recover", approval)["status"], expected)
        self.assertEqual(self.run_update(self.request())["status"], "valid")

    def test_migration_revises_drifted_documents_and_restores_normal_revision(self):
        request = self.migration_request()
        self.add_history()
        request["expected"]["index_sha256"] = digest(self.index_path.read_bytes())
        before = self.snapshot()
        normal = copy.deepcopy(request)
        normal["schema"] = "work-spec-update-request/v1"
        del normal["instruction_review"]
        with self.assertRaises(WorkError) as error:
            self.run_update(normal)
        self.assertEqual(error.exception.code, "work_instruction_selection_sources_mismatch")
        preview = self.run_migration(request)
        self.assertEqual(before, self.snapshot())
        self.assertTrue(preview["migration"]["plan_edits"])
        self.run_migration(request, "apply", preview["approved_sha256"])
        for name, raw in before.items():
            if "/ATTEMPT-" in name:
                self.assertEqual((self.root / name).read_bytes(), raw)
        index = json.loads(self.index_path.read_bytes())
        self.assertTrue(all(row["status"] == "pending_retry" for row in index["tasks"]))
        self.assertEqual(self.run_update(self.request())["status"], "valid")

    def test_migration_rejects_stale_candidate_and_conflicting_baseline(self):
        request = self.migration_request()
        before = self.snapshot()
        stale = copy.deepcopy(request)
        stale["plan"]["work_instruction_selection"] = json.loads(self.plan_path.read_bytes())["work_instruction_selection"]
        with self.assertRaises(WorkError) as error:
            self.run_migration(stale)
        self.assertEqual(error.exception.code, "work_instruction_selection_sources_mismatch")
        self.assertEqual(before, self.snapshot())
        broken = json.loads(self.task_path.read_bytes())
        broken["instruction_selection"]["sources"][0]["canonical_sha256"] = "e" * 64
        self.task_path.write_bytes(render_task_contract(broken))
        request["expected"]["task_sha256"] = digest(self.task_path.read_bytes())
        with self.assertRaises(WorkError) as error:
            self.run_migration(request)
        self.assertEqual(error.exception.code, "historical_instruction_union_mismatch")

    def test_migration_approval_binds_execute_sources_and_review(self):
        request = self.migration_request()
        preview = self.run_migration(request)
        before = self.snapshot()
        builder = specification.build_instruction_selection
        def changed(**kwargs):
            selection = builder(**kwargs)
            selection["instructions_sha256"] = "e" * 64
            return selection
        with patch.object(specification, "build_instruction_selection", side_effect=changed):
            with self.assertRaises(WorkError) as error:
                self.run_migration(request, "apply", preview["approved_sha256"])
        self.assertEqual(error.exception.code, "spec_update_approval_changed")
        request["instruction_review"]["execute"] += " changed"
        with self.assertRaises(WorkError) as error:
            self.run_migration(request, "apply", preview["approved_sha256"])
        self.assertEqual(error.exception.code, "spec_update_approval_changed")
        self.assertEqual(before, self.snapshot())

    def test_migration_cli_and_recovery_use_the_reviewed_transaction(self):
        request = self.migration_request()
        output, errors = io.StringIO(), io.StringIO()
        code = main(["--project-root", str(self.root), "task", "migrate-validate", "--stdin",
                     "--user-config-root", str(self.root)], stdin=io.StringIO(json.dumps(request)),
                    stdout=output, stderr=errors)
        self.assertEqual(code, 0, errors.getvalue())
        approval = json.loads(output.getvalue())["approved_sha256"]
        replace = specification._replace
        def interrupted(path, *args, **kwargs):
            if path == self.task_path:
                raise OSError("injected interruption after Plan publication")
            return replace(path, *args, **kwargs)
        with patch.object(specification, "_replace", side_effect=interrupted):
            with self.assertRaises(WorkError) as error:
                self.run_migration(request, "apply", approval)
        self.assertEqual(error.exception.code, "spec_update_interrupted")
        for expected in ("recovered", "already_completed"):
            self.assertEqual(self.run_migration(request, "recover", approval)["status"], expected)

    def add_history(self):
        for row in self.index["tasks"]:
            row.update(status="completed", latest_attempt="ATTEMPT-001", latest_correction="ATTEMPT-001-CORRECTION-001")
            directory = self.index_path.parent / row["id"] / "ATTEMPT-001"
            (directory / "corrections").mkdir(parents=True)
            (directory / "attempt.json").write_bytes(b'{"immutable":"attempt"}\n')
            (directory / "corrections" / "ATTEMPT-001-CORRECTION-001.json").write_bytes(b'{"immutable":"correction"}\n')
        self.index["overall_status"] = "completed"
        self.index_path.write_bytes(render_execution_index(self.index))

    def test_preview_is_read_only_and_validates_candidate_plan_in_memory(self):
        request = self.request()
        before = self.snapshot()
        result = self.run_update(request)
        self.assertEqual(before, self.snapshot())
        self.assertEqual(result["status"], "valid")
        self.assertEqual(result["candidate"]["task"]["source_plan"]["canonical_sha256"],
                         digest(render_plan_contract(request["plan"])))
        self.assertEqual(result["candidate"]["index"]["task_spec_id"], "TASK-SPEC-002")

    def test_publish_and_second_revision_keep_canonical_identity_and_history(self):
        self.add_history()
        history = {key: value for key, value in self.snapshot().items() if "/ATTEMPT-" in key}
        for expected_spec in ("TASK-SPEC-002", "TASK-SPEC-003"):
            request = self.request()
            preview = self.run_update(request)
            result = self.run_update(request, "apply", preview["approved_sha256"])
            self.assertEqual(result["status"], "updated")
            stored = validate_task_contract(self.task_path.read_bytes(), source="stored",
                actual_task_path=self.artifacts["task"], project_root=self.root,
                user_config_root=str(self.root), validate_file_state=False)
            index = json.loads(self.index_path.read_bytes())
            self.assertEqual(stored["spec_id"], expected_spec)
            self.assertEqual(index["task_sha256"], stored["task_sha256"])
            self.assertTrue(all(row["status"] == "pending_retry" for row in index["tasks"]))
            self.assertTrue(all(row["latest_correction"] == "ATTEMPT-001-CORRECTION-001" for row in index["tasks"]))
            require_no_spec_update(self.root, self.artifacts["execution"])
        self.assertEqual(history, {key: value for key, value in self.snapshot().items() if "/ATTEMPT-" in key})
        records = list(self.index_path.parent.glob(".work-spec-update-*.json"))
        self.assertEqual(len(records), 2)
        self.assertEqual(json.loads(records[0].read_bytes())["schema"], "work-spec-update-record/v1")

    def test_task_change_invalidates_only_task_and_transitive_dependents(self):
        self.add_history()
        request = self.request(change_plan=False)
        preview = self.run_update(request)
        self.assertEqual(preview["affected_task_ids"], ["TASK-001", "TASK-002"])
        self.run_update(request, "apply", preview["approved_sha256"])
        index = json.loads(self.index_path.read_bytes())
        self.assertEqual([row["status"] for row in index["tasks"]], ["pending_retry", "pending_retry", "completed"])
        self.assertEqual(self.plan_path.read_bytes(), render_plan_contract(self.plan))

    def test_lock_and_inaccurate_change_evidence_are_rejected_without_writes(self):
        for defect in ("lock", "edits", "source", "version"):
            with self.subTest(defect=defect):
                request = self.request()
                if defect == "lock":
                    locked = copy.deepcopy(self.index)
                    locked["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-009"}
                    self.index_path.write_bytes(render_execution_index(locked))
                    request = self.request()
                elif defect == "edits":
                    request["task"]["changes"][0]["edits"][0]["before"] = "invented"
                elif defect == "source":
                    request["expected"]["plan_sha256"] = "0" * 64
                else:
                    request["task"]["spec_id"] = "TASK-SPEC-004"
                    request["task"]["readiness"]["spec_id"] = "TASK-SPEC-004"
                    request["task"]["changes"][0]["spec_id"] = "TASK-SPEC-004"
                before = self.snapshot()
                with self.assertRaises(WorkError):
                    self.run_update(request)
                self.assertEqual(before, self.snapshot())
                if defect == "lock":
                    self.index_path.write_bytes(render_execution_index(self.index))

    def test_history_change_invalidates_preview_approval(self):
        self.add_history()
        request = self.request()
        preview = self.run_update(request)
        history = self.index_path.parent / "TASK-001/ATTEMPT-001/attempt.json"
        history.write_bytes(b'{"immutable":"changed elsewhere"}\n')
        before = self.snapshot()
        with self.assertRaises(WorkError) as error:
            self.run_update(request, "apply", preview["approved_sha256"])
        self.assertEqual(error.exception.code, "spec_update_approval_changed")
        self.assertEqual(before, self.snapshot())

    def test_partial_publication_blocks_execute_and_recovers_identical_request(self):
        request = self.request()
        preview = self.run_update(request)
        real_replace = os.replace

        def fail_task(source, target):
            if Path(target) == self.task_path:
                raise OSError("injected interruption")
            return real_replace(source, target)

        with patch("worklib.artifacts.specification.os.replace", side_effect=fail_task):
            with self.assertRaises(WorkError) as error:
                self.run_update(request, "apply", preview["approved_sha256"])
        self.assertTrue(error.exception.details["recovery_required"])
        self.assertEqual(json.loads(self.index_path.read_bytes())["lock"]["kind"], "spec_update")
        with self.assertRaises(WorkError):
            require_no_spec_update(self.root, self.artifacts["execution"])
        output, errors = io.StringIO(), io.StringIO()
        code = main(["--project-root", str(self.root), "execute", "preflight",
            "--user-config-root", str(self.root), "--task-path", self.artifacts["task"],
            "--execution-dir", self.artifacts["execution"], "--task-id", "TASK-001"],
            stdout=output, stderr=errors)
        self.assertNotEqual(code, 0)
        self.assertEqual(json.loads(errors.getvalue())["code"], "spec_update_pending")
        result = self.run_update(request, "recover", preview["approved_sha256"])
        self.assertEqual(result["status"], "recovered")
        require_no_spec_update(self.root, self.artifacts["execution"])
        self.assertNotIn("lock", json.loads(self.index_path.read_bytes()))
        self.assertEqual(self.run_update(request, "recover", preview["approved_sha256"])["status"], "already_completed")

    def test_recovery_refuses_conflicting_artifact_without_overwrite(self):
        request = self.request()
        preview = self.run_update(request)
        with patch("worklib.artifacts.specification.os.replace", side_effect=OSError("interrupted")):
            with self.assertRaises(WorkError):
                self.run_update(request, "apply", preview["approved_sha256"])
        self.task_path.write_bytes(b"unrelated edit\n")
        before = self.snapshot()
        with self.assertRaises(WorkError):
            self.run_update(request, "recover", preview["approved_sha256"])
        self.assertEqual(before, self.snapshot())

    def test_cli_preview_and_update_use_fingerprint_bound_request(self):
        request = self.request()
        args = ["--project-root", str(self.root), "task", "spec-validate", "--stdin", "--user-config-root", str(self.root)]
        output, errors = io.StringIO(), io.StringIO()
        code = main(args, stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=errors)
        self.assertEqual(code, 0, errors.getvalue())
        approval = json.loads(output.getvalue())["approved_sha256"]
        args[3] = "spec-update"
        args += ["--approved-sha256", approval]
        output, errors = io.StringIO(), io.StringIO()
        code = main(args, stdin=io.StringIO(json.dumps(request)), stdout=output, stderr=errors)
        self.assertEqual(code, 0, errors.getvalue())
        self.assertEqual(json.loads(output.getvalue())["status"], "updated")



    def test_short_journal_temporary_and_marker_recover_by_appending_only(self):
        # Each stage uses a separate requirement workspace and an injected short write.
        for suffix in (".json", ".lock", ".tmp", ".index", ".done"):
            with self.subTest(suffix=suffix):
                case = SpecificationUpdateTests()
                case.setUp()
                try:
                    request = case.request()
                    preview = case.run_update(request)
                    write = specification._write
                    failed = []

                    def short_write(path, raw):
                        if not failed and path.name.endswith(suffix):
                            failed.append(path)
                            with path.open("xb") as stream:
                                stream.write(raw[:max(1, len(raw) // 3)])
                            raise OSError("injected short write")
                        return write(path, raw)

                    with patch("worklib.artifacts.specification._write", side_effect=short_write):
                        with self.assertRaises(WorkError):
                            case.run_update(request, "apply", preview["approved_sha256"])
                    self.assertEqual(len(failed), 1)
                    prefix = failed[0].read_bytes()
                    with self.assertRaises(WorkError):
                        require_no_spec_update(case.root, case.artifacts["execution"])
                    result = case.run_update(request, "recover", preview["approved_sha256"])
                    self.assertEqual(result["status"], "recovered")
                    require_no_spec_update(case.root, case.artifacts["execution"])
                    if failed[0].exists():
                        self.assertTrue(failed[0].read_bytes().startswith(prefix))
                    self.assertEqual(json.loads(case.task_path.read_bytes())["spec_id"], "TASK-SPEC-002")
                finally:
                    case.doCleanups()

    def test_work_writers_are_mutually_exclusive_across_processes(self):
        program = (
            "import sys\nfrom pathlib import Path\n"
            f"sys.path.insert(0, {str(SCRIPT_ROOT)!r})\n"
            "from worklib.foundation.spec_update import state_writer\n"
            "from worklib.foundation.errors import WorkError\n"
            "try:\n"
            f"    with state_writer(Path({str(self.root)!r}), {self.artifacts['execution']!r}):\n"
            "        print('acquired')\n"
            "except WorkError as error:\n"
            "    print(error.code)\n"
        )
        with state_writer(self.root, self.artifacts["execution"]):
            result = subprocess.run([sys.executable, "-B", "-c", program], capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "work_state_writer_busy")
            request = self.request()
            preview = self.run_update(request)
            before = {key: path.read_bytes() for key, path in self.paths().items()}
            with self.assertRaises(WorkError) as error:
                self.run_update(request, "apply", preview["approved_sha256"])
            self.assertEqual(error.exception.code, "work_state_writer_busy")
            self.assertEqual(before, {key: path.read_bytes() for key, path in self.paths().items()})
            output, errors = io.StringIO(), io.StringIO()
            with patch("worklib.cli_commands.execute.begin_record") as operation:
                code = main(["--project-root", str(self.root), "execute", "record-begin",
                    "--user-config-root", str(self.root), "--task-path", self.artifacts["task"],
                    "--execution-dir", self.artifacts["execution"], "--task-id", "TASK-001", "--record-id", "VAL-001"],
                    stdout=output, stderr=errors)
                self.assertNotEqual(code, 0)
                self.assertEqual(json.loads(errors.getvalue())["code"], "work_state_writer_busy")
                operation.assert_not_called()
        result = subprocess.run([sys.executable, "-B", "-c", program], capture_output=True, text=True, timeout=15)
        self.assertEqual(result.stdout.strip(), "acquired")


if __name__ == "__main__":
    unittest.main()
