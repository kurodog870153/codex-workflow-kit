from __future__ import annotations

from pathlib import Path
from typing import Any

from ..execution.attempt_close import close_attempt
from ..execution.attempt_start import recover_attempt_start, start_attempt
from ..execution.command_correction import record_command_correction
from ..execution.command_run import prepare_command, run_command
from ..execution.correction import create_correction
from ..execution.preflight import execute_preflight
from ..execution.record_begin import begin_record
from ..execution.record_finish import finish_record
from ..execution.recovery import recover_execution
from ..execution.recovery_prepare import prepare_execution_recovery
from ..execution.worktree import inspect_execute_worktree
from ..foundation.spec_update import (
    require_no_spec_update,
    storage_path,
)
from ..infrastructure.writer_lock import state_writer
from .skill_catalog import SkillRoot
from .task_collection import load_task_execution_context


class ExecutionService:
    READ_ONLY_OPERATIONS = {
        "preflight",
        "worktree",
        "recovery-prepare",
        "command-prepare",
        "command-run",
    }

    FILE_OPERATIONS = {
        "command-correction": record_command_correction,
        "record-finish": finish_record,
        "attempt-close": close_attempt,
        "correction-create": create_correction,
        "recover": recover_execution,
        "recovery-prepare": prepare_execution_recovery,
        "command-prepare": prepare_command,
    }

    def execute(
        self,
        operation: str,
        *,
        project_root: Path,
        user_config_root: str,
        raw_task_path: str,
        raw_execution_dir: str,
        task_id: str,
        skill_roots: list[SkillRoot],
        raw_request: bytes | None = None,
        source: str | None = None,
        confirmed_inputs: list[str] | None = None,
        base_record_id: str | None = None,
        approved_sha256: str | None = None,
        authorization_evidence: str | None = None,
    ) -> dict[str, object]:
        require_no_spec_update(project_root, raw_execution_dir)
        common = {
            "project_root": project_root,
            "user_config_root": user_config_root,
            "raw_task_path": raw_task_path,
            "raw_execution_dir": raw_execution_dir,
            "task_id": task_id,
            "skill_roots": skill_roots,
        }
        if operation not in self.READ_ONLY_OPERATIONS:
            task_path = storage_path(project_root, raw_task_path)
            if task_path.is_file():
                load_task_execution_context(
                    project_root,
                    user_config_root,
                    raw_task_path,
                    task_id,
                    skill_roots=skill_roots,
                )
            directory = storage_path(project_root, raw_execution_dir)
            if directory.is_dir():
                with state_writer(project_root, raw_execution_dir):
                    return self._execute_operation(
                        operation,
                        common=common,
                        raw_request=raw_request,
                        source=source,
                        confirmed_inputs=confirmed_inputs,
                        base_record_id=base_record_id,
                        approved_sha256=approved_sha256,
                        authorization_evidence=authorization_evidence,
                    )
        return self._execute_operation(
            operation,
            common=common,
            raw_request=raw_request,
            source=source,
            confirmed_inputs=confirmed_inputs,
            base_record_id=base_record_id,
            approved_sha256=approved_sha256,
            authorization_evidence=authorization_evidence,
        )

    def _execute_operation(
        self,
        operation: str,
        *,
        common: dict[str, Any],
        raw_request: bytes | None,
        source: str | None,
        confirmed_inputs: list[str] | None,
        base_record_id: str | None,
        approved_sha256: str | None,
        authorization_evidence: str | None,
    ) -> dict[str, object]:
        if operation == "record-begin":
            return begin_record(**common, base_record_id=base_record_id)
        if operation == "command-run":
            return run_command(
                raw_request,
                source=source,
                approved_sha256=approved_sha256,
                authorization_evidence=authorization_evidence,
                **common,
            )
        if operation in self.FILE_OPERATIONS:
            return self.FILE_OPERATIONS[operation](
                raw_request,
                source=source,
                **common,
            )
        inspection_common = {
            **common,
            "confirmed_inputs": confirmed_inputs or [],
        }
        if operation == "preflight":
            return execute_preflight(**inspection_common)
        if operation == "worktree":
            return inspect_execute_worktree(**inspection_common)
        lifecycle = start_attempt if operation == "attempt-start" else recover_attempt_start
        return lifecycle(raw_request, source=source, **inspection_common)


execution_service = ExecutionService()
