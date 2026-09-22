<!-- work-compatibility-revision: 2 -->
# Close one Attempt


1. After all authorized records finish, save pure `work-attempt-close-request/v1` JSON as the request file and invoke `<work-cli> execute attempt-close --input-file "<request-path>"` with the same TASK and execution paths without another confirmation when the reviewed normal-completion condition is satisfied. A stopped or blocked outcome, uncertain effects, an unreviewed deviation or any closure outside the authorized condition requires a new decision before close. A reserved record may close only when its recorded, approved blocking deviation anchors that same record and the close is the freshly authorized specification-defect stop.
2. Use `status: completed` without final details only when every formal VAL has a latest passed result or valid carried evidence. Use `stopped` or `blocked` with an allowed `final_type` and non-empty reason.
3. The command verifies no record remains reserved, checks current instruction fingerprints, derives local-offset end time, writes the canonical closed Attempt first, then atomically synchronizes TASK and overall status while releasing the matching execution lock. It never executes a record.
4. Require `work-attempt-close/v1` with `lock_status: released`. If it reports pending deviations, route to reconciliation before completion review or another Attempt. `recovery_required: true` is a hard stop: preserve the closed Attempt, lock, and `.work-attempt-close-*.tmp`; do not repeat close or manually unlock until separately authorized recovery handles it.
