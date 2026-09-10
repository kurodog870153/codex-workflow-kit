# Internal Specification Revision

Load only for the private artifact editor after confirmed discussion requires coordinated revision of existing formal artifacts. This is not an invocation mode. Initial Plan creation and saved Task planning keep their existing workflows; a checkpoint is not a formal TASK and must not be promoted automatically.

## Prepare and review

1. Preserve the explicit requirement ID and all three confirmed artifact paths. Read the formal Plan, TASK and index; retain their raw-byte SHA-256 values as expected.plan_sha256, expected.task_sha256 and expected.index_sha256. Formal source JSON must already be canonical. Stop for any lock, ongoing Attempt, unresolved transaction, unknown user edits, or source drift.
2. Build one work-spec-update-request/v1 object with exactly schema, reason, expected, plan and task. The latter two are complete candidate objects, including an unchanged Plan when appropriate. Do not supply a candidate execution index.
3. Preserve Plan change history and append change evidence when Plan changes. Use the Plan renderer/validator to obtain its canonical fingerprint; put it in the candidate TASK source reference. Do not write the Plan separately to make TASK validation pass.
4. Increment TASK spec exactly once. Preserve existing TASK IDs; append new IDs after the highest existing ID. Existing TASK removal, requirement/path renaming and cancelled-TASK lifecycle changes are outside this transaction.
5. TASK changes describes this revision using one next TASK-CHANGE-*, the request reason, actual ISO date, all affected TASK IDs and optional valid Plan change IDs. Previous TASK content and revision evidence remain in immutable specification transaction records.
6. Record exact edits for every changed top-level TASK field except spec_id, readiness and changes. Sort by field name, use JSON Pointer paths, and include the complete old/new value with replace, or add/remove as appropriate. Include derived source_plan changes. This makes recorded evidence deterministically comparable to the actual revision.
7. Invoke the following CLI with that JSON:

   <python-command> <skill-root>/scripts/work.py --project-root <project-root> task spec-validate --stdin --user-config-root <user-config-root> [--skill-root <scope:locator=path> ...]

   This is read-only, validates the candidate Plan and TASK together in memory, and returns work-spec-update/v1, the complete canonical candidate set, affected_task_ids and approved_sha256.
8. Review the derived index with the user. Changed TASKs and transitive dependents are affected; Plan, shared decision or execution-default changes conservatively affect all TASKs. Pending rows remain pending. Affected closed-attempt rows become pending_retry; affected blocked rows without attempts remain blocked. Unaffected row states, latest Attempt/Correction pointers and all history files are preserved.
9. Contract validation does not re-run file lifecycle checks against already executed steps. Gather readiness evidence for future changes during review; Execute must perform its normal preflight before any new Attempt.

## Publish and recover

1. Obtain or reuse explicit write approval bound to this complete candidate and approved_sha256. Run the identical request and root arguments through task spec-update --stdin --approved-sha256 <approved-sha256>. A changed source, candidate or history invalidates approval.
2. Work CLI mutations share a process-released OS mutex in .work-state-writer.lock; the publisher rechecks sources after acquisition. This coordinates Work writers, not unrelated editors, which must remain paused during publication. The command first preserves original/proposed bytes in an exclusive .work-spec-update-SPEC-UPDATE-nnn.json transaction record, acquires the existing spec_update index lock, replaces Plan and TASK, publishes the synchronized index, and writes a fingerprinted completion marker. Execution is blocked while any record lacks its valid completion marker. This is a recoverable logical transaction, not a filesystem-wide atomic rename.
3. Require status updated and report its spec, affected TASKs and checks. Transaction records retain old/new specifications; Attempt and Correction files are never rewritten. No CMD, OP, VAL or implementation is executed.
4. On interruption preserve every current file, record, temporary and lock. Report the observed state and obtain separate recovery authorization. Only then use the identical request, roots and approved fingerprint with task spec-recover --stdin --approved-sha256 <approved-sha256>.
5. Recovery revalidates original/candidate contracts and history and advances only matching original, locked or final bytes. A short journal is recoverable only when unchanged original artifacts reconstruct the same approved record. For a short journal, temporary file or completion marker, recovery may append only the missing suffix of the exact approved bytes; it never truncates or replaces conflicting bytes. Unknown bytes, changed instructions, changed history or another unfinished transaction remain stops. Recovery never rolls back, overwrites a conflict, deletes history or starts execution.
6. Return to the originating workflow with the retained discussion. Revalidate its current sources; existing draft checkpoints can become stale after a formal source revision and require the separately authorized draft-source review workflow. Never silently rewrite checkpoint history or claim its previous decisions are still current.
