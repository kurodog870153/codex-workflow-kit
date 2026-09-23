<!-- work-compatibility-revision: 1 -->
# Use the canonical Attempt contract


1. Before proposing an Attempt write, save the proposed pure `work-attempt/v1` JSON as the request file and invoke `<work-cli> attempt validate --input-file "<request-path>"` and require `work-attempt-validation/v1` with `result: valid`.
2. Use `<work-cli> attempt render --input-file "<request-path>"` to canonicalize field order. Validate an existing canonical document with `<work-cli> attempt validate --path "<attempt-path>"`.
3. These commands are read-only contract operations. They do not create an Attempt, acquire a lock, update the index, execute a record, or authorize state changes.
