from __future__ import annotations

from dataclasses import asdict, dataclass
from pathlib import Path

from ...models.workflow import InstructionSelectionManifestContract
from ...models.common.errors import ExitCode, WorkError
from ...technical.foundation.fingerprint import canonical_json_sha256, raw_sha256
from ...technical.infrastructure.file_io import read_raw
from ...technical.infrastructure.text_codec import canonical_sha256


ROUTER_COMPATIBILITY_REVISION = 3
BOOTSTRAP = "work.instruction-loading"

MODES = frozenset({"plan", "task", "execute", "progress", "repair", "specification"})
ROLES = frozenset({
    "main", "explorer", "worker", "reviewer", "monitor", "plan",
    "task-coordinator", "execute", "task-skill", "artifact-editor", "progress-saver",
})
AUTHORIZATION_STATES = frozenset({"read_only", "confirmation_required", "authorized"})
FORMAL_EVENTS = frozenset({
    "invocation", "handoff", "progress_read", "progress_save", "migration",
    "revision", "reconciliation", "recovery", "correction", "command_correction",
    "command_execution", "attempt_start", "attempt_close", "delegation",
    "invalid_artifact", "skill_load", "safety_rejection", "file_failure",
})
TERMINAL_EVENTS = frozenset({"handoff", "recovery", "safety_rejection"})


@dataclass(frozen=True)
class RoutingInput:
    mode: str
    status: str
    operation: str
    artifact_lifecycle: str
    formal_events: tuple[str, ...]
    role: str
    authorization_state: str
    verified_state_sha256: str


def build_verified_state_sha256(value: object) -> str:
    return canonical_json_sha256(value)


def build_raw_state_sha256(value: bytes) -> str:
    return raw_sha256(value)

