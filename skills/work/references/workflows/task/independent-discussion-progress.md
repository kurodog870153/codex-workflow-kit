# Shared DiscussionSession

1. All TASKs of one requirement share its committed Session. `$work task -- resume <requirement-id>` and the requirement entry restore the same state through `discussion read`; neither creates an independent checkpoint authority.
2. Preserve unresolved choices and reasons, stable D/TASK links, planning reviews and the exact continuation mapping. Show only pending/current and necessary dependencies; read complete committed history on demand. Saving is covered by bounded authorization and does not grant formal approval or execution.
