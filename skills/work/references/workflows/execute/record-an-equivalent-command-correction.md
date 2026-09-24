<!-- work-compatibility-revision: 1 -->
# Record an equivalent command correction


1. If a reserved CMD needs an equivalent syntax, quoting, escaping, argument-format, or executable-path correction, apply the execution-deviation boundary above. Obtain a new decision for the exact replacement unless that exact correction was already included in the current Attempt authorization; never treat semantic, side-effect or safety changes as an equivalent correction.
2. Save pure `work-command-correction-request/v1` JSON as the request file and invoke `<work-cli> execute command-correction --input-file "<request-path>"`. Include only the authorized `actual_command` and a non-empty semantic reason. Use `{"mode":"argv","argv":["tool","--fixed"]}` or `{"mode":"shell","script":"tool --fixed"}` for `actual_command`; each form requires its listed value and forbids the other form's fields and formal record IDs. Python derives the reserved record and effective original command from the active lock, TASK, Attempt, and approved deviations.
3. Require `work-command-correction/v1` with `correction_status: recorded` and `lock_status: record_reserved` before executing the corrected command. The tool verifies the original formal command and stores the correction in the lock; it does not execute the command or decide semantic equivalence.
4. `recovery_required: true` is a hard stop. Preserve the record lock and `.work-command-correction-*.tmp`; do not execute or resubmit until separately authorized recovery handles it.
