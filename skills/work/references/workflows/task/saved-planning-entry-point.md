<!-- work-compatibility-revision: 1 -->
# Saved planning entry point


1. For `$work task -- <requirement-id>`, resolve the formal Plan and TASK paths using the shared path rules. Use `task draft-status --requirement-id <requirement-id>` as the read-only planning entry point, with `--task-id` only for an explicitly selected TASK. Follow its `next_action` and the checkpoint reference's source checks and confirmation gates. A missing index with no stored residue proposes a new planning list; invalid storage stops, and reserved history requires inspection before any continuation.
2. Read Task draft checkpoints before initializing, saving or resuming planning. Use its CLI procedure and existing contracts; do not manually write draft JSON files.
3. First confirm the full list of TASK outcomes, scope, direct dependencies and assigned skills, then initialize its planning index with user authorization. Detail only one TASK at a time.
4. Resume by reading the index and only the selected TASK's historical draft. Briefly show confirmed progress, pending questions and the next discussion point; obtain confirmation before continuing discussion. Prefer the last current TASK; ask when the selected item is complete or the user's intent differs.
