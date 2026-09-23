from __future__ import annotations

import sys
import unittest
from pathlib import Path
from typing import ClassVar, Literal

from pydantic import Field


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.common.base import WorkContract
from worklib.services.contract import ContractRegistry, registry
from worklib.models.common.errors import WorkError


class SampleContract(WorkContract):
    contract_id: ClassVar[str] = "work-sample/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "value")
    contract_example: ClassVar[dict[str, object]] = {
        "schema": "work-sample/v1",
        "value": "example",
    }

    schema_: Literal["work-sample/v1"] = Field(alias="schema")
    value: str


class ContractRegistryTests(unittest.TestCase):
    def test_catalog_is_sorted_and_contains_public_description_contracts(self) -> None:
        catalog = registry.catalog().to_canonical_dict()

        self.assertEqual(catalog["schema"], "work-contract-catalog/v1")
        self.assertEqual(
            [item["id"] for item in catalog["contracts"]],
            [
                "work-attempt-authorization/v1",
                "work-attempt-close-request/v1",
                "work-attempt-close/v1",
                "work-attempt-start-recovery/v1",
                "work-attempt-start-request/v1",
                "work-attempt-start/v1",
                "work-attempt-validation/v1",
                "work-attempt/v1",
                "work-cli-result/v1",
                "work-command-correction-request/v1",
                "work-command-correction/v1",
                "work-command-preview/v1",
                "work-command-result/v1",
                "work-command-run-request/v1",
                "work-command-started/v1",
                "work-contract-catalog/v1",
                "work-contract-description/v1",
                "work-contract-scaffold/v1",
                "work-correction-create-request/v1",
                "work-correction-create/v1",
                "work-correction/v1",
                "work-delegation-envelope/v1",
                "work-delegation-validation/v1",
                "work-discussion-progress/v1",
                "work-error/v1",
                "work-execute-preflight/v1",
                "work-execute-worktree-snapshot/v1",
                "work-execute-worktree/v1",
                "work-execution-deviation-preview/v1",
                "work-execution-deviation-proposal/v1",
                "work-execution-deviation-record/v1",
                "work-execution-deviation/v1",
                "work-execution-index/v1",
                "work-execution-recovery-evidence/v1",
                "work-execution-recovery-prepare-request/v1",
                "work-execution-recovery-prepare/v1",
                "work-execution-recovery-request/v1",
                "work-execution-recovery/v1",
                "work-handoff-source-validation/v1",
                "work-handoff-validation/v1",
                "work-handoff/v1",
                "work-hierarchy-selection-validation/v1",
                "work-hierarchy-selection/v1",
                "work-hierarchy/v1",
                "work-instruction-catalog/v1",
                "work-instruction-migration-preview/v1",
                "work-instruction-migration-publication/v1",
                "work-instruction-selection-manifest/v1",
                "work-instruction-selection/v1",
                "work-instructions/v1",
                "work-invocation/v1",
                "work-operation-envelope/v1",
                "work-operation-result/v1",
                "work-plan-create/v1",
                "work-plan-prepare-request/v1",
                "work-plan-prepare/v1",
                "work-plan-semantic-request/v1",
                "work-plan-validation/v1",
                "work-plan/v1",
                "work-progress-prepare/v1",
                "work-progress-preview/v1",
                "work-progress-read/v1",
                "work-progress-save-request/v1",
                "work-progress-save/v1",
                "work-record-begin/v1",
                "work-record-finish-request/v1",
                "work-record-finish/v1",
                "work-skill-bundle/v1",
                "work-skill-catalog/v1",
                "work-skill-selection-validation/v1",
                "work-skill-selection/v1",
                "work-skill-snapshot/v1",
                "work-source-impact/v1",
                "work-source-refresh-preview/v1",
                "work-source-refresh-publication/v1",
                "work-spec-migration-preview-request/v1",
                "work-spec-migration-preview/v1",
                "work-spec-migration-publication/v1",
                "work-spec-prepare-request/v1",
                "work-spec-prepare/v1",
                "work-spec-reconciliation-preview-request/v1",
                "work-spec-reconciliation-preview/v1",
                "work-spec-reconciliation-publication/v1",
                "work-spec-transaction/v1",
                "work-spec-update-request/v1",
                "work-spec-update/v1",
                "work-spec-verification-request/v1",
                "work-spec-verification/v1",
                "work-task-collection-diagnostics/v1",
                "work-task-collection-fingerprint/v1",
                "work-task-collection-projection/v1",
                "work-task-collection-validation/v1",
                "work-task-draft-prepare/v1",
                "work-task-draft-recovery/v1",
                "work-task-draft-save/v1",
                "work-task-draft-source-check/v1",
                "work-task-draft-validation/v1",
                "work-task-draft/v1",
                "work-task-index-validation/v1",
                "work-task-index/v1",
                "work-task-item-validation/v1",
                "work-task-item/v1",
                "work-task-planning-index-validation/v1",
                "work-task-planning-index/v1",
                "work-task-repair-prepare-request/v1",
                "work-task-repair-prepare/v1",
                "work-task-repair-request/v1",
                "work-task-repair/v1",
                "work-task-semantic-request/v1",
                "work-workflow-state/v1",
            ],
        )

    def test_description_is_stable_and_example_validates(self) -> None:
        local = ContractRegistry()
        local.register(SampleContract)

        description = local.describe("work-sample/v1").to_canonical_dict()

        self.assertEqual(description["required"], ["schema", "value"])
        self.assertEqual(description["optional"], [])
        self.assertEqual(description["canonical_order"], ["schema", "value"])
        self.assertEqual(description["fields"][1]["type"], "string")
        self.assertNotIn("$defs", description)
        local.model(description["id"]).model_validate(description["example"])

    def test_all_public_request_contracts_have_complete_scaffolds(self) -> None:
        request_ids = [
            item.id for item in registry.catalog().contracts if item.kind == "request"
        ]

        self.assertTrue(request_ids)
        for contract_id in request_ids:
            with self.subTest(contract_id=contract_id):
                result = registry.scaffold(contract_id).to_canonical_dict()
                model = registry.model(contract_id)
                self.assertEqual(result["canonical_order"], list(model.canonical_order))
                self.assertEqual(list(result["scaffold"]), list(model.canonical_order))
                model.model_validate(result["example"])

    def test_scaffold_rejects_nonrequest_contract(self) -> None:
        with self.assertRaises(WorkError) as caught:
            registry.scaffold("work-contract-catalog/v1")

        self.assertEqual(caught.exception.code, "contract_scaffold_requires_request")

    def test_scaffold_contract_example_references_registered_request(self) -> None:
        scaffold = registry.model("work-contract-scaffold/v1").contract_example
        self.assertIsNotNone(scaffold)
        target = registry.model(scaffold["id"])

        self.assertEqual(target.contract_kind, "request")
        self.assertEqual(scaffold["canonical_order"], list(target.canonical_order))
        target.model_validate(scaffold["example"])

    def test_duplicate_contract_id_is_rejected_atomically(self) -> None:
        local = ContractRegistry()
        local.register(SampleContract)

        with self.assertRaises(ValueError):
            local.register(SampleContract)

        self.assertEqual(len(local.catalog().contracts), 1)

    def test_unknown_contract_id_uses_stable_work_error(self) -> None:
        with self.assertRaises(WorkError) as caught:
            registry.describe("work-unknown/v1")

        self.assertEqual(caught.exception.code, "unknown_contract_id")
        self.assertEqual(
            caught.exception.details, {"contract_id": "work-unknown/v1"}
        )


if __name__ == "__main__":
    unittest.main()
