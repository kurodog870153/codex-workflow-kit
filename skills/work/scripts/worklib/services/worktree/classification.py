from __future__ import annotations


def _within_directory(path: str, directory: str) -> bool:
    return path == directory or path.startswith(f"{directory}/")


def classify_changes(
    records: list[dict[str, str]],
    *,
    execution_dir: str,
    target_task_id: str,
    target_paths: set[str],
    dependency_paths: dict[str, set[str]],
) -> tuple[list[dict[str, object]], int]:
    changes: list[dict[str, object]] = []
    excluded_count = 0
    for record in records:
        record_paths = [record["path"]]
        if "original_path" in record:
            record_paths.append(record["original_path"])
        if all(_within_directory(path, execution_dir) for path in record_paths):
            excluded_count += 1
            continue
        matching_dependencies = [
            task_id
            for task_id, paths in dependency_paths.items()
            if any(path in paths for path in record_paths)
        ]
        if any(path in target_paths for path in record_paths):
            classification = "target_task"
            matched_task_ids = [target_task_id]
        elif matching_dependencies:
            classification = "completed_dependency"
            matched_task_ids = matching_dependencies
        else:
            classification = "unrelated"
            matched_task_ids = []
        change: dict[str, object] = {
            "index_status": record["index_status"],
            "worktree_status": record["worktree_status"],
            "path": record["path"],
        }
        if "original_path" in record:
            change["original_path"] = record["original_path"]
        change["path_classification"] = classification
        if matched_task_ids:
            change["matched_task_ids"] = matched_task_ids
        changes.append(change)
    return changes, excluded_count
