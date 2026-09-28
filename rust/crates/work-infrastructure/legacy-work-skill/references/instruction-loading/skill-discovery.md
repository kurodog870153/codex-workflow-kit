<!-- work-compatibility-revision: 1 -->
# Discover and recommend external skills


1. Use `skills catalog` over enabled current skill roots. Read only name, description, metadata, invocation policy, declared dependencies, and summary fingerprints during discovery.
2. Treat skills with identical names but different scope, root, or source as distinct identities.
3. Check dependencies before recommendation. Mark unavailable skills as unselectable; never install packages or connect services automatically.
4. Recommend the smallest set needed by the request. Show identity, description, recommendation reason, source and scope, mode support, dependency status, invocation policy, and estimated context cost.
5. Explicit-only skills may be recommended. User confirmation inside `$work` is explicit authorization to load those confirmed skills for this run.
6. Ask the user to accept, add, remove, or cancel. If no suitable skill exists, ask whether to continue `base_only`.
7. Snapshot confirmed skills and validate `work-skill-selection/v1`. Stop on any drift before or after delegation.
