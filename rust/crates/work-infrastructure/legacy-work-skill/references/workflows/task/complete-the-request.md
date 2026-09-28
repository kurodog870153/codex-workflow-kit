<!-- work-compatibility-revision: 1 -->
# Complete the request


1. Perform the user's Task request under the loaded instructions.
2. Loading Task instructions alone does not create or modify a TASK document. Create or modify artifacts only when the request and applicable authorization permit it.
3. Do not execute a TASK merely because Task instructions or a TASK document were loaded. Execution requires the Execute workflow and its applicable authorization.
4. Save structured Task draft checkpoints only through the authorized draft CLI. Independent user-requested discussion progress uses the parent-owned progress saver above. An unfinished discussion can be saved only when the user requests it; completing a structured TASK discussion uses one confirmation for the discussed content and its draft save. Neither progress nor draft status grants formal approval.
5. After saving a refined TASK, stop and let the user choose this session or a new session. Return `$work task -- <requirement-id>` and a short progress summary. Do not automatically refine the next TASK.
