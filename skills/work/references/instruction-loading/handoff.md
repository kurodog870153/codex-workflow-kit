<!-- work-compatibility-revision: 1 -->
# Shared handoff procedure


Apply this procedure to the matching commands in the selected workflow's **Use deterministic handoffs** section. Internal role envelopes use the separate internal envelope contract.

1. Use the CLI file request and JSON response contract above. Preserve incoming handoff JSON unchanged. Select the receiver's paths, TASK and Attempt/preflight context independently of the incoming handoff, and supply every confirmed skill root to source verification.
2. Before relying on formal source claims, run the matching `handoff verify-*` command and require `work-handoff-source-validation/v1` with `status: valid`. The verifier checks current sources, identities, fingerprints and applicable affected IDs. A mismatch stops the source-dependent operation; never rewrite handoff fields to make verification pass.
3. For saved formal sources, use the matching `handoff build-*` command with semantic input only. Let the program derive machine fields and validate sources; do not reconstruct identities, paths, statuses or fingerprints in the model. Each workflow defines its command, required context and supported source state.
4. Relay only the rendered JSON from successful response `data` in one conversation code block, without edits. Preserve the `WORK-HANDOFF` marker, actual requirement ID, all artifact paths, skill-selection fingerprint and any applicable single skill ID. The handoff exists only in the conversation and never modifies Plan, TASK, index, Attempt or lock state.
5. Task-to-Plan returns require `source.task_sha256`; closed Execute returns also require `source.attempt_sha256`, binding the exact Attempt bytes as well as ID and status. Older handoffs missing evidence may pass `handoff validate` for format only. Unsaved discussion or revisions remain unverified discussion input; clarify their baseline and regenerate from saved formal sources when available. Never insert current hashes as historical evidence.
6. Source verification does not approve semantic proposals, artifact writes or execution. It does not replace Execute preflight, worktree review or Attempt authorization. Confirmed same-session formal revisions use the coordinated editor procedure, preserving each role's scope.
