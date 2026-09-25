<!-- work-compatibility-revision: 1 -->
# Reserve one execution record


1. Before executing one formal CMD, OP, or VAL, verify that its exact base ID, action and effects remain inside the current Attempt authorization, then run `<work-cli> execute record-begin --task-path "<task-path>" --execution-dir "<execution-dir>" --task-id <task-id> --record-id <base-record-id>`. Do not request another confirmation solely for this reservation.
2. Pass only the formal base ID such as `CMD-001`. The command validates the active Attempt, TASK, and current instruction fingerprints, derives any retry suffix, and adds the exact `record_id` to the execution lock.
3. Require `work-record-begin/v1` with `lock_status: record_reserved` before performing the authorized record. The command does not execute or append the CMD, OP, or VAL.
4. `recovery_required: true` is a hard stop. Preserve the lock and transaction file; do not retry, execute the record, or modify the index until separately authorized recovery handles it.
