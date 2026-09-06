from __future__ import annotations

from ..foundation.errors import ExitCode, WorkError


def resolve_task_dependencies(
    task_ids: list[str], dependencies: dict[str, list[str]]
) -> tuple[list[str], dict[str, set[str]]]:
    visiting: set[str] = set()
    visited: set[str] = set()
    ancestors: dict[str, set[str]] = {}

    def visit(task_id: str) -> set[str]:
        if task_id in visiting:
            raise WorkError(
                ExitCode.CONTRACT,
                "cyclic_task_dependency",
                "TASK dependencies must not contain a cycle.",
                {"task_id": task_id},
            )
        if task_id in visited:
            return ancestors[task_id]
        visiting.add(task_id)
        result: set[str] = set()
        for dependency in dependencies[task_id]:
            result.add(dependency)
            result.update(visit(dependency))
        visiting.remove(task_id)
        visited.add(task_id)
        ancestors[task_id] = result
        return result

    for task_id in task_ids:
        visit(task_id)
    for task_id, direct in dependencies.items():
        for dependency in direct:
            if any(
                dependency in ancestors[other]
                for other in direct
                if other != dependency
            ):
                raise WorkError(
                    ExitCode.CONTRACT,
                    "indirect_task_dependency",
                    "Only direct TASK dependencies may be listed.",
                    {"task_id": task_id, "dependency": dependency},
                )
    order: list[str] = []
    pending = set(task_ids)
    while pending:
        ready = [
            task_id
            for task_id in task_ids
            if task_id in pending and all(dep in order for dep in dependencies[task_id])
        ]
        if not ready:
            raise WorkError(
                ExitCode.CONTRACT,
                "cyclic_task_dependency",
                "TASK dependency cycle.",
            )
        order.extend(ready)
        pending.difference_update(ready)
    return order, ancestors
