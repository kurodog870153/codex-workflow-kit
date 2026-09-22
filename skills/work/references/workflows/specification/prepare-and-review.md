<!-- work-compatibility-revision: 2 -->
# Prepare and review


Execution-deviation reconciliation begins only after the blocking or final Attempt is closed. Use `task reconciliation-preview` with the immutable closed Attempt path and the `all`, `selective`, or `retain_only` choice. `all` and `selective` require a complete AI-produced migration candidate set; `retain_only` requires none and prepares a ledger-only publication. The command validates the Attempt bytes, excludes deviations already present in the reconciliation ledger, deterministically aggregates TASK-only, Plan-and-TASK and retain-only targets, and validates any candidate contracts, diff and relationships. After fingerprint-bound approval, `task reconciliation-apply` revalidates the same evidence and publishes either the specification candidates with the ledger or only the ledger through a recoverable transaction. The ledger records incorporated, retained and declined outcomes, workflow routing subtracts those IDs, and closed Attempt and Correction bytes never change.

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
