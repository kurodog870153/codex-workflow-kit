# Plan Workflow

Use this workflow only after the user-confirmed hierarchy and Plan skill selections have been validated and their applicable instructions loaded.

## Apply confirmed skills

1. Treat `skill_selection` as final for this Plan run. Do not rediscover skills in the subagent.
2. Read each selected `SKILL.md` completely only after confirmation. Confirmation through `$work` explicitly authorizes loading an explicit-only skill for this run.
3. Stop when a dependency is unavailable, a snapshot changed, selected skill content exceeds 40% of available Plan context, or loaded skills contain unresolved conflicting instructions.
4. Plan-only skills are valid. A selected skill does not need Task or Execute support unless the proposed Plan requires it in those phases.
5. For `base_only`, load no external skill and proceed only after the recorded explicit confirmation.

## Complete the request

1. Perform the user's Plan request under the loaded instructions.
2. Loading Plan instructions alone does not create or modify a Plan document. Create or modify artifacts only when the request and applicable authorization permit it.
3. Keep an unapproved Plan candidate in the conversation. Do not create a draft file.

## Use the deterministic Plan contract

1. Build the complete proposed `work-plan/v1` JSON object in the conversation.
2. Include validated `hierarchy_selection`, mode-resolved `work_instruction_selection`, and `skill_selection`. Before requesting authorization to create a formal Plan, pipe that object to `<python-command> <skill-root>/scripts/work.py --project-root <project-root> plan validate --stdin --plan-path <plan-path>` with every confirmed `--skill-root`.
3. Require a successful validation result containing the canonical Plan and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
4. After the user authorizes creation, pipe the identical approved JSON object to the same `plan create` command with every confirmed `--skill-root`. Do not assemble or write the Markdown manually.
5. After creation, run `plan validate --path <plan-path>` with the same skill roots and require the same canonical Plan, hierarchy-selection, Work instruction, and skill-selection fingerprints as the pre-write validation.
6. Never use `plan create` for an existing Plan. Keep an approved revision in the conversation and hand it to the Task workflow for the authorized Plan, TASK, and execution-index specification transaction.

## Use deterministic handoffs

1. Before accepting `task_to_plan` or `execute_to_plan`, pipe its pure JSON to `<python-command> <skill-root>/scripts/work.py --project-root <project-root> handoff validate --stdin` and require `work-handoff-validation/v1` with `status: valid`.
2. When handing a confirmed Plan to Task, build a complete `plan_to_task` `work-handoff/v1` object and pipe it to `<python-command> <skill-root>/scripts/work.py --project-root <project-root> handoff render --stdin`.
3. Place the rendered JSON in one conversation code block without edits. Require the fixed `WORK-HANDOFF` marker, actual requirement ID, all three artifact paths, and validated skill-selection fingerprint.
4. A handoff exists only in the conversation and never authorizes artifact writes by itself.
