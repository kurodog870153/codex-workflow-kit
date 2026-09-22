<!-- work-compatibility-revision: 1 -->
# Build the confirmed selection


After the user confirms choices, invoke `skills selection-build --input-file "<request-path>"` with each configured `--root <scope:locator=path>`. The request contains exactly `decision` (`external_skills` or explicitly confirmed `base_only`) and ordered `skills`. Each choice contains `scope`, `root`, `source`, `recommendation_reason` and explicitly verified `dependency_status: available`. For skills without declared Work modes, also supply confirmed `mode_support` for `plan`, `task` and `execute`, using `inferred` or `unsupported`; never infer these values from a successful snapshot. Declared modes are derived from metadata; any supplied mode map must match them exactly. Use `{"decision":"base_only","skills":[]}` for confirmed base-only; roots may be omitted in that case.

The read-only builder returns the complete `work-skill-selection/v1` in response `data`, preserving choice order and deriving stable identities, descriptions, invocation policy, source fingerprints and `selection_sha256`. It reuses selection validation and rejects source drift during construction. Review the resulting snapshot before relying on it: building a new snapshot does not prove it matches earlier discovery evidence or grant dependency access, loading or write authorization. Preserve the returned object without rebuilding machine fields; existing `skills selection-validate` remains available for later validation.

1. Preserve confirmed skill order and complete stable identities. Do not collapse equal names across roots.
2. Save descriptions, recommendation reasons, mode support, dependency status, invocation policy, summary hash, and bundle hash.
3. Use decision `external_skills` for a non-empty selection and `base_only` only for an explicitly confirmed empty selection.
4. Full selected skill content must not exceed 40% of available Plan context. Stop before loading when the estimate exceeds the limit.
5. After confirmation, read each selected `SKILL.md` completely and resolve only resources required by that skill's instructions.
