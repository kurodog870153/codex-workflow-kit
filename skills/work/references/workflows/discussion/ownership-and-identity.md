# Ownership and identity

1. Task owns one DiscussionSession per explicit portable requirement ID, shared across TASK plans and stable decisions. The main flow manages user-facing questions and authorized saves; private refiners return evidence without taking saving or publication authority.
2. Record bounded save authorization for the canonical project root, exact discussions directory, allowed actions and evidence. Reject missing/revoked/out-of-scope authorization. Preserve fixed Source, main acceptance, hierarchy and skills independently from discussion state.
3. Current Session plus its exact immutable history are the sole authority. Do not read, convert or merge retired Progress/TaskDraft outputs. They remain untouched evidence and confer no new authority.
