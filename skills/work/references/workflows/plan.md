<!-- work-compatibility-revision: 2 -->
# Plan Workflow

1. Use this entry only when it appears in the Python-produced canonical selection manifest.
2. Load only the same-mode operation modules listed after this entry in `source_order`; do not scan this workflow directory or follow Markdown links to discover rules.
3. Treat one `next_action` as one isolated operation context. End the context on state, lifecycle, selection, authorization, role, event, success, failure, blocked, or interrupted change.
4. Return only structured results and evidence to the main flow. Do not carry full instruction text into the next operation.
5. `workflow next` includes a CLI command, argument template, request contract ID and semantic input contract ID when a safe entry point is known. Placeholders require explicit user-provided values. A null command means selection or review must occur before a concrete CLI operation is chosen; no hint grants authorization.
