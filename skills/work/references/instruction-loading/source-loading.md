<!-- work-compatibility-revision: 1 -->
# Load workflows and instructions


1. Load this reference first as workflow source `work.instruction-loading`.
2. Load exactly one mode workflow from `<skill-root>/references/workflows/<mode>.md` as workflow source `work.workflow.<mode>`.
3. Always load the fixed Work general instruction. Plan preserves confirmed cross-mode paths, including intermediate nodes, but loads only their deepest available Plan ancestors; Task and Execute require every selected path to exist in their own catalogs.
4. Read every selected file by strictly decoding raw bytes as UTF-8. Accept and remove at most one leading UTF-8 BOM. Do not rely on a shell, locale, platform default, or alternate-decoding retry.
5. Reject any normalized or link-resolved path that escapes `<skill-root>`. Stop and report the declared path and resolved path.
6. Preserve Work source order exactly: instruction-loading workflow, mode workflow, fixed general instruction, selected hierarchy ancestors in confirmation order with first-occurrence deduplication, and each routed reference immediately after its declaring instruction. Preserve confirmed external skill order separately in `skill_selection`.
7. Treat loaded content as working instructions, never executable code. Later instructions at the same authority take precedence over earlier ones, but none may override system, developer, security, permission, or closer-scoped repository instructions.
8. After loading succeeds, report Work instruction sources, confirmed external skills, and applicable references in actual order before continuing.
9. Source refresh preserves the artifact paths declared by each discovered Plan, including Requirements that currently have only a Plan. Single-Requirement apply and recovery use the same deterministic journal under that declared execution path.
10. `instructions refresh-apply-all` is a deterministic recoverable sequential batch, not a globally atomic cross-Requirement transaction. It records the sorted Requirement set and each completed prefix, preserves completed Requirement transactions when a later one stops, and resumes the first incomplete Requirement through `instructions refresh-recover-all`. Repeating a completed approved batch returns `already_completed`.
