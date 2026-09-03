# Execute Workflow

Use this workflow only after the formal target TASK and its single external skill selection have been validated. In this file, `<work-cli>` means `<python-command> <skill-root>/scripts/work.py --project-root <project-root>`.

## Load only the target skill

1. Use only the target TASK `skill_id`. `null` means base-only and loads no external skill.
2. Resolve a non-null ID only from the source Plan `skill_selection`. Do not scan, recommend, or combine other skills.
3. Require Execute mode support, available dependencies, the same root identity, and unchanged summary and bundle fingerprints.
4. Pass the same `--skill-root` values to every Execute CLI command. Any missing root or drift is a hard stop.

## Identify the execution target

1. Loading Execute instructions alone does not execute or authorize a TASK.
2. Require the user to identify one formal TASK document and one `TASK-*` from that document before eligibility checks. Do not select either on the user's behalf.
3. Use the default TASK path `outputs/work/tasks/<requirement-id>.md` unless the user explicitly supplies and confirms a permitted project-relative path.
4. When Plan, TASK, or execution uses a non-default path, require the same requirement ID and all three confirmed project-relative paths. Never infer one path from another.
5. After the target is complete, perform only authorized read-only eligibility checks. Obtain every required authorization before changing state, modifying files, running side-effecting commands, or performing external operations.

## Run deterministic read-only eligibility checks

1. Run `<work-cli> execute preflight --task-path <task-path> --execution-dir <execution-dir> --task-id <task-id> [--skill-root <scope:locator=path> ...] [--confirmed-input <TASK-ID>/<INPUT-ID> ...]`.
2. Pass `--confirmed-input` once for each current-TASK `user_provided` or `external` input only after the user or applicable external evidence confirms it. Pass IDs only, never secret values.
3. Require `work-execute-preflight/v1` with `eligibility: passed`. Treat every nonzero exit code as a hard stop.
4. Require the returned `hierarchy_selection_sha256` to match the formal TASK and index, and use the returned TASK, Task instruction, Execute instruction, input, and file-readiness results without recomputing them in the model. Preflight is read-only and does not authorize or create an Attempt, lock, state transition, record, or artifact change.
5. Run the same arguments through `<work-cli> execute worktree`. Treat `path_classification` only as path-overlap evidence; `target_task`, `completed_dependency`, and `unrelated` never prove ownership.
6. When `review_status` is `required`, explain every returned change using the approved TASK, a valid Attempt or Correction, or known prior results. Stop when any path or content may belong to the user or cannot be explained. The command excludes only the current execution directory and never modifies Git.

## Use deterministic handoffs

1. Before using `task_to_execute`, pipe its pure JSON to `<work-cli> handoff validate --stdin` and require `work-handoff-validation/v1` with `status: valid`. This does not replace eligibility checks.
2. For a specification defect, render `execute_to_task` or `execute_to_plan` with actual Attempt or preflight context, `skill_id`, `execute_skill_selection_sha256`, confirmed approach, requested changes, preserved scope, affected IDs, and validation requirements.
3. Place the rendered JSON in one conversation code block without edits. A handoff exists only in the conversation and never modifies Plan, TASK, index, Attempt, or lock state.

## Use the canonical Attempt contract

1. Before proposing an Attempt write, pipe the proposed pure `work-attempt/v1` JSON to `<work-cli> attempt validate --stdin` and require `work-attempt-validation/v1` with `result: valid`.
2. Use `<work-cli> attempt render --stdin` to canonicalize field order. Validate an existing canonical document with `<work-cli> attempt validate --path <attempt-path>`.
3. These commands are read-only contract operations. They do not create an Attempt, acquire a lock, update the index, execute a record, or authorize state changes.

## Start an Attempt transaction

1. Require the user to review the complete `execute worktree` result and retain `snapshot_sha256`. The hash is not approval by itself.
2. After explicit start authorization, pipe pure `work-attempt-start-request/v1` JSON containing `worktree_snapshot_sha256` to `<work-cli> execute attempt-start --stdin` with the same preflight arguments. For `pending_retry`, also provide the latest source Attempt and only carried record IDs whose current evidence was explicitly confirmed.
3. The command rechecks preflight and Git snapshot, derives canonical Attempt identity, `skill_id`, hierarchy-selection, skill-selection and instruction fingerprints, and local-offset time, then installs the lock and creates the Attempt.
4. `recovery_required: true` is a hard stop. Summarize the transaction stage and obtain separate authorization before running `<work-cli> execute recover-attempt-start --stdin` with identical paths, inputs, and request. Recovery advances only an exactly matching state and never rolls back, deletes, unlocks, or resolves conflicts.

## Reserve one execution record

