<!-- work-compatibility-revision: 1 -->
# Saved planning entry point


1. For `$work task -- <requirement-id>`, resolve the formal Plan and TASK paths using shared path rules. Use `task status --requirement-id <requirement-id> --plan-path <plan-path> --user-config-root <user-config-root>` with confirmed skill roots as the read-only planning entry point; add `--task-id` for an explicit TASK. Follow its `next_action` and source checks. A missing index with no residue proposes a new planning list; invalid storage stops.
2. Read Task planning checkpoints before initializing, saving or resuming planning. Use its CLI procedure and existing contracts; do not manually write draft JSON files.
3. First confirm the full list of TASK outcomes, scope, direct dependencies and assigned skills, then initialize its planning index with user authorization. Detail only one TASK at a time.
4. Resume by reading the index and only the selected TASK's historical draft. Briefly show confirmed progress, pending questions and the next discussion point; obtain confirmation before continuing discussion. Prefer the last current TASK; ask when the selected item is complete or the user's intent differs.
