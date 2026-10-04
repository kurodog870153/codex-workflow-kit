<!-- work-compatibility-revision: 1 -->
# Handle invalid related artifacts before execution


1. Normal execution still requires a validated current formal TASK. When the immutable Source evidence, TASK collection, execution index, or their bindings instead require reconstruction, stop eligibility and use `$work migration` and the shared artifact migration procedure. Invalid originals remain evidence and need not pass current validators first.
2. Run `work migration analyze`, then use `work migration prepare` in `reconstruction` mode only when the source set and confirmed semantic content are supported. The CLI derives current `v1` candidates and bindings. Use current Execute rules and immutable Attempt and Correction history as constraints; do not author complete formal JSON candidates, infer compatibility from matching schema IDs, or rewrite historical execution evidence.
3. Ask through the parent when preserved execution evidence and current rules permit more than one lifecycle or specification meaning. If immutable history or unknown TASK sources are unsupported by semantic preparation, preserve the rejection and request a dedicated operation; do not overwrite those sources.
4. Review every affected execution-index candidate with its TASK candidates and verified immutable Source evidence through `work migration preview`. Publish only through approved `work migration apply`; separately authorize `work migration recover` after interruption. Migration discussion, candidate validation, or approval does not authorize an Attempt, execution record, or independent file write.
