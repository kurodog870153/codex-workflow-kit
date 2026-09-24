from __future__ import annotations

import copy
import hashlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills/work/scripts"
sys.path.insert(0, str(SCRIPT_ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from cli_support import FileInputTestCase

from worklib.orchestration.handoff import build_plan_to_task_handoff, build_task_to_execute_handoff, build_task_to_plan_handoff
from worklib.orchestration.handoff import verify_return_handoff
from worklib.cli import main
from worklib.services.handoff import validate_handoff_contract
from worklib.business_services.plan import render_plan_contract, validate_plan_file
from worklib.business_services.task.document import render_task_contract
from worklib.business_services.task.creation import prepare_task_collection_create
from worklib.business_services.task import load_task_collection
from worklib.models.common.errors import ExitCode, WorkError
from worklib.technical.infrastructure.file_io import read_raw
from worklib.business_services.hierarchy import build_hierarchy_selection
from worklib.business_services.instruction import build_work_instruction_selection
from worklib.business_services.instruction import build_instruction_selection
from worklib.business_services.instruction import build_task_document_instruction_selection
from worklib.services.skill_selection import selection_sha256
from worklib.orchestration.handoff import build_execute_return_handoff, build_preflight_return_handoff
from worklib.orchestration.handoff import verify_plan_to_task_handoff, verify_task_to_execute_handoff
from worklib.services.attempt import render_attempt_contract
from worklib.services.attempt import authorization_sha256, minimal_authorization
from worklib.services.attempt import build_initial_execution_index, render_execution_index, derive_overall_status


class HandoffArtifactTests(FileInputTestCase):
    def test_unsaved_discussion_builder_does_not_claim_formal_source(self):
        request = {"schema": "work-discussion-handoff-request/v1", "direction": "task_to_plan",
                   "requirement_id": "example", "summary": "Review unfinished discussion.",
                   "requested_changes": ["Clarify the scope"]}
        args = self.input_arguments(["--project-root", str(self.root), "--verbose", "handoff",
                                     "build-discussion", "--input-file", "request.json"], json.dumps(request))
        output = io.StringIO()
        self.assertEqual(main(args, stdout=output), 0, output.getvalue())
        handoff = json.loads(output.getvalue())["data"]
        self.assertEqual(handoff["schema"], "work-discussion-handoff/v1")
        self.assertEqual((handoff["source_stage"], handoff["target_stage"]), ("task", "plan"))
        self.assertEqual(handoff["source_validation"], "not_checked")
        self.assertFalse(handoff["grants_authorization"])
        self.assertNotIn("artifacts", handoff)
        with self.assertRaises(WorkError):
            validate_handoff_contract(handoff, project_root=self.root)

    def verify_return(self, contract, *, task_id=None, attempt_id=None, preflight=False, direction=None, plan_path=None):
        before = self.snapshot()
        original = copy.deepcopy(contract)
        try:
            return verify_return_handoff(self.root, contract,
                direction=direction or contract["direction"],
                plan_path=plan_path or self.plan["artifacts"]["plan"],
                task_path=self.task["artifacts"]["task"], task_id=task_id,
                attempt_id=attempt_id, preflight=preflight, user_config_root=str(self.root))
        finally:
            self.assertEqual(self.snapshot(), before)
            self.assertEqual(contract, original)

    def test_verify_task_return_whole_spec_and_selected_task(self):
        self.write_task()
        for task_id in (None, "TASK-002"):
            with self.subTest(task_id=task_id):
                contract = self.build_return(task_id=task_id)
                result = self.verify_return(contract, task_id=task_id)
                self.assertEqual(result["schema"], "work-handoff-source-validation/v1")
                self.assertEqual(result["source"], contract["source"])
        with self.assertRaises(WorkError) as caught:
            self.verify_return(self.build_return(task_id="TASK-002"), task_id="TASK-001")
        self.assertEqual(caught.exception.code, "handoff_source_mismatch")

    def test_verify_task_return_rejects_same_spec_changed_content_and_missing_collection_hash(self):
        self.write_task()
        contract = self.build_return()
        legacy = copy.deepcopy(contract)
        del legacy["source"]["task_collection_sha256"]
        with self.assertRaises(WorkError) as caught:
            validate_handoff_contract(legacy, project_root=self.root)
        self.assertEqual(caught.exception.code, "invalid_object_fields")
        self.task["summary"] = "Changed under the same spec ID"
        self._write_task_collection()
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract)
        self.assertEqual(caught.exception.code, "handoff_source_mismatch")

    def test_verify_return_rejects_wrong_receiver_and_unknown_affected_ids(self):
        self.write_task()
        contract = self.build_return()
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract, direction="execute_to_plan")
        self.assertEqual(caught.exception.code, "handoff_direction_mismatch")
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract, plan_path="custom/plans/example.json")
        self.assertEqual(caught.exception.code, "handoff_source_mismatch")
        contract["affected_ids"] = ["GOAL-999"]
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract)
        self.assertEqual(caught.exception.code, "handoff_unknown_affected_ids")

    def test_verify_preflight_returns_both_directions_without_attempt(self):
        self.write_unstarted_execution()
        for direction in ("execute_to_task", "execute_to_plan"):
            contract = self.build_preflight_return(direction)
            result = self.verify_return(contract, task_id="TASK-001", preflight=True)
            self.assertEqual(result["status"], "valid")
            with self.assertRaises(WorkError) as caught:
                self.verify_return(contract, task_id="TASK-001")
            self.assertEqual(caught.exception.code, "handoff_context_required")
        self.index["lock"] = {"kind": "spec_update", "record": "SPEC-UPDATE-001"}
        self.index_path.write_bytes(render_execution_index(self.index))
        with self.assertRaises(WorkError):
            self.verify_return(contract, task_id="TASK-001", preflight=True)

    def test_verify_closed_returns_bind_exact_attempt_for_both_directions(self):
        self.write_closed_execution()
        for direction in ("execute_to_task", "execute_to_plan"):
            contract = self.build_execution_return(direction)
            self.assertEqual(self.verify_return(contract, task_id="TASK-001", attempt_id="ATTEMPT-001")["status"], "valid")
        self.attempt["reason"] = "Changed reason under the same Attempt ID"
        self.save_execution()
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract, task_id="TASK-001", attempt_id="ATTEMPT-001")
        self.assertEqual(caught.exception.code, "handoff_source_mismatch")

    def test_verify_closed_return_rejects_changed_status_and_missing_hash(self):
        self.write_closed_execution()
        contract = self.build_execution_return()
        legacy = copy.deepcopy(contract)
        del legacy["source"]["attempt_sha256"]
        self.assertEqual(validate_handoff_contract(legacy, project_root=self.root)["status"], "valid")
        with self.assertRaises(WorkError):
            self.verify_return(legacy, task_id="TASK-001", attempt_id="ATTEMPT-001")
        self.attempt["status"] = "blocked"
        self.attempt["final_type"] = "required_input"
        self.save_execution()
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract, task_id="TASK-001", attempt_id="ATTEMPT-001")
        self.assertEqual(caught.exception.code, "handoff_source_mismatch")

    def test_verify_return_rejects_incomplete_specification_transaction(self):
        self.write_task()
        contract = self.build_return()
        directory = self.root / self.task["artifacts"]["execution"]
        directory.mkdir(parents=True, exist_ok=True)
        (directory / ".work-spec-update-SPEC-UPDATE-001.json").write_bytes(b"unfinished")
        with self.assertRaises(WorkError) as caught:
            self.verify_return(contract)
        self.assertEqual(caught.exception.code, "spec_update_pending")

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        work_root = SCRIPT_ROOT.parent
        self.plan = {
            "schema": "work-plan/v1", "requirement_id": "example", "status": "confirmed",
            "title": "Plan", "summary": "Result",
            "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
            "hierarchy_selection": build_hierarchy_selection({"decision": "general_only", "selections": []}, skill_root=work_root),
            "work_instruction_selection": build_work_instruction_selection(skill_root=work_root, mode="plan", selected_paths=[]),
            "skill_selection": {"schema": "work-skill-selection/v1", "decision": "base_only", "skills": [], "selection_sha256": selection_sha256("base_only", [])},
            "goals": [{"id": "GOAL-001", "statement": "Result"}],
            "scope": [{"id": "SCOPE-001", "kind": "in_scope", "statement": "Source", "goal_ids": ["GOAL-001"]}],
            "deliverables": [{"id": "DELIVERABLE-001", "statement": "Result", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}],
            "acceptance_criteria": [{"id": "ACCEPTANCE-001", "statement": "Observable", "deliverable_ids": ["DELIVERABLE-001"]}],
            "constraints": [{"id": "CONSTRAINT-001", "statement": "Keep scope", "applies_to": ["PLAN"]}],
        }
        self.request = {"summary": "接續已確認的規劃", "affected_ids": ["CONSTRAINT-001", "GOAL-001"]}
        self.write_plan()

    def write_plan(self):
        self.plan_path = self.root / self.plan["artifacts"]["plan"]
        self.plan_path.parent.mkdir(parents=True, exist_ok=True)
        self.plan_path.write_bytes(render_plan_contract(self.plan))

    def write_spec_journal(self, completed=False):
        execution = self.root / self.plan["artifacts"]["execution"]
        execution.mkdir(parents=True, exist_ok=True)
        journal = execution / ".work-spec-update-SPEC-UPDATE-001.json"
        journal.write_bytes(b'{"transaction": "test"}\n')
        if completed:
            Path(str(journal) + ".done").write_bytes(hashlib.sha256(journal.read_bytes()).hexdigest().encode("ascii") + b"\n")
        return journal

    def test_plan_and_task_handoffs_reject_pending_specification_transaction(self):
        self.write_task()
        received = self.build()
        self.write_spec_journal()
        for operation in (self.build, self.build_execute, self.build_return, lambda: self.verify_received_plan(received)):
            with self.subTest(operation=operation), self.assertRaises(WorkError) as error:
                operation()
            self.assertEqual(error.exception.code, "spec_update_pending")
            self.assertTrue(error.exception.details["recovery_required"])

    def test_execute_handoffs_reject_pending_specification_transaction(self):
        self.write_closed_execution()
        self.write_spec_journal()
        for direction in ("execute_to_task", "execute_to_plan"):
            with self.subTest(direction=direction), self.assertRaises(WorkError) as error:
                self.build_execution_return(direction)
            self.assertEqual(error.exception.code, "spec_update_pending")

    def test_preflight_handoffs_reject_pending_specification_transaction(self):
        self.write_unstarted_execution()
        self.write_spec_journal()
        for direction in ("execute_to_task", "execute_to_plan"):
            with self.subTest(direction=direction), self.assertRaises(WorkError) as error:
                self.build_preflight_return(direction)
            self.assertEqual(error.exception.code, "spec_update_pending")

    def test_completed_specification_transaction_allows_handoffs(self):
        self.write_unstarted_execution()
        self.write_spec_journal(completed=True)
        self.verify_received_plan(self.build())
        self.build_execute()
        self.build_return()
        for direction in ("execute_to_task", "execute_to_plan"):
            self.build_preflight_return(direction)
        self.write_closed_execution()
        for direction in ("execute_to_task", "execute_to_plan"):
            self.build_execution_return(direction)

    def test_handoff_rejects_invalid_specification_completion_marker(self):
        journal = self.write_spec_journal(completed=True)
        Path(str(journal) + ".done").write_bytes(b"incorrect\n")
        with self.assertRaises(WorkError) as error:
            self.build()
        self.assertEqual(error.exception.code, "spec_update_pending")

    def test_handoff_rechecks_specification_transaction_before_return(self):
        from worklib.business_services.handoff import _build_handoff
        before = self.snapshot()
        journals = []
        def start_transaction(*args, **kwargs):
            journals.append(self.write_spec_journal())
            return _build_handoff(*args, **kwargs)
        with patch("worklib.business_services.handoff._build_handoff", side_effect=start_transaction):
            with self.assertRaises(WorkError) as error:
                build_plan_to_task_handoff(self.root, self.request, plan_path=self.plan["artifacts"]["plan"], user_config_root=str(self.root))
        self.assertEqual(error.exception.code, "spec_update_pending")
        self.assertTrue(journals[0].is_file())
        after = self.snapshot()
        self.assertTrue(all(after[path] == value for path, value in before.items()))

    def verify_received_plan(self, contract, plan_path=None):
        before = self.snapshot()
        original = copy.deepcopy(contract)
        try:
            return verify_plan_to_task_handoff(
                self.root, contract, plan_path=plan_path or self.plan["artifacts"]["plan"],
                user_config_root=str(self.root),
            )
        finally:
            self.assertEqual(contract, original)
            self.assertEqual(self.snapshot(), before)

    def verify_received_task(self, contract, task_id="TASK-001"):
        before, original = self.snapshot(), copy.deepcopy(contract)
        try:
            return verify_task_to_execute_handoff(self.root, contract, task_path=self.task["artifacts"]["task"],
                                                 task_id=task_id, user_config_root=str(self.root))
        finally:
            self.assertEqual(self.snapshot(), before)
            self.assertEqual(contract, original)

    def test_verify_received_task_uses_explicit_target_and_task_fingerprint(self):
        self.write_task()
        for task_id in ("TASK-001", "TASK-002"):
            handoff = self.build_execute(task_id)
            result = self.verify_received_task(handoff, task_id)
            self.assertEqual(result["schema"], "work-handoff-source-validation/v1")
            self.assertEqual(result["status"], "valid")
            self.assertEqual(result["source"], handoff["source"])
            self.assertEqual(result["task_path"], self.task["artifacts"]["task"])
        with self.assertRaises(WorkError) as error:
            self.verify_received_task(self.build_execute("TASK-002"))
        self.assertEqual(error.exception.code, "handoff_source_mismatch")
        with self.assertRaises(WorkError) as error:
            self.verify_received_task(self.build_execute(), "TASK-999")
        self.assertEqual(error.exception.code, "handoff_task_not_found")

    def test_verify_received_task_rejects_changed_specification_and_plan(self):
        self.write_task()
        handoff = self.build_execute()
        self.task["summary"] = "Revised task summary"
        self._write_task_collection()
        with self.assertRaises(WorkError) as error:
            self.verify_received_task(handoff)
        self.assertEqual(error.exception.code, "handoff_source_mismatch")
        self.write_task()
        handoff = self.build_execute()
        self.plan["summary"] = "Revised Plan"
        self.write_plan()
        with self.assertRaises(WorkError):
            self.verify_received_task(handoff)

    def test_verify_received_task_rejects_forged_source_fields_and_artifacts(self):
        self.write_task()
        original = self.build_execute()
        fields = {"task_spec_id": "TASK-SPEC-999", "skill_id": "unknown",
                  "task_collection_sha256": "0" * 64, "task_index_sha256": "0" * 64,
                  "task_item_sha256": "0" * 64, "task_instructions_sha256": "0" * 64,
                  "skill_selection_sha256": "0" * 64}
        for field, value in fields.items():
            handoff = copy.deepcopy(original)
            handoff["source"][field] = value
            with self.subTest(field=field), self.assertRaises(WorkError) as error:
                self.verify_received_task(handoff)
            self.assertEqual(error.exception.code, "handoff_source_mismatch")
        for field, path in (("plan", "custom/example.json"), ("task", "custom/tasks/example/index.json"), ("execution", "custom/executions/example")):
            handoff = copy.deepcopy(original)
            handoff["artifacts"][field] = path
            with self.subTest(field=field), self.assertRaises(WorkError) as error:
                self.verify_received_task(handoff)
            self.assertEqual(error.exception.code, "handoff_source_mismatch")

    def test_verify_received_task_rejects_wrong_direction_and_pending_transaction(self):
        self.write_task()
        with self.assertRaises(WorkError) as error:
            self.verify_received_task(self.build())
        self.assertEqual(error.exception.code, "handoff_direction_mismatch")
        handoff = self.build_execute()
        self.write_spec_journal()
        with self.assertRaises(WorkError) as error:
            self.verify_received_task(handoff)
        self.assertEqual(error.exception.code, "spec_update_pending")

    def test_verify_received_task_rechecks_source_snapshots(self):
        from worklib.technical.infrastructure.file_io import read_raw
        self.write_task()
        handoff = self.build_execute()
        for target in (self.task_path, self.plan_path):
            reads = []
            def changed(path):
                reads.append(path)
                return b"changed" if path == target and reads.count(path) > 1 else read_raw(path)
            with self.subTest(target=target), patch("worklib.business_services.handoff.read_raw", side_effect=changed):
                with self.assertRaises(WorkError) as error:
                    self.verify_received_task(handoff)
            self.assertEqual(error.exception.code, "handoff_source_changed")

    def test_verify_received_task_cli_uses_custom_paths(self):
        self.plan["artifacts"] = {"plan": "custom/example.json", "task": "custom/tasks/example/index.json", "execution": "custom/executions/example"}
        self.write_plan()
        self.write_task()
        handoff = self.build_execute("TASK-002")
        before = self.snapshot()
        output, error = io.StringIO(), io.StringIO()
        code = main(self.input_arguments(["--project-root", str(self.root), "handoff", "verify-task-to-execute", "--input-file", "request.json",
                     "--task-path", self.task["artifacts"]["task"], "--task-id", "TASK-002", "--user-config-root", str(self.root)], json.dumps(handoff)), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (0, ""))
        self.assertEqual(json.loads(output.getvalue())["data"], self.verify_received_task(handoff, "TASK-002"))
        self.assertEqual(self.snapshot(), before)

    def test_verify_received_plan_returns_distinct_source_validation(self):
        handoff = self.build()
        result = self.verify_received_plan(handoff)
        self.assertEqual(result["schema"], "work-handoff-source-validation/v1")
        self.assertEqual(result["status"], "valid")
        self.assertEqual(result["source"], handoff["source"])
        self.assertEqual(result["plan_path"], self.plan["artifacts"]["plan"])

    def test_verify_received_plan_rejects_stale_handoff(self):
        handoff = self.build()
        self.plan["summary"] = "Revised confirmed scope"
        self.write_plan()
        with self.assertRaises(WorkError) as error:
            self.verify_received_plan(handoff)
        self.assertEqual(error.exception.code, "handoff_source_mismatch")

    def test_verify_received_plan_rejects_forged_fingerprints_and_paths(self):
        original = self.build()
        for field in ("plan_sha256", "skill_selection_sha256"):
            handoff = copy.deepcopy(original)
            handoff["source"][field] = "0" * 64
            with self.subTest(field=field), self.assertRaises(WorkError) as error:
                self.verify_received_plan(handoff)
            self.assertEqual(error.exception.code, "handoff_source_mismatch")
        for field, path in (("plan", "custom/example.json"), ("task", "custom/tasks/example/index.json"), ("execution", "custom/executions/example")):
            handoff = copy.deepcopy(original)
            handoff["artifacts"][field] = path
            with self.subTest(field=field), self.assertRaises(WorkError) as error:
                self.verify_received_plan(handoff)
            self.assertEqual(error.exception.code, "handoff_source_mismatch")

    def test_verify_received_plan_rejects_wrong_requirement_and_unknown_ids(self):
        handoff = self.build()
        handoff["requirement_id"] = "other"
        handoff["artifacts"] = {"plan": "custom/other.json", "task": "custom/tasks/other/index.json", "execution": "custom/executions/other"}
        with self.assertRaises(WorkError) as error:
            self.verify_received_plan(handoff)
        self.assertEqual(error.exception.code, "handoff_source_mismatch")
        handoff = self.build()
        handoff["affected_ids"] = ["GOAL-999"]
        with self.assertRaises(WorkError) as error:
            self.verify_received_plan(handoff)
        self.assertEqual(error.exception.code, "handoff_unknown_plan_ids")

    def test_verify_received_plan_rejects_wrong_direction(self):
        self.write_task()
        with self.assertRaises(WorkError) as error:
            self.verify_received_plan(self.build_execute())
        self.assertEqual(error.exception.code, "handoff_direction_mismatch")

    def test_verify_received_plan_requires_existing_valid_selected_plan(self):
        handoff = self.build()
        with self.assertRaises(WorkError):
            self.verify_received_plan(handoff, plan_path="missing/plan.json")
        self.plan["skill_selection"]["selection_sha256"] = "0" * 64
        self.write_plan()
        with self.assertRaises(WorkError):
            self.verify_received_plan(handoff)

    def test_verify_received_plan_rejects_source_change_during_read(self):
        handoff = self.build()
        with patch("worklib.business_services.handoff.read_raw", side_effect=[self.plan_path.read_bytes(), b"changed"]):
            with self.assertRaises(WorkError) as error:
                self.verify_received_plan(handoff)
        self.assertEqual(error.exception.code, "handoff_source_changed")

    def test_verify_received_plan_cli_with_custom_paths(self):
        self.plan["artifacts"] = {"plan": "custom/example.json", "task": "custom/tasks/example/index.json", "execution": "custom/executions/example"}
        self.write_plan()
        handoff = self.build()
        before = self.snapshot()
        output, error = io.StringIO(), io.StringIO()
        code = main(self.input_arguments(["--project-root", str(self.root), "handoff", "verify-plan-to-task", "--input-file", "request.json",
                     "--plan-path", "custom/example.json", "--user-config-root", str(self.root)], json.dumps(handoff)), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (0, ""))
        self.assertEqual(json.loads(output.getvalue())["data"], self.verify_received_plan(handoff))
        self.assertEqual(self.snapshot(), before)

    def snapshot(self):
        return {path.relative_to(self.root): path.read_bytes() if path.is_file() else None for path in self.root.rglob("*")}

    def write_task(self):
        checked = validate_plan_file(self.root, str(self.root), self.plan["artifacts"]["plan"])
        selections = [build_instruction_selection(skill_root=SCRIPT_ROOT.parent, mode="task", selected_paths=[], reference_names=references)
                      for references in (["task.general.task-records"], ["task.general.task-records", "task.general.instruction-maintenance"])]
        tasks = [{
            "id": f"TASK-{number:03}", "title": "Review instructions", "goal": "Review result", "skill_id": None,
            "instruction_selection": selection,
            "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
            "steps": [{"id": "STEP-001", "action": "Review result", "references": ["VAL-001"]}],
            "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "User", "criteria": "Result is observable", "acceptance_ids": ["ACCEPTANCE-001"]}],
        } for number, selection in enumerate(selections, 1)]
        self.task = {
            "schema": "work-task-collection-projection/v1", "requirement_id": "example", "spec_id": "TASK-SPEC-001", "status": "confirmed",
            "title": "Formal TASK", "summary": "Result", "artifacts": copy.deepcopy(self.plan["artifacts"]),
            "source_plan": {"canonical_sha256": checked["plan_sha256"], "hierarchy_selection_sha256": checked["hierarchy_selection_sha256"]},
            "instruction_selection": build_task_document_instruction_selection(selections, skill_root=SCRIPT_ROOT.parent),
            "tasks": tasks, "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
        }
        self._write_task_collection()

    def _write_task_collection(self):
        bundle = prepare_task_collection_create(
            render_task_contract(self.task), source="handoff fixture",
            raw_plan_path=self.task["artifacts"]["plan"],
            raw_task_path=self.task["artifacts"]["task"],
            project_root=self.root, user_config_root=str(self.root),
        )
        self.task_path = self.root / bundle["normalized_index"]
        item_directory = self.task_path.parent / "tasks"
        item_directory.mkdir(parents=True, exist_ok=True)
        self.task_path.write_bytes(bundle["index_raw"])
        for task_id, raw in bundle["items"].items():
            item_directory.joinpath(f"{task_id}.json").write_bytes(raw)
        return bundle["validation"]

    def build_execute(self, task_id="TASK-001", request=None):
        before = self.snapshot()
        try:
            return build_task_to_execute_handoff(
                self.root, {"summary": "交接指定 TASK"} if request is None else request,
                task_path=self.task["artifacts"]["task"], task_id=task_id, user_config_root=str(self.root),
            )
        finally:
            self.assertEqual(self.snapshot(), before)

    def return_request(self):
        return {
            "summary": "調整已確認範圍", "confirmed_approach": "保留既有介面",
            "requested_changes": ["新增驗收條件"], "preserve": ["既有功能"],
            "affected_ids": ["GOAL-001", "TASK-001"], "validation_requirements": ["重新確認驗收條件"],
        }

    def write_closed_execution(self, status="stopped"):
        self.write_task()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        self.index = build_initial_execution_index(self.task, checked)
        self.attempt = {
            "schema": "work-attempt/v1", "attempt_id": "ATTEMPT-001",
            "task_spec_id": self.task["spec_id"], "task_id": "TASK-001", "skill_id": None,
            "status": status, "task_collection_sha256": checked["task_collection_sha256"],
            "task_index_sha256": checked["task_index_sha256"],
            "task_item_sha256": checked["task_item_sha256"]["TASK-001"],
            "task_instructions_sha256": checked["task_instructions_sha256"]["TASK-001"],
            "hierarchy_selection_sha256": checked["hierarchy_selection_sha256"],
            "execute_instructions_sha256": build_instruction_selection(
                skill_root=SCRIPT_ROOT.parent, mode="execute", selected_paths=[],
                reference_names=["execute.general.execution-records"],
            )["instructions_sha256"],
            "execute_skill_selection_sha256": selection_sha256("base_only", []),
            "authorization": minimal_authorization(),
            "authorization_sha256": authorization_sha256(minimal_authorization()),
            "started_at": "2026-09-01T10:00+08:00", "records": [],
            "ended_at": "2026-09-01T10:05+08:00",
            "final_type": "specification_defect" if status == "stopped" else "required_input",
            "reason": "Missing specification detail",
            "closing_authorization_evidence": "User approved this closure.",
        }
        self.index["tasks"][0].update(status="blocked", latest_attempt="ATTEMPT-001",
                                      status_reason={"kind": "attempt", "ref": "ATTEMPT-001"})
        self.index["overall_status"] = derive_overall_status([row["status"] for row in self.index["tasks"]])
        execution = self.root / self.task["artifacts"]["execution"]
        self.index_path = execution / "index.json"
        self.attempt_path = execution / "TASK-001/ATTEMPT-001/attempt.json"
        self.attempt_path.parent.mkdir(parents=True, exist_ok=True)
        self.save_execution()

    def write_unstarted_execution(self):
        self.write_task()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        self.index = build_initial_execution_index(self.task, checked)
        self.index_path = self.root / self.task["artifacts"]["execution"] / "index.json"
        self.index_path.parent.mkdir(parents=True, exist_ok=True)
        self.index_path.write_bytes(render_execution_index(self.index))

    def build_preflight_return(self, direction="execute_to_task", request=None, task_id="TASK-001"):
        before = self.snapshot()
        try:
            return build_preflight_return_handoff(
                self.root, {**self.return_request(), "reason": "Clarify specification"} if request is None else request,
                direction=direction, task_path=self.task["artifacts"]["task"], task_id=task_id,
                user_config_root=str(self.root),
            )
        finally:
            self.assertEqual(self.snapshot(), before)

    def test_preflight_return_derives_context_and_target_task_fingerprint(self):
        self.write_unstarted_execution()
        for task_id in ("TASK-001", "TASK-002"):
            for direction in ("execute_to_task", "execute_to_plan"):
                with self.subTest(task_id=task_id, direction=direction):
                    result = self.build_preflight_return(direction, task_id=task_id)
                    validate_handoff_contract(result, project_root=self.root)
                    self.assertEqual(result["source"]["execution_context"], {
                        "attempt": {"status": "not_created"}, "phase": "preflight",
                        "issue_type": "specification_defect", "reason": "Clarify specification",
                    })
                    row = next(row for row in self.index["tasks"] if row["id"] == task_id)
                    self.assertEqual(result["source"]["task_instructions_sha256"], row["instructions_sha256"])
                    self.assertEqual(result["source"]["execute_skill_selection_sha256"], selection_sha256("base_only", []))
                    self.assertNotIn("eligibility", result)

    def test_preflight_return_accepts_empty_target_directory(self):
        self.write_unstarted_execution()
        (self.index_path.parent / "TASK-001").mkdir()
        self.build_preflight_return()

    def test_preflight_return_does_not_require_manual_input_readiness(self):
        from worklib.orchestration.execution import execute_preflight
        self.write_unstarted_execution()
        self.task["tasks"][0]["inputs"] = [{"id": "INPUT-001", "kind": "user_provided",
                                            "source": "User specification", "precondition": "User confirms detail"}]
        self._write_task_collection()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        self.index = build_initial_execution_index(self.task, checked)
        self.index_path.write_bytes(render_execution_index(self.index))
        with self.assertRaises(WorkError) as error:
            execute_preflight(project_root=self.root, user_config_root=str(self.root),
                              raw_task_path=self.task["artifacts"]["task"],
                              raw_execution_dir=self.task["artifacts"]["execution"], task_id="TASK-001")
        self.assertEqual(error.exception.code, "execute_preflight_input_confirmation_required")
        self.assertEqual(self.build_preflight_return()["source"]["execution_context"]["phase"], "preflight")

    def test_preflight_return_rejects_closed_attempt_history(self):
        self.write_closed_execution()
        with self.assertRaises(WorkError) as error:
            self.build_preflight_return()
        self.assertEqual(error.exception.code, "handoff_task_already_started")

    def test_preflight_return_rejects_orphan_attempt_directory(self):
        self.write_unstarted_execution()
        (self.index_path.parent / "TASK-001/ATTEMPT-001").mkdir(parents=True)
        with self.assertRaises(WorkError) as error:
            self.build_preflight_return()
        self.assertEqual(error.exception.code, "handoff_attempt_artifacts_present")

    def test_preflight_return_rejects_transaction_residue(self):
        self.write_unstarted_execution()
        (self.index_path.parent / ".work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp").write_bytes(b"pending")
        with self.assertRaises(WorkError) as error:
            self.build_preflight_return()
        self.assertEqual(error.exception.code, "handoff_execution_recovery_required")

    def test_preflight_return_rejects_index_identity_and_skill_drift(self):
        for field in ("task_collection_sha256", "task_index_sha256"):
            self.write_unstarted_execution()
            self.index[field] = "0" * 64
            self.index_path.write_bytes(render_execution_index(self.index))
            with self.subTest(field=field), self.assertRaises(WorkError):
                self.build_preflight_return()
        for field, value in (("skill_id", "unknown"), ("instructions_sha256", "0" * 64)):
            self.write_unstarted_execution()
            self.index["tasks"][0][field] = value
            self.index_path.write_bytes(render_execution_index(self.index))
            with self.subTest(field=field), self.assertRaises(WorkError):
                self.build_preflight_return()

    def test_preflight_return_rejects_machine_fields_and_invalid_semantics(self):
        self.write_unstarted_execution()
        request = {**self.return_request(), "reason": "Specification defect"}
        for invalid in ({**request, "source": {}}, {**request, "attempt": {"status": "not_created"}},
                        {**request, "reason": ""}, {**request, "affected_ids": ["INPUT-999"]}, self.return_request()):
            with self.subTest(request=invalid), self.assertRaises(WorkError):
                self.build_preflight_return(request=invalid)

    def test_preflight_return_rechecks_source_snapshots(self):
        self.write_unstarted_execution()
        from worklib.technical.infrastructure.file_io import read_raw
        for target in (self.task_path, self.plan_path, self.index_path):
            reads = []
            def changed(path):
                reads.append(path)
                return b"changed" if path == target and reads.count(path) > 1 else read_raw(path)
            with self.subTest(target=target), patch("worklib.business_services.handoff.read_raw", side_effect=changed):
                with self.assertRaises(WorkError):
                    self.build_preflight_return()

    def test_preflight_return_rechecks_attempt_absence(self):
        self.write_unstarted_execution()
        from worklib.technical.infrastructure.work_paths import validate_execution_task_layout
        calls = []
        def observed(root, relative):
            calls.append(relative)
            if len(calls) == 2:
                return self.root
            return validate_execution_task_layout(root, relative)
        with patch("worklib.business_services.handoff.validate_execution_task_layout", side_effect=observed):
            with self.assertRaises(WorkError) as error:
                self.build_preflight_return()
        self.assertEqual(error.exception.code, "handoff_attempt_artifacts_present")
        self.assertEqual(len(calls), 2)

    def test_preflight_return_cli_round_trips(self):
        self.write_unstarted_execution()
        for target in ("task", "plan"):
            output, error = io.StringIO(), io.StringIO()
            before = self.snapshot()
            code = main(self.input_arguments(["--project-root", str(self.root), "handoff", f"build-execute-to-{target}",
                         "--input-file", "request.json", "--task-path", self.task["artifacts"]["task"], "--task-id", "TASK-001",
                         "--preflight", "--user-config-root", str(self.root)], json.dumps({**self.return_request(), "reason": "Clarify specification"})), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (0, ""))
            self.assertEqual(json.loads(output.getvalue())["data"], self.build_preflight_return(f"execute_to_{target}"))
            self.assertEqual(self.snapshot(), before)

    def save_execution(self):
        self.index_path.write_bytes(render_execution_index(self.index))
        self.attempt_path.write_bytes(render_attempt_contract(self.attempt, project_root=self.root))

    def build_execution_return(self, direction="execute_to_task", request=None, attempt_id="ATTEMPT-001"):
        before = self.snapshot()
        try:
            return build_execute_return_handoff(
                self.root, {**self.return_request(), "reason": "Clarify specification"} if request is None else request,
                direction=direction, task_path=self.task["artifacts"]["task"], task_id="TASK-001",
                attempt_id=attempt_id, user_config_root=str(self.root),
            )
        finally:
            self.assertEqual(self.snapshot(), before)

    def test_execution_return_derives_both_directions_and_closed_statuses(self):
        for status in ("stopped", "blocked"):
            self.write_closed_execution(status)
            for direction in ("execute_to_task", "execute_to_plan"):
                with self.subTest(status=status, direction=direction):
                    result = self.build_execution_return(direction)
                    validate_handoff_contract(result, project_root=self.root)
                    self.assertEqual(result["source"]["execution_context"], {
                        "attempt": {"id": "ATTEMPT-001", "status": status}, "phase": "execution",
                        "issue_type": "specification_defect", "reason": "Clarify specification",
                    })
                    self.assertEqual(result["source"]["task_instructions_sha256"], self.attempt["task_instructions_sha256"])
                    self.assertEqual(result["source"]["execute_skill_selection_sha256"], self.attempt["execute_skill_selection_sha256"])

    def test_execution_return_rejects_stale_identity_and_skill_fingerprints(self):
        for field in ("task_collection_sha256", "task_index_sha256", "task_item_sha256", "task_instructions_sha256", "execute_instructions_sha256", "execute_skill_selection_sha256", "skill_id"):
            self.write_closed_execution()
            self.attempt[field] = "wrong" if field == "skill_id" else "0" * 64
            self.save_execution()
            with self.subTest(field=field), self.assertRaises(WorkError):
                self.build_execution_return()

    def test_execution_return_rejects_active_completed_and_old_attempts(self):
        for status in ("in_progress", "completed"):
            self.write_closed_execution()
            self.attempt["status"] = status
            for field in (
                "reason", "final_type", "closing_authorization_evidence",
            ):
                self.attempt.pop(field)
            if status == "in_progress":
                self.attempt.pop("ended_at")
            self.save_execution()
            with self.subTest(status=status), self.assertRaises(WorkError):
                self.build_execution_return()
        self.write_closed_execution()
        with self.assertRaises(WorkError):
            self.build_execution_return(attempt_id="ATTEMPT-002")

    def test_execution_return_rejects_machine_fields_and_unknown_ids(self):
        self.write_closed_execution()
        request = {**self.return_request(), "reason": "Specification defect"}
        for invalid in ({**request, "source": {}}, {**request, "phase": "preflight"},
                        {**request, "reason": ""}, {**request, "affected_ids": ["VAL-999"]}, self.return_request()):
            with self.subTest(request=invalid), self.assertRaises(WorkError):
                self.build_execution_return(request=invalid)

    def test_execution_return_rejects_index_drift_and_transactions(self):
        self.write_closed_execution()
        self.index["task_collection_sha256"] = "0" * 64
        self.save_execution()
        with self.assertRaises(WorkError):
            self.build_execution_return()
        self.write_closed_execution()
        (self.index_path.parent / ".work-attempt-close-pending.tmp").write_bytes(b"pending")
        with self.assertRaises(WorkError) as error:
            self.build_execution_return()
        self.assertEqual(error.exception.code, "handoff_execution_recovery_required")

    def test_execution_return_rejects_source_changes_during_construction(self):
        self.write_closed_execution()
        from worklib.technical.infrastructure.file_io import read_raw
        for target in (self.task_path, self.plan_path, self.index_path, self.attempt_path):
            reads = []
            def changed(path):
                reads.append(path)
                return b"changed" if path == target and reads.count(path) > 1 else read_raw(path)
            with self.subTest(target=target), patch("worklib.business_services.handoff.read_raw", side_effect=changed):
                with self.assertRaises(WorkError):
                    self.build_execution_return()

    def test_execution_return_cli_round_trips(self):
        self.write_closed_execution()
        for target in ("task", "plan"):
            output, error = io.StringIO(), io.StringIO()
            before = self.snapshot()
            code = main(self.input_arguments(["--project-root", str(self.root), "handoff", f"build-execute-to-{target}",
                         "--input-file", "request.json", "--task-path", self.task["artifacts"]["task"], "--task-id", "TASK-001",
                         "--attempt-id", "ATTEMPT-001", "--user-config-root", str(self.root)], json.dumps({**self.return_request(), "reason": "Clarify specification"})), stdout=output, stderr=error)
            self.assertEqual((code, error.getvalue()), (0, ""))
            self.assertEqual(json.loads(output.getvalue())["data"], self.build_execution_return(f"execute_to_{target}"))
            self.assertEqual(self.snapshot(), before)

    def build_return(self, task_id=None, request=None):
        before = self.snapshot()
        try:
            return build_task_to_plan_handoff(self.root, self.return_request() if request is None else request,
                                             task_path=self.task["artifacts"]["task"], task_id=task_id, user_config_root=str(self.root))
        finally:
            self.assertEqual(self.snapshot(), before)

    def test_task_return_derives_specification_source_without_inventing_target(self):
        self.write_task()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        request = self.return_request()
        result = self.build_return(request=request)
        self.assertEqual(result["source"], {"stage": "task", "task_spec_id": "TASK-SPEC-001",
                                           "task_collection_sha256": checked["task_collection_sha256"],
                                           "task_index_sha256": checked["task_index_sha256"],
                                           "plan_sha256": checked["source_plan_sha256"], "skill_selection_sha256": checked["skill_selection_sha256"]})
        self.assertEqual(result["target"], {"stage": "plan"})
        for field, value in request.items():
            self.assertEqual(result[field], value)
        self.assertEqual(validate_handoff_contract(result, project_root=self.root)["status"], "valid")
        result["preserve"].append("Different value")
        self.assertEqual(request, self.return_request())
        self.assertFalse((self.root / self.plan["artifacts"]["execution"]).exists())

    def test_task_return_scopes_local_ids_to_explicit_target(self):
        self.write_task()
        request = self.return_request()
        request["affected_ids"] = ["VAL-001"]
        with self.assertRaises(WorkError) as context:
            self.build_return(request=request)
        self.assertEqual(context.exception.code, "handoff_unknown_affected_ids")
        result = self.build_return("TASK-002", request)
        self.assertEqual(result["source"]["task_id"], "TASK-002")
        self.assertIsNone(result["source"]["skill_id"])
        request["affected_ids"] = ["VAL-999"]
        with self.assertRaises(WorkError) as context:
            self.build_return("TASK-002", request)
        self.assertEqual(context.exception.code, "handoff_unknown_affected_ids")

    def test_task_return_rejects_unknown_targets_and_affected_ids(self):
        self.write_task()
        with self.assertRaises(WorkError) as context:
            self.build_return("TASK-999")
        self.assertEqual(context.exception.code, "handoff_task_not_found")
        request = self.return_request()
        for identifiers, code in ((["GOAL-999"], "handoff_unknown_affected_ids"), (["TASK-001", "TASK-001"], "duplicate_array_value")):
            request["affected_ids"] = identifiers
            with self.assertRaises(WorkError) as context:
                self.build_return(request=request)
            self.assertEqual(context.exception.code, code)

    def test_task_return_requires_semantic_fields_and_rejects_machine_overrides(self):
        self.write_task()
        for field in self.return_request():
            request = self.return_request()
            del request[field]
            with self.subTest(field=field), self.assertRaises(WorkError) as context:
                self.build_return(request=request)
            self.assertEqual(context.exception.code, "invalid_object_fields")
        for field in ("source", "artifacts", "task_id", "skill_id", "direction"):
            with self.subTest(field=field), self.assertRaises(WorkError) as context:
                self.build_return(request={**self.return_request(), field: "override"})
            self.assertEqual(context.exception.code, "invalid_object_fields")
        request = self.return_request()
        request["requested_changes"] = []
        with self.assertRaises(WorkError) as context:
            self.build_return(request=request)
        self.assertEqual(context.exception.code, "invalid_string_array")

    def test_task_return_rejects_source_drift(self):
        self.write_task()
        self.plan["summary"] = "Changed source"
        self.write_plan()
        with self.assertRaises(WorkError) as context:
            self.build_return()
        self.assertEqual(context.exception.code, "source_plan_fingerprint_mismatch")

    def test_task_return_rejects_changes_during_construction(self):
        self.write_task()
        for target in (self.task_path, self.plan_path):
            reads = 0

            def changing_read(path):
                nonlocal reads
                raw = read_raw(path)
                if Path(path).resolve() == target.resolve():
                    reads += 1
                    if reads > 1:
                        return raw + b"\n"
                return raw

            with self.subTest(target=target), patch(
                "worklib.business_services.handoff.read_raw", side_effect=changing_read
            ):
                with self.assertRaises(WorkError) as context:
                    self.build_return()
            self.assertEqual(context.exception.code, "handoff_source_changed")

    def test_task_return_cli_round_trips_with_custom_paths(self):
        self.plan["artifacts"] = {"plan": "custom/plans/example.json", "task": "custom/tasks/example/index.json", "execution": "custom/executions/example"}
        self.write_plan()
        self.write_task()
        before = self.snapshot()
        output, error = io.StringIO(), io.StringIO()
        code = main(self.input_arguments([
            "--project-root", str(self.root), "handoff", "build-task-to-plan", "--input-file", "request.json",
            "--task-path", self.task["artifacts"]["task"], "--user-config-root", str(self.root),
        ], json.dumps(self.return_request())), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
        result = json.loads(output.getvalue())["data"]
        self.assertEqual(result["artifacts"], self.plan["artifacts"])
        self.assertEqual(validate_handoff_contract(result, project_root=self.root)["status"], "valid")
        self.assertEqual(self.snapshot(), before)

    def test_execute_handoff_uses_selected_task_fingerprint(self):
        self.write_task()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        first = self.build_execute()
        second = self.build_execute("TASK-002")
        self.assertEqual(first["source"], {
            "stage": "task", "task_spec_id": "TASK-SPEC-001", "task_id": "TASK-001", "skill_id": None,
            "task_collection_sha256": checked["task_collection_sha256"],
            "task_index_sha256": checked["task_index_sha256"],
            "task_item_sha256": checked["task_item_sha256"]["TASK-001"],
            "task_instructions_sha256": checked["task_instructions_sha256"]["TASK-001"],
            "skill_selection_sha256": checked["skill_selection_sha256"],
        })
        self.assertNotEqual(first["source"]["task_instructions_sha256"], second["source"]["task_instructions_sha256"])
        self.assertNotEqual(first["source"]["task_instructions_sha256"], checked["instructions_sha256"])
        self.assertEqual(second["source"]["task_id"], "TASK-002")
        self.assertEqual(first["target"], {"stage": "execute"})
        self.assertEqual(validate_handoff_contract(first, project_root=self.root)["status"], "valid")
        self.assertFalse((self.root / self.plan["artifacts"]["execution"]).exists())

    def test_execute_handoff_rejects_unknown_target_and_machine_fields(self):
        self.write_task()
        for task_id in ("", "TASK-999"):
            with self.assertRaises(WorkError) as context:
                self.build_execute(task_id)
            self.assertEqual(context.exception.code, "handoff_task_not_found")
        for field in ("source", "task_id", "artifacts", "skill_id", "affected_ids"):
            with self.subTest(field=field), self.assertRaises(WorkError) as context:
                self.build_execute(request={"summary": "Summary", field: "override"})
            self.assertEqual(context.exception.code, "invalid_object_fields")

    def test_execute_handoff_rejects_stale_source_plan(self):
        self.write_task()
        self.plan["summary"] = "Changed source"
        self.write_plan()
        with self.assertRaises(WorkError) as context:
            self.build_execute()
        self.assertEqual(context.exception.code, "source_plan_fingerprint_mismatch")

    def test_execute_handoff_rejects_task_or_plan_changed_during_build(self):
        self.write_task()
        for target in (self.task_path, self.plan_path):
            reads = 0

            def changing_read(path):
                nonlocal reads
                raw = read_raw(path)
                if Path(path).resolve() == target.resolve():
                    reads += 1
                    if reads > 1:
                        return raw + b"\n"
                return raw

            with self.subTest(target=target), patch(
                "worklib.business_services.handoff.read_raw", side_effect=changing_read
            ):
                with self.assertRaises(WorkError) as context:
                    self.build_execute()
            self.assertEqual(context.exception.code, "handoff_source_changed")

    def test_execute_handoff_detects_plan_change_between_validations(self):
        self.write_task()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        self.plan["summary"] = "Changed source"
        self.write_plan()
        with patch("worklib.orchestration.handoff.HandoffOperations.load_task_collection", return_value=checked):
            with self.assertRaises(WorkError) as context:
                self.build_execute()
        self.assertEqual(context.exception.code, "handoff_source_changed")

    def test_execute_handoff_uses_validated_skill_binding(self):
        self.write_task()
        checked = load_task_collection(self.root, str(self.root), self.task["artifacts"]["task"])
        checked["task_skill_ids"]["TASK-002"] = "repo:confirmed-skill"
        with patch("worklib.orchestration.handoff.HandoffOperations.load_task_collection", return_value=checked):
            self.assertEqual(self.build_execute("TASK-002")["source"]["skill_id"], "repo:confirmed-skill")

    def test_execute_handoff_rejects_unconfirmed_or_noncanonical_task(self):
        self.write_task()
        index = json.loads(self.task_path.read_bytes())
        index["status"] = "draft"
        self.task_path.write_bytes(json.dumps(index).encode("utf-8"))
        with self.assertRaises(WorkError) as context:
            self.build_execute()
        self.assertEqual(context.exception.code, "invalid_contract_value")
        self.write_task()
        self.task_path.write_bytes(json.dumps(self.task).encode("utf-8"))
        with self.assertRaises(WorkError):
            self.build_execute()

    def test_execute_handoff_cli_uses_custom_paths_and_round_trips(self):
        self.plan["artifacts"] = {"plan": "custom/plans/example.json", "task": "custom/tasks/example/index.json", "execution": "custom/executions/example"}
        self.write_plan()
        self.write_task()
        before = self.snapshot()
        output, error = io.StringIO(), io.StringIO()
        code = main(self.input_arguments([
            "--project-root", str(self.root), "handoff", "build-task-to-execute", "--input-file", "request.json",
            "--task-path", self.task["artifacts"]["task"], "--task-id", "TASK-002", "--user-config-root", str(self.root),
        ], json.dumps({"summary": "Review target"})), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
        result = json.loads(output.getvalue())["data"]
        self.assertEqual(result["artifacts"], self.plan["artifacts"])
        self.assertEqual(validate_handoff_contract(result, project_root=self.root)["status"], "valid")
        self.assertEqual(self.snapshot(), before)

    def build(self):
        before = self.snapshot()
        try:
            return build_plan_to_task_handoff(self.root, self.request, plan_path=self.plan["artifacts"]["plan"], user_config_root=str(self.root))
        finally:
            self.assertEqual(self.snapshot(), before)

    def test_derives_machine_fields_and_preserves_semantic_input_without_writing(self):
        request = copy.deepcopy(self.request)
        checked = validate_plan_file(self.root, str(self.root), self.plan["artifacts"]["plan"])
        result = self.build()
        self.assertEqual(result, {
            "schema": "work-handoff/v1", "marker": "WORK-HANDOFF", "direction": "plan_to_task",
            "requirement_id": "example", "artifacts": self.plan["artifacts"],
            "source": {"stage": "plan", "plan_sha256": checked["plan_sha256"], "skill_selection_sha256": checked["skill_selection_sha256"]},
            "target": {"stage": "task"}, **request,
        })
        self.assertEqual(validate_handoff_contract(result, project_root=self.root)["status"], "valid")
        result["affected_ids"].append("SCOPE-001")
        self.assertEqual(self.request, request)
        self.assertFalse((self.root / self.plan["artifacts"]["task"]).exists())
        self.assertFalse((self.root / self.plan["artifacts"]["execution"]).exists())

    def test_custom_artifact_paths_come_from_plan(self):
        self.plan["artifacts"] = {"plan": "custom/plans/example.json", "task": "custom/tasks/example/index.json", "execution": "custom/executions/example"}
        self.write_plan()
        self.assertEqual(self.build()["artifacts"], self.plan["artifacts"])

    def test_rejects_unknown_duplicate_or_malformed_affected_ids(self):
        for identifiers, code in ((["GOAL-999"], "handoff_unknown_plan_ids"), (["GOAL-001", "GOAL-001"], "duplicate_array_value"), ([], "invalid_string_array"), (["PLAN"], "invalid_handoff_identifier")):
            with self.subTest(identifiers=identifiers):
                self.request["affected_ids"] = identifiers
                with self.assertRaises(WorkError) as context:
                    self.build()
                self.assertEqual(context.exception.code, code)

    def test_rejects_machine_field_overrides(self):
        for field in ("schema", "direction", "source", "target", "artifacts", "requirement_id"):
            with self.subTest(field=field):
                self.request[field] = "override"
                with self.assertRaises(WorkError) as context:
                    self.build()
                self.assertEqual(context.exception.code, "invalid_object_fields")
                del self.request[field]

    def test_rejects_missing_or_empty_semantic_input(self):
        self.request["summary"] = " "
        with self.assertRaises(WorkError) as context:
            self.build()
        self.assertEqual(context.exception.code, "empty_text_value")
        del self.request["summary"]
        with self.assertRaises(WorkError) as context:
            self.build()
        self.assertEqual(context.exception.code, "invalid_object_fields")

    def test_rejects_unconfirmed_plan_and_skill_fingerprint_drift(self):
        self.plan["status"] = "draft"
        self.write_plan()
        with self.assertRaises(WorkError) as context:
            self.build()
        self.assertEqual(context.exception.code, "invalid_plan_status")
        self.plan["status"] = "confirmed"
        self.plan["skill_selection"]["selection_sha256"] = "0" * 64
        self.write_plan()
        with self.assertRaises(WorkError) as context:
            self.build()
        self.assertEqual(context.exception.code, "skill_selection_fingerprint_mismatch")

    def test_rejects_noncanonical_plan_without_rewriting(self):
        self.plan_path.write_bytes(json.dumps(self.plan).encode("utf-8"))
        with self.assertRaises(WorkError):
            self.build()

    def test_rejects_plan_changed_during_construction(self):
        raw = self.plan_path.read_bytes()
        with patch("worklib.business_services.handoff.read_raw", side_effect=[raw, raw + b"\n"]):
            with self.assertRaises(WorkError) as context:
                self.build()
        self.assertEqual(context.exception.code, "handoff_source_changed")

    def test_rejects_plan_path_redirected_during_construction(self):
        relative = self.plan["artifacts"]["plan"]
        with patch("worklib.business_services.handoff.resolve_project_relative_path", side_effect=[(relative, self.plan_path), (relative, self.root / "other.json")]):
            with self.assertRaises(WorkError) as context:
                self.build()
        self.assertEqual(context.exception.code, "handoff_source_changed")

    def test_rejects_unsafe_source_path(self):
        before = self.snapshot()
        with self.assertRaises(WorkError):
            build_plan_to_task_handoff(self.root, self.request, plan_path="../example.json", user_config_root=str(self.root))
        self.assertEqual(self.snapshot(), before)

    def test_cli_build_output_round_trips_through_existing_handoff_validator(self):
        before = self.snapshot()
        output, error = io.StringIO(), io.StringIO()
        code = main(self.input_arguments([
            "--project-root", str(self.root), "handoff", "build-plan-to-task", "--input-file", "request.json",
            "--plan-path", self.plan["artifacts"]["plan"], "--user-config-root", str(self.root),
        ], json.dumps(self.request)), stdout=output, stderr=error)
        self.assertEqual((code, error.getvalue()), (ExitCode.SUCCESS, ""))
        validated, validation_error = io.StringIO(), io.StringIO()
        code = main(self.input_arguments(["--project-root", str(self.root), "handoff", "validate", "--input-file", "request.json"], json.dumps(json.loads(output.getvalue())["data"])), stdout=validated, stderr=validation_error)
        self.assertEqual((code, validation_error.getvalue()), (ExitCode.SUCCESS, ""))
        self.assertEqual(json.loads(validated.getvalue())["data"]["status"], "valid")
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
