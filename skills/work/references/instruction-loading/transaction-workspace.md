<!-- work-compatibility-revision: 1 -->
# Transaction workspaces


1. Store every Work transport input, prepared request, private-role envelope and retained command result below one transaction workspace. When the requirement ID is known, use `outputs/work/transactions/<requirement-id>/<workflow-id>/<transaction-id>/`. Before a requirement ID exists, use `outputs/work/transactions/pending/<workflow-id>/<transaction-id>/`; retain that pending evidence in place and put later files in a new requirement-owned workspace rather than moving or rewriting it.
4. Create the complete workspace before writing its first file. Never place these files directly in `outputs/work`, reuse another transaction's directory, overwrite an existing file, or resolve a path outside the project root. Existing transport files remain untouched. Retain successful and failed transaction workspaces as audit, recovery and approval-fingerprint evidence; do not delete or relocate them automatically.
5. Reuse one workspace only for the uninterrupted logical transaction named by its workflow ID. A changed request, stale preview, failure requiring a user decision, separately authorized recovery, or transition from pending to a known requirement starts a fresh transaction ID. Formal CLI-owned journals and locks keep their existing canonical locations and are not transport files.
