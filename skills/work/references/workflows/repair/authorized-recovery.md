<!-- work-compatibility-revision: 1 -->
# Authorized recovery


1. On interruption, preserve the original request, approval fingerprint, journal and current artifacts. Do not retry apply or roll back automatically. Report `task_repair_interrupted`, the record path and `recovery_required`.
2. After separate recovery authorization, run `task repair-recover` with the identical request and fingerprint. Recovery accepts only the exact approved before/after artifact bytes, preserves historical execution, rejects changed source/instruction evidence and completes only matching partial journal/temp/marker prefixes. Unrelated incomplete transactions remain blockers.
3. Recovery returns `recovered` or `already_completed`; conflicting bytes remain untouched. Return the resulting diagnostics to the originating role at the saved continuation point. Recovery approval does not authorize new repair decisions or execution.
