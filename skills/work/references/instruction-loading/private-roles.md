<!-- work-compatibility-revision: 1 -->
# Shared private role rules


Read and apply this section in every private Work role, including Task skill refinement, artifact editing and parent fallback. The delegating parent or coordinator supplies this shared source alongside the role prompt; this does not authorize loading other roles or unconfirmed skills.

1. Private roles are internal implementation details, never user-invocable skills, custom agent profiles or additional Work modes. Do not register or install them. Each role prompt defines its required runtime configuration, accepted envelope, skill-loading boundary and permitted delegation.
2. Invocation, selection, delegation and handoff are flow control, not authorization for artifact writes, external operations, installation, implementation or state transitions. Preserve applicable system, developer, repository, permission and loaded instruction boundaries; never expand the confirmed scope or a role's authority.
3. Return user-facing questions, decisions and results to the delegating parent or coordinator in Traditional Chinese. Keep machine-readable fields, statuses, CLI arguments and JSON in English. Preserve confirmed decisions and saved evidence instead of replaying settled questions.
4. Use the Work Python CLI for every deterministic operation it supports within the role's permitted scope. This does not give a refinement-only role authority to save, formalize or approve artifacts, nor an editor authority to execute TASK records.
