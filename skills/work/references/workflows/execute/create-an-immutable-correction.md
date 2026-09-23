<!-- work-compatibility-revision: 1 -->
# Create an immutable Correction


1. Use a Correction only for a confirmed factual error in a closed Attempt or its index state. Never modify the original Attempt, TASK, implementation, or execution evidence.
2. After separate authorization, save pure `work-correction-create-request/v1` JSON as the request file and invoke `<work-cli> execute correction-create --input-file "<request-path>"`. Provide `target_attempt_id`, `field`, `correct_value`, `reason`, and explicit boolean `invalidates_completion`.
3. The command derives the next Correction ID, local-offset time, and original Attempt instruction fingerprints; installs a Correction lock, exclusively creates canonical `work-correction/v1`, synchronizes `latest_correction` and invalidated completed downstream TASK statuses, then releases the lock.
4. Require `work-correction-create/v1` with `lock_status: released`. `recovery_required: true` is a hard stop; preserve the immutable target, lock, and `.work-correction-*.tmp` files for separately authorized recovery.
