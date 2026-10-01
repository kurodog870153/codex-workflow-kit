<!-- work-compatibility-revision: 2 -->
# Validate before relying on a formal TASK

1. Before using an existing formal TASK, run `task validate` with the confirmed collection path and current Plan, skill and instruction sources. Treat a nonzero result as a stop; raw bytes are evidence, not confirmed requirements or executable steps.
2. If the TASK collection or related Plan/Execution artifact is missing, malformed, non-canonical, incompatible or has a broken binding, run read-only `migration analyze --requirement-id <id>`. Review each finding, source path, raw fingerprint and suggested route. Preserve original bytes and unresolved semantic choices.
3. Use Specification only for a valid, trusted baseline that needs an approved semantic revision. Use Migration semantic reconstruction when a valid baseline cannot be established. Incomplete transactions follow only their owning domain's approved recovery command; corruption does not become recovery authorization.
4. Revalidate sources before resuming Execute. Execute still performs its own preflight, lock and fingerprint checks. Neither validation nor analysis grants write or execution approval.
