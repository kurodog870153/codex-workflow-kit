"""Assemble structured discussions and bind initial formal creation to review."""

from __future__ import annotations

import copy
import hashlib
from pathlib import Path
from typing import Any

from .creation import create_task_artifacts, prepare_task_collection_create
from ...services.task.draft.storage import _render, read_task_draft, read_task_planning_index
from .plan import validate_task_source_plan as validate_plan_contract
from ...models.common.validation import ContractValuePolicy
from ...models.common.errors import ExitCode, WorkError
from ...services.plan.document import parse as parse_json_contract
from ...services.task.storage import read_project_task_source
from ...services.instruction.root import instruction_root
from ...services.task.draft.validation import resolve_draft_instruction_selection, validate_semantic_task_candidate, build_semantic_task_candidate
from ...models.skill import SkillRoot


sha256 = ContractValuePolicy.sha256
strict_keys = ContractValuePolicy.strict_keys


def assemble_task_drafts(
    project_root: Path, requirement_id: str, metadata: object, *,
    expected_revision: int, plan_path: str, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None, operations=None,
) -> dict[str, Any]:
    """Return a validated complete contract without writing or inferring fields."""
    fields = strict_keys(metadata, location="assembly", required={"title", "summary"}, optional={"execution_defaults", "decisions"})
    index = read_task_planning_index(project_root, requirement_id)
    if type(expected_revision) is not int or index["revision"] != expected_revision:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "The reviewed planning revision is no longer current.")
    normalized, resolved, raw_plan = read_project_task_source(project_root, plan_path, field="plan_path")
    plan = parse_json_contract(raw_plan, source=str(resolved))
    plan_validation = operations.validate_plan_contract(
        raw_plan, source=str(resolved), actual_plan_path=normalized, project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
        _allow_task_index=True,
    )
    if plan_validation["requirement_id"] != requirement_id or any(plan_validation[key] != value for key, value in index["source"].items()):
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "draft_source_drift", "The planning source differs from the current validated Plan.")
    tasks = []
    semantic_candidates = {}
    for entry in index["tasks"]:
        discussion = read_task_draft(project_root, requirement_id, entry["id"])
        if isinstance(discussion.get("task_candidate"), dict):
            semantic_candidates[entry["id"]] = validate_semantic_task_candidate(discussion["task_candidate"], refined=True)
    dependency_files = {task_id: {row["key"]: f"FILE-{position:03d}" for position, row in enumerate(candidate.get("files", []), 1)} for task_id, candidate in semantic_candidates.items()}
    for entry in sorted(index["tasks"], key=lambda item: item["id"]):
        if entry["status"] != "refined":
            raise WorkError(ExitCode.WORKFLOW_STATE, "draft_not_refined", "Every active TASK must complete discussion before assembly.", {"task_id": entry["id"]})
        discussion = read_task_draft(project_root, requirement_id, entry["id"])
        candidate = discussion.get("task_candidate")
        if not isinstance(candidate, dict):
            raise WorkError(ExitCode.CONTRACT, "task_candidate_required", "A structured candidate is required; discussion notes cannot be inferred into a TASK.", {"task_id": entry["id"]})
        candidate = build_semantic_task_candidate(candidate, acceptance_ids=[item["id"] for item in plan["acceptance_criteria"]], dependency_ids=entry["dependencies"], dependency_files=dependency_files)
        selected = resolve_draft_instruction_selection(entry)
        instruction = operations.build_instruction_selection(
            skill_root=instruction_root(), mode="task",
            selected_paths=selected["selected_paths"], reference_names=selected["references"])
        if instruction["instructions_sha256"] != entry["instructions_sha256"]:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "draft_instruction_drift", "The TASK instruction selection differs from its confirmed boundary.")
        traceability = {field: [item["id"] for item in plan[source_field]] for field, source_field in (
            ("goal_ids", "goals"), ("deliverable_ids", "deliverables"),
            ("acceptance_ids", "acceptance_criteria"))}
        tasks.append({"id": entry["id"], "title": entry["title"], "goal": entry["goal"],
                      "skill_id": entry["skill_id"], "dependencies": entry["dependencies"],
                      "instruction_selection": instruction, "traceability": traceability,
                      **copy.deepcopy(candidate)})
    contract = {
        "schema": "work-task-collection-projection/v1", "requirement_id": requirement_id,
        "spec_id": "TASK-SPEC-001", "status": "confirmed", **copy.deepcopy(fields),
        "artifacts": plan["artifacts"],
        "source_plan": {"canonical_sha256": index["source"]["plan_sha256"], "hierarchy_selection_sha256": index["source"]["hierarchy_selection_sha256"]},
        "instruction_selection": operations.build_task_document_instruction_selection([task["instruction_selection"] for task in tasks], skill_root=instruction_root()),
        "tasks": tasks, "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"},
    }
    bundle = prepare_task_collection_create(
        _render(contract), source="assembled drafts",
        raw_plan_path=plan["artifacts"]["plan"],
        raw_task_path=plan["artifacts"]["task"],
        project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    validation, rendered = bundle["validation"], bundle["approval_bytes"]
    if read_task_planning_index(project_root, requirement_id) != index:
        raise WorkError(ExitCode.WORKFLOW_STATE, "draft_revision_conflict", "The planning index changed during assembly.")
    # Bind discussion versions (including notes) as well as the canonical TASK.
    review = hashlib.sha256(b"WORK-TASK-DRAFT-APPROVAL-V1\n" + _render(index) + rendered).hexdigest()
    return {
        "schema": "work-task-draft-assembly/v1", "status": "valid", "requirement_id": requirement_id,
        "revision": expected_revision, "approval_sha256": review,
        "task_collection_sha256": validation["task_collection_sha256"],
        "task_index_sha256": validation["task_index_sha256"],
        "task_item_sha256": validation["task_item_sha256"],
        "contract": contract,
    }


def create_task_from_drafts(
    project_root: Path, requirement_id: str, metadata: object, *, approved_sha256: str,
    expected_revision: int, plan_path: str, user_config_root: str,
    skill_roots: list[SkillRoot] | None = None, operations=None,
) -> dict[str, object]:
    """Reassemble before creation; the supplied hash is not user authorization."""
    sha256(approved_sha256, location="approved_sha256")
    assembled = assemble_task_drafts(
        project_root, requirement_id, metadata, expected_revision=expected_revision,
        plan_path=plan_path, user_config_root=user_config_root, skill_roots=skill_roots,
        operations=operations,
    )
    if assembled["approval_sha256"] != approved_sha256:
        raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "draft_approval_mismatch", "The assembled content differs from the reviewed fingerprint.")
    contract = assembled["contract"]
    result = create_task_artifacts(
        _render(contract), source="approved draft assembly", raw_plan_path=contract["artifacts"]["plan"],
        raw_task_path=contract["artifacts"]["task"], raw_execution_dir=contract["artifacts"]["execution"],
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
    )
    result["approval_sha256"] = assembled["approval_sha256"]
    return result
