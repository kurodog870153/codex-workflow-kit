<!-- work-compatibility-revision: 1 -->
# Run deterministic read-only eligibility checks


1. Run `<work-cli> execute preflight --task-path "<task-path>" --execution-dir "<execution-dir>" --task-id <task-id> [--skill-root <scope:locator=path> ...] [--confirmed-input <TASK-ID>/<INPUT-ID> ...]`.
2. Pass `--confirmed-input` once for each current-TASK `user_provided` or `external` input only after the user or applicable external evidence confirms it. Pass IDs only, never secret values.
3. Require `work-execute-preflight/v1` with `eligibility: passed`. Treat every nonzero exit code as a hard stop.
4. Require the returned `hierarchy_selection_sha256` to match the formal TASK and index, and use the returned TASK, Task instruction, Execute instruction, input, and file-readiness results without recomputing them in the model. Preflight is read-only and does not authorize or create an Attempt, lock, state transition, record, or artifact change.
5. Run the same arguments through `<work-cli> execute worktree`. Treat `path_classification` only as path-overlap evidence; `target_task`, `completed_dependency`, and `unrelated` never prove ownership.
6. When `review_status` is `required`, explain every returned change using the approved TASK, a valid Attempt or Correction, or known prior results. Stop when any path or content may belong to the user or cannot be explained. The command excludes only the current execution directory and never modifies Git.
