<!-- work-compatibility-revision: 1 -->
# Independent discussion progress


1. A user-requested Plan or Task discussion save, or explicit `resume <requirement-id>`, follows discussion progress. The parent alone invokes the private progress saver with its required runtime configuration, or performs the same role under its fallback. It only faithfully organizes and saves the originating role's supplied content; Plan and Task own decisions and continued discussion.
2. A formal validation, source-drift stop does not prohibit independent progress saving or restoration. This narrow exception preserves the observed stop, unfinished decisions and continuation point without changing or validating formal artifacts. Progress path, integrity, revision and write-authorization checks still apply. The saver cannot repair the blocked operation or grant readiness.
3. Restore progress before normal formal-source gates can prevent reading it. Apply current-source checks before relying on restored decisions; unresolved specification decisions may be discussed without publishing partial formal artifacts. Saving or reading progress is neither a new Work mode nor a formal cross-mode handoff. Reject direct `$work progress-saver` invocation.
