<!-- work-compatibility-revision: 1 -->
# Coordinate confirmed skills


1. Task skill discovery is forbidden. Use only skills in the source Plan `skill_selection`.
2. Split work into minimum independently verifiable TASK outcomes before delegation.
3. Bind each TASK to one `skill_id`. Use `null` only for base-only work that needs no external skill.
4. Give each TASK only applicable paths authorized by the source Plan hierarchy selection. An empty selection loads `general`; a non-empty path must be a confirmed path, one of its ancestors, or one of its descendants and must exist in both Task and Execute catalogs. Sharing an ancestor with a confirmed path does not authorize a sibling branch.
5. Skip Plan-only skills when producing executable TASKs. A required skill with Task mode `unsupported` must return to Plan for a new decision.
6. For the current TASK's executable skill, follow the Task coordinator's delegation contract and Task skill subagent prompt, including their runtime configuration and fallback. Supply one skill, one TASK boundary and the relevant saved discussion.
7. Merge the current subagent output into that TASK's discussion. Resolve conflicts through user decisions. After all TASKs are refined, perform the existing complete-contract validation and approval process.
