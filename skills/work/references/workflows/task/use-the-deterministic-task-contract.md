<!-- work-compatibility-revision: 1 -->
# Use the deterministic TASK contract


1. For saved planning, use `task preview` and `task apply` as defined in the checkpoint reference. Review the complete assembled logical result and bind approval to its fingerprint. Initial formalization emits a formal index plus one item file per TASK. For an existing specification revision, use the specification-update workflow.
2. Use only English keys, enums, IDs, statuses, paths, references, and hashes. Semantic strings may use the user's language.
3. Before requesting formal approval, save the complete logical object as the request file and invoke `task validate --input-file "<request-path>" --task-path "<task-index-path>"` with every source Plan `--skill-root`.
4. Require a successful result containing the canonical TASK collection, index, item, and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
5. For initial saved planning, use `task preview` and then `task apply` with the identical metadata and approved fingerprint. The CLI creates the formal TASK collection and execution index; do not assemble or write these JSON files manually.
6. `task apply` publishes only the initial collection and execution index. Existing formal specifications are revised through `$work revise` and Specification.
7. After an interrupted initial publication, use `task recover --input-file <same-metadata-path> --requirement-id <requirement-id> --plan-path <plan-path> --user-config-root <user-config-root> --approved-sha256 <approved-fingerprint>` with the original expected revision and skill roots, then verify the collection.
8. Neither preview, apply nor recover executes CMD or OP or creates an Attempt, execution lock, instruction audit or specification-update transaction.
