---
name: work
description: Route an explicit $work plan, task, or execute invocation through deterministic Python workflow state and necessary-source selection. Use only when the user explicitly invokes $work.
---

# Work

1. Read only `references/instruction-loading.md` as the bootstrap before interpreting an explicit `$work <mode> -- <request>` invocation.
2. Parse the invocation with the Work CLI, then call `workflow status` or `workflow next`. Do not scan instruction, workflow, or subagent directories.
3. Require a `VALID` routing result and verify the selection manifest fingerprint. Load exactly `required_instruction_sources` in `source_order` for the current `next_action`.
4. Keep user interaction, decisions, and authorization in the main flow. Give an isolated worker only the canonical operation envelope and routed sources; accept only its structured result and evidence.
5. Re-run routing whenever the operation, state, lifecycle, selection, authorization, role, or formal event changes. Never reuse a failed or recovery context.
6. Stop on `REVIEW_REQUIRED`, missing source, source drift, state drift, insufficient authorization, or an incomplete envelope. Never fall back to loading every source.
