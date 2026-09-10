# Task Workflow

Use this workflow only after the source Plan and its confirmed skill selection have been validated without drift.

## Saved planning entry point

1. For `$work task -- <requirement-id>`, resolve the formal Plan and TASK paths using the shared path rules. Inspect whether `outputs/work/tasks/<requirement-id>/drafts/index.json` exists. Absence starts a new planning list; a present but unreadable or invalid index is a stop, not permission to initialize again.
2. Read [Task draft checkpoints](task-drafts.md) before initializing, saving or resuming planning. Use its CLI procedure and existing contracts; do not manually write draft JSON files.
3. First confirm the full list of TASK outcomes, scope, direct dependencies and assigned skills, then initialize its planning index with user authorization. Detail only one TASK at a time.
4. Resume by reading the index and only the selected TASK's historical draft. Briefly show confirmed progress, pending questions and the next discussion point; obtain confirmation before continuing discussion. Prefer the last current TASK; ask when the selected item is complete or the user's intent differs.

## Coordinate confirmed skills

1. Task skill discovery is forbidden. Use only skills in the source Plan `skill_selection`.
2. Split work into minimum independently verifiable TASK outcomes before delegation.
3. Bind each TASK to one `skill_id`. Use `null` only for base-only work that needs no external skill.
4. Give each TASK only applicable paths authorized by the source Plan hierarchy selection. An empty selection loads `general`; a non-empty path must be a confirmed path, one of its ancestors, or one of its descendants and must exist in both Task and Execute catalogs. Sharing an ancestor with a confirmed path does not authorize a sibling branch.
5. Skip Plan-only skills when producing executable TASKs. A required skill with Task mode `unsupported` must return to Plan for a new decision.
6. For the current TASK's executable skill, follow the [Task coordinator's delegation contract](../subagents/task-coordinator.md) and [Task skill subagent prompt](../subagents/task-skill.md), including their runtime configuration and fallback. Supply one skill, one TASK boundary and the relevant saved discussion.
7. Merge the current subagent output into that TASK's discussion. Resolve conflicts through user decisions. After all TASKs are refined, perform the existing complete-contract validation and approval process.

## Complete the request

1. Perform the user's Task request under the loaded instructions.
2. Loading Task instructions alone does not create or modify a TASK document. Create or modify artifacts only when the request and applicable authorization permit it.
3. Do not execute a TASK merely because Task instructions or a TASK document were loaded. Execution requires the Execute workflow and its applicable authorization.
4. Save planning discussions only through the authorized draft CLI. An unfinished discussion can be saved only when the user requests it; completing a TASK uses one confirmation for the discussed content and its save. Draft status never grants formal approval.
5. After saving a refined TASK, stop and let the user choose this session or a new session. Return `$work task -- <requirement-id>` and a short progress summary. Do not automatically refine the next TASK.

## Use the deterministic TASK contract

1. For saved planning, use `task draft-assemble` and `task draft-create` as defined in the checkpoint reference. Review the complete assembled result and bind approval to its fingerprint. For an existing formal specification revision, retain the complete candidate JSON procedure below.
2. Use only English keys, enums, IDs, statuses, paths, references, and hashes. Semantic strings may use the user's language.
3. Before requesting formal approval, pipe the complete object to `task validate --stdin --task-path <task-path>` with every source Plan `--skill-root`.
4. Require a successful result containing the canonical TASK and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
5. For initial saved planning, use `task draft-create` after fingerprint-bound approval. Direct complete-contract creation uses the identical approved `task create` request with the same skill roots. Do not assemble or write TASK or index JSON manually.
6. `task create` exclusively creates the canonical TASK and initial execution index. Treat an existing target or partial failure as a hard stop.
7. Use `<python-command> <skill-root>/scripts/work.py --project-root <project-root> task recover-create --stdin --plan-path <plan-path> --task-path <task-path> --execution-dir <execution-dir>` only after the user explicitly authorizes recovery and only with the identical approved JSON and three paths.
8. Neither validation, creation, nor recovery executes CMD or OP or creates an Attempt, execution lock, instruction audit, or specification-update transaction.

## Request coordinated revision

1. For confirmed changes to existing formal artifacts or confirmed Work instruction migration, follow [the shared coordinated revision procedure](../instruction-loading.md#coordinated-formal-artifact-revision). Initial creation and cross-session handoffs retain the procedures in this workflow.
2. Task discussion resolves new technical or specification decisions returned by the editor. Send confirmed decisions back through the parent so the editor can complete the same migration; Task does not independently publish migrated Plan, TASK or index files.

## Use deterministic handoffs

1. Before accepting `plan_to_task` or `execute_to_task`, pipe its pure JSON to `<python-command> <skill-root>/scripts/work.py --project-root <project-root> handoff validate --stdin` and require `work-handoff-validation/v1` with `status: valid`.
2. After TASK formalization, use `<python-command> <skill-root>/scripts/work.py --project-root <project-root> handoff render --stdin` to produce `task_to_execute`. Use `task_to_plan` for a cross-session Plan handoff; use the internal editor above for a confirmed same-session revision.
3. Place rendered JSON in one conversation code block without edits. Require the fixed marker, actual requirement ID, all artifact paths, Plan skill-selection hash, and applicable single `skill_id`.
4. A handoff exists only in the conversation and never modifies Plan, TASK, index, Attempt, or lock state.
