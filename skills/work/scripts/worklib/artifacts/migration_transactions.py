"""Read-only identification of completed migration transactions."""
import hashlib
import json
import re

from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.spec_update import storage_path


def completed_migrations(root, execution):
    directory = storage_path(root, execution)
    records = []
    for marker in directory.glob(".work-spec-update-*.json.done"):
        relative = marker.relative_to(root).as_posix()
        storage_path(root, relative)
        if not storage_path(root, relative[:-5]).is_file():
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "migration_orphan_marker",
                            "A completion marker has no journal.", {"path": relative})
    for path in sorted(directory.glob(".work-spec-update-*.json")):
        relative = path.relative_to(root).as_posix()
        raw = read_raw(storage_path(root, relative))
        marker = storage_path(root, relative + ".done")
        if not marker.is_file() or read_raw(marker) != hashlib.sha256(raw).hexdigest().encode("ascii") + b"\n":
            continue
        value = parse_json_contract(raw, source=relative)
        request = value.get("request")
        record_id = value.get("record_id")
        if (value.get("schema") != "work-spec-update-record/v1"
                or not isinstance(request, dict) or not isinstance(record_id, str)
                or not re.fullmatch(r"SPEC-UPDATE-\d{3}", record_id)
                or path.name != ".work-spec-update-" + record_id + ".json"
                or raw != (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")):
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "migration_record_invalid",
                            "The completed transaction has an invalid identity or encoding.", {"path": relative})
        if request.get("schema") == "work-spec-migration-request/v1":
            for field in ("before", "after"):
                snapshot = value.get(field)
                if (not isinstance(snapshot, dict) or set(snapshot) != {"plan", "task", "index"}
                        or any(not isinstance(item, str) for item in snapshot.values())):
                    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, "migration_record_invalid",
                                    "The completed migration has invalid snapshots.", {"path": relative})
            records.append(value)
    return records


def require_new_migration(root, execution, request):
    task = request.get("task")
    spec = task.get("spec_id") if isinstance(task, dict) else None
    target = "SPEC-UPDATE-" + spec[-3:] if isinstance(spec, str) and re.fullmatch(r"TASK-SPEC-\d{3}", spec) else None
    for record in completed_migrations(root, execution):
        code = None
        if record["record_id"] == target:
            code = "migration_already_completed" if record["request"] == request else "migration_record_conflict"
        elif record["request"].get("expected") == request.get("expected"):
            code = "migration_source_already_used"
        if code:
            raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code,
                            "This migration identity or source snapshot was already used; inspect the completed record.",
                            {"record_id": record["record_id"], "next_step": "migrate-verify"})


def verification_selection(root, execution, snapshots):
    records = completed_migrations(root, execution)
    matches = [record["record_id"] for record in records
               if all(snapshots.get(key) == record["after"][key].encode("utf-8") for key in ("plan", "task", "index"))]
    return {
        "latest_completed_record_id": max((r["record_id"] for r in records), default=None),
        "applicable_record_id": matches[0] if len(matches) == 1 else None,
        "matching_record_ids": matches,
        "selection_basis": "exact_plan_task_index_bytes",
    }
