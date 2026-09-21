# Plan Workflow

Before relying on an existing formal TASK, apply [the shared TASK diagnostic gate](../instruction-loading.md#validate-before-relying-on-a-formal-task). Inspect raw content only for diagnosis until validation passes; preserve independent discussion progress and initial-artifact exceptions.

Use this workflow for Plan. Independent discussion restoration follows the progress procedure before formal-source gates; normal source-dependent planning and formalization require validated user-confirmed hierarchy and skill selections and their applicable instructions.

## Reconstruct an invalid related specification

1. When an existing Plan belongs to an old, malformed, or cross-file-incompatible artifact set, follow [AI cross-file migration](../instruction-loading.md#ai-cross-file-migration). Read the current Plan contract and reconstruct the Plan candidate directly in its current `v1` representation; do not require the source Plan to validate and do not route it through a Python migration.
2. Preserve requirement meaning, goals, scope, deliverables, acceptance criteria, and confirmed decisions when they are uniquely evidenced. Ask through the parent only when the evidence permits materially different Plan meanings. Formatting or schema-shape differences alone are not a reason to ask.
3. Keep the Plan candidate in the same indivisible candidate set as every affected TASK and execution-index candidate. Do not publish the Plan independently or treat a valid Plan candidate as permission to proceed while related candidates remain invalid.

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

1. For an initial Plan, inspect the current `work-plan-prepare-request/v1` through the shared contract scaffold and recursive description procedure immediately before constructing the request; use that result as the complete structural source of truth and do not infer nested fields. Then use `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" plan prepare --input-file "<request-path>" --user-config-root "<user-config-root>"` with every confirmed `--skill-root`. Preserve explicit item IDs and references; the tool does not infer requirement meaning or relationships. The command derives the initial contract and current Work instruction selection, validates all sources and references, and returns `work-plan-prepare/v1` with `plan`, `path` and `validation`. Without `--output-file`, preparation is read-only. To retain the canonical Plan candidate, add `--output-file "<new-candidate-path>"`; the parent directory must already exist, the target must not exist, and the CLI writes UTF-8 without BOM with LF on every supported platform. Never capture or transform the JSON response through shell redirection. Review the loaded guidance and full candidate; derived `status: confirmed` is a required candidate field, not proof of approval. Existing targets are rejected. Preparation never creates the formal Plan, TASK collection, execution index or approval. Complete-object validation remains available for explicitly reviewed candidates.
2. Include validated `hierarchy_selection`, mode-resolved `work_instruction_selection`, and `skill_selection`. Before requesting authorization to create a formal Plan, save that object as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" plan validate --input-file "<request-path>" --plan-path "<plan-path>"` with every confirmed `--skill-root`.
3. Require a successful validation result containing the canonical Plan and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
4. After the user authorizes creation, pass the identical approved JSON request file to the same `plan create` command with every confirmed `--skill-root`. Do not assemble or write the JSON artifact manually.
5. After creation, run `plan validate --path "<plan-path>"` with the same skill roots and require the same canonical Plan, hierarchy-selection, Work instruction, and skill-selection fingerprints as the pre-write validation.
6. Never use `plan create` for an existing Plan. Keep the approved revision in the conversation and return it to the parent for the internal artifact editor's combined transaction when formal TASK and index exist.

## Request coordinated revision

1. For confirmed changes to existing formal artifacts, follow [the shared coordinated revision procedure](../instruction-loading.md#coordinated-formal-artifact-revision). Initial creation and cross-session handoffs retain the procedures in this workflow.

## Use deterministic handoffs

Apply [the shared handoff procedure](../instruction-loading.md#shared-handoff-procedure) for transport, source verification, legacy evidence and authorization boundaries.

1. Before relying on a return from saved formal sources, preserve the incoming JSON unchanged and run `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff verify-task-to-plan --input-file "<request-path>" --plan-path "<confirmed-plan-path>" --task-path "<confirmed-task-path>" --user-config-root "<user-config-root>"` with all confirmed skill roots. Add `--task-id` only for an independently selected TASK; omission explicitly verifies a whole-specification return. For `execute_to_plan`, use `verify-execute-to-plan` with an explicit `--task-id` and exactly one independently confirmed `--attempt-id` or `--preflight`. Require `work-handoff-source-validation/v1` with `status: valid`. Paths and context must come from receiver confirmation, not solely from the incoming handoff. These checks validate live sources and compare identity, fingerprints and affected IDs; they do not approve proposed semantic revisions.
2. When handing a saved confirmed Plan to Task, save only `{"summary": <handoff-summary>, "affected_ids": [<confirmed-Plan-item-IDs>]}` as the request file and invoke `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" handoff build-plan-to-task --input-file "<request-path>" --plan-path "<plan-path>" --user-config-root "<user-config-root>"` with every confirmed `--skill-root`. The read-only command validates the live Plan, hierarchy, instructions and skills, derives the requirement ID, all three artifact paths and source fingerprints, verifies that affected IDs exist, and rejects a Plan changed during construction. Do not rebuild machine fields in the model. Unsaved Plan revisions continue to use the authorized Task specification-update procedure; this command only describes the saved formal Plan.
