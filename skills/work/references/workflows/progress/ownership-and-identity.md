# Ownership and identity

1. Task owns discussion, decision status and resumption. It returns complete checkpoint content through the parent, which invokes the private progress saver. Task skill subagents return evidence through the coordinator; they do not spawn a saver.
2. The parent supplies the routed progress-saver prompt and one ephemeral `WORK_PROGRESS_SAVE` envelope with `skill=$work`, `mode=task`, resolved roots, complete fixed `task_source`, content, expected revision and continuation point. Inherit model and reasoning settings. If delegation is unavailable, follow the same private prompt directly.
3. Retain the explicit safe requirement ID from the fixed Source. The saved `context.planning_source` contains its complete immutable Snapshot and confirmed Task-owned hierarchy, skill selections, artifact paths and main acceptance criteria. A checkpoint must not change this Source binding.
4. Store discussion only at `outputs/work/progress/<requirement-id>/task/progress.json` with immutable `history/<revision>/progress.json` copies. Discussion has no executable status and grants no formal approval, selection authority or execution permission.
