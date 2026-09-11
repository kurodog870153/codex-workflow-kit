from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_sha256, read_raw
from ..foundation.markdown import (
    parse_json_contract,
    render_json_contract,
    require_canonical_json_contract,
)
from ..foundation.paths import (
    normalize_relative_path,
    portable_path_identity,
    resolve_project_relative_path,
    validate_artifact_paths,
)
from ..foundation.runtime import installed_work_root
from .plan import validate_plan_contract
from ..instructions.historical import stored_document_selection
from ..skills.catalog import SkillRoot
from ..hierarchy.selection import validate_task_hierarchy_paths
from ..instructions.task_selection import validate_task_document_instruction_selection
from .task_changes import validate_task_changes
from .task_dependencies import resolve_task_dependencies
from .task_ordering import order_task_contract
from .validation import (
    nonempty_string as _nonempty_string,
    sha256 as _sha256,
    strict_keys as _strict_keys,
)


TASK_ID_PATTERN = re.compile(r"^TASK-(\d{3})$")
SPEC_ID_PATTERN = re.compile(r"^TASK-SPEC-(\d{3})$")
QUALIFIED_FILE_PATTERN = re.compile(r"^(TASK-\d{3})/(FILE-\d{3})$")
TOP_REQUIRED = {
    "schema",
    "requirement_id",
    "spec_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "source_plan",
    "instruction_selection",
    "tasks",
    "readiness",
}
TOP_OPTIONAL = {"execution_defaults", "decisions", "changes"}
OS_VALUES = {"windows", "macos", "linux"}
SHELL_VALUES = {"powershell", "pwsh", "cmd", "bash", "zsh", "sh"}


def _string_array(value: object, *, location: str, allow_empty: bool = False) -> list[str]:
    if not isinstance(value, list) or (not allow_empty and not value):
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_string_array",
            "A string array with the required cardinality is required.",
            {"location": location},
        )
    result = [_nonempty_string(item, location=f"{location}[]") for item in value]
    if len(result) != len(set(result)):
        raise WorkError(
            ExitCode.CONTRACT,
            "duplicate_array_value",
            "Array values must be unique.",
            {"location": location},
        )
    return result


def render_task_contract(contract: dict[str, Any]) -> bytes:
    ordered = order_task_contract(contract)
    return render_json_contract(ordered)


def _execution(value: object, *, location: str) -> dict[str, Any]:
    execution = _strict_keys(
        value,
        location=location,
        required={"working_directory", "os", "shell"},
    )
    working_directory = _nonempty_string(
        execution["working_directory"], location=f"{location}.working_directory"
    )
    if working_directory != ".":
        normalize_relative_path(working_directory, field=f"{location}.working_directory")
    if execution["os"] not in OS_VALUES or execution["shell"] not in SHELL_VALUES:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_execution_environment",
            "The execution OS or shell is invalid.",
            {"location": location},
        )
    return execution


def _local_items(
    task: dict[str, Any],
    key: str,
    prefix: str,
    *,
    required: set[str],
    optional: set[str] | None = None,
) -> list[dict[str, Any]]:
    raw_items = task.get(key)
    if not isinstance(raw_items, list) or not raw_items:
        raise WorkError(
            ExitCode.CONTRACT,
            "invalid_item_array",
            "A present TASK item array must be non-empty.",
            {"location": f"{task['id']}.{key}"},
        )
    result: list[dict[str, Any]] = []
    previous = 0
    for index, raw_item in enumerate(raw_items):
        item = _strict_keys(
            raw_item,
            location=f"{task['id']}.{key}[{index}]",
            required={"id", *required},
            optional=optional,
        )
        item_id = _nonempty_string(item["id"], location=f"{task['id']}.{key}[{index}].id")
        match = re.fullmatch(rf"{re.escape(prefix)}-(\d{{3}})", item_id)
        if not match or int(match.group(1)) <= previous:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_or_unsorted_id",
                "TASK item IDs must use the expected prefix and ascending order.",
                {"location": f"{task['id']}.{key}", "id": item_id},
            )
        previous = int(match.group(1))
        result.append(item)
    return result


