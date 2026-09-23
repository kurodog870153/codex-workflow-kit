<!-- work-compatibility-revision: 2 -->
# Prepare and review


Execution-deviation reconciliation begins only after the blocking or final Attempt is closed. Use `task reconciliation-prepare` with the requirement ID, one-based TASK and latest Attempt positions, and the `all`, `selective`, or `retain_only` choice. For `selective`, use one-based positions in the closed Attempt's deviation list. For `all` or `selective`, provide a reason and confirmed semantic specification edits as in `spec-prepare`. Python derives the Attempt and Plan paths, formal deviation IDs and SHA values, complete Plan/TASK/execution candidates, and a migration request. `retain_only` needs no edits and prepares a ledger-only request. The preparation response includes the exact preview request and its validation preview; `--output-file` saves a new request file without overwriting. Use the saved request with `task reconciliation-preview`; after fingerprint-bound approval, `task reconciliation-apply` revalidates the same evidence and publishes either the specification candidates with the ledger or only the ledger through a recoverable transaction. The ledger records incorporated, retained and declined outcomes, workflow routing subtracts those IDs, and closed Attempt and Correction bytes never change.

### Optional field replacement preparation

For confirmed ordinary revisions, `task spec-prepare` can assemble the complete
request and validate the derived Plan, TASK and execution index in memory. It does
not publish formal artifacts or authorize execution or repair.

Supply a UTF-8 `work-spec-prepare-request/v1` file with the requirement ID, reason and semantic edits. Python finds the unique Plan by its verified artifact binding; missing or ambiguous sources stop preparation. Query
`<work-cli> contract describe work-spec-prepare-request/v1` for its required
and optional fields, canonical key order, edit structure, constraints, nested
contract references, and current valid example. Treat the registered Pydantic
contract as the structural source of truth; do not duplicate or independently
maintain complete JSON structures in this workflow.

1. Use `{"target": {"artifact": "plan|task_index|task_item", "task_id": "TASK-NNN" when selecting an existing TASK}, "field": "title|summary|goal", "after": "new text"}` for simple text. Plan collections and TASK machine-bearing fields use `semantic_after`, never formal `after`. Collection rows use lowercase local `key`, optional one-based `existing_position` to retain an ID, and semantic relation keys or one-based positions. For example, a constraint row uses `{"key":"boundary","existing_position":1,"statement":"Confirmed boundary","applies_to":[{"collection":"goals","position":1}]}`. TASK nested records use the same semantic candidate fields as TASK drafts; references to unchanged records use local keys such as `existing-1`. TASK traceability uses goal/deliverable/acceptance positions, and dependencies use TASK positions.
2. Add a TASK with `{"operation":"add_task","task":{"title":"Implement","goal":"Deliver","skill_id":null,"selected_paths":[],"references":[],"dependency_positions":[],"candidate":{"steps":[...],"validations":[...]}}}`. Python allocates its TASK and nested IDs, builds instruction and Plan bindings, and updates the index. `{"operation":"remove_task","task_position":2}` selects an unexecuted TASK for removal. Python reads `before` from validated artifacts and derives operations, affected IDs, paths, references and SHA evidence. Unknown, duplicate, unchanged, ambiguous and unsupported operations are rejected. Caller-authored complete TASK items, formal nested IDs, direct formal references, `field: "/"`, and caller-authored `before`, `path` or `affected_ids` are unsupported.
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

### Unsupported semantic changes

1. If `spec-prepare`, `reconciliation-prepare` or `migration-prepare` rejects a confirmed semantic change as unsupported, retain the rejection and source evidence. Return to the parent for a narrower semantic decision or implement and review a dedicated prepare operation. Do not construct `work-spec-update-request/v1`, full Plan/TASK candidates, derived index fields or candidate bytes with AI as a fallback.
2. The prepared `data.request` is an internal machine request for the existing validator and publication command. Review its complete candidate set, affected TASKs, history preservation and approval fingerprint; only Python creates or revises those bytes. Execute still performs normal preflight before a new Attempt.
