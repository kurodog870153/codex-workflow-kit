"""Execution lifecycle orchestration."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ...services.skill_catalog import SkillRoot, parse_skill_root
from ...services.specification.transaction import require_no_spec_update
from ...services.execution.dispatch import state_writer, storage_path


class ExecutionService:
    def __init__(self, capabilities):
        self.capabilities = capabilities
        self.file_operations = {
            "command-correction": capabilities.record_command_correction,
            "record-finish": capabilities.finish_record,
            "attempt-close": capabilities.close_attempt,
            "correction-create": capabilities.create_correction,
            "recover": capabilities.recover_execution,
            "recovery-prepare": capabilities.prepare_execution_recovery,
            "command-prepare": capabilities.prepare_command,
            "deviation-prepare": capabilities.prepare_execution_deviation,
        }
    READ_ONLY_OPERATIONS = {
        "preflight",
        "worktree",
        "recovery-prepare",
        "command-prepare",
        "deviation-prepare",
        "command-run",
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
        skill_roots: list[SkillRoot | str],
        raw_request: bytes | None = None,
        source: str | None = None,
        confirmed_inputs: list[str] | None = None,
        base_record_id: str | None = None,
        approved_sha256: str | None = None,
        authorization_evidence: str | None = None,
    ) -> dict[str, object]:
        parsed_skill_roots = [
            parse_skill_root(root) if isinstance(root, str) else root
            for root in skill_roots
        ]
        require_no_spec_update(project_root, raw_execution_dir)
        common = {
            "project_root": project_root,
            "user_config_root": user_config_root,
            "raw_task_path": raw_task_path,
            "raw_execution_dir": raw_execution_dir,
            "task_id": task_id,
            "skill_roots": parsed_skill_roots,
        }
        if operation not in self.READ_ONLY_OPERATIONS:
            task_path = storage_path(project_root, raw_task_path)
            if task_path.is_file():
                self.capabilities.load_task_execution_context(
                    project_root,
                    user_config_root,
                    raw_task_path,
                    task_id,
                    skill_roots=parsed_skill_roots,
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
            return self.capabilities.begin_record(
                **common,
                base_record_id=base_record_id,
                authorization_evidence=authorization_evidence,
            )
        if operation == "command-run":
            return self.capabilities.run_command(
                raw_request,
                source=source,
                approved_sha256=approved_sha256,
                **common,
            )
        if operation == "deviation-record":
            return self.capabilities.record_execution_deviation(
                raw_request,
                source=source,
                approved_sha256=approved_sha256,
                **common,
            )
        if operation in self.file_operations:
            return self.file_operations[operation](
                raw_request,
                source=source,
                **common,
            )
        inspection_common = {
            **common,
            "confirmed_inputs": confirmed_inputs or [],
        }
        if operation == "preflight":
            return self.capabilities.execute_preflight(**inspection_common)
        if operation == "worktree":
            return self.capabilities.inspect_execute_worktree(**inspection_common)
        lifecycle = (
            self.capabilities.start_attempt
            if operation == "attempt-start"
            else self.capabilities.recover_attempt_start
        )
        return lifecycle(raw_request, source=source, **inspection_common)
