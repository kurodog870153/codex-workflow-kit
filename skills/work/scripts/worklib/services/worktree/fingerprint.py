from __future__ import annotations

import json

from ...technical.infrastructure.text_codec import canonical_sha256
from ...models.execution.inspection import ExecuteWorktreeSnapshotContract


def _within_directory(path: str, directory: str) -> bool:
    return path == directory or path.startswith(f"{directory}/")


def worktree_snapshot_sha256(
    records: list[dict[str, str]], *, execution_dir: str
) -> str:
    included: list[dict[str, str]] = []
    for record in records:
        record_paths = [record["path"]]
        if "original_path" in record:
            record_paths.append(record["original_path"])
        if all(_within_directory(path, execution_dir) for path in record_paths):
            continue
        included.append(record)
    snapshot = ExecuteWorktreeSnapshotContract.model_validate(
        {"schema": "work-execute-worktree-snapshot/v1", "records": included}
    ).to_canonical_dict()
    payload = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return canonical_sha256(payload, source="Git worktree snapshot")
