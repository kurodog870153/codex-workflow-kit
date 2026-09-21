from __future__ import annotations

import types
from typing import Any, Literal, Union, get_args, get_origin

from pydantic import BaseModel

from ...models.common.errors import ExitCode, WorkError
from ...models.common.base import WorkContract
from ...models.common.cli import CliResultContract, ErrorContract
from ...models.contract import (
    ContractCatalog,
    ContractCatalogEntry,
    ContractDescription,
    ContractFieldDescription,
    ContractScaffold,
)
from ...models.delegation import DelegationEnvelopeContract, DelegationValidationContract
from ...models.invocation import InvocationContract
from ...models.hierarchy import (
    HierarchyContract, HierarchySelectionContract,
    HierarchySelectionValidationContract,
)
from ...models.instruction import (
    InstructionCatalogContract, InstructionSelectionContract, InstructionsContract,
)
from ...models.skill import (
    SkillBundleContract, SkillCatalogContract, SkillSelectionContract,
    SkillSelectionValidationContract, SkillSnapshotContract,
)
from ...models.plan import PlanContract, PlanCreateContract, PlanPrepareContract, PlanValidationContract
from ...models.progress import (
    DiscussionProgressContract, ProgressPrepareContract, ProgressPreviewContract,
    ProgressReadContract, ProgressSaveContract, ProgressSaveRequestContract,
)
from ...models.handoff import HandoffContract, HandoffSourceValidationContract, HandoffValidationContract
from ...models.execution.inspection import (
    ExecutePreflightContract, ExecuteWorktreeContract, ExecuteWorktreeSnapshotContract,
)
from ...models.execution import (
    ExecutionDeviationContract, ExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract, ExecutionDeviationRecordContract,
)
from ...models.execution.index import ExecutionIndexContract
from ...models.execution.attempt import AttemptContract, AttemptValidationContract
from ...models.execution.correction import CorrectionContract
from ...models.execution.correction import CorrectionCreateContract, CorrectionCreateRequestContract
from ...models.execution.attempt_start import AttemptStartContract, AttemptStartRecoveryContract, AttemptStartRequestContract
from ...models.execution.authorization import AttemptAuthorizationContract
from ...models.execution.attempt_close import AttemptCloseContract, AttemptCloseRequestContract
from ...models.execution.record import RecordBeginContract, RecordFinishContract, RecordFinishRequestContract
from ...models.execution.command import (
    CommandCorrectionContract, CommandCorrectionRequestContract,
    CommandPreviewContract, CommandResultContract, CommandRunRequestContract,
    CommandStartedContract,
)
from ...models.execution.recovery import (
    ExecutionRecoveryContract, ExecutionRecoveryPrepareContract,
    ExecutionRecoveryPrepareRequestContract, ExecutionRecoveryRequestContract,
    RecoveryEvidenceContract,
)
from ...models.task_collection import (
    TaskCollectionDiagnosticsContract, TaskCollectionFingerprintContract,
    TaskCollectionProjectionContract,
    TaskCollectionValidationContract,
    TaskIndexContract, TaskIndexValidationContract,
    TaskItemContract, TaskItemValidationContract,
)
from ...models.task_draft import (
    TaskDraftContract, TaskDraftPrepareContract, TaskDraftRecoveryContract,
    TaskDraftSaveContract, TaskDraftSourceCheckContract, TaskDraftValidationContract,
    TaskPlanningIndexContract, TaskPlanningIndexValidationContract,
)
from ...models.specification.transaction import SpecTransactionContract
from ...models.task_collection.repair import (
    TaskRepairPrepareContract, TaskRepairPrepareRequestContract,
    TaskRepairContract, TaskRepairRequestContract,
)
from ...models.specification.contracts import (
    SpecificationPrepareRequestContract, SpecificationUpdateRequestContract,
    SpecificationVerificationRequestContract,
    SpecificationPrepareContract, SpecificationUpdateContract, SpecificationVerificationContract,
)
from ...models.specification.migration import (
    SpecificationMigrationPreviewContract, SpecificationMigrationPreviewRequestContract,
    SpecificationMigrationPublicationContract,
)
from ...models.specification.reconciliation import (
    SpecificationReconciliationPreviewContract,
    SpecificationReconciliationPreviewRequestContract,
    SpecificationReconciliationPublicationContract,
)


