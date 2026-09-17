from __future__ import annotations

import base64
import binascii
import re
from typing import Any

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_json_sha256, raw_sha256
from ..foundation.markdown import parse_json_contract, render_json_contract, require_canonical_json_contract
from .validation import sha256, strict_keys


TRANSACTION_PATTERN = re.compile(r"^[A-Z][A-Z0-9-]{2,63}$")


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, details)


def _bytes(value: object, *, location: str) -> bytes:
    if not isinstance(value, str):
        _fail("spec_transaction_invalid_bytes", "Transaction bytes must use canonical base64.", location=location)
    try:
        raw = base64.b64decode(value, validate=True)
    except (ValueError, binascii.Error) as error:
        _fail("spec_transaction_invalid_bytes", "Transaction bytes must use canonical base64.", location=location)
    if base64.b64encode(raw).decode("ascii") != value:
        _fail("spec_transaction_invalid_bytes", "Transaction bytes must use canonical base64.", location=location)
    return raw


def encode_snapshot(raw: bytes) -> dict[str, str]:
    return {
        "raw_sha256": raw_sha256(raw),
        "base64": base64.b64encode(raw).decode("ascii"),
    }


def transaction_approval_sha256(files: list[dict[str, Any]], metadata: dict[str, Any]) -> str:
    return canonical_json_sha256({"files": files, "metadata": metadata})


def canonicalize_spec_transaction(contract: object) -> dict[str, Any]:
    value = strict_keys(
        contract,
        location="spec_transaction",
        required={"schema", "transaction_id", "approval_sha256", "state", "published_count", "metadata", "files"},
    )
    if value["schema"] != "work-spec-transaction/v2":
        _fail("spec_transaction_invalid_schema", "The transaction schema is invalid.")
    transaction_id = value["transaction_id"]
    if not isinstance(transaction_id, str) or not TRANSACTION_PATTERN.fullmatch(transaction_id):
        _fail("spec_transaction_invalid_id", "The transaction ID is invalid.")
    if value["state"] not in {"prepared", "publishing", "published"}:
        _fail("spec_transaction_invalid_state", "The transaction state is invalid.")
    files = value["files"]
    if not isinstance(files, list) or not files:
        _fail("spec_transaction_invalid_files", "The transaction requires files.")
    normalized: list[dict[str, Any]] = []
    for index, item in enumerate(files):
        row = strict_keys(
            item,
            location=f"files[{index}]",
            required={"phase", "path", "operation"},
            optional={"before", "after"},
        )
        phase = row["phase"]
        if not isinstance(phase, int) or isinstance(phase, bool) or phase < 0:
            _fail("spec_transaction_invalid_phase", "Transaction publication phases must be non-negative integers.")
        path = row["path"]
        if (not isinstance(path, str) or not path or "\\" in path or path.startswith("/")
                or any(part in {"", ".", ".."} for part in path.split("/"))):
            _fail("spec_transaction_invalid_path", "Transaction paths must be safe project-relative POSIX paths.", path=path)
        operation = row["operation"]
        expected = {
            "add": {"phase", "path", "operation", "after"},
            "replace": {"phase", "path", "operation", "before", "after"},
            "remove": {"phase", "path", "operation", "before"},
        }
        if operation not in expected or set(row) != expected[operation]:
            _fail("spec_transaction_invalid_file", "Transaction file fields do not match the operation.", path=path)
        result = {"phase": phase, "path": path, "operation": operation}
        for side in ("before", "after"):
            if side not in row:
                continue
            snapshot = strict_keys(row[side], location=f"files[{index}].{side}", required={"raw_sha256", "base64"})
            sha256(snapshot["raw_sha256"], location=f"files[{index}].{side}.raw_sha256")
            raw = _bytes(snapshot["base64"], location=f"files[{index}].{side}.base64")
            if raw_sha256(raw) != snapshot["raw_sha256"]:
                _fail("spec_transaction_snapshot_mismatch", "Transaction bytes do not match their fingerprint.", path=path, side=side)
            result[side] = dict(snapshot)
        normalized.append(result)
    paths = [item["path"] for item in normalized]
    order = [(item["phase"], item["path"]) for item in normalized]
    if order != sorted(order) or len(paths) != len(set(paths)):
        _fail("spec_transaction_invalid_order", "Transaction files must have unique paths in phase and lexical order.")
    metadata = strict_keys(
        value["metadata"], location="metadata",
        required={"request", "artifacts", "affected_task_ids", "history_sha256", "source_sha256", "candidate_sha256"},
    )
    if not isinstance(metadata["request"], dict) or not isinstance(metadata["artifacts"], dict):
        _fail("spec_transaction_invalid_metadata", "Transaction request and artifacts metadata must be objects.")
    if (not isinstance(metadata["affected_task_ids"], list)
            or any(not isinstance(item, str) or not item for item in metadata["affected_task_ids"])):
        _fail("spec_transaction_invalid_metadata", "Affected TASK IDs must be a string array.")
    if (not isinstance(metadata["history_sha256"], dict)
            or any(not isinstance(key, str) or not isinstance(item, str) for key, item in metadata["history_sha256"].items())):
        _fail("spec_transaction_invalid_metadata", "History fingerprints must be an object of strings.")
    for field in ("source_sha256", "candidate_sha256"):
        if (not isinstance(metadata[field], dict)
                or any(not isinstance(key, str) or not isinstance(item, str) for key, item in metadata[field].items())):
            _fail("spec_transaction_invalid_metadata", "Source and candidate fingerprints must be objects of strings.")
    sha256(value["approval_sha256"], location="approval_sha256")
    if value["approval_sha256"] != transaction_approval_sha256(normalized, metadata):
        _fail("spec_transaction_approval_mismatch", "The approval fingerprint does not match the file set.")
    count = value["published_count"]
    if not isinstance(count, int) or isinstance(count, bool) or not 0 <= count <= len(normalized):
        _fail("spec_transaction_invalid_progress", "The published file count is invalid.")
    expected_state = "prepared" if count == 0 else ("published" if count == len(normalized) else "publishing")
    if value["state"] != expected_state:
        _fail("spec_transaction_invalid_progress", "The transaction state does not match its progress.")
    return {
        "schema": "work-spec-transaction/v2",
        "transaction_id": transaction_id,
        "approval_sha256": value["approval_sha256"],
        "state": value["state"],
        "published_count": count,
        "metadata": dict(metadata),
        "files": normalized,
    }


def render_spec_transaction(contract: object) -> bytes:
    return render_json_contract(canonicalize_spec_transaction(contract))


def validate_spec_transaction(raw: bytes, *, source: str) -> dict[str, Any]:
    contract = canonicalize_spec_transaction(parse_json_contract(raw, source=source))
    require_canonical_json_contract(raw, contract=contract, source=source)
    return contract
