//! Public schema identifiers shared by all Work data models.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PublicSchema {
    #[serde(rename = "work-source-snapshot/v1")]
    WorkSourceSnapshotV1,
    #[serde(rename = "work-source-read/v1")]
    WorkSourceReadV1,
    #[serde(rename = "work-source-validation/v1")]
    WorkSourceValidationV1,
    #[serde(rename = "work-attempt-authorization/v1")]
    WorkAttemptAuthorizationV1,
    #[serde(rename = "work-attempt-close-request/v1")]
    WorkAttemptCloseRequestV1,
    #[serde(rename = "work-attempt-close/v1")]
    WorkAttemptCloseV1,
    #[serde(rename = "work-attempt-start-prepare-request/v1")]
    WorkAttemptStartPrepareRequestV1,
    #[serde(rename = "work-attempt-start-prepare/v1")]
    WorkAttemptStartPrepareV1,
    #[serde(rename = "work-attempt-start-recovery/v1")]
    WorkAttemptStartRecoveryV1,
    #[serde(rename = "work-attempt-start-request/v1")]
    WorkAttemptStartRequestV1,
    #[serde(rename = "work-attempt-start/v1")]
    WorkAttemptStartV1,
    #[serde(rename = "work-attempt-validation/v1")]
    WorkAttemptValidationV1,
    #[serde(rename = "work-attempt/v1")]
    WorkAttemptV1,
    #[serde(rename = "work-cli-result/v1")]
    WorkCliResultV1,
    #[serde(rename = "work-command-correction-request/v1")]
    WorkCommandCorrectionRequestV1,
    #[serde(rename = "work-command-correction/v1")]
    WorkCommandCorrectionV1,
    #[serde(rename = "work-command-preview/v1")]
    WorkCommandPreviewV1,
    #[serde(rename = "work-command-result/v1")]
    WorkCommandResultV1,
    #[serde(rename = "work-command-run-request/v1")]
    WorkCommandRunRequestV1,
    #[serde(rename = "work-command-started/v1")]
    WorkCommandStartedV1,
    #[serde(rename = "work-contract-catalog/v1")]
    WorkContractCatalogV1,
    #[serde(rename = "work-contract-description/v1")]
    WorkContractDescriptionV1,
    #[serde(rename = "work-contract-scaffold/v1")]
    WorkContractScaffoldV1,
    #[serde(rename = "work-correction-create-request/v1")]
    WorkCorrectionCreateRequestV1,
    #[serde(rename = "work-correction-create/v1")]
    WorkCorrectionCreateV1,
    #[serde(rename = "work-correction/v1")]
    WorkCorrectionV1,
    #[serde(rename = "work-delegation-build-request/v1")]
    WorkDelegationBuildRequestV1,
    #[serde(rename = "work-delegation-envelope/v1")]
    WorkDelegationEnvelopeV1,
    #[serde(rename = "work-delegation-validation/v1")]
    WorkDelegationValidationV1,
    #[serde(rename = "work-discussion-handoff-request/v1")]
    WorkDiscussionHandoffRequestV1,
    #[serde(rename = "work-discussion-handoff/v1")]
    WorkDiscussionHandoffV1,
    #[serde(rename = "work-discussion-progress/v1")]
    WorkDiscussionProgressV1,
    #[serde(rename = "work-error/v1")]
    WorkErrorV1,
    #[serde(rename = "work-execute-preflight/v1")]
    WorkExecutePreflightV1,
    #[serde(rename = "work-execute-worktree-snapshot/v1")]
    WorkExecuteWorktreeSnapshotV1,
    #[serde(rename = "work-execute-worktree/v1")]
    WorkExecuteWorktreeV1,
    #[serde(rename = "work-execution-deviation-preview/v1")]
    WorkExecutionDeviationPreviewV1,
    #[serde(rename = "work-execution-deviation-proposal/v1")]
    WorkExecutionDeviationProposalV1,
    #[serde(rename = "work-execution-deviation-record/v1")]
    WorkExecutionDeviationRecordV1,
    #[serde(rename = "work-execution-deviation-semantic-request/v1")]
    WorkExecutionDeviationSemanticRequestV1,
    #[serde(rename = "work-execution-deviation/v1")]
    WorkExecutionDeviationV1,
    #[serde(rename = "work-execution-index/v1")]
    WorkExecutionIndexV1,
    #[serde(rename = "work-execution-recovery-evidence/v1")]
    WorkExecutionRecoveryEvidenceV1,
    #[serde(rename = "work-execution-recovery-prepare-request/v1")]
    WorkExecutionRecoveryPrepareRequestV1,
    #[serde(rename = "work-execution-recovery-prepare/v1")]
    WorkExecutionRecoveryPrepareV1,
    #[serde(rename = "work-execution-recovery-request/v1")]
    WorkExecutionRecoveryRequestV1,
    #[serde(rename = "work-execution-recovery/v1")]
    WorkExecutionRecoveryV1,
    #[serde(rename = "work-handoff-source-validation/v1")]
    WorkHandoffSourceValidationV1,
    #[serde(rename = "work-handoff-validation/v1")]
    WorkHandoffValidationV1,
    #[serde(rename = "work-handoff/v1")]
    WorkHandoffV1,
    #[serde(rename = "work-hierarchy-selection-validation/v1")]
    WorkHierarchySelectionValidationV1,
    #[serde(rename = "work-hierarchy-selection/v1")]
    WorkHierarchySelectionV1,
    #[serde(rename = "work-hierarchy/v1")]
    WorkHierarchyV1,
    #[serde(rename = "work-instruction-catalog/v1")]
    WorkInstructionCatalogV1,
    #[serde(rename = "work-instruction-migration-preview/v1")]
    WorkInstructionMigrationPreviewV1,
    #[serde(rename = "work-instruction-migration-publication/v1")]
    WorkInstructionMigrationPublicationV1,
    #[serde(rename = "work-instruction-selection-manifest/v1")]
    WorkInstructionSelectionManifestV1,
    #[serde(rename = "work-instruction-selection/v1")]
    WorkInstructionSelectionV1,
    #[serde(rename = "work-instructions/v1")]
    WorkInstructionsV1,
    #[serde(rename = "work-invocation/v1")]
    WorkInvocationV1,
    #[serde(rename = "work-operation-envelope/v1")]
    WorkOperationEnvelopeV1,
    #[serde(rename = "work-operation-result/v1")]
    WorkOperationResultV1,
    #[serde(rename = "work-progress-prepare/v1")]
    WorkProgressPrepareV1,
    #[serde(rename = "work-progress-preview/v1")]
    WorkProgressPreviewV1,
    #[serde(rename = "work-progress-read/v1")]
    WorkProgressReadV1,
    #[serde(rename = "work-progress-save-request/v1")]
    WorkProgressSaveRequestV1,
    #[serde(rename = "work-progress-save/v1")]
    WorkProgressSaveV1,
    #[serde(rename = "work-record-begin/v1")]
    WorkRecordBeginV1,
    #[serde(rename = "work-record-finish-request/v1")]
    WorkRecordFinishRequestV1,
    #[serde(rename = "work-record-finish/v1")]
    WorkRecordFinishV1,
    #[serde(rename = "work-skill-bundle/v1")]
    WorkSkillBundleV1,
    #[serde(rename = "work-skill-catalog/v1")]
    WorkSkillCatalogV1,
    #[serde(rename = "work-skill-selection-validation/v1")]
    WorkSkillSelectionValidationV1,
    #[serde(rename = "work-skill-selection/v1")]
    WorkSkillSelectionV1,
    #[serde(rename = "work-skill-snapshot/v1")]
    WorkSkillSnapshotV1,
    #[serde(rename = "work-source-impact/v1")]
    WorkSourceImpactV1,
    #[serde(rename = "work-source-refresh-preview/v1")]
    WorkSourceRefreshPreviewV1,
    #[serde(rename = "work-source-refresh-publication/v1")]
    WorkSourceRefreshPublicationV1,
    #[serde(rename = "work-artifact-migration-analysis/v1")]
    WorkArtifactMigrationAnalysisV1,
    #[serde(rename = "work-artifact-migration-request/v1")]
    WorkArtifactMigrationRequestV1,
    #[serde(rename = "work-spec-migration-prepare-request/v1")]
    WorkSpecMigrationPrepareRequestV1,
    #[serde(rename = "work-spec-migration-preview-request/v1")]
    WorkSpecMigrationPreviewRequestV1,
    #[serde(rename = "work-spec-migration-preview/v1")]
    WorkSpecMigrationPreviewV1,
    #[serde(rename = "work-spec-migration-publication/v1")]
    WorkSpecMigrationPublicationV1,
    #[serde(rename = "work-spec-prepare-request/v1")]
    WorkSpecPrepareRequestV1,
    #[serde(rename = "work-spec-prepare/v1")]
    WorkSpecPrepareV1,
    #[serde(rename = "work-spec-reconciliation-prepare-request/v1")]
    WorkSpecReconciliationPrepareRequestV1,
    #[serde(rename = "work-spec-reconciliation-preview-request/v1")]
    WorkSpecReconciliationPreviewRequestV1,
    #[serde(rename = "work-spec-reconciliation-preview/v1")]
    WorkSpecReconciliationPreviewV1,
    #[serde(rename = "work-spec-reconciliation-publication/v1")]
    WorkSpecReconciliationPublicationV1,
    #[serde(rename = "work-spec-transaction/v1")]
    WorkSpecTransactionV1,
    #[serde(rename = "work-spec-update-request/v1")]
    WorkSpecUpdateRequestV1,
    #[serde(rename = "work-spec-update/v1")]
    WorkSpecUpdateV1,
    #[serde(rename = "work-spec-verification-request/v1")]
    WorkSpecVerificationRequestV1,
    #[serde(rename = "work-spec-verification/v1")]
    WorkSpecVerificationV1,
    #[serde(rename = "work-task-collection-fingerprint/v1")]
    WorkTaskCollectionFingerprintV1,
    #[serde(rename = "work-task-collection-projection/v1")]
    WorkTaskCollectionProjectionV1,
    #[serde(rename = "work-task-collection-validation/v1")]
    WorkTaskCollectionValidationV1,
    #[serde(rename = "work-task-draft-prepare/v1")]
    WorkTaskDraftPrepareV1,
    #[serde(rename = "work-task-draft-recovery/v1")]
    WorkTaskDraftRecoveryV1,
    #[serde(rename = "work-task-draft-save/v1")]
    WorkTaskDraftSaveV1,
    #[serde(rename = "work-task-draft-source-check/v1")]
    WorkTaskDraftSourceCheckV1,
    #[serde(rename = "work-task-draft-validation/v1")]
    WorkTaskDraftValidationV1,
    #[serde(rename = "work-task-draft/v1")]
    WorkTaskDraftV1,
    #[serde(rename = "work-task-index-validation/v1")]
    WorkTaskIndexValidationV1,
    #[serde(rename = "work-task-index/v1")]
    WorkTaskIndexV1,
    #[serde(rename = "work-task-item-validation/v1")]
    WorkTaskItemValidationV1,
    #[serde(rename = "work-task-item/v1")]
    WorkTaskItemV1,
    #[serde(rename = "work-task-planning-index-validation/v1")]
    WorkTaskPlanningIndexValidationV1,
    #[serde(rename = "work-task-planning-index/v1")]
    WorkTaskPlanningIndexV1,
    #[serde(rename = "work-task-semantic-request/v1")]
    WorkTaskSemanticRequestV1,
    #[serde(rename = "work-workflow-state/v1")]
    WorkWorkflowStateV1,
}