def _plan_id_array(
    value: object,
    *,
    prefix: str,
    valid_ids: set[str],
    location: str,
) -> list[str]:
    ids = _string_array(value, location=location)
    numbers: list[int] = []
    for item_id in ids:
        match = re.fullmatch(rf"{re.escape(prefix)}-(\d{{3}})", item_id)
        if not match or item_id not in valid_ids:
            raise WorkError(
                ExitCode.CONTRACT,
                "invalid_reference",
                "A referenced Plan ID does not exist or has the wrong type.",
                {"location": location, "id": item_id},
            )
        numbers.append(int(match.group(1)))
    if numbers != sorted(numbers):
        raise WorkError(
            ExitCode.CONTRACT,
            "unsorted_references",
            "Referenced IDs must use ascending numeric order.",
            {"location": location},
        )
    return ids


def _validate_task_contract_object(
    contract: dict[str, Any],
    *,
    source: str,
    actual_task_path: str,
    project_root: Path,
    user_config_root: str,
    validate_file_state: bool,
    skill_roots: list[SkillRoot] | None = None,
    _source_plan_raw: bytes | None = None,
    _historical_work_sources: bool = False,
    _reviewed_source_plan_binding: tuple[str, str] | None = None,
) -> dict[str, object]:
    if _reviewed_source_plan_binding is not None and (
        not _historical_work_sources or _source_plan_raw is None
    ):
        raise WorkError(
            ExitCode.CONTRACT, "invalid_source_plan_repair_context",
            "Reviewed binding repair requires an explicit historical source Plan.",
        )
    _strict_keys(contract, location="task", required=TOP_REQUIRED, optional=TOP_OPTIONAL)
    if contract["schema"] != "work-task/v1" or contract["status"] != "confirmed":
        raise WorkError(ExitCode.CONTRACT, "invalid_task_identity", "Invalid TASK schema or status.")
    requirement_id = _nonempty_string(contract["requirement_id"], location="requirement_id")
    spec_id = _nonempty_string(contract["spec_id"], location="spec_id")
    spec_match = SPEC_ID_PATTERN.fullmatch(spec_id)
    if not spec_match:
        raise WorkError(ExitCode.CONTRACT, "invalid_task_spec_id", "Invalid TASK spec ID.")
    title = _nonempty_string(contract["title"], location="title")
    if "\n" in title or "\r" in title:
        raise WorkError(ExitCode.CONTRACT, "invalid_task_title", "TASK title must fit on one line.")
    _nonempty_string(contract["summary"], location="summary")
    artifacts = validate_artifact_paths(
        project_root,
        requirement_id,
        contract["artifacts"],
        actual_plan_path=contract["artifacts"].get("plan") if isinstance(contract["artifacts"], dict) else "",
    )
    normalized_task_path = normalize_relative_path(actual_task_path, field="actual_task_path")
    if normalized_task_path != artifacts["task"]:
        raise WorkError(
            ExitCode.CONTRACT,
            "task_artifact_path_mismatch",
            "The TASK contract path does not match artifacts.task.",
        )

    source_plan = _strict_keys(
        contract["source_plan"],
        location="source_plan",
        required={"canonical_sha256", "hierarchy_selection_sha256"},
    )
    source_plan_sha = _sha256(
        source_plan["canonical_sha256"], location="source_plan.canonical_sha256"
    )
    source_hierarchy_sha = _sha256(
        source_plan["hierarchy_selection_sha256"],
        location="source_plan.hierarchy_selection_sha256",
    )
    if _source_plan_raw is None:
        _, plan_path = resolve_project_relative_path(project_root, artifacts["plan"], field="plan_path")
        plan_raw = read_raw(plan_path)
    else:
        plan_raw = _source_plan_raw
    plan_validation = validate_plan_contract(
        plan_raw, source="TASK source Plan", actual_plan_path=artifacts["plan"],
        project_root=project_root, user_config_root=user_config_root, skill_roots=skill_roots,
        _historical_work_sources=_historical_work_sources,
    )
    if (
        plan_validation["plan_sha256"] != source_plan_sha
        and _reviewed_source_plan_binding != (source_plan_sha, plan_validation["plan_sha256"])
    ):
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "source_plan_fingerprint_mismatch",
            "The TASK source Plan fingerprint does not match the validated Plan.",
            {
                "source": source, "task_path": normalized_task_path, "plan_path": artifacts["plan"],
                "recorded_sha256": source_plan_sha, "actual_sha256": plan_validation["plan_sha256"],
            },
        )
    if plan_validation["hierarchy_selection_sha256"] != source_hierarchy_sha:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "source_plan_hierarchy_selection_mismatch",
            "The TASK hierarchy selection fingerprint does not match the formal Plan.",
        )
    plan_contract = parse_json_contract(plan_raw, source="TASK source Plan")
    if plan_contract["requirement_id"] != requirement_id or plan_contract["artifacts"] != artifacts:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "source_plan_identity_mismatch",
            "The TASK identity or artifact paths do not match the source Plan.",
        )
    plan_ids = {
        key: {item["id"] for item in plan_contract.get(key, [])}
        for key in ("goals", "deliverables", "acceptance_criteria", "milestones")
    }
    plan_skill_selection = plan_contract["skill_selection"]
    assert isinstance(plan_skill_selection, dict)
    selected_plan_skills = {
        skill["id"]: skill
        for skill in plan_skill_selection["skills"]
        if isinstance(skill, dict)
    }

    raw_tasks = contract["tasks"]
    if not isinstance(raw_tasks, list) or not raw_tasks:
        raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "tasks must be non-empty.")
    tasks: list[dict[str, Any]] = []
    task_ids: list[str] = []
    previous_task = 0
    for index, raw_task in enumerate(raw_tasks):
        task = _strict_keys(
            raw_task,
            location=f"tasks[{index}]",
            required={"id", "title", "skill_id", "instruction_selection", "traceability", "goal", "steps", "validations"},
            optional={"dependencies", "inputs", "decisions", "files", "risks", "commands", "operations"},
        )
        task_id = _nonempty_string(task["id"], location=f"tasks[{index}].id")
        match = TASK_ID_PATTERN.fullmatch(task_id)
        if not match or int(match.group(1)) <= previous_task:
            raise WorkError(ExitCode.CONTRACT, "invalid_or_unsorted_id", "Invalid or unsorted TASK ID.")
        previous_task = int(match.group(1))
        _nonempty_string(task["title"], location=f"{task_id}.title")
        _nonempty_string(task["goal"], location=f"{task_id}.goal")
        skill_id = task["skill_id"]
        if skill_id is not None:
            skill_id = _nonempty_string(skill_id, location=f"{task_id}.skill_id")
            selected_skill = selected_plan_skills.get(skill_id)
            if selected_skill is None:
                raise WorkError(
                    ExitCode.CONTRACT,
                    "task_skill_not_selected_in_plan",
                    "A TASK skill must be selected by its source Plan.",
                    {"task_id": task_id, "skill_id": skill_id},
                )
            mode_support = selected_skill["mode_support"]
            assert isinstance(mode_support, dict)
            if mode_support["task"] == "unsupported":
                raise WorkError(
                    ExitCode.CONTRACT,
                    "task_skill_mode_unsupported",
                    "A TASK skill must support Task mode.",
                    {"task_id": task_id, "skill_id": skill_id},
                )
        task_ids.append(task_id)
        tasks.append(task)
    task_id_set = set(task_ids)
    selections = [task["instruction_selection"] for task in tasks]
    if _historical_work_sources:
        document_selection = stored_document_selection(contract["instruction_selection"], selections)
    else:
        document_selection = validate_task_document_instruction_selection(
            contract["instruction_selection"], selections, skill_root=installed_work_root(),
        )
    plan_hierarchy_selection = plan_contract["hierarchy_selection"]
    for task in tasks:
        task_selection = task["instruction_selection"]
        assert isinstance(task_selection, dict)
        validate_task_hierarchy_paths(
            task_selection["selected_paths"],
            confirmed_selection=plan_hierarchy_selection,
            skill_root=installed_work_root(),
            location=f"{task['id']}.instruction_selection.selected_paths",
        )

    dependencies: dict[str, list[str]] = {}
    for task in tasks:
        task_id = task["id"]
        direct = _string_array(
            task.get("dependencies", []),
            location=f"{task_id}.dependencies",
            allow_empty=True,
        )
        if any(dep not in task_id_set or dep == task_id for dep in direct):
            raise WorkError(ExitCode.CONTRACT, "invalid_task_dependency", "A TASK dependency is invalid.")
        dependencies[task_id] = direct
    topo_order, ancestors = resolve_task_dependencies(task_ids, dependencies)
    if spec_match.group(1) == "001":
        positions = {task_id: index for index, task_id in enumerate(task_ids)}
        if any(positions[dep] >= positions[task_id] for task_id, deps in dependencies.items() for dep in deps):
            raise WorkError(ExitCode.CONTRACT, "initial_task_order_mismatch", "Initial TASK IDs must follow dependency order.")

    shared_decisions: set[str] = set()
    if "decisions" in contract:
        raw_decisions = contract["decisions"]
        if not isinstance(raw_decisions, list) or not raw_decisions:
            raise WorkError(ExitCode.CONTRACT, "invalid_item_array", "decisions must be non-empty.")
        previous = 0
        for index, raw_decision in enumerate(raw_decisions):
            decision = _strict_keys(
                raw_decision,
                location=f"decisions[{index}]",
                required={"id", "statement", "rationale", "task_ids"},
            )
            match = re.fullmatch(r"DECISION-(\d{3})", str(decision["id"]))
            if not match or int(match.group(1)) <= previous:
                raise WorkError(ExitCode.CONTRACT, "invalid_or_unsorted_id", "Invalid shared decision ID.")
            previous = int(match.group(1))
            _nonempty_string(decision["statement"], location="decision.statement")
            _nonempty_string(decision["rationale"], location="decision.rationale")
            applies = _string_array(decision["task_ids"], location="decision.task_ids")
            if len(applies) < 2 or any(item not in task_id_set for item in applies):
                raise WorkError(ExitCode.CONTRACT, "invalid_shared_decision_scope", "A shared decision must apply to at least two TASKs.")
            shared_decisions.add(decision["id"])

    task_instruction_hashes: dict[str, str] = {}
    plan_coverage = {key: set() for key in plan_ids}
    task_files: dict[str, list[dict[str, str]]] = {}
    task_file_ids: dict[str, set[str]] = {}
    any_commands = False

    for task in tasks:
        task_id = task["id"]
        selection = task["instruction_selection"]
        assert isinstance(selection, dict)
        references = _string_array(selection["references"], location=f"{task_id}.references", allow_empty=True)
        if "task.general.task-records" not in references:
            raise WorkError(ExitCode.CONTRACT, "task_records_reference_required", "Formal TASKs require task.general.task-records.")
        task_instruction_hashes[task_id] = _sha256(
            selection["instructions_sha256"],
            location=f"{task_id}.instructions_sha256",
        )

        traceability = _strict_keys(
            task["traceability"],
            location=f"{task_id}.traceability",
            required={"goal_ids", "deliverable_ids", "acceptance_ids"},
            optional={"milestone_ids"},
        )
        mapping = {
            "goal_ids": ("GOAL", "goals"),
            "deliverable_ids": ("DELIVERABLE", "deliverables"),
            "acceptance_ids": ("ACCEPTANCE", "acceptance_criteria"),
            "milestone_ids": ("MILESTONE", "milestones"),
        }
        for field, (prefix, plan_key) in mapping.items():
            if field not in traceability:
                continue
            ids = _plan_id_array(
                traceability[field],
                prefix=prefix,
                valid_ids=plan_ids[plan_key],
                location=f"{task_id}.traceability.{field}",
            )
            plan_coverage[plan_key].update(ids)

        local_ids: set[str] = set()
        inputs: list[dict[str, Any]] = []
        if "inputs" in task:
            inputs = _local_items(task, "inputs", "INPUT", required={"kind", "source", "precondition"})
            for item in inputs:
                if item["kind"] not in {"task_output", "project_state", "user_provided", "external"}:
                    raise WorkError(ExitCode.CONTRACT, "invalid_input_kind", "Invalid TASK input kind.")
                source = _nonempty_string(item["source"], location=f"{task_id}.{item['id']}.source")
                _nonempty_string(item["precondition"], location=f"{task_id}.{item['id']}.precondition")
                if item["kind"] == "task_output":
                    match = QUALIFIED_FILE_PATTERN.fullmatch(source)
                    if not match or match.group(1) not in dependencies[task_id]:
                        raise WorkError(ExitCode.CONTRACT, "invalid_task_output_input", "A task_output must reference a direct dependency FILE.")
                elif item["kind"] == "project_state":
                    normalized_source, _ = resolve_project_relative_path(
                        project_root,
                        source,
                        field=f"{task_id}.{item['id']}.source",
                    )
                    if normalized_source != source:
                        raise WorkError(
                            ExitCode.CONTRACT,
                            "noncanonical_project_state_source",
                            "A project_state input source must be a normalized project-relative path.",
                            {"task_id": task_id, "input_id": item["id"]},
                        )
                local_ids.add(item["id"])
        if "decisions" in task:
            for item in _local_items(task, "decisions", "TASK-DECISION", required={"statement", "rationale"}):
                _nonempty_string(item["statement"], location=f"{task_id}.{item['id']}.statement")
                _nonempty_string(item["rationale"], location=f"{task_id}.{item['id']}.rationale")
                local_ids.add(item["id"])
        files: list[dict[str, str]] = []
        if "files" in task:
            for item in _local_items(task, "files", "FILE", required={"action"}, optional={"path", "source", "destination"}):
                action = item["action"]
                expected_fields = {
                    "create": {"id", "action", "path"},
                    "modify": {"id", "action", "path"},
                    "move": {"id", "action", "source", "destination"},
                }
                if action not in expected_fields or set(item) != expected_fields[action]:
                    raise WorkError(ExitCode.CONTRACT, "invalid_file_action", "Invalid fields for TASK file action.")
                normalized_item: dict[str, str] = {"id": item["id"], "action": action}
                for field in ("path", "source", "destination"):
                    if field in item:
                        normalized, _ = resolve_project_relative_path(
                            project_root, item[field], field=f"{task_id}.{item['id']}.{field}"
                        )
                        normalized_item[field] = normalized
                files.append(normalized_item)
                local_ids.add(item["id"])
        task_files[task_id] = files
        task_file_ids[task_id] = {item["id"] for item in files}
        if "risks" in task:
            for item in _local_items(task, "risks", "RISK", required={"condition", "impact", "mitigation"}):
                for field in ("condition", "impact", "mitigation"):
                    _nonempty_string(item[field], location=f"{task_id}.{item['id']}.{field}")
                local_ids.add(item["id"])

        commands: list[dict[str, Any]] = []
        command_ids: set[str] = set()
        if "commands" in task:
            any_commands = True
            commands = _local_items(task, "commands", "CMD", required={"mode"}, optional={"argv", "script", "execution"})
            for item in commands:
                if item["mode"] == "argv" and set(item) in ({"id", "mode", "argv"}, {"id", "mode", "argv", "execution"}):
                    _string_array(item["argv"], location=f"{task_id}.{item['id']}.argv")
                elif item["mode"] == "shell" and set(item) in ({"id", "mode", "script"}, {"id", "mode", "script", "execution"}):
                    _nonempty_string(item["script"], location=f"{task_id}.{item['id']}.script")
                else:
                    raise WorkError(ExitCode.CONTRACT, "invalid_command_mode", "Invalid command mode or fields.")
                if "execution" in item:
                    _execution(item["execution"], location=f"{task_id}.{item['id']}.execution")
                command_ids.add(item["id"])
                local_ids.add(item["id"])

        validations = _local_items(
            task,
            "validations",
            "VAL",
            required={"kind"},
            optional={"command_ids", "pass_condition", "confirmer", "criteria", "acceptance_ids"},
        )
        validation_ids: set[str] = set()
        covered_acceptances: set[str] = set()
        for item in validations:
            common = {"id", "kind"} | ({"acceptance_ids"} if "acceptance_ids" in item else set())
            if item["kind"] == "automated" and set(item) == common | {"command_ids", "pass_condition"}:
                referenced_commands = _string_array(item["command_ids"], location=f"{task_id}.{item['id']}.command_ids")
                if any(command_id not in command_ids for command_id in referenced_commands):
                    raise WorkError(ExitCode.CONTRACT, "invalid_reference", "Automated VAL references an unknown CMD.")
                _nonempty_string(item["pass_condition"], location=f"{task_id}.{item['id']}.pass_condition")
            elif item["kind"] == "manual" and set(item) == common | {"confirmer", "criteria"}:
                _nonempty_string(item["confirmer"], location=f"{task_id}.{item['id']}.confirmer")
                _nonempty_string(item["criteria"], location=f"{task_id}.{item['id']}.criteria")
            else:
                raise WorkError(ExitCode.CONTRACT, "invalid_validation_kind", "Invalid validation kind or fields.")
            if "acceptance_ids" in item:
                acceptance_ids = _plan_id_array(
                    item["acceptance_ids"],
                    prefix="ACCEPTANCE",
                    valid_ids=set(traceability["acceptance_ids"]),
                    location=f"{task_id}.{item['id']}.acceptance_ids",
                )
                covered_acceptances.update(acceptance_ids)
            validation_ids.add(item["id"])
            local_ids.add(item["id"])

        operations: list[dict[str, Any]] = []
        if "operations" in task:
            operations = _local_items(
                task,
                "operations",
                "OP",
                required={"kind", "action", "target", "validation_id"},
                optional={"command_id"},
            )
            for item in operations:
                if item["kind"] not in {"local_state", "external_state"}:
                    raise WorkError(ExitCode.CONTRACT, "invalid_operation_kind", "Invalid operation kind.")
                _nonempty_string(item["action"], location=f"{task_id}.{item['id']}.action")
                _nonempty_string(item["target"], location=f"{task_id}.{item['id']}.target")
                if item["validation_id"] not in validation_ids:
                    raise WorkError(ExitCode.CONTRACT, "invalid_reference", "OP references an unknown VAL.")
                if "command_id" in item and item["command_id"] not in command_ids:
                    raise WorkError(ExitCode.CONTRACT, "invalid_reference", "OP references an unknown CMD.")
                if item["kind"] == "external_state" and "task.general.external-operations" not in references:
                    raise WorkError(ExitCode.CONTRACT, "external_operations_reference_required", "External operations require the external-operations reference.")
                local_ids.add(item["id"])
        if any(
            path.startswith("skills/work/references/instructions/")
            for item in files
            for path in (item.get("path"), item.get("source"), item.get("destination"))
            if path
        ) and "task.general.instruction-maintenance" not in references:
            raise WorkError(
                ExitCode.CONTRACT,
                "instruction_maintenance_reference_required",
                "Instruction changes require the instruction-maintenance reference.",
            )

        steps = _local_items(task, "steps", "STEP", required={"action", "references"})
        referenced_by_steps: set[str] = set()
        allowed_step_ids = local_ids | shared_decisions
        for item in steps:
            _nonempty_string(item["action"], location=f"{task_id}.{item['id']}.action")
            references_in_step = _string_array(item["references"], location=f"{task_id}.{item['id']}.references")
            if any(reference not in allowed_step_ids for reference in references_in_step):
                raise WorkError(ExitCode.CONTRACT, "invalid_reference", "STEP references an unknown formal ID.")
            referenced_by_steps.update(references_in_step)
        required_step_refs = set(task_file_ids[task_id]) | command_ids | validation_ids | {item["id"] for item in operations}
        if not required_step_refs.issubset(referenced_by_steps):
            raise WorkError(
                ExitCode.CONTRACT,
                "unreferenced_task_item",
                "Every FILE, CMD, OP, and VAL must be referenced by a STEP.",
                {"task_id": task_id, "ids": sorted(required_step_refs - referenced_by_steps)},
            )
        if not set(traceability["acceptance_ids"]).issubset(covered_acceptances):
            raise WorkError(ExitCode.CONTRACT, "acceptance_without_validation", "Every traced Acceptance must be covered by a final VAL.")

    for key, valid_ids in plan_ids.items():
        if plan_coverage[key] != valid_ids:
            raise WorkError(
                ExitCode.CONTRACT,
                "incomplete_plan_coverage",
                "The TASK collection does not fully cover the source Plan.",
                {"type": key, "missing_ids": sorted(valid_ids - plan_coverage[key])},
            )
    if any_commands != ("execution_defaults" in contract):
        raise WorkError(ExitCode.CONTRACT, "execution_defaults_mismatch", "execution_defaults must exist exactly when commands exist.")
    if "execution_defaults" in contract:
        _execution(contract["execution_defaults"], location="execution_defaults")

    for task in tasks:
        for item in task.get("inputs", []):
            if item["kind"] == "task_output":
                match = QUALIFIED_FILE_PATTERN.fullmatch(item["source"])
                assert match is not None
                if match.group(2) not in task_file_ids.get(match.group(1), set()):
                    raise WorkError(ExitCode.CONTRACT, "invalid_task_output_input", "task_output references an unknown FILE.")

    path_owners: dict[str, set[str]] = {}
    path_aliases: dict[str, set[str]] = {}
    path_identities: dict[str, str] = {}
    identity_paths: dict[str, list[Path]] = {}
    for task_id, items in task_files.items():
        for item in items:
            for field in ("path", "source", "destination"):
                if field in item:
                    raw_path = item[field]
                    if raw_path not in path_identities:
                        _, resolved = resolve_project_relative_path(
                            project_root, raw_path, field="task_file"
                        )
                        identity = portable_path_identity(resolved)
                        path_identities[raw_path] = identity
                        identity_paths.setdefault(identity, []).append(resolved)
                    identity = path_identities[raw_path]
                    path_aliases.setdefault(identity, set()).add(raw_path)
                    path_owners.setdefault(identity, set()).add(task_id)
    for identity, owners in path_owners.items():
        owner_list = sorted(owners)
        for index, first in enumerate(owner_list):
            for second in owner_list[index + 1 :]:
                if first not in ancestors[second] and second not in ancestors[first]:
                    aliases = sorted(path_aliases[identity])
                    raise WorkError(
                        ExitCode.CONTRACT,
                        "parallel_file_conflict",
                        "TASKs touching the same path must have an explicit dependency order.",
                        {"path": aliases[0], "aliases": aliases, "tasks": [first, second]},
                    )
    if validate_file_state:
        existence = {
            identity: any(path.exists() for path in paths)
            for identity, paths in identity_paths.items()
        }
        for task_id in topo_order:
            for item in task_files[task_id]:
                if item["action"] == "create":
                    path = item["path"]
                    identity = path_identities[path]
                    if existence[identity]:
                        raise WorkError(ExitCode.CONTRACT, "file_create_target_exists", "A create target already exists.", {"path": path})
                    existence[identity] = True
                elif item["action"] == "modify":
                    path = item["path"]
                    identity = path_identities[path]
                    if not existence[identity]:
                        raise WorkError(ExitCode.CONTRACT, "file_modify_target_missing", "A modify target does not exist.", {"path": path})
                else:
                    source = item["source"]
                    destination = item["destination"]
                    source_identity = path_identities[source]
                    destination_identity = path_identities[destination]
                    if not existence[source_identity] or existence[destination_identity]:
                        raise WorkError(ExitCode.CONTRACT, "invalid_file_move_state", "A file move source or destination state is invalid.")
                    existence[source_identity] = False
                    existence[destination_identity] = True

    readiness = _strict_keys(
        contract["readiness"],
        location="readiness",
        required={"status", "spec_id"},
    )
    if readiness["status"] != "passed" or readiness["spec_id"] != spec_id:
        raise WorkError(ExitCode.CONTRACT, "invalid_readiness", "Formal TASK readiness must pass for the current spec.")
    if int(spec_match.group(1)) == 1 and "changes" in contract:
        raise WorkError(ExitCode.CONTRACT, "initial_task_has_changes", "Initial TASK must omit changes.")
    if int(spec_match.group(1)) > 1 and "changes" not in contract:
        raise WorkError(ExitCode.CONTRACT, "task_changes_required", "A revised TASK must contain changes.")
    known_ids = task_id_set | shared_decisions
    if "changes" in contract:
        validate_task_changes(contract["changes"], spec_id, known_ids)

    return {
        "schema": "work-task-validation/v1",
        "requirement_id": requirement_id,
        "spec_id": spec_id,
        "status": contract["status"],
        "source_plan_sha256": source_plan_sha,
        "hierarchy_selection_sha256": source_hierarchy_sha,
        "skill_selection_sha256": plan_validation["skill_selection_sha256"],
        "instructions_sha256": document_selection["instructions_sha256"],
        "task_instructions_sha256": task_instruction_hashes,
        "task_skill_ids": {task["id"]: task["skill_id"] for task in tasks},
        "task_count": len(tasks),
    }


