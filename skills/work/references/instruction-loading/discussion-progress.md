<!-- work-compatibility-revision: 1 -->
# Independent Task discussion progress

1. Task alone owns discussion progress. An explicit save or `$work task -- resume <requirement-id>` uses the private progress-saver under the main flow's actual runtime ownership, or the same role's fallback. The saver faithfully preserves supplied content; it does not choose requirements, skills or decisions.
2. Progress uses Task mode and the fixed `context.planning_source`: captured Source snapshot, artifact routes, independently confirmed hierarchy/skill selections and main acceptance criteria. Save does not fetch requirement sources again or silently replace this context. Revision, path, canonical integrity and concrete save approval remain mandatory.
3. Read progress before formal-source failures can prevent restoration. Preserve the observed failure, unfinished decisions and continuation point as historical context. Revalidate the original Source and current choices/instructions before source-dependent decisions; a successful read grants no formal readiness or write approval.
4. Progress is neither a public mode, formal TASK candidate nor cross-mode handoff. Reject direct private-role activation and any legacy mode. A saver cannot repair invalid artifacts, overwrite history, merge conflicting drafts or continue execution.
