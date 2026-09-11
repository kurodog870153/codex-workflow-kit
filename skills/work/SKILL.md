---
name: work
description: Route an explicit $work plan, task, or execute invocation, discover and confirm suitable enabled skills, and run the selected workflow through a private dedicated subagent when available or the parent as fallback. Use only when the user explicitly invokes $work.
---

# Work

Provide one explicit entry point for Plan, Task, and Execute workflows without exposing their internal role prompts as user-callable agents.

## Parse and select

1. Read [references/instruction-loading.md](references/instruction-loading.md) completely before interpreting the invocation.
2. Accept only `$work <mode> -- <request>`. Do not accept or request user-facing hierarchy paths.
3. For an explicit Plan or Task `resume <requirement-id>` request, first follow [discussion progress](references/workflows/progress.md) to restore that mode's saved context before selection or formal-source gates. For a new Plan, inspect the cross-mode instruction catalog metadata, recommend the smallest suitable path set at the requested scope, including intermediate nodes when appropriate, show each description and recommendation reason, and ask the user to confirm it. Confirm `general_only` explicitly when no specialized path applies.
4. Discover enabled skills from configured roots using summary metadata only, recommend the smallest suitable set, and confirm it separately. Let the user accept, add, remove, or cancel either selection. Do not load specialized Work instructions or full external skill instructions before confirmation.
5. The parent owns mode, request, catalog discovery, recommendation, dependency checks, and selection confirmation. It delegates the selected workflow when the required subagent runtime is available and performs it only under the fallback defined below.
6. For `$work task -- <requirement-id>`, use the Task workflow's saved-planning entry point. A saved checkpoint is a valid end of the current run; do not continue to another TASK until the user chooses to continue in this session or resume in a new one.

## Run the selected workflow

1. Read only the private prompt matching the selected mode:
   1. Plan: [references/subagents/plan.md](references/subagents/plan.md)
   2. Task: [references/subagents/task-coordinator.md](references/subagents/task-coordinator.md)
   3. Execute: [references/subagents/execute.md](references/subagents/execute.md)
2. When delegation is available, use the required runtime configuration defined in each linked private role prompt:
   1. Plan uses exactly one ephemeral subagent.
   2. Task uses one coordinator; it creates one isolated ephemeral subagent per executable confirmed skill, sequentially, using the Task skill prompt's configuration.
   3. Execute uses exactly one ephemeral subagent.
3. Send a delegation envelope containing all of the following. For explicit progress restoration, the progress workflow permits saved context in place of not-yet-valid source selections solely for restoration and clarification; source-dependent work retains normal validation:
   1. `WORK_DELEGATION_V1`
   2. `skill=$work`
   3. `mode=<plan|task|execute>`
   4. `skill_root=<resolved-skill-root>`
   5. `project_root=<resolved-project-root>`
   6. `work_instruction_selection=<validated-work-instruction-selection>`
   7. `hierarchy_selection=<validated-work-hierarchy-selection>`
   8. `skill_selection=<validated-work-skill-selection>`
   9. `request=<complete-user-request>`
4. Include the matching private prompt and [shared private role rules](references/instruction-loading.md#shared-private-role-rules) as role instructions. For each Task skill subagent, the coordinator reads and includes [references/subagents/task-skill.md](references/subagents/task-skill.md), the same shared source, exactly one selected skill snapshot and one proposed TASK boundary.
5. If the required subagent capability, model, or reasoning configuration is unavailable for Plan, Task, or Execute, do not stop solely for that reason. The parent must perform the selected workflow directly with its current runtime, following the matching private prompt as workflow instructions and preserving the confirmed selections, permissions, role scope, and machine fields.
6. When delegated, the Plan subagent loads all confirmed external skills. The Task coordinator loads one confirmed skill per isolated TASK subagent. The Execute subagent loads only the target TASK's one confirmed skill, or none for base-only. The parent does not preload full external instructions before delegation.
7. Under parent fallback, apply the same loading boundaries: Plan loads all confirmed external skills; Task handles each executable confirmed skill sequentially with exactly one selected skill snapshot and one proposed TASK boundary at a time; Execute loads only the target TASK's one confirmed skill, or none for base-only.

## Coordinate an internal artifact revision

1. Follow [the shared coordinated revision procedure](references/instruction-loading.md#coordinated-formal-artifact-revision) when confirmed discussion requires changes to existing formal artifacts. Read [the private artifact editor prompt](references/subagents/artifact-editor.md) for its required runtime configuration, accepted envelope and document-only transaction scope.

## Save discussion progress

1. When Plan or Task returns a user-requested progress checkpoint, follow [discussion progress](references/workflows/progress.md). The parent alone invokes [the private progress saver](references/subagents/progress-saver.md), using `gpt-5.6-terra` with `low` reasoning or its documented parent fallback. This role faithfully records supplied content; Plan and Task retain discussion and resumption ownership.

## Relay and continue

1. Relay the subagent's user-facing question or result in Traditional Chinese without changing its decision boundary, options, machine fields, or requested authorization. Under parent fallback, present the same user-facing content directly.
2. When the user answers a subagent question, send the answer back to the same subagent and continue that delegated workflow. If that subagent becomes unavailable, continue the same workflow directly under parent fallback. Do not create a replacement subagent unless the user authorizes restarting the delegated workflow.
3. Apply the shared private role authorization and stop rules during delegation and parent fallback.
4. Except for the parent's internal artifact revision and progress-saving procedures above, do not delegate work beyond the selected role. Plan and Execute subagents cannot spawn subagents. Only the Task coordinator may create its specified per-skill subagents; those subagents cannot delegate further. Under parent fallback, the parent performs the selected role without further delegation, and handles Task skill work sequentially.
5. End only after the selected workflow returns a completed result, an authorized saved Plan/Task progress checkpoint or structured Task checkpoint, or a genuine stop condition that has been reported to the user. A checkpoint is discussion progress, never formal approval or Execute authorization.
