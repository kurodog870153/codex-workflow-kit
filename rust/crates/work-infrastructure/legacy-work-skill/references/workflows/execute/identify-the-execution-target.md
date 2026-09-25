<!-- work-compatibility-revision: 1 -->
# Identify the execution target


1. Loading Execute instructions alone does not execute or authorize a TASK.
2. Require the user to identify one formal TASK collection entry point and one `TASK-*` from that specification before eligibility checks. Do not select either on the user's behalf. The loader validates the collection and exposes the selected dependency closure required by the operation.
3. Use the default TASK collection entry path `outputs/work/tasks/<requirement-id>/index.json` unless the user explicitly supplies and confirms a permitted project-relative collection entry path. The entry filename must be `index.json`; single-file `task.json` artifacts are unsupported.
4. When Plan, TASK, or execution uses a non-default path, require the same requirement ID and all three confirmed project-relative paths. Never infer one path from another.
5. After the target is complete, perform only authorized read-only eligibility checks. Obtain every required authorization before changing state, modifying files, running side-effecting commands, or performing external operations.
