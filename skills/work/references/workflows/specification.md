# Internal Specification Revision

Load only for the private artifact editor after confirmed discussion requires coordinated revision of existing formal artifacts. This is not an invocation mode. Initial Plan creation and saved Task planning keep their existing workflows; a checkpoint is not a formal TASK and must not be promoted automatically.

When the user wants to preserve unfinished revision discussion,
return the retained content and blocking evidence through the parent to the
originating Plan or Task role for [independent discussion progress](progress.md).
The parent's progress saver can record that content without completing this
transaction. Do not publish partial candidates or refresh hashes to permit saving.

## Active TASK collection revision boundary

The Plan's `artifacts.task` selects the formal `index.json` entry point.
Single-file `task.json` artifacts are unsupported.

1. A specification revision treats the formal TASK index and every referenced
   TASK item as one logical collection. Validation and approval cover the complete
   candidate collection even when publication changes only one item.
2. A TASK change edit identifies its target with `artifact`. The allowed
   values are `task_index` and `task_item`. A `task_item` edit requires the exact
   `task_id`; a `task_index` edit rejects `task_id`. The edit `path` is a JSON
   Pointer within the selected document, not within a reconstructed monolith.
3. The existing add, replace, and remove evidence rules remain unchanged. Derived
   fields, index references, item fingerprints, collection fingerprints,
   readiness, and execution-index changes are generated and validated rather
   than supplied as unreviewed manual edits.
4. Preparation may avoid re-rendering unchanged items, but full candidate
   validation still enforces every cross-TASK invariant. Unchanged item bytes and
   fingerprints must remain identical.
5. The journal records a path-keyed variable file set for before and after
   TASK collection bytes, together with Plan and execution-index bytes, affected
   TASK IDs, execution history fingerprints, request evidence, and the approval
   fingerprint. A completion marker binds the canonical journal SHA-256.
6. Publication remains a recoverable logical transaction rather than a
   filesystem-wide atomic rename. It publishes prepared item bytes before the
   formal index commit point, publishes the synchronized execution index, and
   only then writes the completion marker. An incomplete record blocks Execute.
7. Recovery advances only files whose bytes are an exact recognized before,
   after, or missing state from the approved journal. Conflicting or ambiguous
   bytes remain a stop. Removed formal items stay recoverable through immutable
   transaction evidence and are never destructively cleaned as part of ordinary
   publication.
8. The Plan's `artifacts.task` remains the sole formal collection entry point.

## Prepare and review

Execution-deviation reconciliation uses `task reconciliation-preview` with the immutable closed Attempt path, the `all`, `selective`, or `retain_only` choice, and a complete AI-produced migration candidate set when publication is requested. The command validates the Attempt bytes, selected pending deviations, candidate contracts, diff and relationships. `retain_only` performs no write. After fingerprint-bound approval, `task reconciliation-apply` revalidates the same evidence and publishes through the migration specification transaction; it never modifies Attempt or Correction history.

### Optional field replacement preparation

For confirmed ordinary revisions, `task spec-prepare` can assemble the complete
request and validate the derived Plan, TASK and execution index in memory. It does
not publish formal artifacts or authorize execution or repair.

Supply a UTF-8 `work-spec-prepare-request/v1` file. Query
`<work-cli> contract describe work-spec-prepare-request/v1` for its required
and optional fields, canonical key order, edit structure, constraints, nested
contract references, and current valid example. Treat the registered Pydantic
contract as the structural source of truth; do not duplicate or independently
maintain complete JSON structures in this workflow.

