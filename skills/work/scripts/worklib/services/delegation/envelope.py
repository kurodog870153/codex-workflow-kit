"""Delegation envelope validation without cross-feature orchestration."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from pydantic import ValidationError

from ...models.common.errors import ExitCode, WorkError
from ...models.delegation import DelegationEnvelopeContract, DelegationValidationContract


ROLES = ("plan", "task-coordinator", "execute", "task-skill", "artifact-editor", "progress-saver")
MAIN_MODES = {"plan": "plan", "task-coordinator": "task", "execute": "execute"}
MARKERS = {**{role: "WORK_DELEGATION_V1" for role in MAIN_MODES},
    "task-skill": "WORK_TASK_SKILL_V1", "artifact-editor": "WORK_ARTIFACT_EDIT_V1",
    "progress-saver": "WORK_PROGRESS_SAVE_V1"}
FIELDS = {
    "schema", "marker", "skill", "role", "sender", "mode",
    "project_root", "skill_root", "request", "context",
}


def fail(message: str) -> None:
    raise WorkError(ExitCode.CONTRACT, "delegation_boundary_mismatch", message)


def build_delegation_envelope(*, role: str, mode: str, request: str,
                              project_root: Path, skill_root: Path,
                              context: dict[str, Any]) -> dict[str, Any]:
    if role not in ROLES:
        fail("Unknown delegation role.")
    sender = "task-coordinator" if role == "task-skill" else "parent"
    return DelegationEnvelopeContract.model_validate({
        "schema": "work-delegation-envelope/v1", "marker": MARKERS[role],
        "skill": "$work", "role": role, "sender": sender, "mode": mode,
        "project_root": str(project_root.resolve()), "skill_root": str(skill_root.resolve()),
        "request": request, "context": context,
    }).to_canonical_dict()


def validate_delegation_envelope(
    value: Any,
    *,
    role: str,
    sender: str,
    project_root: Path,
    skill_root: Path,
) -> tuple[dict[str, Any], str, str, dict[str, Any], bool]:
    try:
        DelegationEnvelopeContract.model_validate(value)
    except ValidationError as error:
        raise WorkError(
            ExitCode.CONTRACT,
            "delegation_boundary_mismatch",
            "The delegation envelope structure is invalid.",
        ) from error
    if role not in ROLES or sender != ("task-coordinator" if role == "task-skill" else "parent"):
        fail("The expected sender cannot delegate to this role.")
    if set(value) != FIELDS:
        fail("delegation must contain exactly the required fields.")
    envelope = value
    if (envelope["schema"] != "work-delegation-envelope/v1" or envelope["marker"] != MARKERS[role]
        or envelope["skill"] != "$work" or envelope["role"] != role or envelope["sender"] != sender):
        fail("Envelope marker, skill, role or sender differs from the receiving context.")
    for field, expected in (("project_root", project_root), ("skill_root", skill_root)):
        declared = Path(envelope[field])
        if not declared.is_absolute() or str(declared) != str(declared.resolve()) or declared.resolve() != expected.resolve():
            fail("Envelope roots must match the resolved receiving roots.")
    request = envelope["request"]
    if not isinstance(request, str) or not request.strip():
        fail("request must be a nonempty string.")
    mode = envelope["mode"]
    allowed = (MAIN_MODES[role],) if role in MAIN_MODES else (("task",) if role == "task-skill" else (
        ("plan", "task") if role == "progress-saver" else ("plan", "task", "execute")))
    if mode not in allowed:
        fail("The role does not accept this mode.")
    context = envelope["context"]
    if not context:
        fail("context must be a nonempty object.")
    return envelope, request, mode, context, "saved_progress" in context


def delegation_validation_result(*, role: str, mode: str, resume: bool) -> dict[str, Any]:
    return DelegationValidationContract(
        schema="work-delegation-validation/v1", status="valid", role=role,
        mode=mode, scope="discussion_restoration" if resume else "role_context",
        source_validation="not_checked", sender_authentication="not_checked",
        grants_authorization=False,
    ).to_canonical_dict()
