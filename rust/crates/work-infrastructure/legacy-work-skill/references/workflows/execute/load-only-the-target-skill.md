<!-- work-compatibility-revision: 1 -->
# Load only the target skill


1. Use only the target TASK `skill_id`. `null` means base-only and loads no external skill.
2. Resolve a non-null ID only from the source Plan `skill_selection`. Do not scan, recommend, or combine other skills.
3. Require Execute mode support, available dependencies, the same root identity, and unchanged summary and bundle fingerprints.
4. Pass the same `--skill-root` values to every Execute CLI command. Any missing root or drift is a hard stop.
