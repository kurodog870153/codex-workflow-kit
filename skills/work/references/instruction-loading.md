<!-- work-compatibility-revision: 2 -->
# Work Instruction Loading Bootstrap

1. Parse the explicit Work mode, then call `workflow status` or `workflow next` and use only its canonical selection manifest.
2. Load sources exactly in `source_order`. Do not guess sources, scan instruction directories, preload a complete workflow, or retain an earlier operation's instruction context.
3. Stop with `REVIEW_REQUIRED` when routing is unmatched or ambiguous, a required source is missing, its compatibility revision or SHA-256 drifts, or the verified workflow state changes.
4. The main flow alone coordinates user decisions and authorization. A routed operation receives a fingerprint-bound envelope and cannot expand scope, reinterpret authorization, select instructions, or delegate.
5. Authorization, safety, transaction, recovery, source validation, and artifact integrity gates remain mandatory even when their detailed modules are not selected for the current read-only operation.
6. Never fall back to loading the complete instruction or workflow tree.