1. This initial interface replaces existing fields only. Plan fields are `title`, `summary`,  `goals`, `scope`, `constraints`, `dependencies`, `risks`, `milestones`, `deliverables`, `acceptance_criteria` and `decisions`; each Plan edit also requires confirmed `affected_ids` referencing Plan items. TASK document fields are `title`, `summary`, `decisions` and `execution_defaults`. With `task_id`, editable TASK  fields are `title`, `goal`, `traceability`, `dependencies`, `steps`, `validations`, `commands` and `operations`.
2. `before` must exactly match the existing JSON value. Unknown TASK IDs, repeated targets, unchanged values and protected fields are rejected. Arrays are replaced as complete field values. `affected_ids` is required only for Plan edits and is rejected on TASK edits. Rejections identify the zero-based edit index, artifact, field and TASK ID when supplied; invalid fields list the allowed fields, while a stale `before` reports only the current value fingerprint. TASK IDs, sources, selections, history and index fields cannot be edited through this interface. No missing semantic decisions are inferred.
3. Version, Plan binding and exact change evidence are assembled automatically. The existing specification validator determines affected TASKs and index states. The response contains `data.request` and `data.preview`; the latter includes the complete candidates and `approved_sha256`. Review all three candidates.
4. Without `--output-file`, preparation is read-only. With it, only a new request file is created after validation; existing files are never overwritten. Keep input, output and retained response files in the same requirement-owned specification transaction workspace. Paths for input/output files are relative to the process cwd. Parent directories must already exist. Output uses UTF-8 without BOM and LF. If writing is interrupted, retain the partial file and stop; it is not an approved publication request.
5. Use the saved request with `spec-validate`, then obtain or reuse continuation approval for its exact preview before `spec-update`. If the user requested a discussion checkpoint for this continuation, prepare and validate that complete progress candidate before confirmation and bind the same confirmation to both displayed fingerprints. After approval, publish, run the derived read-only verification and save the identical prevalidated checkpoint without routine confirmation between successful steps. Retain the identical requests for separately authorized recovery. Preparation preserves the existing recoverable logical transaction; it does not make publication filesystem-wide atomic.

macOS example (replace the explicit paths):

```text
python3 "/path/to/work/scripts/work.py" --project-root "/path/to/project" task spec-prepare --input-file "/path/to/project/outputs/work/transactions/example/specification/20260915T103000Z-a1b2c3d4/spec-prepare.json" --output-file "/path/to/project/outputs/work/transactions/example/specification/20260915T103000Z-a1b2c3d4/spec-prepared-request.json" --user-config-root "/path/to/user-config"
```

Windows PowerShell example with the Python launcher:

```text
py -3 "C:\skills\work\scripts\work.py" --project-root "C:\project" task spec-prepare --input-file "C:\project\outputs\work\transactions\example\specification\20260915T103000Z-a1b2c3d4\spec-prepare.json" --output-file "C:\project\outputs\work\transactions\example\specification\20260915T103000Z-a1b2c3d4\spec-prepared-request.json" --user-config-root "C:\user-config"
```

Use the installed Python 3.10+ command and the same confirmed roots for subsequent
commands. Add the same `--skill-root` selections when required. Do not transport
JSON through shell interpolation, redirection or pipelines.

### Complete candidate validation

2. Build one work-spec-update-request/v1 object with exactly schema, reason, expected, plan and task. The latter two are complete candidate objects, including an unchanged Plan when appropriate. Do not supply a candidate execution index.
3. Preserve Plan change history and append change evidence when Plan changes. Use the Plan renderer/validator to obtain its canonical fingerprint; put it in the candidate TASK source reference. Do not write the Plan separately to make TASK validation pass.
4. Increment TASK spec exactly once. Preserve existing TASK IDs; append new IDs after the highest existing ID. Existing TASK removal, requirement/path renaming and cancelled-TASK lifecycle changes are outside this transaction.
5. TASK changes describes this revision using one next TASK-CHANGE-*, the request reason, actual ISO date, all affected TASK IDs and optional valid Plan change IDs. Previous TASK content and revision evidence remain in immutable specification transaction records.
6. Record exact edits for every changed top-level TASK field except spec_id, readiness and changes. Sort by field name, use JSON Pointer paths, and include the complete old/new value with replace, or add/remove as appropriate. Include derived source_plan changes. This makes recorded evidence deterministically comparable to the actual revision.
7. Invoke the following CLI with that JSON:

   <python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" task spec-validate --input-file "<request-path>" --user-config-root "<user-config-root>" [--skill-root <scope:locator=path> ...]

   This is read-only, validates the candidate Plan and TASK together in memory, and returns work-spec-update/v1, the complete canonical candidate set, affected_task_ids and approved_sha256.