SOURCE_CATALOG: dict[str, tuple[str, int, int]] = {
    "work.instruction-loading": ("references/instruction-loading.md", 2, 0),
    "work.workflow.specification": ("references/workflows/specification.md", 2, 100),
    "work.workflow.progress": ("references/workflows/progress.md", 2, 100),
    "work.workflow.task": ("references/workflows/task.md", 2, 100),
    "work.workflow.execute": ("references/workflows/execute.md", 2, 100),
    "work.workflow.plan": ("references/workflows/plan.md", 2, 100),
    "work.workflow.repair": ("references/workflows/repair.md", 2, 100),
    "work.workflow.task-drafts": ("references/workflows/task-drafts.md", 2, 100),
    "work.shared.cli-response": ("references/instruction-loading/cli-response.md", 1, 10),
    "work.shared.transaction-workspace": ("references/instruction-loading/transaction-workspace.md", 1, 11),
    "work.shared.invocation": ("references/instruction-loading/invocation.md", 1, 12),
    "work.shared.private-roles": ("references/instruction-loading/private-roles.md", 1, 13),
    "work.shared.internal-envelope": ("references/instruction-loading/internal-envelope.md", 1, 14),
    "work.shared.formal-task-validation": ("references/instruction-loading/formal-task-validation.md", 1, 15),
    "work.shared.discussion-progress": ("references/instruction-loading/discussion-progress.md", 1, 16),
    "work.shared.artifact-migration": ("references/instruction-loading/artifact-migration.md", 1, 17),
    "work.shared.artifact-revision": ("references/instruction-loading/artifact-revision.md", 1, 18),
    "work.shared.runtime": ("references/instruction-loading/runtime.md", 1, 19),
    "work.shared.cli-transport": ("references/instruction-loading/cli-transport.md", 1, 20),
    "work.shared.handoff": ("references/instruction-loading/handoff.md", 1, 21),
    "work.shared.skill-discovery": ("references/instruction-loading/skill-discovery.md", 1, 22),
    "work.shared.skill-selection": ("references/instruction-loading/skill-selection.md", 1, 23),
    "work.shared.source-loading": ("references/instruction-loading/source-loading.md", 1, 24),
    "work.shared.routed-references": ("references/instruction-loading/routed-references.md", 1, 25),
    "work.shared.artifact-paths": ("references/instruction-loading/artifact-paths.md", 1, 26),
    "work.shared.fingerprints": ("references/instruction-loading/fingerprints.md", 1, 27),
    "work.workflow.specification.active-task-collection-revision-boundary": ("references/workflows/specification/active-task-collection-revision-boundary.md", 1, 200),
    "work.workflow.specification.prepare-and-review": ("references/workflows/specification/prepare-and-review.md", 2, 201),
    "work.workflow.specification.publish-and-recover": ("references/workflows/specification/publish-and-recover.md", 1, 202),
    "work.workflow.specification.specification-verification": ("references/workflows/specification/specification-verification.md", 1, 203),
    "work.workflow.progress.ownership-and-identity": ("references/workflows/progress/ownership-and-identity.md", 1, 204),
    "work.workflow.progress.content-and-saving": ("references/workflows/progress/content-and-saving.md", 1, 205),
    "work.workflow.progress.resume-in-plan-or-task": ("references/workflows/progress/resume-in-plan-or-task.md", 1, 206),
    "work.workflow.task.reconstruct-an-invalid-related-specification": ("references/workflows/task/reconstruct-an-invalid-related-specification.md", 1, 207),
    "work.workflow.task.independent-discussion-progress": ("references/workflows/task/independent-discussion-progress.md", 1, 208),
    "work.workflow.task.saved-planning-entry-point": ("references/workflows/task/saved-planning-entry-point.md", 1, 209),
    "work.workflow.task.coordinate-confirmed-skills": ("references/workflows/task/coordinate-confirmed-skills.md", 1, 210),
    "work.workflow.task.complete-the-request": ("references/workflows/task/complete-the-request.md", 1, 211),
    "work.workflow.task.use-the-deterministic-task-contract": ("references/workflows/task/use-the-deterministic-task-contract.md", 1, 212),
    "work.workflow.task.request-coordinated-revision": ("references/workflows/task/request-coordinated-revision.md", 1, 213),
    "work.workflow.task.use-deterministic-handoffs": ("references/workflows/task/use-deterministic-handoffs.md", 1, 214),
    "work.workflow.execute.handle-invalid-related-artifacts-before-execution": ("references/workflows/execute/handle-invalid-related-artifacts-before-execution.md", 1, 215),
    "work.workflow.execute.load-only-the-target-skill": ("references/workflows/execute/load-only-the-target-skill.md", 1, 216),
    "work.workflow.execute.identify-the-execution-target": ("references/workflows/execute/identify-the-execution-target.md", 1, 217),
    "work.workflow.execute.run-deterministic-read-only-eligibility-checks": ("references/workflows/execute/run-deterministic-read-only-eligibility-checks.md", 1, 218),
    "work.workflow.execute.request-coordinated-revision": ("references/workflows/execute/request-coordinated-revision.md", 1, 219),
    "work.workflow.execute.apply-one-attempt-authorization-boundary": ("references/workflows/execute/apply-one-attempt-authorization-boundary.md", 2, 220),
    "work.workflow.execute.use-deterministic-handoffs": ("references/workflows/execute/use-deterministic-handoffs.md", 1, 221),
    "work.workflow.execute.use-the-canonical-attempt-contract": ("references/workflows/execute/use-the-canonical-attempt-contract.md", 1, 222),
    "work.workflow.execute.start-an-attempt-transaction": ("references/workflows/execute/start-an-attempt-transaction.md", 1, 223),
    "work.workflow.execute.reserve-one-execution-record": ("references/workflows/execute/reserve-one-execution-record.md", 1, 224),
    "work.workflow.execute.record-an-equivalent-command-correction": ("references/workflows/execute/record-an-equivalent-command-correction.md", 1, 225),
    "work.workflow.execute.execute-one-authorized-argv-cmd": ("references/workflows/execute/execute-one-authorized-argv-cmd.md", 1, 226),
    "work.workflow.execute.record-one-execution-result": ("references/workflows/execute/record-one-execution-result.md", 1, 227),
    "work.workflow.execute.close-one-attempt": ("references/workflows/execute/close-one-attempt.md", 2, 228),
    "work.workflow.execute.create-an-immutable-correction": ("references/workflows/execute/create-an-immutable-correction.md", 1, 229),
    "work.workflow.execute.recover-one-execution-transaction": ("references/workflows/execute/recover-one-execution-transaction.md", 1, 230),
    "work.workflow.plan.reconstruct-an-invalid-related-specification": ("references/workflows/plan/reconstruct-an-invalid-related-specification.md", 1, 231),
    "work.workflow.plan.apply-confirmed-skills": ("references/workflows/plan/apply-confirmed-skills.md", 1, 232),
    "work.workflow.plan.complete-the-request": ("references/workflows/plan/complete-the-request.md", 1, 233),
    "work.workflow.plan.resume-discussion-progress": ("references/workflows/plan/resume-discussion-progress.md", 1, 234),
    "work.workflow.plan.use-the-deterministic-plan-contract": ("references/workflows/plan/use-the-deterministic-plan-contract.md", 1, 235),
    "work.workflow.plan.request-coordinated-revision": ("references/workflows/plan/request-coordinated-revision.md", 1, 236),
    "work.workflow.plan.use-deterministic-handoffs": ("references/workflows/plan/use-deterministic-handoffs.md", 1, 237),
    "work.workflow.repair.rebuild-a-related-artifact-set-with-ai": ("references/workflows/repair/rebuild-a-related-artifact-set-with-ai.md", 1, 238),
    "work.workflow.repair.prepare-the-request": ("references/workflows/repair/prepare-the-request.md", 1, 239),
    "work.workflow.repair.programmatic-preparation": ("references/workflows/repair/programmatic-preparation.md", 1, 240),
    "work.workflow.repair.review-approve-and-publish": ("references/workflows/repair/review-approve-and-publish.md", 1, 241),
    "work.workflow.repair.authorized-recovery": ("references/workflows/repair/authorized-recovery.md", 1, 242),
    "work.workflow.task-drafts.read-and-verify": ("references/workflows/task-drafts/read-and-verify.md", 1, 243),
    "work.workflow.task-drafts.initialize-and-save": ("references/workflows/task-drafts/initialize-and-save.md", 1, 244),
    "work.workflow.task-drafts.file-and-failure-semantics": ("references/workflows/task-drafts/file-and-failure-semantics.md", 1, 245),
    "work.workflow.task-drafts.formalization-boundary": ("references/workflows/task-drafts/formalization-boundary.md", 1, 246),
}

