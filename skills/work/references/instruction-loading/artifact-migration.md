<!-- work-compatibility-revision: 1 -->
# AI cross-file migration


Apply the AI reconstruction procedure when existing related artifacts cannot satisfy the current representation as a set.

1. Read the raw source artifacts, current Plan, Task, and Execute workflows, and every current contract needed for the affected relationships. Source parsing and validation may fail; preserve those failures as evidence and continue reading any safely inspectable bytes rather than requiring a legacy object to validate.
2. Have AI construct complete replacement candidates directly in the current `v1` representations. Do not use Python to migrate meaning, add a compatibility adapter, retain two incompatible meanings under one schema ID, or convert through intermediate versions.
3. Separate deterministic reconstruction from semantic choice. Apply uniquely implied structural, canonical, and derived-field corrections in the candidates. Return only genuine semantic alternatives to the user, with their source evidence and affected files, and resume the same candidate set after the decision.
4. Validate the complete candidates, not the originals, against the current per-file contracts and cross-file identities, references, fingerprints, lifecycle state, and immutable execution history. A candidate that validates individually is insufficient when another related candidate or relationship is missing.
5. Keep the Plan, TASK index and items, execution index, and any other affected formal document as one atomic candidate set. One approval must cover its exact bytes and evidence; drift, an unresolved ambiguity, or a failed relationship check invalidates the whole candidate set.
6. Use `task migration-preview` for the complete request. Only a ready report with no unresolved item may receive fingerprint-bound write approval. Use `task migration-apply` with that identical request and fingerprint; use `task migration-recover` only after a failed publication and separate recovery authorization. Do not substitute TASK repair, coordinated revision, manual writes, or ad hoc scripts.
