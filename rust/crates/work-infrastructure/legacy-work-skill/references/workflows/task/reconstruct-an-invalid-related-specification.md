<!-- work-compatibility-revision: 1 -->
# Reconstruct an invalid related specification


1. When a TASK index, TASK item, or its Plan and execution bindings use an old, malformed, or incompatible representation, follow AI cross-file migration. Read the current TASK contracts and rebuild complete current `v1` candidates directly from preserved evidence; neither the source TASK nor the related source files must validate first.
2. Do not use TASK repair preparation as a semantic migration engine, create a Python migration or compatibility adapter, or infer compatibility from a shared schema ID. Preserve IDs, dependencies, technical decisions, records, and lifecycle meaning only when uniquely supported.
3. Ask through the parent only for real semantic ambiguity, such as conflicting TASK identity, dependency, action, acceptance linkage, or lifecycle meaning. Deterministic field placement, canonical ordering, and derivable bindings belong in the candidate without another decision.
4. Keep all affected TASK files with the Plan and execution-index candidates as one indivisible set. Preview, publish and, when separately authorized, recover that set only through the dedicated migration commands; never independently create, repair or publish one TASK candidate.