def validate_task_contract(
    raw: bytes,
    *,
    source: str,
    actual_task_path: str,
    project_root: Path,
    user_config_root: str,
    validate_file_state: bool = True,
    skill_roots: list[SkillRoot] | None = None,
    _source_plan_raw: bytes | None = None,
    _historical_work_sources: bool = False,
    _reviewed_source_plan_binding: tuple[str, str] | None = None,
) -> dict[str, object]:
    contract = parse_json_contract(raw, source=source)
    result = _validate_task_contract_object(
        contract,
        source=source,
        actual_task_path=actual_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        validate_file_state=validate_file_state,
        skill_roots=skill_roots,
        _source_plan_raw=_source_plan_raw,
        _historical_work_sources=_historical_work_sources,
        _reviewed_source_plan_binding=_reviewed_source_plan_binding,
    )
    ordered = order_task_contract(contract)
    require_canonical_json_contract(
        raw,
        contract=ordered,
        source=source,
    )
    result["task_sha256"] = canonical_sha256(raw, source=source)
    return result


def prepare_task_json_contract(
    raw: bytes,
    *,
    source: str,
    actual_task_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> tuple[dict[str, object], bytes]:
    contract = parse_json_contract(raw, source=source)
    rendered = render_task_contract(contract)
    result = validate_task_contract(
        rendered,
        source=source,
        actual_task_path=actual_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    return result, rendered


def validate_task_json_contract(
    raw: bytes,
    *,
    source: str,
    actual_task_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    result, _ = prepare_task_json_contract(
        raw,
        source=source,
        actual_task_path=actual_task_path,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    return result


def validate_task_file(
    project_root: Path,
    user_config_root: str,
    raw_path: str,
    *,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    normalized, path = resolve_project_relative_path(project_root, raw_path, field="task_path")
    return validate_task_contract(
        read_raw(path),
        source=str(path),
        actual_task_path=normalized,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