impl PublicSchema {
    pub const ALL: [Self; 112] = [
        Self::WorkSourceSnapshotV1,
        Self::WorkSourceReadV1,
        Self::WorkSourceValidationV1,
        Self::WorkAttemptAuthorizationV1,
        Self::WorkAttemptCloseRequestV1,
        Self::WorkAttemptCloseV1,
        Self::WorkAttemptStartPrepareRequestV1,
        Self::WorkAttemptStartPrepareV1,
        Self::WorkAttemptStartRecoveryV1,
        Self::WorkAttemptStartRequestV1,
        Self::WorkAttemptStartV1,
        Self::WorkAttemptValidationV1,
        Self::WorkAttemptV1,
        Self::WorkCliResultV1,
        Self::WorkCommandCorrectionRequestV1,
        Self::WorkCommandCorrectionV1,
        Self::WorkCommandPreviewV1,
        Self::WorkCommandResultV1,
        Self::WorkCommandRunRequestV1,
        Self::WorkCommandStartedV1,
        Self::WorkContractCatalogV1,
        Self::WorkContractDescriptionV1,
        Self::WorkContractScaffoldV1,
        Self::WorkCorrectionCreateRequestV1,
        Self::WorkCorrectionCreateV1,
        Self::WorkCorrectionV1,
        Self::WorkDelegationBuildRequestV1,
        Self::WorkDelegationEnvelopeV1,
        Self::WorkDelegationValidationV1,
        Self::WorkDiscussionHandoffRequestV1,
        Self::WorkDiscussionHandoffV1,
        Self::WorkDiscussionProgressV1,
        Self::WorkErrorV1,
        Self::WorkExecutePreflightV1,
        Self::WorkExecuteWorktreeSnapshotV1,
        Self::WorkExecuteWorktreeV1,
        Self::WorkExecutionDeviationPreviewV1,
        Self::WorkExecutionDeviationProposalV1,
        Self::WorkExecutionDeviationRecordV1,
        Self::WorkExecutionDeviationSemanticRequestV1,
        Self::WorkExecutionDeviationV1,
        Self::WorkExecutionIndexV1,
        Self::WorkExecutionRecoveryEvidenceV1,
        Self::WorkExecutionRecoveryPrepareRequestV1,
        Self::WorkExecutionRecoveryPrepareV1,
        Self::WorkExecutionRecoveryRequestV1,
        Self::WorkExecutionRecoveryV1,
        Self::WorkHandoffSourceValidationV1,
        Self::WorkHandoffValidationV1,
        Self::WorkHandoffV1,
        Self::WorkHierarchySelectionValidationV1,
        Self::WorkHierarchySelectionV1,
        Self::WorkHierarchyV1,
        Self::WorkInstructionCatalogV1,
        Self::WorkInstructionMigrationPreviewV1,
        Self::WorkInstructionMigrationPublicationV1,
        Self::WorkInstructionSelectionManifestV1,
        Self::WorkInstructionSelectionV1,
        Self::WorkInstructionsV1,
        Self::WorkInvocationV1,
        Self::WorkOperationEnvelopeV1,
        Self::WorkOperationResultV1,
        Self::WorkProgressPrepareV1,
        Self::WorkProgressPreviewV1,
        Self::WorkProgressReadV1,
        Self::WorkProgressSaveRequestV1,
        Self::WorkProgressSaveV1,
        Self::WorkRecordBeginV1,
        Self::WorkRecordFinishRequestV1,
        Self::WorkRecordFinishV1,
        Self::WorkSkillBundleV1,
        Self::WorkSkillCatalogV1,
        Self::WorkSkillSelectionValidationV1,
        Self::WorkSkillSelectionV1,
        Self::WorkSkillSnapshotV1,
        Self::WorkSourceImpactV1,
        Self::WorkSourceRefreshPreviewV1,
        Self::WorkSourceRefreshPublicationV1,
        Self::WorkArtifactMigrationAnalysisV1,
        Self::WorkArtifactMigrationRequestV1,
        Self::WorkSpecMigrationPrepareRequestV1,
        Self::WorkSpecMigrationPreviewRequestV1,
        Self::WorkSpecMigrationPreviewV1,
        Self::WorkSpecMigrationPublicationV1,
        Self::WorkSpecPrepareRequestV1,
        Self::WorkSpecPrepareV1,
        Self::WorkSpecReconciliationPrepareRequestV1,
        Self::WorkSpecReconciliationPreviewRequestV1,
        Self::WorkSpecReconciliationPreviewV1,
        Self::WorkSpecReconciliationPublicationV1,
        Self::WorkSpecTransactionV1,
        Self::WorkSpecUpdateRequestV1,
        Self::WorkSpecUpdateV1,
        Self::WorkSpecVerificationRequestV1,
        Self::WorkSpecVerificationV1,
        Self::WorkTaskCollectionFingerprintV1,
        Self::WorkTaskCollectionProjectionV1,
        Self::WorkTaskCollectionValidationV1,
        Self::WorkTaskDraftPrepareV1,
        Self::WorkTaskDraftRecoveryV1,
        Self::WorkTaskDraftSaveV1,
        Self::WorkTaskDraftSourceCheckV1,
        Self::WorkTaskDraftValidationV1,
        Self::WorkTaskDraftV1,
        Self::WorkTaskIndexValidationV1,
        Self::WorkTaskIndexV1,
        Self::WorkTaskItemValidationV1,
        Self::WorkTaskItemV1,
        Self::WorkTaskPlanningIndexValidationV1,
        Self::WorkTaskPlanningIndexV1,
        Self::WorkTaskSemanticRequestV1,
        Self::WorkWorkflowStateV1,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkSourceSnapshotV1 => "work-source-snapshot/v1",
            Self::WorkSourceReadV1 => "work-source-read/v1",
            Self::WorkSourceValidationV1 => "work-source-validation/v1",
            Self::WorkAttemptAuthorizationV1 => "work-attempt-authorization/v1",
            Self::WorkAttemptCloseRequestV1 => "work-attempt-close-request/v1",
            Self::WorkAttemptCloseV1 => "work-attempt-close/v1",
            Self::WorkAttemptStartPrepareRequestV1 => "work-attempt-start-prepare-request/v1",
            Self::WorkAttemptStartPrepareV1 => "work-attempt-start-prepare/v1",
            Self::WorkAttemptStartRecoveryV1 => "work-attempt-start-recovery/v1",
            Self::WorkAttemptStartRequestV1 => "work-attempt-start-request/v1",
            Self::WorkAttemptStartV1 => "work-attempt-start/v1",
            Self::WorkAttemptValidationV1 => "work-attempt-validation/v1",
            Self::WorkAttemptV1 => "work-attempt/v1",
            Self::WorkCliResultV1 => "work-cli-result/v1",
            Self::WorkCommandCorrectionRequestV1 => "work-command-correction-request/v1",
            Self::WorkCommandCorrectionV1 => "work-command-correction/v1",
            Self::WorkCommandPreviewV1 => "work-command-preview/v1",
            Self::WorkCommandResultV1 => "work-command-result/v1",
            Self::WorkCommandRunRequestV1 => "work-command-run-request/v1",
            Self::WorkCommandStartedV1 => "work-command-started/v1",
            Self::WorkContractCatalogV1 => "work-contract-catalog/v1",
            Self::WorkContractDescriptionV1 => "work-contract-description/v1",
            Self::WorkContractScaffoldV1 => "work-contract-scaffold/v1",
            Self::WorkCorrectionCreateRequestV1 => "work-correction-create-request/v1",
            Self::WorkCorrectionCreateV1 => "work-correction-create/v1",
            Self::WorkCorrectionV1 => "work-correction/v1",
            Self::WorkDelegationBuildRequestV1 => "work-delegation-build-request/v1",
            Self::WorkDelegationEnvelopeV1 => "work-delegation-envelope/v1",
            Self::WorkDelegationValidationV1 => "work-delegation-validation/v1",
            Self::WorkDiscussionHandoffRequestV1 => "work-discussion-handoff-request/v1",
            Self::WorkDiscussionHandoffV1 => "work-discussion-handoff/v1",
            Self::WorkDiscussionProgressV1 => "work-discussion-progress/v1",
            Self::WorkErrorV1 => "work-error/v1",
            Self::WorkExecutePreflightV1 => "work-execute-preflight/v1",
            Self::WorkExecuteWorktreeSnapshotV1 => "work-execute-worktree-snapshot/v1",
            Self::WorkExecuteWorktreeV1 => "work-execute-worktree/v1",
            Self::WorkExecutionDeviationPreviewV1 => "work-execution-deviation-preview/v1",
            Self::WorkExecutionDeviationProposalV1 => "work-execution-deviation-proposal/v1",
            Self::WorkExecutionDeviationRecordV1 => "work-execution-deviation-record/v1",
            Self::WorkExecutionDeviationSemanticRequestV1 => {
                "work-execution-deviation-semantic-request/v1"
            }
            Self::WorkExecutionDeviationV1 => "work-execution-deviation/v1",
            Self::WorkExecutionIndexV1 => "work-execution-index/v1",
            Self::WorkExecutionRecoveryEvidenceV1 => "work-execution-recovery-evidence/v1",
            Self::WorkExecutionRecoveryPrepareRequestV1 => {
                "work-execution-recovery-prepare-request/v1"
            }
            Self::WorkExecutionRecoveryPrepareV1 => "work-execution-recovery-prepare/v1",
            Self::WorkExecutionRecoveryRequestV1 => "work-execution-recovery-request/v1",
            Self::WorkExecutionRecoveryV1 => "work-execution-recovery/v1",
            Self::WorkHandoffSourceValidationV1 => "work-handoff-source-validation/v1",
            Self::WorkHandoffValidationV1 => "work-handoff-validation/v1",
            Self::WorkHandoffV1 => "work-handoff/v1",
            Self::WorkHierarchySelectionValidationV1 => "work-hierarchy-selection-validation/v1",
            Self::WorkHierarchySelectionV1 => "work-hierarchy-selection/v1",
            Self::WorkHierarchyV1 => "work-hierarchy/v1",
            Self::WorkInstructionCatalogV1 => "work-instruction-catalog/v1",
            Self::WorkInstructionMigrationPreviewV1 => "work-instruction-migration-preview/v1",
            Self::WorkInstructionMigrationPublicationV1 => {
                "work-instruction-migration-publication/v1"
            }
            Self::WorkInstructionSelectionManifestV1 => "work-instruction-selection-manifest/v1",
            Self::WorkInstructionSelectionV1 => "work-instruction-selection/v1",
            Self::WorkInstructionsV1 => "work-instructions/v1",
            Self::WorkInvocationV1 => "work-invocation/v1",
            Self::WorkOperationEnvelopeV1 => "work-operation-envelope/v1",
            Self::WorkOperationResultV1 => "work-operation-result/v1",
            Self::WorkProgressPrepareV1 => "work-progress-prepare/v1",
            Self::WorkProgressPreviewV1 => "work-progress-preview/v1",
            Self::WorkProgressReadV1 => "work-progress-read/v1",
            Self::WorkProgressSaveRequestV1 => "work-progress-save-request/v1",
            Self::WorkProgressSaveV1 => "work-progress-save/v1",
            Self::WorkRecordBeginV1 => "work-record-begin/v1",
            Self::WorkRecordFinishRequestV1 => "work-record-finish-request/v1",
            Self::WorkRecordFinishV1 => "work-record-finish/v1",
            Self::WorkSkillBundleV1 => "work-skill-bundle/v1",
            Self::WorkSkillCatalogV1 => "work-skill-catalog/v1",
            Self::WorkSkillSelectionValidationV1 => "work-skill-selection-validation/v1",
            Self::WorkSkillSelectionV1 => "work-skill-selection/v1",
            Self::WorkSkillSnapshotV1 => "work-skill-snapshot/v1",
            Self::WorkSourceImpactV1 => "work-source-impact/v1",
            Self::WorkSourceRefreshPreviewV1 => "work-source-refresh-preview/v1",
            Self::WorkSourceRefreshPublicationV1 => "work-source-refresh-publication/v1",
            Self::WorkArtifactMigrationAnalysisV1 => "work-artifact-migration-analysis/v1",
            Self::WorkArtifactMigrationRequestV1 => "work-artifact-migration-request/v1",
            Self::WorkSpecMigrationPrepareRequestV1 => "work-spec-migration-prepare-request/v1",
            Self::WorkSpecMigrationPreviewRequestV1 => "work-spec-migration-preview-request/v1",
            Self::WorkSpecMigrationPreviewV1 => "work-spec-migration-preview/v1",
            Self::WorkSpecMigrationPublicationV1 => "work-spec-migration-publication/v1",
            Self::WorkSpecPrepareRequestV1 => "work-spec-prepare-request/v1",
            Self::WorkSpecPrepareV1 => "work-spec-prepare/v1",
            Self::WorkSpecReconciliationPrepareRequestV1 => {
                "work-spec-reconciliation-prepare-request/v1"
            }
            Self::WorkSpecReconciliationPreviewRequestV1 => {
                "work-spec-reconciliation-preview-request/v1"
            }
            Self::WorkSpecReconciliationPreviewV1 => "work-spec-reconciliation-preview/v1",
            Self::WorkSpecReconciliationPublicationV1 => "work-spec-reconciliation-publication/v1",
            Self::WorkSpecTransactionV1 => "work-spec-transaction/v1",
            Self::WorkSpecUpdateRequestV1 => "work-spec-update-request/v1",
            Self::WorkSpecUpdateV1 => "work-spec-update/v1",
            Self::WorkSpecVerificationRequestV1 => "work-spec-verification-request/v1",
            Self::WorkSpecVerificationV1 => "work-spec-verification/v1",
            Self::WorkTaskCollectionFingerprintV1 => "work-task-collection-fingerprint/v1",
            Self::WorkTaskCollectionProjectionV1 => "work-task-collection-projection/v1",
            Self::WorkTaskCollectionValidationV1 => "work-task-collection-validation/v1",
            Self::WorkTaskDraftPrepareV1 => "work-task-draft-prepare/v1",
            Self::WorkTaskDraftRecoveryV1 => "work-task-draft-recovery/v1",
            Self::WorkTaskDraftSaveV1 => "work-task-draft-save/v1",
            Self::WorkTaskDraftSourceCheckV1 => "work-task-draft-source-check/v1",
            Self::WorkTaskDraftValidationV1 => "work-task-draft-validation/v1",
            Self::WorkTaskDraftV1 => "work-task-draft/v1",
            Self::WorkTaskIndexValidationV1 => "work-task-index-validation/v1",
            Self::WorkTaskIndexV1 => "work-task-index/v1",
            Self::WorkTaskItemValidationV1 => "work-task-item-validation/v1",
            Self::WorkTaskItemV1 => "work-task-item/v1",
            Self::WorkTaskPlanningIndexValidationV1 => "work-task-planning-index-validation/v1",
            Self::WorkTaskPlanningIndexV1 => "work-task-planning-index/v1",
            Self::WorkTaskSemanticRequestV1 => "work-task-semantic-request/v1",
            Self::WorkWorkflowStateV1 => "work-workflow-state/v1",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn all_public_schema_literals_are_unique_and_round_trip() {
        let ids: HashSet<_> = PublicSchema::ALL
            .iter()
            .map(|schema| schema.as_str())
            .collect();
        assert_eq!(ids.len(), 112);
        for schema in PublicSchema::ALL {
            let literal = serde_json::to_value(schema).unwrap();
            assert_eq!(literal, schema.as_str());
            assert_eq!(
                serde_json::from_value::<PublicSchema>(literal).unwrap(),
                schema
            );
        }
    }
}
