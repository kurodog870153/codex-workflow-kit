# Use the deterministic TASK contract

1. Use `task preview` to assemble the saved refined candidates from their fixed Source and Task-owned selections. Review the complete collection and initial Execution bytes, and bind approval to the returned fingerprint.
2. Use English keys, enums, IDs, statuses, paths, references and hashes. Semantic strings may use the user's language.
3. Independently verify a published collection using `task validate --path <task-index-path> --user-config-root <user-config-root>` with every confirmed Task-owned skill root. Require complete Source, collection, item and instruction validation.
4. Inspect every nonzero exit and identify its cause before a new write. Resolve semantic ambiguity through the user; correct deterministic errors only within existing authorization. Preserve rejected evidence and historical bytes.
5. Initial `task apply` requires identical approved metadata, expected revision, user config and skill roots plus `--approved-sha256 <approved-fingerprint>`. It creates the initial TASK collection and Execution index. Existing formal specifications use `$work revise` and Specification.
6. Recover interrupted initial publication using `task recover --input-file <same-metadata-path> --requirement-id <requirement-id> --expected-revision <revision> --user-config-root <user-config-root> --approved-sha256 <approved-fingerprint>` with the original skill roots. Conflicting existing bytes stop recovery; verify the complete result.
7. Preview, apply and recover do not execute CMD or OP, create an Attempt or grant execution authorization.
