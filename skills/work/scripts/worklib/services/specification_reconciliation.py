"""Preview and publish specification reconciliation for immutable Attempts."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from ..contracts.attempt import render_attempt_json_contract
from ..contracts.specification_reconciliation_models import (
    SpecificationReconciliationPreviewContract,
    SpecificationReconciliationPreviewRequestContract,
    SpecificationReconciliationPublicationContract,
)
from ..models.common.errors import ExitCode, WorkError
from ..foundation.fingerprint import canonical_json_sha256, raw_sha256, read_raw
from ..foundation.markdown import render_json_contract
from ..foundation.paths import resolve_project_relative_path
from .specification_migration import preview_specification_migration, publish_specification_migration


def _fail(code: str, message: str, **details: object) -> None:
    raise WorkError(ExitCode.ARTIFACT_INTEGRITY, code, message, details)


def preview_specification_reconciliation(
    raw_request: bytes, *, project_root: Path, user_config_root: str, skill_roots=None,
) -> dict[str, object]:
    request = SpecificationReconciliationPreviewRequestContract.parse_json_bytes(
        raw_request, source="specification reconciliation preview",
    ).to_canonical_dict()
    attempt_path, resolved = resolve_project_relative_path(
        project_root, request["attempt_path"], field="attempt_path",
    )
    attempt_raw = read_raw(resolved)
    attempt = render_attempt_json_contract(attempt_raw, source=attempt_path, project_root=project_root)
    if attempt["status"] == "in_progress":
        _fail("reconciliation_attempt_open", "Reconciliation requires a closed Attempt.")
    deviations = [
        row for row in attempt.get("execution_deviations", [])
        if row["decision"]["outcome"] == "approved" and row["reconciliation_status"] == "pending"
    ]
    pending = [row["deviation_id"] for row in deviations]
    if not pending:
        _fail("reconciliation_nothing_pending", "The Attempt has no approved pending deviations.")
    if request["choice"] == "all":
        selected = pending
    elif request["choice"] == "selective":
        selected = request["deviation_ids"]
        unknown = sorted(set(selected) - set(pending))
        if unknown:
            _fail("reconciliation_unknown_deviation", "Selected deviations are not approved and pending.", ids=unknown)
    else:
        selected = []
    retained = [item for item in pending if item not in selected]
    migration_preview = None
    if request["migration"] is not None:
        migration_preview = preview_specification_migration(
            render_json_contract(request["migration"]), project_root=project_root,
            user_config_root=user_config_root, skill_roots=skill_roots,
        )
    evidence: dict[str, Any] = {
        "request": request, "attempt_sha256": raw_sha256(attempt_raw),
        "pending_deviation_ids": pending, "selected_deviation_ids": selected,
        "retained_deviation_ids": retained,
        "migration_fingerprint": None if migration_preview is None else migration_preview["fingerprint"],
    }
    ready = migration_preview is None or migration_preview["writable_ready"]
    return SpecificationReconciliationPreviewContract.model_validate({
        "schema": "work-spec-reconciliation-preview/v1",
        "status": "ready" if ready else "blocked", "attempt_path": attempt_path,
        "attempt_sha256": raw_sha256(attempt_raw), "choice": request["choice"],
        "pending_deviation_ids": pending, "selected_deviation_ids": selected,
        "retained_deviation_ids": retained, "migration_preview": migration_preview,
        "fingerprint": canonical_json_sha256(evidence),
        "publication_required": migration_preview is not None,
        "publication_ready": migration_preview is not None and migration_preview["writable_ready"],
    }).to_canonical_dict()


def publish_specification_reconciliation(
    raw_request: bytes, *, approved_sha256: str, project_root: Path,
    user_config_root: str, skill_roots=None,
) -> dict[str, object]:
    preview = preview_specification_reconciliation(
        raw_request, project_root=project_root, user_config_root=user_config_root,
        skill_roots=skill_roots,
    )
    if preview["fingerprint"] != approved_sha256:
        _fail("reconciliation_approval_changed", "The approved reconciliation fingerprint changed.")
    if not preview["publication_ready"]:
        _fail("reconciliation_not_publishable", "This reconciliation does not have a writable candidate set.")
    request = SpecificationReconciliationPreviewRequestContract.parse_json_bytes(
        raw_request, source="specification reconciliation publication",
    ).to_canonical_dict()
    migration = request["migration"]
    publication = publish_specification_migration(
        render_json_contract(migration), project_root=project_root,
        user_config_root=user_config_root, skill_roots=skill_roots,
        operation="apply", approved_sha256=preview["migration_preview"]["fingerprint"],
    )
    return SpecificationReconciliationPublicationContract.model_validate({
        "schema": "work-spec-reconciliation-publication/v1", "status": "updated",
        "reconciliation_fingerprint": approved_sha256,
        "attempt_path": preview["attempt_path"], "attempt_sha256": preview["attempt_sha256"],
        "selected_deviation_ids": preview["selected_deviation_ids"],
        "retained_deviation_ids": preview["retained_deviation_ids"],
        "publication": publication,
    }).to_canonical_dict()
