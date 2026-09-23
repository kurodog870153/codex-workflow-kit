from __future__ import annotations

import base64
from datetime import datetime, timezone
from pathlib import Path
import secrets
from typing import Any

from ...technical.foundation.fingerprint import canonical_json_sha256, raw_sha256
from ...technical.infrastructure.file_io import read_raw
from ...technical.infrastructure.json_contract import parse_json_contract, render_json_contract, require_canonical_json_contract
from ...technical.infrastructure.specification_storage import (
    publish_snapshot,
    replace_journal,
    storage_path,
    write_exclusive,
)
from ...models.common.errors import ExitCode, WorkError
from ...models.common.identifiers import IdentifierPolicy
from ...models.specification.transaction import SpecTransactionContract
from ...technical.infrastructure.path_safety import resolve_project_relative_path


def transaction_directory(
    project_root: Path,
    *,
    requirement_id: str | None,
    workflow_id: str,
    transaction_id: str,
) -> tuple[str, Path]:
    owner = "pending" if requirement_id is None else IdentifierPolicy.requirement_id(requirement_id)
    if owner == "pending" and requirement_id is not None:
        raise WorkError(ExitCode.CONTRACT, "reserved_transaction_owner", "The pending transaction owner is reserved for work without a requirement ID.", {"requirement_id": requirement_id})
    workflow = IdentifierPolicy.workflow_id(workflow_id)
    transaction = IdentifierPolicy.transaction_id(transaction_id)
    return resolve_project_relative_path(project_root, f"outputs/work/transactions/{owner}/{workflow}/{transaction}", field="transaction_directory")


def _new_workspace_id() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + secrets.token_hex(4)


def create_transaction_workspace(project_root: Path, *, requirement_id: str | None, workflow_id: str) -> dict[str, str | None]:
    """Allocate and exclusively create one transport workspace; never reuse evidence."""
    transaction_id = _new_workspace_id()
    relative, directory = transaction_directory(project_root, requirement_id=requirement_id,
                                                workflow_id=workflow_id, transaction_id=transaction_id)
    if directory.exists() or directory.is_symlink():
        raise WorkError(ExitCode.WORKFLOW_STATE, "transaction_workspace_exists", "A generated transaction workspace already exists.", {"path": relative})
    try:
        directory.mkdir(parents=True, exist_ok=False)
    except FileExistsError as error:
        raise WorkError(ExitCode.WORKFLOW_STATE, "transaction_workspace_exists", "A generated transaction workspace already exists.", {"path": relative}) from error
    except OSError as error:
        raise WorkError(ExitCode.IO_FAILURE, "transaction_workspace_create_failed", "The transaction workspace could not be created.", {"path": relative}) from error
    return {"schema": "work-transaction-workspace/v1", "requirement_id": requirement_id,
            "workflow_id": workflow_id, "transaction_id": transaction_id, "path": relative}


def encode_snapshot(raw: bytes) -> dict[str, str]:
    return {
        "raw_sha256": raw_sha256(raw),
        "base64": base64.b64encode(raw).decode("ascii"),
    }


def transaction_approval_sha256(files: list[dict[str, Any]], metadata: dict[str, Any]) -> str:
    return canonical_json_sha256({"files": files, "metadata": metadata})


def derived_transaction_id(kind: str, approval_sha256: str) -> str:
    if kind not in {"UPDATE", "MIGRATION", "RECONCILIATION"} or len(approval_sha256) != 64 or any(
        character not in "0123456789abcdef" for character in approval_sha256
    ):
        raise WorkError(ExitCode.CONTRACT, "spec_transaction_identity", "A validated transaction kind and approval fingerprint are required.")
    return f"SPEC-{kind}-{approval_sha256[:12].upper()}"


def canonicalize_spec_transaction(contract: object) -> dict[str, Any]:
    try:
        model = SpecTransactionContract.model_validate(contract)
    except Exception as error:
        from pydantic import ValidationError

        if isinstance(error, ValidationError):
            raise SpecTransactionContract._work_error(error) from error
        raise
    return model.to_canonical_dict()


