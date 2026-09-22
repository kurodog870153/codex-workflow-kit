<!-- work-compatibility-revision: 1 -->
# Specification verification


1. After a successful ordinary `spec-update` or its explicitly authorized recovery,
   immediately transport the returned `verification_request` unchanged to the read-only `task
   spec-verify --input-file "<request-path>" --user-config-root "<user-config-root>"`
   command with the same project and confirmed skill roots without requesting another routine confirmation. The request contains
   exactly `schema: "work-spec-verification-request/v1"`, `requirement_id`, all three
   artifact paths and the actual `record_id`.
2. The verifier checks the canonical ordinary-update journal and completion marker,
   exact installed Plan/TASK/index bytes, current formal contracts and source binding,
   derived transaction evidence, preserved execution history, all transaction
   retain their separate verification procedure.
3. Success returns `work-spec-verification/v1`, `verified: true`, verification scope
   `exact_specification_result`, `execution_authorized: false` and next step
   `normal_execute_preflight`. Failed or unavailable required checks return exit 5
   with `specification_verification_failed` and the complete report.
4. Verification is an immediate exact-result check. A later legitimate specification
   revision or execution history change can make an older record inapplicable; inspect
   the cause rather than rolling back or reusing a stale request. Verification never
   grants Execute approval or modifies formal, transaction, history or Git state.
