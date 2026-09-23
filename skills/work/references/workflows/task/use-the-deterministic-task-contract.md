<!-- work-compatibility-revision: 1 -->
# Use the deterministic TASK contract


1. For saved planning, use `task draft-assemble` and `task draft-create` as defined in the checkpoint reference. Review the complete assembled logical result and bind approval to its fingerprint. Initial formalization emits a formal index plus one item file per TASK. For an existing specification revision, use the specification-update workflow.
2. Use only English keys, enums, IDs, statuses, paths, references, and hashes. Semantic strings may use the user's language.
3. Before requesting formal approval, save the complete logical object as the request file and invoke `task validate --input-file "<request-path>" --task-path "<task-index-path>"` with every source Plan `--skill-root`.
4. Require a successful result containing the canonical TASK collection, index, item, and instruction fingerprints. Treat any nonzero exit code as a hard stop; do not repair, rewrite, retry, or reinterpret a rejected contract without new user direction.
5. For initial saved planning, use `task draft-create` after fingerprint-bound approval. Direct complete-contract creation uses the identical approved `task create` request with the same skill roots. Do not assemble or write TASK or index JSON manually.
6. `task create` exclusively creates the canonical TASK index, all referenced item files, and the initial execution index. Treat an existing target or partial failure as a hard stop.
7. Use `<python-command> "<skill-root>/scripts/work.py" --project-root "<project-root>" task recover-create --input-file "<request-path>" --plan-path "<plan-path>" --task-path "<task-path>" --execution-dir "<execution-dir>"` only after the user explicitly authorizes recovery and only with the identical approved JSON and three paths.
8. Neither validation, creation, nor recovery executes CMD or OP or creates an Attempt, execution lock, instruction audit, or specification-update transaction.