def render_spec_transaction(contract: object) -> bytes:
    return render_json_contract(canonicalize_spec_transaction(contract))


def validate_spec_transaction(raw: bytes, *, source: str) -> dict[str, Any]:
    contract = canonicalize_spec_transaction(parse_json_contract(raw, source=source))
    require_canonical_json_contract(raw, contract=contract, source=source)
    return contract

def _error(code: str, message: str) -> WorkError:
    return WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message)


def completion_marker_matches(record_raw: bytes, marker_raw: bytes) -> bool:
    """Match exact raw-byte SHA-256 evidence, including one trailing LF."""
    return marker_raw == raw_sha256(record_raw).encode("ascii") + b"\n"


def transaction_completion_state(record_raw: bytes, marker_raw: bytes | None) -> str:
    """Classify storage evidence without treating corrupt markers as complete."""
    if marker_raw is None:
        return "incomplete"
    return "completed" if completion_marker_matches(record_raw, marker_raw) else "corrupt"


def require_no_spec_update(
    root: Path,
    execution_dir: str,
    *,
    ignored_record: str | None = None,
) -> None:
    directory = storage_path(root, execution_dir)
    records = sorted([
        *directory.glob(".work-spec-update-*.json"),
        *directory.glob(".work-task-repair-*.json"),
        *directory.glob(".work-spec-migration-*.json"),
        *directory.glob(".work-source-refresh-*.json"),
    ])
    for record in records:
        if record.relative_to(root).as_posix() == ignored_record:
            continue
        record = storage_path(root, record.relative_to(root).as_posix())
        done = storage_path(root, record.relative_to(root).as_posix() + ".done")
        record_raw = record.read_bytes()
        if not done.is_file() or not completion_marker_matches(record_raw, done.read_bytes()):
            raise WorkError(
                ExitCode.LOCK_CONFLICT,
                "spec_update_pending",
                "An incomplete specification update requires separately authorized recovery.",
                {"recovery_required": True, "record": record.relative_to(root).as_posix()},
            )


def write_journal(path: Path, contract: object) -> bytes:
    raw = render_spec_transaction(contract)
    write_exclusive(path, raw)
    return raw


def _snapshot_bytes(row: dict[str, object], side: str) -> bytes | None:
    snapshot = row.get(side)
    return None if snapshot is None else base64.b64decode(snapshot["base64"])  # type: ignore[index]


def publish_journal(
    root: Path,
    journal_relative: str,
    marker_relative: str,
) -> dict[str, object]:
    journal_path = storage_path(root, journal_relative)
    marker_path = storage_path(root, marker_relative)
    raw = read_raw(journal_path)
    contract = validate_spec_transaction(raw, source=journal_relative)
    if marker_path.exists():
        marker_raw = read_raw(marker_path)
        if contract["state"] == "published" and completion_marker_matches(raw, marker_raw):
            return {"status": "already_published", "published_count": contract["published_count"]}
        raise _error(
            "spec_transaction_marker_conflict",
            "The completion marker does not match the final journal.",
        )
    files = contract["files"]
    for index in range(contract["published_count"], len(files)):
        row = files[index]
        target = storage_path(root, row["path"])
        publish_snapshot(
            target,
            _snapshot_bytes(row, "before"),
            _snapshot_bytes(row, "after"),
            Path(str(journal_path) + f".{index}.tmp"),
        )
        contract["published_count"] = index + 1
        contract["state"] = "published" if index + 1 == len(files) else "publishing"
        raw = render_spec_transaction(contract)
        replace_journal(journal_path, raw)
    marker_raw = raw_sha256(raw).encode("ascii") + b"\n"
    write_exclusive(marker_path, marker_raw)
    return {"status": "published", "published_count": contract["published_count"]}


__all__ = [
    "canonicalize_spec_transaction",
    "derived_transaction_id",
    "completion_marker_matches",
    "encode_snapshot",
    "publish_journal",
    "render_spec_transaction",
    "require_no_spec_update",
    "transaction_approval_sha256",
    "transaction_completion_state",
    "validate_spec_transaction",
    "write_journal",
]
