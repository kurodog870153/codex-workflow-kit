<!-- work-compatibility-revision: 1 -->
# Record one execution result


For a reserved argv CMD, use the single-command executor before recording its result. Shell-mode CMDs and OP/VAL retain their existing approved execution tools.

1. After the reserved CMD, OP, or VAL finishes, save pure `work-record-finish-request/v1` JSON as the request file and invoke `<work-cli> execute record-finish --input-file "<request-path>"` with the same TASK and execution paths. `record.id` must exactly match the reserved lock ID, including any retry suffix. Accurate recording of an authorized operation does not require another confirmation; uncertain or expanded effects stop under the Attempt authorization boundary.
2. Command uses `exit_code` and `result`; operation uses `outcome` and `state`; validation uses `outcome` and `evidence`. Do not repeat command-correction data. Include normalized `modified_files` only when the record changed files.
3. The command validates the formal record and current instruction fingerprints, moves a locked command correction into the command record, atomically appends the canonical Attempt record, updates cumulative modified files and operation outcome, then removes `record_id` and its correction from the lock. It never executes the record or releases the Attempt lock.
4. `recovery_required: true` is a hard stop. Preserve the Attempt, lock, and transaction files; do not resubmit the result or begin another record until separately authorized recovery handles the state.
