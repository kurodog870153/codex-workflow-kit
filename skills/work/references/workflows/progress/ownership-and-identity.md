<!-- work-compatibility-revision: 1 -->
# Ownership and identity


1. Plan and Task own discussion, decision status and resumption. They send the complete content to the parent, which alone invokes the private progress saver. Task skill subagents return their content through the Task coordinator. Neither Plan nor Task directly spawns this saver.
2. The parent reads the progress saver prompt, pauses the originating role, and invokes one ephemeral saver with that prompt's model and reasoning configuration and `WORK_PROGRESS_SAVE_V1` envelope. If the required runtime is unavailable, the parent follows the same prompt directly with its current runtime. Do not request a public mode switch or copied handoff JSON.
3. Preserve an explicit requirement ID. Before an initial Plan has an ID, ask the user for a safe ID for this progress file; apply the shared requirement-ID filename rules. Choosing it authorizes neither a formal Plan nor formal artifact paths. Retain it on resume and obtain the normal complete-content approval before formal Plan creation; do not ask the user to repeat an unchanged ID.
4. Each mode has independent storage at `outputs/work/progress/<requirement-id>/<mode>/progress.json`, with immutable `history/<revision>/progress.json` copies. Use fixed paths only. Progress has no executable status and is not a formal Plan, TASK, planning index, structured Task draft or cross-mode handoff.