1. Before executing one formal CMD, OP, or VAL, obtain its specific authorization and run `<work-cli> execute record-begin --task-path <task-path> --execution-dir <execution-dir> --task-id <task-id> --record-id <base-record-id>`.
2. Pass only the formal base ID such as `CMD-001`. The command validates the active Attempt, TASK, and current instruction fingerprints, derives any retry suffix, and adds the exact `record_id` to the execution lock.
3. Require `work-record-begin/v1` with `lock_status: record_reserved` before performing the authorized record. The command does not execute or append the CMD, OP, or VAL.
4. `recovery_required: true` is a hard stop. Preserve the lock and transaction file; do not retry, execute the record, or modify the index until separately authorized recovery handles it.

## Record an equivalent command correction

1. If a reserved CMD needs an equivalent syntax, quoting, escaping, argument-format, or executable-path correction, obtain explicit authorization for the exact replacement before execution.
2. Pipe pure `work-command-correction-request/v1` JSON to `<work-cli> execute command-correction --stdin`. Include the reserved `record_id`, canonical `original_command` and `actual_command`, a non-empty reason, and minimal authorization evidence.
3. Require `work-command-correction/v1` with `correction_status: recorded` and `lock_status: record_reserved` before executing the corrected command. The tool verifies the original formal command and stores the correction in the lock; it does not execute the command or decide semantic equivalence.
4. `recovery_required: true` is a hard stop. Preserve the record lock and `.work-command-correction-*.tmp`; do not execute or resubmit until separately authorized recovery handles it.

## Record one execution result

1. After the reserved CMD, OP, or VAL finishes, pipe pure `work-record-finish-request/v1` JSON to `<work-cli> execute record-finish --stdin` with the same TASK and execution paths. `record.id` must exactly match the reserved lock ID, including any retry suffix.
2. Command uses `exit_code` and `result`; operation uses `outcome` and `state`; validation uses `outcome` and `evidence`. Do not repeat command-correction data. Include normalized `modified_files` only when the record changed files.
3. The command validates the formal record and current instruction fingerprints, moves a locked command correction into the command record, atomically appends the canonical Attempt record, updates cumulative modified files and operation outcome, then removes `record_id` and its correction from the lock. It never executes the record or releases the Attempt lock.
4. `recovery_required: true` is a hard stop. Preserve the Attempt, lock, and transaction files; do not resubmit the result or begin another record until separately authorized recovery handles the state.

## Close one Attempt

1. After all authorized records finish or execution must stop, obtain specific close authorization and pipe pure `work-attempt-close-request/v1` JSON to `<work-cli> execute attempt-close --stdin` with the same TASK and execution paths.
2. Use `status: completed` without final details only when every formal VAL has a latest passed result or valid carried evidence. Use `stopped` or `blocked` with an allowed `final_type` and non-empty reason.
3. The command verifies no record remains reserved, checks current instruction fingerprints, derives local-offset end time, writes the canonical closed Attempt first, then atomically synchronizes TASK and overall status while releasing the matching execution lock. It never executes a record.
4. Require `work-attempt-close/v1` with `lock_status: released`. `recovery_required: true` is a hard stop: preserve the closed Attempt, lock, and `.work-attempt-close-*.tmp`; do not repeat close or manually unlock until separately authorized recovery handles it.

## Create an immutable Correction

1. Use a Correction only for a confirmed factual error in a closed Attempt or its index state. Never modify the original Attempt, TASK, implementation, or execution evidence.
2. After separate authorization, pipe pure `work-correction-create-request/v1` JSON to `<work-cli> execute correction-create --stdin`. Provide `target_attempt_id`, `field`, `correct_value`, `reason`, and explicit boolean `invalidates_completion`.
3. The command derives the next Correction ID, local-offset time, and original Attempt instruction fingerprints; installs a Correction lock, exclusively creates canonical `work-correction/v1`, synchronizes `latest_correction` and invalidated completed downstream TASK statuses, then releases the lock.
4. Require `work-correction-create/v1` with `lock_status: released`. `recovery_required: true` is a hard stop; preserve the immutable target, lock, and `.work-correction-*.tmp` files for separately authorized recovery.

## Recover one execution transaction

1. Use `<work-cli> execute recover --stdin` only after record-begin, command-correction, record-finish, attempt-close, or Correction reports `recovery_required: true` and the user separately authorizes recovery of the observed state. Attempt-start continues to use `recover-attempt-start`.
2. Pipe pure `work-execution-recovery-request/v1` JSON containing the exact `transaction`, target `attempt_id`, and complete sorted `.work-*.tmp` filenames currently present. Use an empty list only when the preserved Attempt and lock uniquely prove a record-finish or attempt-close post-write stage.
3. Require `work-execution-recovery/v1` with `status: recovered`. The command revalidates formal TASK identity, canonical Attempt, index, original lock fingerprint, and every prepared byte before advancing only the uniquely determined transaction.
4. Any file-set change, byte mismatch, ambiguous state, unsupported attempt-start transaction, or write failure is a hard stop. Preserve all artifacts and locks; never retry automatically, roll back, delete a transaction file, rewrite a closed Attempt, or manually unlock.
