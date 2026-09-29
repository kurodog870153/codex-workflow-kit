---
name: work
description: Route an explicit $work plan, task, execute, or migration invocation through Work CLI state and necessary-source selection. Use only when the user explicitly invokes $work.
---

# Work

1. Read only `references/instruction-loading.md` as the bootstrap before interpreting an explicit `$work <mode> -- <request>` invocation.
2. For an explicit invocation, call `workspace create --workflow-id invocation` without a requirement ID. Save the unchanged input as `invocation.txt` only under the returned path, then run `invocation parse --input-file` on that file. Never choose a path directly under `outputs/`. For Plan, Task, and Execute, call `workflow status` or `workflow next`. For Migration, load `references/instruction-loading/artifact-migration.md` and use the dedicated `work migration` commands. Do not scan instruction, workflow, or subagent directories.
3. For Plan, Task, and Execute, require a `VALID` routing result and verify the selection manifest fingerprint. Load exactly `required_instruction_sources` in `source_order` for the current `next_action`. Migration is maintenance mode and does not enter the lifecycle routing state machine.
4. Keep user interaction, decisions, and authorization in the main flow. Give an isolated worker only the canonical operation envelope and routed sources; accept only its structured result and evidence.
5. Re-run routing whenever the operation, state, lifecycle, selection, authorization, role, or formal event changes. Never reuse a failed or recovery context.
6. Stop on `REVIEW_REQUIRED`, missing source, source drift, state drift, insufficient authorization, or an incomplete envelope. Never fall back to loading every source.
