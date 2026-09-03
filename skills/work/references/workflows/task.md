# Task Workflow

Use this workflow only after the source Plan and its confirmed skill selection have been validated without drift.

## Coordinate confirmed skills

1. Task skill discovery is forbidden. Use only skills in the source Plan `skill_selection`.
2. Split work into minimum independently verifiable TASK outcomes before delegation.
3. Bind each TASK to one `skill_id`. Use `null` only for base-only work that needs no external skill.
4. Give each TASK only an applicable subset of the source Plan hierarchy selection. An empty subset loads `general`; a non-empty path must be a confirmed leaf or one of its ancestors and must exist in both Task and Execute catalogs.
5. Skip Plan-only skills when producing executable TASKs. A required skill with Task mode `unsupported` must return to Plan for a new decision.
6. Create one isolated ephemeral subagent for each executable skill, sequentially in TASK order. Each receives one skill and one TASK boundary; it cannot call other skills or delegate.
7. Merge subagent outputs into one candidate TASK contract. Resolve conflicts through user decisions; never silently combine incompatible instructions.

## Complete the request

1. Perform the user's Task request under the loaded instructions.
2. Loading Task instructions alone does not create or modify a TASK document. Create or modify artifacts only when the request and applicable authorization permit it.
3. Do not execute a TASK merely because Task instructions or a TASK document were loaded. Execution requires the Execute workflow and its applicable authorization.
4. Keep an unapproved TASK candidate in the conversation. Do not create a draft file.

## Use the deterministic TASK contract

1. Build the complete proposed `work-task/v1` JSON object in the conversation.
2. Use only English keys, enums, IDs, statuses, paths, references, and hashes. Semantic strings may use the user's language.
3. Before requesting formal approval, pipe the complete object to `task validate --stdin --task-path <task-path>` with every source Plan `--skill-root`.
4. Require a successful result containing the canonical TASK and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
5. After approval, run the identical `task create` request with the same skill roots. Do not assemble or write TASK or index Markdown manually.
6. `task create` exclusively creates the canonical TASK and initial execution index. Treat an existing target or partial failure as a hard stop.
7. Use `<python-command> <skill-root>/scripts/work.py --project-root <project-root> task recover-create --stdin --plan-path <plan-path> --task-path <task-path> --execution-dir <execution-dir>` only after the user explicitly authorizes recovery and only with the identical approved JSON and three paths.
8. Neither validation, creation, nor recovery executes CMD or OP or creates an Attempt, execution lock, instruction audit, or specification-update transaction.

## Use deterministic handoffs

1. Before accepting `plan_to_task` or `execute_to_task`, pipe its pure JSON to `<python-command> <skill-root>/scripts/work.py --project-root <project-root> handoff validate --stdin` and require `work-handoff-validation/v1` with `status: valid`.
2. After TASK formalization, use `<python-command> <skill-root>/scripts/work.py --project-root <project-root> handoff render --stdin` to produce `task_to_execute`. Use the same command to produce `task_to_plan` when the Plan must change.
3. Place rendered JSON in one conversation code block without edits. Require the fixed marker, actual requirement ID, all artifact paths, Plan skill-selection hash, and applicable single `skill_id`.
4. A handoff exists only in the conversation and never modifies Plan, TASK, index, Attempt, or lock state.
