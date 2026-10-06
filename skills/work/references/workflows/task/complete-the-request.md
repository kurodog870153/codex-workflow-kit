# Complete the request

1. Perform the user's Task request under the loaded instructions and confirmed scope. Loading Task instructions alone grants no artifact writes or execution authority.
2. Persist discussion and local changes within the recorded bounded Session save authorization. Read committed state each turn, preserve unchanged confirmed choices, and ask only after the exact question mapping is committed and verified.
3. Continue the user's authorized scope without treating a normal saved revision or completed TASK discussion as a new approval requirement. If the user stops or switches conversations, return the saved revision, unresolved decisions and `$work task -- resume <requirement-id>`.
4. After all required decisions and planning reviews are complete, show the full formal preview and obtain its distinct publication approval. Saving discussion or confirming requirements never authorizes publication or Execute.
