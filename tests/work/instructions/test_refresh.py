from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.business_services.instruction.refresh import (
    _compatibility, _current, _discover_requirements, _routing_compatibility,
    apply_source_refresh_all, source_impact,
)
from worklib.business_services.instruction import build_instruction_selection
from worklib.services.instruction.validation_session import ValidationSession
from worklib.services.instruction.catalog import build_instruction_catalog
from worklib.business_services.instruction import refresh as refresh_service
from worklib.models.common.errors import WorkError


class InstructionRefreshTests(unittest.TestCase):
    def test_same_source_selection_is_loaded_once_then_rechecked(self) -> None:
        skill_root = Path(__file__).resolve().parents[3] / "skills" / "work"
        selection = build_instruction_selection(
            skill_root=skill_root, mode="task", selected_paths=[], reference_names=[],
        )
        with patch.object(refresh_service, "load_instruction_sources",
                          wraps=refresh_service.load_instruction_sources) as load:
            session = ValidationSession(skill_root, build_catalog=build_instruction_catalog,
                                        load_sources=load)
            first = _current(skill_root, "task", selection, session=session)
            second = _current(skill_root, "task", selection, session=session)
            session.recheck()
        self.assertIs(first, second)
        self.assertEqual(load.call_count, 2)

    def source(self, digest: str, revision: int | None = 1):
        value = {"kind": "workflow", "logical_name": "work.workflow.plan", "canonical_sha256": digest}
        if revision is not None:
            value["compatibility_revision"] = revision
        return value

    def test_content_change_with_same_revision_is_refreshable(self) -> None:
        status, changed = _compatibility(
            {"sources": [self.source("a" * 64)]}, [self.source("b" * 64)],
        )
        self.assertEqual((status, changed), ("REFRESHABLE", ["work.workflow.plan"]))

    def test_revision_change_requires_review(self) -> None:
        status, _ = _compatibility(
            {"sources": [self.source("a" * 64, 1)]}, [self.source("b" * 64, 2)],
        )
        self.assertEqual(status, "REVIEW_REQUIRED")

    def test_legacy_changed_source_requires_review(self) -> None:
        status, _ = _compatibility(
            {"sources": [self.source("a" * 64, None)]}, [self.source("b" * 64, 1)],
        )
        self.assertEqual(status, "REVIEW_REQUIRED")

    def test_legacy_unchanged_source_is_refreshable_for_revision_backfill(self) -> None:
        status, changed = _compatibility(
            {"sources": [self.source("a" * 64, None)]}, [self.source("a" * 64, 1)],
        )
        self.assertEqual((status, changed), ("REFRESHABLE", ["work.workflow.plan"]))

    def test_discovers_non_default_plan_artifact_relationship(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = root / "outputs/work/custom/specification.json"
            plan.parent.mkdir(parents=True)
            plan.write_text(
                '{"schema":"work-plan/v1","requirement_id":"custom","artifacts":'
                '{"plan":"outputs/work/custom/specification.json",'
                '"task":"outputs/work/custom/tasks/index.json",'
                '"execution":"outputs/work/custom/execution"}}\n',
                encoding="utf-8",
            )
            discovered = _discover_requirements(root)
        self.assertEqual(discovered["custom"]["plan"], "outputs/work/custom/specification.json")

    def test_source_impact_discovers_requirements_once(self) -> None:
        artifacts = {
            name: {"plan": f"{name}.json", "task": f"{name}/task.json", "execution": name}
            for name in ("alpha", "beta")
        }
        def built(_root, _skill_root, requirement_id, *, artifacts):
            return {"preview": {"status": "valid", "files": [], "blocked": [],
                                 "requirement_id": requirement_id},
                    "changed_source_names": set()}
        with patch("worklib.business_services.instruction.refresh._discover_requirements",
                   return_value=artifacts) as discover, \
             patch("worklib.business_services.instruction.refresh._build_requirement",
                   side_effect=built) as build:
            result = source_impact(Path("."), Path("."))
        discover.assert_called_once()
        self.assertEqual(build.call_count, 2)
        self.assertEqual(result["affected_requirements"], 0)

    def test_routing_source_change_with_same_revision_is_refreshable(self) -> None:
        stored = {"router_compatibility_revision": 3, "selection_sha256": "a" * 64,
                  "sources": [{"logical_name": "work.workflow.plan", "compatibility_revision": 2,
                               "canonical_sha256": "a" * 64}]}
        current = {"router_compatibility_revision": 3, "selection_sha256": "b" * 64,
                   "sources": [{"logical_name": "work.workflow.plan", "compatibility_revision": 2,
                                "canonical_sha256": "b" * 64}]}
        self.assertEqual(_routing_compatibility(stored, current),
                         ("REFRESHABLE", ["work.workflow.plan"]))

    def test_routing_revision_change_requires_review(self) -> None:
        stored = {"router_compatibility_revision": 2, "selection_sha256": "a" * 64, "sources": []}
        current = {"router_compatibility_revision": 3, "selection_sha256": "b" * 64, "sources": []}
        self.assertEqual(_routing_compatibility(stored, current)[0], "REVIEW_REQUIRED")

    def test_batch_recovery_resumes_first_incomplete_requirement(self) -> None:
        preview = {
            "status": "refreshable", "approved_sha256": "f" * 64,
            "requirements": [
                {"status": "refreshable", "requirement_id": "alpha", "approved_sha256": "a" * 64},
                {"status": "refreshable", "requirement_id": "beta", "approved_sha256": "b" * 64},
            ],
        }
        def publication(requirement_id, digest, status="updated"):
            return {
                "schema": "work-source-refresh-publication/v1", "status": status,
                "requirement_id": requirement_id, "approved_sha256": digest,
                "transaction_approval_sha256": "c" * 64,
                "journal": f"outputs/{requirement_id}.json",
                "completion_marker": f"outputs/{requirement_id}.json.done",
                "updated_files": [],
            }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch(
                "worklib.business_services.instruction.refresh.preview_source_refresh_all",
                return_value=preview,
            ), patch(
                "worklib.business_services.instruction.refresh.apply_source_refresh",
                side_effect=[publication("alpha", "a" * 64), WorkError(5, "injected", "stop")],
            ):
                with self.assertRaises(WorkError):
                    apply_source_refresh_all(root, root, "f" * 64)
            with patch(
                "worklib.business_services.instruction.refresh.apply_source_refresh",
                return_value=publication("beta", "b" * 64),
            ) as resume:
                result = apply_source_refresh_all(
                    root, root, "f" * 64, operation="recover"
                )
            self.assertEqual(result["semantics"], "recoverable_sequential")
            self.assertEqual(result["completed_requirement_ids"], ["alpha", "beta"])
            resume.assert_called_once()
            repeated = apply_source_refresh_all(root, root, "f" * 64)
            self.assertEqual(repeated["status"], "already_completed")


if __name__ == "__main__":
    unittest.main()
