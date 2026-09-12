# Plan Workflow

Before relying on an existing formal TASK, apply [the shared TASK diagnostic gate](../instruction-loading.md#validate-before-relying-on-a-formal-task). Inspect raw content only for diagnosis until validation passes; preserve independent discussion progress and initial-artifact exceptions.

Use this workflow for Plan. Independent discussion restoration follows the progress procedure before formal-source gates; normal source-dependent planning and formalization require validated user-confirmed hierarchy and skill selections and their applicable instructions.

## Apply confirmed skills

1. Treat `skill_selection` as final for this Plan run. Do not rediscover skills in the subagent.
2. Read each selected `SKILL.md` completely only after confirmation. Confirmation through `$work` explicitly authorizes loading an explicit-only skill for this run.
3. Stop when a dependency is unavailable, a snapshot changed, selected skill content exceeds 40% of available Plan context, or loaded skills contain unresolved conflicting instructions.
4. Plan-only skills are valid. A selected skill does not need Task or Execute support unless the proposed Plan requires it in those phases.
5. For `base_only`, load no external skill and proceed only after the recorded explicit confirmation.

## Complete the request

1. Perform the user's Plan request under the loaded instructions.
2. Loading Plan instructions alone does not create or modify a Plan document. Create or modify artifacts only when the request and applicable authorization permit it.
3. Keep an unapproved formal Plan candidate in the conversation. When the user requests a checkpoint, return the discussion through the parent for [independent progress saving](progress.md); this permits unfinished discussion memory, not a partial formal Plan or its directories.

## Resume discussion progress

1. For `$work plan -- resume <requirement-id>`, follow [discussion progress](progress.md). Restore the saved context, confirm continuation, and review current sources before relying on decisions. Preserve unanswered questions and resume at the saved point without repeating unchanged confirmed decisions. The saver does not conduct Plan discussion.

## Use the deterministic Plan contract

1. Build the complete proposed `work-plan/v1` JSON object in the conversation.
2. Include validated `hierarchy_selection`, mode-resolved `work_instruction_selection`, and `skill_selection`. Before requesting authorization to create a formal Plan, save that object as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" plan validate --input-file "<request-path>" --plan-path "<plan-path>"` with every confirmed `--skill-root`.
3. Require a successful validation result containing the canonical Plan and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
4. After the user authorizes creation, pass the identical approved JSON request file to the same `plan create` command with every confirmed `--skill-root`. Do not assemble or write the JSON artifact manually.
5. After creation, run `plan validate --path "<plan-path>"` with the same skill roots and require the same canonical Plan, hierarchy-selection, Work instruction, and skill-selection fingerprints as the pre-write validation.
6. Never use `plan create` for an existing Plan. Keep the approved revision in the conversation and return it to the parent for the internal artifact editor's combined transaction when formal TASK and index exist.

## Request coordinated revision

1. For confirmed changes to existing formal artifacts or confirmed Work instruction migration, follow [the shared coordinated revision procedure](../instruction-loading.md#coordinated-formal-artifact-revision). Initial creation and cross-session handoffs retain the procedures in this workflow.

## Use deterministic handoffs

1. Before accepting `task_to_plan` or `execute_to_plan`, save its pure JSON as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff validate --input-file "<request-path>"` and require `work-handoff-validation/v1` with `status: valid`.
2. When handing a confirmed Plan to Task, build a complete `plan_to_task` `work-handoff/v1` object, save it as the request file, and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff render --input-file "<request-path>"`.
3. Place only the rendered JSON from response.data in one conversation code block without edits. Require the fixed `WORK-HANDOFF` marker, actual requirement ID, all three artifact paths, and validated skill-selection fingerprint.
4. A handoff exists only in the conversation and never authorizes artifact writes by itself.
