from __future__ import annotations

import os
from pathlib import Path

from ..contracts.plan import prepare_plan_json_contract, validate_plan_file
from ..foundation.errors import ExitCode, WorkError
from ..foundation.paths import resolve_project_relative_path
from ..skills.catalog import SkillRoot


def create_plan_file(
    raw: bytes,
    *,
    source: str,
    raw_plan_path: str,
    project_root: Path,
    user_config_root: str,
    skill_roots: list[SkillRoot] | None = None,
) -> dict[str, object]:
    normalized, path = resolve_project_relative_path(
        project_root,
        raw_plan_path,
        field="plan_path",
    )
    validation, rendered = prepare_plan_json_contract(
        raw,
        source=source,
        actual_plan_path=normalized,
        project_root=project_root,
        user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if path.exists():
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        )
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as output:
            output.write(rendered)
            output.flush()
            os.fsync(output.fileno())
    except FileExistsError as error:
        raise WorkError(
            ExitCode.WORKFLOW_STATE,
            "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            {"path": normalized},
        ) from error
    except OSError as error:
        raise WorkError(
            ExitCode.IO_FAILURE,
            "plan_create_failed",
            "The canonical Plan could not be created.",
            {"path": normalized},
        ) from error

    stored = validate_plan_file(
        project_root, user_config_root, normalized, skill_roots=skill_roots
    )
    if stored != validation:
        raise WorkError(
            ExitCode.ARTIFACT_INTEGRITY,
            "plan_post_write_mismatch",
            "The stored Plan does not match the validated canonical Plan.",
            {"path": normalized},
        )
    result = dict(stored)
    result["schema"] = "work-plan-create/v1"
    result["path"] = normalized
    return result
