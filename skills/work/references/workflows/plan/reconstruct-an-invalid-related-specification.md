<!-- work-compatibility-revision: 1 -->
# Reconstruct an invalid related specification


1. When an existing Plan belongs to an old, malformed, or cross-file-incompatible artifact set, use `$work migration` and the shared artifact migration procedure. Run `work migration analyze` to inspect source evidence, then use `work migration semantic-prepare` in `reconstruction` mode for a confirmed semantic Plan. The CLI derives the current `v1` candidate and cross-file bindings; the source Plan need not validate first.
2. Preserve requirement meaning, goals, scope, deliverables, acceptance criteria, and confirmed decisions when they are uniquely evidenced. Ask through the parent only when the evidence permits materially different Plan meanings. Formatting or schema-shape differences alone are not a reason to ask.
3. Review the Plan candidate together with every affected TASK and execution-index candidate through `work migration semantic-preview`. Publish the complete candidate set only through approved `work migration semantic-apply`; recover an interrupted publication only through separately authorized `semantic-recover`. Do not publish the Plan independently or proceed while related candidates remain invalid.
