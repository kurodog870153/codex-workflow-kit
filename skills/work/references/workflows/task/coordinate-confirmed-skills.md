# Coordinate confirmed skills

1. Discover and recommend skills within Task planning, then obtain explicit confirmation of the Task-owned skill selection before using it. The immutable Source preserves the requirement; it does not select skills or instructions.
2. Split work into minimum independently verifiable TASK outcomes and confirm boundaries, dependencies and main acceptance responsibilities.
3. Bind each TASK to exactly one confirmed executable skill ID. Use `null` only for explicitly confirmed base-only work.
4. Give each TASK only applicable paths authorized by the Task-owned hierarchy selection. An empty selection loads `general`; a non-empty path must be confirmed, an ancestor or a descendant and exist in both Task and Execute catalogs. A shared ancestor does not authorize sibling branches.
5. A required skill must support Task and Execute. Return missing or unsupported skills to Task planning for an explicit selection decision; never silently replace a skill.
6. For the selected TASK, supply the complete TASK boundary, its one confirmed skill snapshot, fixed Source, current Work instructions, repository evidence and saved discussion under the private delegation contract. Inherit model and reasoning settings and use its fallback when delegation is unavailable.
7. Merge the result into the selected TASK discussion. Resolve semantic conflicts through user decisions. Formalize only after all TASK candidates satisfy the complete contract and receive approval.
