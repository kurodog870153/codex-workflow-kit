from __future__ import annotations

import copy
from typing import Any

from ...models.common.errors import ExitCode, WorkError


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.CONTRACT, code, message, details or None)


def overall_operation_result(records: list[dict[str, Any]]) -> dict[str, Any] | None:
    operations = [
        item for item in records
        if item["kind"] == "operation" and item.get("status") != "skipped"
    ]
    if not operations:
        return None
    effective = [item["id"] for item in operations if item["outcome"] == "success"]
    not_effective = [item["id"] for item in operations if item["outcome"] == "failure"]
    unknown = [item["id"] for item in operations if item["outcome"] == "unknown"]
    status = "uncertain_result" if unknown else "partial_success" if effective and not_effective else "failure" if not_effective else "complete_success"
    result: dict[str, Any] = {"status": status}
    for field, values in (("effective", effective), ("not_effective", not_effective), ("unknown", unknown)):
        if values:
            result[field] = values
    return result


def finish_attempt_candidate(
    attempt: dict[str, Any], request: dict[str, Any], *, expected_record_id: str,
    expected_kind: str, command_correction: dict[str, Any] | None = None,
) -> dict[str, Any]:
    record = copy.deepcopy(request["record"])
    repeated = sorted(set(record) & {"id", "kind", "correction"})
    if repeated:
        _fail("record_finish_machine_fields", "Record identity and command correction are derived from the active lock.", fields=repeated)
    record["id"] = expected_record_id
    record["kind"] = expected_kind
    if command_correction is not None:
        if expected_kind != "command":
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "record_finish_invalid_command_correction_lock", "Only a reserved command can carry command correction data.", None)
        record["correction"] = copy.deepcopy(command_correction)
    candidate = copy.deepcopy(attempt)
    candidate["records"].append(record)
    if "modified_files" in request:
        current_files = list(candidate.get("modified_files", []))
        for path in request["modified_files"]:
            if path not in current_files:
                current_files.append(path)
        candidate["modified_files"] = current_files
    overall = overall_operation_result(candidate["records"])
    if overall is None:
        candidate.pop("overall_result", None)
    else:
        candidate["overall_result"] = overall
    return candidate