def _type_name(annotation: Any) -> str:
    origin = get_origin(annotation)
    arguments = get_args(annotation)
    if origin is Literal:
        return "literal"
    if origin in {Union, types.UnionType}:
        names = sorted({_type_name(argument) for argument in arguments})
        return " | ".join(names)
    if origin in {list, tuple, set, frozenset}:
        item = _type_name(arguments[0]) if arguments else "any"
        return f"array<{item}>"
    if origin is dict:
        value = _type_name(arguments[1]) if len(arguments) == 2 else "any"
        return f"object<{value}>"
    if isinstance(annotation, type) and issubclass(annotation, BaseModel):
        return "object"
    return {
        str: "string",
        int: "integer",
        float: "number",
        bool: "boolean",
        type(None): "null",
        Any: "any",
    }.get(annotation, "unknown")


def _scaffold_value(annotation: Any, example: Any = None) -> Any:
    origin = get_origin(annotation)
    arguments = get_args(annotation)
    if origin in {Union, types.UnionType}:
        target = next((item for item in arguments if item is not type(None)), Any)
        return _scaffold_value(target, example)
    if origin is Literal:
        return arguments[0] if arguments else None
    if origin in {list, tuple, set, frozenset}:
        if isinstance(example, list) and example:
            return [_scaffold_value(arguments[0], example[0])]
        return []
    if origin is dict:
        return {}
    if isinstance(annotation, type) and issubclass(annotation, BaseModel):
        source = example if isinstance(example, dict) else {}
        return {
            field.alias or name: _scaffold_value(
                field.annotation,
                source.get(field.alias or name),
            )
            for name, field in annotation.model_fields.items()
        }
    return None


class ContractRegistry:
    def __init__(self) -> None:
        self._contracts: dict[str, type[WorkContract]] = {}

    def register(self, *contracts: type[WorkContract]) -> None:
        pending = dict(self._contracts)
        for contract in contracts:
            if not isinstance(contract, type) or not issubclass(contract, WorkContract):
                raise TypeError("Only WorkContract subclasses can be registered.")
            if contract.contract_id in pending:
                raise ValueError(f"Duplicate contract ID: {contract.contract_id}")
            if contract.contract_example is None:
                raise TypeError(f"{contract.__name__} must declare contract_example.")
            contract.model_validate(contract.contract_example)
            pending[contract.contract_id] = contract
        self._contracts = pending

    def model(self, contract_id: str) -> type[WorkContract]:
        try:
            return self._contracts[contract_id]
        except KeyError as error:
            raise WorkError(
                ExitCode.CONTRACT,
                "unknown_contract_id",
                "The contract ID is not registered.",
                {"contract_id": contract_id},
            ) from error

    def catalog(self) -> ContractCatalog:
        return ContractCatalog(
            schema="work-contract-catalog/v1",
            contracts=[
                ContractCatalogEntry(id=contract_id, kind=contract.contract_kind)
                for contract_id, contract in sorted(self._contracts.items())
            ],
        )

    def describe(self, contract_id: str) -> ContractDescription:
        contract = self.model(contract_id)
        fields: list[ContractFieldDescription] = []
        required: list[str] = []
        optional: list[str] = []
        for name, field in contract.model_fields.items():
            alias = field.alias or name
            target = required if field.is_required() else optional
            target.append(alias)
            fields.append(
                ContractFieldDescription(
                    name=alias,
                    required=field.is_required(),
                    type=_type_name(field.annotation),
                    constraints=dict(contract.field_constraints.get(alias, {})),
                    reference=contract.field_references.get(alias),
                )
            )
        return ContractDescription(
            schema="work-contract-description/v1",
            id=contract.contract_id,
            kind=contract.contract_kind,
            required=required,
            optional=optional,
            canonical_order=list(contract.canonical_order),
            fields=fields,
            example=dict(contract.contract_example or {}),
        )

    def scaffold(self, contract_id: str) -> ContractScaffold:
        contract = self.model(contract_id)
        if contract.contract_kind != "request":
            raise WorkError(
                ExitCode.CONTRACT,
                "contract_scaffold_requires_request",
                "Only request contracts have public input scaffolds.",
                {"contract_id": contract_id, "kind": contract.contract_kind},
            )
        example = dict(contract.contract_example or {})
        scaffold = _scaffold_value(contract, example)
        return ContractScaffold(
            schema="work-contract-scaffold/v1",
            id=contract.contract_id,
            canonical_order=list(contract.canonical_order),
            scaffold=scaffold,
            example=example,
        )