8. Review the derived index with the user. Changed TASKs and transitive dependents are affected; Plan, shared decision or execution-default changes conservatively affect all TASKs. Pending rows remain pending. Affected closed-attempt rows become pending_retry; affected blocked rows without attempts remain blocked. Unaffected row states, latest Attempt/Correction pointers and all history files are preserved.
9. Contract validation does not re-run file lifecycle checks against already executed steps. Gather readiness evidence for future changes during review; Execute must perform its normal preflight before any new Attempt.

## Publish and recover

1. Obtain or reuse explicit continuation approval bound to this complete candidate and approved_sha256, plus the distinct progress fingerprint when a checkpoint is included. Run the identical request and root arguments through task spec-update --input-file "<request-path>" --approved-sha256 <approved-sha256>. A changed source, candidate, history or included checkpoint invalidates the applicable approval.
2. Work CLI mutations share a process-released OS mutex in .work-state-writer.lock; the publisher rechecks sources after acquisition. This coordinates Work writers, not unrelated editors, which must remain paused during publication. The command first preserves original/proposed bytes in an exclusive .work-spec-update-SPEC-UPDATE-nnn.json transaction record, acquires the existing spec_update index lock, replaces Plan and TASK, publishes the synchronized index, and writes a fingerprinted completion marker. Execution is blocked while any record lacks its valid completion marker. This is a recoverable logical transaction, not a filesystem-wide atomic rename.
3. Require status updated and report its spec, affected TASKs and checks. A normal
   update returns a complete `work-spec-verification-request/v1` and names `task
   specifications; Attempt and Correction files are never rewritten. No CMD, OP,
   VAL or implementation is executed.
4. On interruption preserve every current file, record, temporary and lock. Report the observed state and obtain separate recovery authorization. Only then use the identical request, roots and approved fingerprint with task spec-recover --input-file "<request-path>" --approved-sha256 <approved-sha256>.
5. Recovery revalidates original/candidate contracts and history and advances only matching original, locked or final bytes. A short journal is recoverable only when unchanged original artifacts reconstruct the same approved record. For a short journal, temporary file or completion marker, recovery may append only the missing suffix of the exact approved bytes; it never truncates or replaces conflicting bytes. Unknown bytes, changed instructions, changed history or another unfinished transaction remain stops. Recovery never rolls back, overwrites a conflict, deletes history or starts execution.
6. Return to the originating workflow with the retained discussion. Revalidate its current sources; existing draft checkpoints can become stale after a formal source revision and require the separately authorized draft-source review workflow. Never silently rewrite checkpoint history or claim its previous decisions are still current.

## Specification verification

1. After a successful ordinary `spec-update` or its explicitly authorized recovery,
   immediately transport the returned `verification_request` unchanged to the read-only `task
   spec-verify --input-file "<request-path>" --user-config-root "<user-config-root>"`
   command with the same project and confirmed skill roots without requesting another routine confirmation. The request contains
   exactly `schema: "work-spec-verification-request/v1"`, `requirement_id`, all three
   artifact paths and the actual `record_id`.
2. The verifier checks the canonical ordinary-update journal and completion marker,
   exact installed Plan/TASK/index bytes, current formal contracts and source binding,
   derived transaction evidence, preserved execution history, all transaction
   retain their separate verification procedure.
3. Success returns `work-spec-verification/v1`, `verified: true`, verification scope
   `exact_specification_result`, `execution_authorized: false` and next step
   `normal_execute_preflight`. Failed or unavailable required checks return exit 5
   with `specification_verification_failed` and the complete report.
4. Verification is an immediate exact-result check. A later legitimate specification
   revision or execution history change can make an older record inapplicable; inspect
   the cause rather than rolling back or reusing a stale request. Verification never
   grants Execute approval or modifies formal, transaction, history or Git state.