DECISION_TABLE: dict[str, tuple[str, ...]] = {
    "prepare_plan": ("work.instruction-loading", "work.workflow.plan", "work.shared.invocation", "work.shared.skill-discovery", "work.shared.skill-selection", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.plan.apply-confirmed-skills", "work.workflow.plan.complete-the-request", "work.workflow.plan.use-the-deterministic-plan-contract"),
    "confirm_task_list": ("work.instruction-loading", "work.workflow.task", "work.workflow.task-drafts", "work.shared.invocation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.task.saved-planning-entry-point", "work.workflow.task-drafts.initialize-and-save"),
    "choose_task": ("work.instruction-loading", "work.workflow.task", "work.workflow.task-drafts", "work.shared.invocation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.task.coordinate-confirmed-skills", "work.workflow.task-drafts.read-and-verify"),
    "confirm_start": ("work.instruction-loading", "work.workflow.task", "work.workflow.task-drafts", "work.shared.invocation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.task.coordinate-confirmed-skills", "work.workflow.task-drafts.initialize-and-save"),
    "confirm_resume": ("work.instruction-loading", "work.workflow.task", "work.workflow.task-drafts", "work.shared.discussion-progress", "work.shared.invocation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.task.independent-discussion-progress", "work.workflow.task.saved-planning-entry-point"),
    "confirm_review": ("work.instruction-loading", "work.workflow.task", "work.workflow.task-drafts", "work.shared.invocation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.task.complete-the-request", "work.workflow.task-drafts.formalization-boundary"),
    "assemble_for_review": ("work.instruction-loading", "work.workflow.task", "work.workflow.task-drafts", "work.shared.invocation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.task.use-the-deterministic-task-contract", "work.workflow.task-drafts.formalization-boundary"),
    "select_task_for_execution": ("work.instruction-loading", "work.workflow.execute", "work.shared.formal-task-validation", "work.shared.source-loading", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.execute.identify-the-execution-target", "work.workflow.execute.run-deterministic-read-only-eligibility-checks"),
    "continue_execution": ("work.instruction-loading", "work.workflow.execute", "work.shared.formal-task-validation", "work.shared.runtime", "work.shared.cli-transport", "work.shared.transaction-workspace", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.execute.apply-one-attempt-authorization-boundary", "work.workflow.execute.reserve-one-execution-record", "work.workflow.execute.record-one-execution-result", "work.workflow.execute.close-one-attempt"),
    "confirm_retry": ("work.instruction-loading", "work.workflow.execute", "work.shared.formal-task-validation", "work.shared.runtime", "work.shared.transaction-workspace", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.execute.apply-one-attempt-authorization-boundary", "work.workflow.execute.start-an-attempt-transaction"),
    "inspect_recovery": ("work.instruction-loading", "work.workflow.repair", "work.shared.transaction-workspace", "work.shared.runtime", "work.shared.artifact-migration", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.repair.rebuild-a-related-artifact-set-with-ai", "work.workflow.repair.prepare-the-request", "work.workflow.repair.programmatic-preparation", "work.workflow.repair.review-approve-and-publish", "work.workflow.repair.authorized-recovery"),
    "review_reconciliation": ("work.instruction-loading", "work.workflow.specification", "work.shared.artifact-revision", "work.shared.transaction-workspace", "work.shared.artifact-paths", "work.shared.fingerprints", "work.workflow.specification.active-task-collection-revision-boundary", "work.workflow.specification.prepare-and-review", "work.workflow.specification.publish-and-recover", "work.workflow.specification.specification-verification"),
    "review_completion": ("work.instruction-loading", "work.workflow.execute", "work.shared.formal-task-validation", "work.shared.fingerprints", "work.workflow.execute.close-one-attempt"),
}

MODE_SOURCES: dict[str, tuple[str, ...]] = {
    "progress": (
        "work.workflow.progress", "work.workflow.progress.ownership-and-identity",
        "work.workflow.progress.content-and-saving",
    ),
}

EVENT_SOURCES: dict[str, tuple[str, ...]] = {
    "invocation": ("work.shared.invocation", "work.shared.cli-response"),
    "handoff": ("work.shared.handoff", "work.workflow.task.use-deterministic-handoffs",
                "work.workflow.plan.use-deterministic-handoffs",
                "work.workflow.execute.use-deterministic-handoffs"),
    "progress_read": ("work.shared.discussion-progress",
                      "work.workflow.progress.ownership-and-identity",
                      "work.workflow.progress.resume-in-plan-or-task",
                      "work.workflow.plan.resume-discussion-progress"),
    "progress_save": ("work.shared.discussion-progress",
                      "work.workflow.progress.ownership-and-identity",
                      "work.workflow.progress.content-and-saving"),
    "migration": ("work.shared.artifact-migration",),
    "revision": ("work.shared.artifact-revision",
                 "work.workflow.task.request-coordinated-revision",
                 "work.workflow.plan.request-coordinated-revision",
                 "work.workflow.execute.request-coordinated-revision"),
    "reconciliation": ("work.shared.artifact-revision",),
    "recovery": ("work.shared.transaction-workspace",
                 "work.workflow.execute.recover-one-execution-transaction"),
    "correction": ("work.workflow.execute.create-an-immutable-correction",),
    "command_correction": ("work.workflow.execute.record-an-equivalent-command-correction",),
    "command_execution": ("work.shared.cli-transport",
                          "work.workflow.execute.execute-one-authorized-argv-cmd"),
    "attempt_start": ("work.workflow.execute.use-the-canonical-attempt-contract",
                      "work.workflow.execute.start-an-attempt-transaction"),
    "attempt_close": ("work.workflow.execute.use-the-canonical-attempt-contract",
                      "work.workflow.execute.close-one-attempt"),
    "delegation": ("work.shared.private-roles", "work.shared.internal-envelope"),
    "invalid_artifact": (
        "work.workflow.task.reconstruct-an-invalid-related-specification",
        "work.workflow.plan.reconstruct-an-invalid-related-specification",
        "work.workflow.execute.handle-invalid-related-artifacts-before-execution",
    ),
    "skill_load": ("work.workflow.execute.load-only-the-target-skill",),
    "safety_rejection": ("work.shared.routed-references",),
    "file_failure": ("work.workflow.task-drafts.file-and-failure-semantics",),
}


class RoutingSourceSession:
    """Source fingerprints shared within one routing build."""

    def __init__(self, skill_root: Path) -> None:
        self.skill_root = skill_root
        self._sources: dict[str, tuple[str, str]] = {}

    def _read(self, logical_name: str) -> bytes:
        relative = SOURCE_CATALOG[logical_name][0]
        try:
            return read_raw(self.skill_root / relative)
        except (OSError, WorkError) as error:
            raise WorkError(
                ExitCode.ARTIFACT_INTEGRITY,
                "routed_instruction_source_missing",
                "A routed instruction source is missing or unreadable.",
                {"logical_name": logical_name, "path": relative},
            ) from error

    def fingerprint(self, logical_name: str) -> str:
        if logical_name not in self._sources:
            path = self.skill_root / SOURCE_CATALOG[logical_name][0]
            raw = self._read(logical_name)
            self._sources[logical_name] = (
                raw_sha256(raw), canonical_sha256(raw, source=str(path)),
            )
        return self._sources[logical_name][1]

    def recheck(self) -> None:
        for logical_name, (expected_raw_sha256, _) in self._sources.items():
            if raw_sha256(self._read(logical_name)) != expected_raw_sha256:
                raise WorkError(
                    ExitCode.ARTIFACT_INTEGRITY,
                    "routed_instruction_source_changed",
                    "A routed instruction source changed during routing construction.",
                    {"logical_name": logical_name},
                )


def _infer_mode(next_action: str) -> str:
    if next_action == "prepare_plan":
        return "plan"
    if next_action in {"confirm_task_list", "choose_task", "confirm_start", "confirm_resume",
                       "confirm_review", "assemble_for_review"}:
        return "task"
    if next_action == "inspect_recovery":
        return "repair"
    if next_action == "review_reconciliation":
        return "specification"
    return "execute"


def _review_reasons(value: RoutingInput) -> list[str]:
    reasons: list[str] = []
    if value.mode not in MODES:
        reasons.append(f"unknown_mode:{value.mode}")
    if value.role not in ROLES:
        reasons.append(f"unknown_role:{value.role}")
    if value.authorization_state not in AUTHORIZATION_STATES:
        reasons.append(f"unknown_authorization_state:{value.authorization_state}")
    unknown_events = sorted(set(value.formal_events) - FORMAL_EVENTS)
    reasons.extend(f"unknown_event:{event}" for event in unknown_events)
    terminal = sorted(set(value.formal_events) & TERMINAL_EVENTS)
    if len(terminal) > 1:
        reasons.append("conflicting_terminal_events:" + ",".join(terminal))
    if value.role != "main" and "delegation" not in value.formal_events:
        reasons.append("delegated_role_without_delegation_event")
    if value.authorization_state == "authorized" and value.operation not in DECISION_TABLE:
        reasons.append("authorized_unknown_operation")
    if not value.status or not value.operation or not value.artifact_lifecycle:
        reasons.append("incomplete_routing_input")
    return reasons


def build_routing_selection(skill_root: Path, *, status: str, next_action: str,
                            confirmation: bool, mode: str | None = None,
                            artifact_lifecycle: str = "current",
                            formal_events: tuple[str, ...] = (), role: str = "main",
                            authorization_state: str | None = None,
                            verified_state_sha256: str = "",
                            source_session: RoutingSourceSession | None = None) -> dict[str, object]:
    routing_input = RoutingInput(
        mode=mode or _infer_mode(next_action), status=status, operation=next_action,
        artifact_lifecycle=artifact_lifecycle,
        formal_events=tuple(sorted(set(formal_events))), role=role,
        authorization_state=authorization_state or (
            "confirmation_required" if confirmation else "read_only"
        ),
        verified_state_sha256=verified_state_sha256,
    )
    review_reasons = _review_reasons(routing_input)
    base_sources = DECISION_TABLE.get(next_action)
    if base_sources is None:
        review_reasons.append(f"unmatched_operation:{next_action}")
    if review_reasons:
        logical_names = (BOOTSTRAP,)
        routing_status = "REVIEW_REQUIRED"
        reasons = review_reasons
    else:
        additions = MODE_SOURCES.get(routing_input.mode, ())
        for event in routing_input.formal_events:
            additions += EVENT_SOURCES[event]
        logical_names = tuple(dict.fromkeys((*base_sources, *additions)))
        routing_status = "VALID"
        reasons = [f"operation:{next_action}", f"workflow_status:{status}",
                   f"mode:{routing_input.mode}"]
        reasons.extend(f"event:{event}" for event in routing_input.formal_events)
    ordered = tuple(sorted(logical_names, key=lambda name: SOURCE_CATALOG[name][2]))
    sources_for_build = source_session or RoutingSourceSession(skill_root)
    if sources_for_build.skill_root != skill_root:
        raise WorkError(
            ExitCode.CONTRACT, "routed_instruction_root_mismatch",
            "The routing source session belongs to another skill root.",
        )
    sources: list[dict[str, object]] = []
    for logical_name in ordered:
        relative, revision, _order = SOURCE_CATALOG[logical_name]
        sources.append({
            "logical_name": logical_name,
            "path": relative,
            "compatibility_revision": revision,
            "canonical_sha256": sources_for_build.fingerprint(logical_name),
        })
    manifest: dict[str, object] = {
        "schema": "work-instruction-selection-manifest/v1",
        "router_compatibility_revision": ROUTER_COMPATIBILITY_REVISION,
        "routing_input": asdict(routing_input),
        "routing_status": routing_status,
        "sources": sources,
        "confirmation_required": confirmation,
    }
    selection_sha256 = canonical_json_sha256(manifest)
    manifest["selection_sha256"] = selection_sha256
    manifest = InstructionSelectionManifestContract.model_validate(manifest).to_canonical_dict()
    return {
        "routing_status": routing_status,
        "router_compatibility_revision": ROUTER_COMPATIBILITY_REVISION,
        "required_instruction_sources": list(ordered),
        "source_order": list(ordered),
        "selection_sha256": selection_sha256,
        "routing_reasons": reasons,
        "selection_manifest": manifest,
    }