registry = ContractRegistry()
registry.register(
    AttemptAuthorizationContract,
    AttemptCloseContract,
    AttemptCloseRequestContract,
    AttemptStartContract,
    AttemptStartRecoveryContract,
    AttemptStartRequestContract,
    AttemptValidationContract,
    AttemptContract,
    CommandCorrectionContract,
    CommandCorrectionRequestContract,
    CommandPreviewContract,
    CommandResultContract,
    CommandRunRequestContract,
    CommandStartedContract,
    CorrectionContract,
    CorrectionCreateContract,
    CorrectionCreateRequestContract,
    ExecutionDeviationContract,
    ExecutionDeviationPreviewContract,
    ExecutionDeviationProposalContract,
    ExecutionDeviationRecordContract,
    ExecutionIndexContract,
    ExecutionRecoveryContract,
    ExecutionRecoveryPrepareContract,
    ExecutionRecoveryPrepareRequestContract,
    ExecutionRecoveryRequestContract,
    RecoveryEvidenceContract,
    RecordBeginContract,
    RecordFinishContract,
    RecordFinishRequestContract,
    CliResultContract,
    ContractCatalog,
    ContractDescription,
    ContractScaffold,
    DelegationEnvelopeContract,
    DelegationValidationContract,
    ErrorContract,
    ExecutePreflightContract,
    ExecuteWorktreeContract,
    ExecuteWorktreeSnapshotContract,
    InvocationContract,
    HierarchyContract,
    HierarchySelectionContract,
    HierarchySelectionValidationContract,
    InstructionCatalogContract,
    InstructionSelectionContract,
    InstructionsContract,
    SkillBundleContract,
    SkillCatalogContract,
    SkillSelectionContract,
    SkillSelectionValidationContract,
    SkillSnapshotContract,
    PlanContract,
    PlanCreateContract,
    PlanPrepareContract,
    PlanValidationContract,
    DiscussionProgressContract,
    ProgressPrepareContract,
    ProgressPreviewContract,
    ProgressReadContract,
    ProgressSaveContract,
    ProgressSaveRequestContract,
    HandoffContract,
    HandoffSourceValidationContract,
    HandoffValidationContract,
    SpecTransactionContract,
    SpecificationPrepareRequestContract,
    SpecificationUpdateRequestContract,
    SpecificationVerificationRequestContract,
    SpecificationPrepareContract, SpecificationUpdateContract, SpecificationVerificationContract,
    SpecificationMigrationPreviewContract,
    SpecificationMigrationPreviewRequestContract,
    SpecificationMigrationPublicationContract,
    SpecificationReconciliationPreviewContract,
    SpecificationReconciliationPreviewRequestContract,
    SpecificationReconciliationPublicationContract,
    TaskRepairPrepareContract,
    TaskRepairPrepareRequestContract,
    TaskRepairContract,
    TaskRepairRequestContract,
    TaskCollectionDiagnosticsContract,
    TaskCollectionFingerprintContract,
    TaskCollectionProjectionContract,
    TaskCollectionValidationContract,
    TaskIndexContract,
    TaskIndexValidationContract,
    TaskItemContract,
    TaskItemValidationContract,
    TaskDraftContract,
    TaskDraftPrepareContract,
    TaskDraftRecoveryContract,
    TaskDraftSaveContract,
    TaskDraftSourceCheckContract,
    TaskDraftValidationContract,
    TaskPlanningIndexContract,
    TaskPlanningIndexValidationContract,
)


