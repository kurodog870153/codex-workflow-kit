<!-- work-compatibility-revision: 1 -->
# Active TASK collection revision boundary


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
