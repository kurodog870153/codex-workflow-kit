<!-- work-compatibility-revision: 1 -->
# Apply confirmed skills


1. Treat `skill_selection` as final for this Plan run. Do not rediscover skills in the subagent.
2. Read each selected `SKILL.md` completely only after confirmation. Confirmation through `$work` explicitly authorizes loading an explicit-only skill for this run.
3. Stop when a dependency is unavailable, a snapshot changed, selected skill content exceeds 40% of available Plan context, or loaded skills contain unresolved conflicting instructions.
4. Plan-only skills are valid. A selected skill does not need Task or Execute support unless the proposed Plan requires it in those phases.
5. For `base_only`, load no external skill and proceed only after the recorded explicit confirmation.
