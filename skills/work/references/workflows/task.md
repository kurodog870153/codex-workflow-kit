# Task Workflow

Before relying on an existing formal TASK, apply [the shared TASK diagnostic gate](../instruction-loading.md#validate-before-relying-on-a-formal-task). Inspect raw content only for diagnosis until validation passes; preserve independent discussion progress and initial-artifact exceptions.

Use this workflow for Task. Independent discussion restoration follows the progress procedure before formal-source gates; normal source-dependent planning and formalization require a validated source Plan and confirmed skill selection without drift.

## Independent discussion progress

1. For `$work task -- resume <requirement-id>`, follow [discussion progress](progress.md) before the saved-planning entry point. Restore the Task discussion even when its formal Plan is stale is incomplete. Review current sources and any referenced structured draft before relying on decisions; only Task conducts continued discussion.
2. When the user asks to preserve the current discussion, return the complete content and continuation point to the parent for its private progress saver. Missing formal decisions or blocked revision do not prevent this independent save. Do not initialize or change a structured planning index to make progress saving possible.

## Saved planning entry point

1. For `$work task -- <requirement-id>`, resolve the formal Plan and TASK paths using the shared path rules. Use `task draft-status --requirement-id <requirement-id>` as the read-only planning entry point, with `--task-id` only for an explicitly selected TASK. Follow its `next_action` and the checkpoint reference's source checks and confirmation gates. A missing index with no stored residue proposes a new planning list; invalid storage stops, and reserved history requires inspection before any continuation.
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
4. Save structured Task draft checkpoints only through the authorized draft CLI. Independent user-requested discussion progress uses the parent-owned progress saver above. An unfinished discussion can be saved only when the user requests it; completing a structured TASK discussion uses one confirmation for the discussed content and its draft save. Neither progress nor draft status grants formal approval.
5. After saving a refined TASK, stop and let the user choose this session or a new session. Return `$work task -- <requirement-id>` and a short progress summary. Do not automatically refine the next TASK.

## Use the deterministic TASK contract

1. For saved planning, use `task draft-assemble` and `task draft-create` as defined in the checkpoint reference. Review the complete assembled logical result and bind approval to its fingerprint. Initial formalization emits a formal index plus one item file per TASK. For an existing specification revision, use the specification-update workflow.
2. Use only English keys, enums, IDs, statuses, paths, references, and hashes. Semantic strings may use the user's language.
3. Before requesting formal approval, save the complete logical object as the request file and invoke `task validate --input-file "<request-path>" --task-path "<task-index-path>"` with every source Plan `--skill-root`.
4. Require a successful result containing the canonical TASK collection, index, item, and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
5. For initial saved planning, use `task draft-create` after fingerprint-bound approval. Direct complete-contract creation uses the identical approved `task create` request with the same skill roots. Do not assemble or write TASK or index JSON manually.
6. `task create` exclusively creates the canonical TASK index, all referenced item files, and the initial execution index. Treat an existing target or partial failure as a hard stop.
7. Use `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" task recover-create --input-file "<request-path>" --plan-path "<plan-path>" --task-path "<task-path>" --execution-dir "<execution-dir>"` only after the user explicitly authorizes recovery and only with the identical approved JSON and three paths.
8. Neither validation, creation, nor recovery executes CMD or OP or creates an Attempt, execution lock, instruction audit, or specification-update transaction.

## Request coordinated revision

1. For confirmed changes to existing formal artifacts, follow [the shared coordinated revision procedure](../instruction-loading.md#coordinated-formal-artifact-revision). Initial creation and cross-session handoffs retain the procedures in this workflow.
2. Task discussion resolves new technical or specification decisions returned by the editor. Send confirmed decisions back through the parent so the editor can complete the same revision; Task does not independently publish revised Plan, TASK or index files.

## Use deterministic handoffs

Apply [the shared handoff procedure](../instruction-loading.md#shared-handoff-procedure) for transport, source verification, legacy evidence and authorization boundaries.

1. Before accepting `plan_to_task`, save the received JSON unchanged as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff verify-plan-to-task --input-file "<request-path>" --plan-path "<confirmed-plan-path>" --user-config-root "<user-config-root>"` with every confirmed `--skill-root`. Use the explicitly selected Plan path, not a path chosen solely from the incoming handoff. Require `work-handoff-source-validation/v1` with `status: valid`; this checks the current formal Plan, requirement ID, all artifact paths, Plan and skill-selection fingerprints, and affected IDs. Reject stale or mismatched handoffs without rewriting their fields. It does not approve semantic content or authorize TASK creation. For `execute_to_task`, preserve the received JSON unchanged and use `handoff verify-execute-to-task --input-file "<request-path>" --plan-path "<confirmed-plan-path>" --task-path "<confirmed-task-path>" --task-id <confirmed-task-id> --user-config-root "<user-config-root>"` with every confirmed skill root and exactly one independently confirmed `--attempt-id` or `--preflight`. Require `work-handoff-source-validation/v1` with `status: valid`. The verifier uses current formal sources, index and Attempt state; closed returns also compare exact Attempt bytes. It does not approve semantic proposals or authorize writes.
2. After TASK formalization and explicit selection of one TASK, save only `{"summary": <handoff-summary>}` as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff build-task-to-execute --input-file "<request-path>" --task-path "<task-path>" --task-id <task-id> --user-config-root "<user-config-root>"` with every confirmed `--skill-root`. The read-only command validates the formal TASK and source Plan, derives artifact paths, specification identity, the selected TASK's instruction fingerprint and skill ID, and rejects changed source files. Do not select the first TASK automatically or rebuild machine fields. This is not Execute preflight, worktree review or authorization to start an Attempt. Use the return procedure below for a cross-session Plan handoff; use the internal editor above for a confirmed same-session revision.

For a cross-session return from a saved formal TASK to Plan, save `{"summary": <text>, "confirmed_approach": <text>, "requested_changes": [...], "preserve": [...], "affected_ids": [...], "validation_requirements": [...]}` as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff build-task-to-plan --input-file "<request-path>" --task-path "<task-path>" --user-config-root "<user-config-root>"` with all confirmed `--skill-root` values. Add `--task-id <task-id>` only when returning a specifically selected TASK; omission describes the whole specification and omits both task and skill IDs. The command derives machine fields and validates sources and affected IDs. Plan item IDs, TASK IDs and shared decision IDs may be referenced; TASK-local IDs require the explicit target TASK. A proposed new item belongs in `requested_changes`, not among existing affected IDs. Cross-session handoffs for unsaved planning or revisions continue to use `handoff render --input-file "<request-path>"` with the existing complete contract; this builder requires a valid saved formal TASK and its matching Plan. Confirmed same-session revisions use the internal editor above. Neither handoff path authorizes modifying either artifact.
