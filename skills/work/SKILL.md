---
name: work
description: Route an explicit $work plan, task, or execute invocation, discover and confirm suitable enabled skills, and run the selected workflow through a private dedicated subagent when available or the parent as fallback. Use only when the user explicitly invokes $work.
---

# Work

Provide one explicit entry point for Plan, Task, and Execute workflows without exposing their internal role prompts as user-callable agents.

## Parse and select

1. Read [references/instruction-loading.md](references/instruction-loading.md) completely before interpreting the invocation.
2. Accept only `$work <mode> -- <request>`. Do not accept or request user-facing hierarchy paths.
3. For Plan, inspect the cross-mode instruction catalog metadata, recommend the smallest suitable leaf-path set, show each description and recommendation reason, and ask the user to confirm it. Confirm `general_only` explicitly when no specialized path applies.
4. Discover enabled skills from configured roots using summary metadata only, recommend the smallest suitable set, and confirm it separately. Let the user accept, add, remove, or cancel either selection. Do not load specialized Work instructions or full external skill instructions before confirmation.
5. The parent owns mode, request, catalog discovery, recommendation, dependency checks, and selection confirmation. It delegates the selected workflow when the required subagent runtime is available and performs it only under the fallback defined below.

## Run the selected workflow

1. Read only the private prompt matching the selected mode:
   1. Plan: [references/subagents/plan.md](references/subagents/plan.md)
   2. Task: [references/subagents/task.md](references/subagents/task.md)
   3. Execute: [references/subagents/execute.md](references/subagents/execute.md)
2. When delegation is available, use the matching runtime configuration:
   1. Plan uses exactly one ephemeral subagent with model `gpt-5.6-terra` and reasoning effort `high`.
   2. Task uses one coordinator with model `gpt-5.6-sol` and reasoning effort `high`; the coordinator creates one isolated ephemeral subagent per executable confirmed skill, sequentially.
   3. Execute uses exactly one ephemeral subagent with model `gpt-5.6-luna` and reasoning effort `medium`.
3. Send a delegation envelope containing all of the following:
   1. `WORK_DELEGATION_V1`
   2. `skill=$work`
   3. `mode=<plan|task|execute>`
   4. `skill_root=<resolved-skill-root>`
   5. `project_root=<resolved-project-root>`
   6. `work_instruction_selection=<validated-work-instruction-selection>`
   7. `hierarchy_selection=<validated-work-hierarchy-selection>`
   8. `skill_selection=<validated-work-skill-selection>`
   9. `request=<complete-user-request>`
4. Include the matching private prompt as role instructions. Task skill subagents also receive exactly one selected skill snapshot and one proposed TASK boundary. Do not register, install, or select a custom agent profile.
5. If the required subagent capability, model, or reasoning configuration is unavailable for Plan, Task, or Execute, do not stop solely for that reason. The parent must perform the selected workflow directly with its current runtime, following the matching private prompt as workflow instructions and preserving the confirmed selections, permissions, role scope, and machine fields.
6. When delegated, the Plan subagent loads all confirmed external skills. The Task coordinator loads one confirmed skill per isolated TASK subagent. The Execute subagent loads only the target TASK's one confirmed skill, or none for base-only. The parent does not preload full external instructions before delegation.
7. Under parent fallback, apply the same loading boundaries: Plan loads all confirmed external skills; Task handles each executable confirmed skill sequentially with exactly one selected skill snapshot and one proposed TASK boundary at a time; Execute loads only the target TASK's one confirmed skill, or none for base-only.

## Relay and continue

1. Relay the subagent's user-facing question or result in Traditional Chinese without changing its decision boundary, options, machine fields, or requested authorization. Under parent fallback, present the same user-facing content directly.
2. When the user answers a subagent question, send the answer back to the same subagent and continue that delegated workflow. If that subagent becomes unavailable, continue the same workflow directly under parent fallback. Do not create a replacement subagent unless the user authorizes restarting the delegated workflow.
3. Never treat skill invocation, hierarchy selection, delegation, or a handoff as authorization for file changes, commands with side effects, external operations, installation, or state transitions.
4. Do not delegate work beyond the selected role. Plan and Execute subagents cannot spawn subagents. Only the Task coordinator may create its specified per-skill subagents; those subagents cannot delegate further. Under parent fallback, the parent performs the selected role without further delegation, and handles Task skill work sequentially.
5. End only after the selected workflow returns a completed result or a genuine stop condition that has been reported to the user.
