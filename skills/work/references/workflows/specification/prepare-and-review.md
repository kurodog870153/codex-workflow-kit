<!-- work-compatibility-revision: 2 -->
# Prepare and review

1. Use `specification prepare` with a UTF-8 `work-spec-prepare-request/v1` input containing the requirement ID, reason and confirmed semantic edits. Work CLI resolves the unique validated TASK collection, its immutable Source proof and Execution binding; missing or ambiguous evidence stops preparation. Query `contract describe` or `contract scaffold` for the current structure instead of constructing formal candidates manually.
2. Text edits target `task_index` or `task_item`; item targets require the exact TASK ID. Use supported semantic fields and `semantic_after` for collections and machine-bearing records. `add_task` supplies semantic Task data and one-based dependency positions; `remove_task` selects an unexecuted Task by position. Work CLI derives IDs, references, before evidence, affected TASKs, candidate bytes and fingerprints. Unsupported or ambiguous edits remain rejected.
3. A `source_update` must include the newly confirmed Source, instruction selections and complete `source_confirmation`. Review the complete requirement, retained/removed/added main acceptance criteria and every existing TASK's impact. Preserve previous Source Snapshot bytes. Review the derived TASK/Execution changes and affected acceptance evidence before approval.
4. Without `--output-file`, preparation is read-only. With it, only a new validated request file is created; existing files are never overwritten. Keep input, output and retained responses in the same requirement-owned transaction workspace. Parent directories must exist. Input/output paths are relative to process cwd; use UTF-8 without BOM and LF. Preserve an interrupted output file for review.
5. Use `--verbose` to inspect the complete `data.request` and `data.preview`, or use the saved request from `--output-file`. The ordinary brief response is a summary. Run `specification preview` on the identical request and obtain or reuse approval bound to its exact fingerprint before `specification apply`. Review the complete candidate collection, Source proof, affected evidence and history preservation.
6. Execution-deviation reconciliation starts only after the latest selected Attempt is closed. `specification reconciliation-prepare` takes requirement ID, one-based TASK/Attempt positions and `all`, `selective` or `retain_only`. Selective positions refer to the closed Attempt's deviation list. For incorporation, supply a reason, confirmed semantic edits, explicit reviewed raw `sources` fingerprints and resolved semantic decisions. Work CLI derives the exact Attempt path, deviation IDs and complete TASK/Execution migration request.
7. `retain_only` requires no migration or edits and prepares only a ledger transaction. Review the saved request with `specification reconciliation-preview`; then use `specification reconciliation-apply` with the exact approved fingerprint. Incorporation publishes the approved migration set with its ledger. Ledger outcomes account for incorporated, retained and declined deviations; historical Attempt and Correction bytes remain unchanged.
8. Preserve the identical request and approved fingerprint for `specification recover` or `specification reconciliation-recover`. Approval does not authorize a new Attempt, CMD, OP or VAL. If a semantic operation is unsupported, retain the rejection and return to the parent for a confirmed decision; never substitute caller-authored formal candidates or derived bytes.

macOS example:

```text
"/path/to/work/scripts/work" --project-root "/path/to/project" --verbose specification prepare --input-file "/path/to/project/outputs/work/transactions/example/specification/spec-prepare.json" --output-file "/path/to/project/outputs/work/transactions/example/specification/spec-prepared-request.json" --user-config-root "/path/to/user-config"
```

Windows PowerShell example:

```text
& "C:\skills\work\scripts\work.exe" --project-root "C:\project" --verbose specification prepare --input-file "C:\project\outputs\work\transactions\example\specification\spec-prepare.json" --output-file "C:\project\outputs\work\transactions\example\specification\spec-prepared-request.json" --user-config-root "C:\user-config"
```

Use the installed Work binary and identical confirmed roots for subsequent commands. Include the same `--skill-root` selections when needed. Transport JSON through UTF-8 files, without shell interpolation, redirection or pipelines.
