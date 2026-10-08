# CLI response loading


1. Successful Work CLI commands return a brief projection by default. Follow `status` and `next_action`, retaining artifact identities, approval or source fingerprints, confirmation gates and recovery fields.
2. Add the global `--verbose` option immediately after `--project-root <project-root>` only when a workflow step explicitly requires the complete canonical candidate/evidence, or when debugging a reviewed result. Do not request verbose output merely to repeat large evidence already retained in the transaction workspace.
3. Error, safety-rejection and recovery contexts retain the evidence needed to diagnose or recover. Compact output never grants authorization and never permits skipping source validation, approval-fingerprint checks, writer locks or recovery gates.
4. Exclusively retain each full `work-cli-result` as `<returned data.paths.responses>/<NNN>-<command>.json`, including failures. Allocate a new increasing step for each invocation. Save reviewed generated requests separately in `data.paths.requests` and canonical private-role envelopes in `data.paths.envelopes`; a response envelope is not a formal document. Preserve original attachments in `data.paths.inputs` without re-encoding.
