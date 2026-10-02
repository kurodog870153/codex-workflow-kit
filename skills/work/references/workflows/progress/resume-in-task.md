<!-- work-compatibility-revision: 1 -->
# Resume Task discussion

1. Handle explicit `$work task -- resume <requirement-id>` before the structured planning entry point. Run `progress read --requirement-id <requirement-id> --mode task` and validate Task mode, requirement identity and the complete fixed `context.planning_source`. Missing or malformed progress stops restoration; do not invent a new source.
2. Show saved revision, confirmed decisions, tentative items, open questions, known source problems and continuation point. Return discussion to Task after the requested continuation decision. The saver does not conduct discussion or classify decisions.
3. Resume the same captured Source. Do not refresh external requirement sources or discover replacement selections automatically. Validate original Snapshot bytes and Task-owned choices, Work instructions and skills before source-dependent decisions. A successful progress read retains historical trust and grants no current readiness.
4. Missing or stale formal artifacts permit reading and clarification, while the affected formal operation remains stopped. Use Task planning for initial creation, `$work revise` for confirmed formal revisions and `$work migration` for incompatible artifacts under their normal approvals. Execute requires a complete validated formal TASK.
5. Check any referenced structured draft independently. Surface conflicts with progress for user resolution; do not choose by timestamp, merge decisions or rewrite historical fingerprints. Discussion progress is not a `task_candidate` or formal contract.
