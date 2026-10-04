//! Public schema identifiers shared by all Work data models.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PublicSchema {
    #[serde(rename = "work-source-snapshot")]
    WorkSourceSnapshot,
    #[serde(rename = "work-source-read")]
    WorkSourceRead,
    #[serde(rename = "work-source-validation")]
    WorkSourceValidation,
    #[serde(rename = "work-attempt-authorization")]
    WorkAttemptAuthorization,
    #[serde(rename = "work-attempt-close-request")]
    WorkAttemptCloseRequest,
    #[serde(rename = "work-attempt-close")]
    WorkAttemptClose,
    #[serde(rename = "work-attempt-start-prepare-request")]
    WorkAttemptStartPrepareRequest,
    #[serde(rename = "work-attempt-start-prepare")]
    WorkAttemptStartPrepare,
    #[serde(rename = "work-attempt-start-recovery")]
    WorkAttemptStartRecovery,
    #[serde(rename = "work-attempt-start-request")]
    WorkAttemptStartRequest,
    #[serde(rename = "work-attempt-start")]
    WorkAttemptStart,
    #[serde(rename = "work-attempt-validation")]
    WorkAttemptValidation,
    #[serde(rename = "work-attempt")]
    WorkAttempt,
    #[serde(rename = "work-cli-result")]
    WorkCliResult,
    #[serde(rename = "work-command-correction-request")]
    WorkCommandCorrectionRequest,
    #[serde(rename = "work-command-correction")]
    WorkCommandCorrection,
    #[serde(rename = "work-command-preview")]
    WorkCommandPreview,
    #[serde(rename = "work-command-result")]
    WorkCommandResult,
    #[serde(rename = "work-command-run-request")]
    WorkCommandRunRequest,
    #[serde(rename = "work-command-started")]
    WorkCommandStarted,
    #[serde(rename = "work-contract-catalog")]
    WorkContractCatalog,
    #[serde(rename = "work-contract-description")]
    WorkContractDescription,
    #[serde(rename = "work-contract-scaffold")]
    WorkContractScaffold,
    #[serde(rename = "work-correction-create-request")]
    WorkCorrectionCreateRequest,
    #[serde(rename = "work-correction-create")]
    WorkCorrectionCreate,
    #[serde(rename = "work-correction")]
    WorkCorrection,
    #[serde(rename = "work-delegation-build-request")]
    WorkDelegationBuildRequest,
    #[serde(rename = "work-delegation-envelope")]
    WorkDelegationEnvelope,
    #[serde(rename = "work-delegation-validation")]
    WorkDelegationValidation,
    #[serde(rename = "work-discussion-handoff-request")]
    WorkDiscussionHandoffRequest,
    #[serde(rename = "work-discussion-handoff")]
    WorkDiscussionHandoff,
    #[serde(rename = "work-discussion-progress")]
    WorkDiscussionProgress,
    #[serde(rename = "work-error")]
    WorkError,
    #[serde(rename = "work-execute-preflight")]
    WorkExecutePreflight,
    #[serde(rename = "work-execute-worktree-snapshot")]
    WorkExecuteWorktreeSnapshot,
    #[serde(rename = "work-execute-worktree")]
    WorkExecuteWorktree,
    #[serde(rename = "work-execution-deviation-preview")]
    WorkExecutionDeviationPreview,
    #[serde(rename = "work-execution-deviation-proposal")]
    WorkExecutionDeviationProposal,
    #[serde(rename = "work-execution-deviation-record")]
    WorkExecutionDeviationRecord,
    #[serde(rename = "work-execution-deviation-semantic-request")]
    WorkExecutionDeviationSemanticRequest,
    #[serde(rename = "work-execution-deviation")]
    WorkExecutionDeviation,
    #[serde(rename = "work-execution-index")]
    WorkExecutionIndex,
    #[serde(rename = "work-execution-recovery-evidence")]
    WorkExecutionRecoveryEvidence,
    #[serde(rename = "work-execution-recovery-prepare-request")]
    WorkExecutionRecoveryPrepareRequest,
    #[serde(rename = "work-execution-recovery-prepare")]
    WorkExecutionRecoveryPrepare,
    #[serde(rename = "work-execution-recovery-request")]
    WorkExecutionRecoveryRequest,
    #[serde(rename = "work-execution-recovery")]
    WorkExecutionRecovery,
    #[serde(rename = "work-handoff-source-validation")]
    WorkHandoffSourceValidation,
    #[serde(rename = "work-handoff-validation")]
    WorkHandoffValidation,
    #[serde(rename = "work-handoff")]
    WorkHandoff,
    #[serde(rename = "work-hierarchy-selection-validation")]
    WorkHierarchySelectionValidation,
    #[serde(rename = "work-hierarchy-selection")]
    WorkHierarchySelection,
    #[serde(rename = "work-hierarchy")]
    WorkHierarchy,
    #[serde(rename = "work-instruction-catalog")]
    WorkInstructionCatalog,
    #[serde(rename = "work-instruction-selection-manifest")]
    WorkInstructionSelectionManifest,
    #[serde(rename = "work-instruction-selection")]
    WorkInstructionSelection,
    #[serde(rename = "work-instructions")]
    WorkInstructions,
    #[serde(rename = "work-invocation")]
    WorkInvocation,
    #[serde(rename = "work-operation-envelope")]
    WorkOperationEnvelope,
    #[serde(rename = "work-operation-result")]
    WorkOperationResult,
    #[serde(rename = "work-progress-prepare")]
    WorkProgressPrepare,
    #[serde(rename = "work-progress-preview")]
    WorkProgressPreview,
    #[serde(rename = "work-progress-read")]
    WorkProgressRead,
    #[serde(rename = "work-progress-save-request")]
    WorkProgressSaveRequest,
    #[serde(rename = "work-progress-save")]
    WorkProgressSave,
    #[serde(rename = "work-record-begin")]
    WorkRecordBegin,
    #[serde(rename = "work-record-finish-request")]
    WorkRecordFinishRequest,
    #[serde(rename = "work-record-finish")]
    WorkRecordFinish,
    #[serde(rename = "work-skill-bundle")]
    WorkSkillBundle,
    #[serde(rename = "work-skill-catalog")]
    WorkSkillCatalog,
    #[serde(rename = "work-skill-selection-validation")]
    WorkSkillSelectionValidation,
    #[serde(rename = "work-skill-selection")]
    WorkSkillSelection,
    #[serde(rename = "work-skill-snapshot")]
    WorkSkillSnapshot,
    #[serde(rename = "work-source-impact")]
    WorkSourceImpact,
    #[serde(rename = "work-artifact-migration-analysis")]
    WorkArtifactMigrationAnalysis,
    #[serde(rename = "work-artifact-migration-request")]
    WorkArtifactMigrationRequest,
    #[serde(rename = "work-spec-migration-prepare-request")]
    WorkSpecMigrationPrepareRequest,
    #[serde(rename = "work-spec-migration-preview-request")]
    WorkSpecMigrationPreviewRequest,
    #[serde(rename = "work-spec-migration-preview")]
    WorkSpecMigrationPreview,
    #[serde(rename = "work-spec-migration-publication")]
    WorkSpecMigrationPublication,
    #[serde(rename = "work-spec-prepare-request")]
    WorkSpecPrepareRequest,
    #[serde(rename = "work-spec-prepare")]
    WorkSpecPrepare,
    #[serde(rename = "work-spec-reconciliation-prepare-request")]
    WorkSpecReconciliationPrepareRequest,
    #[serde(rename = "work-spec-reconciliation-preview-request")]
    WorkSpecReconciliationPreviewRequest,
    #[serde(rename = "work-spec-reconciliation-preview")]
    WorkSpecReconciliationPreview,
    #[serde(rename = "work-spec-reconciliation-publication")]
    WorkSpecReconciliationPublication,
    #[serde(rename = "work-spec-transaction")]
    WorkSpecTransaction,
    #[serde(rename = "work-spec-update-request")]
    WorkSpecUpdateRequest,
    #[serde(rename = "work-spec-update")]
    WorkSpecUpdate,
    #[serde(rename = "work-spec-verification-request")]
    WorkSpecVerificationRequest,
    #[serde(rename = "work-spec-verification")]
    WorkSpecVerification,
    #[serde(rename = "work-task-collection-fingerprint")]
    WorkTaskCollectionFingerprint,
    #[serde(rename = "work-task-collection-projection")]
    WorkTaskCollectionProjection,
    #[serde(rename = "work-task-collection-validation")]
    WorkTaskCollectionValidation,
    #[serde(rename = "work-task-draft-prepare")]
    WorkTaskDraftPrepare,
    #[serde(rename = "work-task-draft-recovery")]
    WorkTaskDraftRecovery,
    #[serde(rename = "work-task-draft-save")]
    WorkTaskDraftSave,
    #[serde(rename = "work-task-draft-source-check")]
    WorkTaskDraftSourceCheck,
    #[serde(rename = "work-task-draft-validation")]
    WorkTaskDraftValidation,
    #[serde(rename = "work-task-draft")]
    WorkTaskDraft,
    #[serde(rename = "work-task-index-validation")]
    WorkTaskIndexValidation,
    #[serde(rename = "work-task-index")]
    WorkTaskIndex,
    #[serde(rename = "work-task-item-validation")]
    WorkTaskItemValidation,
    #[serde(rename = "work-task-item")]
    WorkTaskItem,
    #[serde(rename = "work-task-planning-index-validation")]
    WorkTaskPlanningIndexValidation,
    #[serde(rename = "work-task-planning-index")]
    WorkTaskPlanningIndex,
    #[serde(rename = "work-task-semantic-request")]
    WorkTaskSemanticRequest,
    #[serde(rename = "work-workflow-state")]
    WorkWorkflowState,
}

impl PublicSchema {
    pub const ALL: [Self; 108] = [
        Self::WorkSourceSnapshot,
        Self::WorkSourceRead,
        Self::WorkSourceValidation,
        Self::WorkAttemptAuthorization,
        Self::WorkAttemptCloseRequest,
        Self::WorkAttemptClose,
        Self::WorkAttemptStartPrepareRequest,
        Self::WorkAttemptStartPrepare,
        Self::WorkAttemptStartRecovery,
        Self::WorkAttemptStartRequest,
        Self::WorkAttemptStart,
        Self::WorkAttemptValidation,
        Self::WorkAttempt,
        Self::WorkCliResult,
        Self::WorkCommandCorrectionRequest,
        Self::WorkCommandCorrection,
        Self::WorkCommandPreview,
        Self::WorkCommandResult,
        Self::WorkCommandRunRequest,
        Self::WorkCommandStarted,
        Self::WorkContractCatalog,
        Self::WorkContractDescription,
        Self::WorkContractScaffold,
        Self::WorkCorrectionCreateRequest,
        Self::WorkCorrectionCreate,
        Self::WorkCorrection,
        Self::WorkDelegationBuildRequest,
        Self::WorkDelegationEnvelope,
        Self::WorkDelegationValidation,
        Self::WorkDiscussionHandoffRequest,
        Self::WorkDiscussionHandoff,
        Self::WorkDiscussionProgress,
        Self::WorkError,
        Self::WorkExecutePreflight,
        Self::WorkExecuteWorktreeSnapshot,
        Self::WorkExecuteWorktree,
        Self::WorkExecutionDeviationPreview,
        Self::WorkExecutionDeviationProposal,
        Self::WorkExecutionDeviationRecord,
        Self::WorkExecutionDeviationSemanticRequest,
        Self::WorkExecutionDeviation,
        Self::WorkExecutionIndex,
        Self::WorkExecutionRecoveryEvidence,
        Self::WorkExecutionRecoveryPrepareRequest,
        Self::WorkExecutionRecoveryPrepare,
        Self::WorkExecutionRecoveryRequest,
        Self::WorkExecutionRecovery,
        Self::WorkHandoffSourceValidation,
        Self::WorkHandoffValidation,
        Self::WorkHandoff,
        Self::WorkHierarchySelectionValidation,
        Self::WorkHierarchySelection,
        Self::WorkHierarchy,
        Self::WorkInstructionCatalog,
        Self::WorkInstructionSelectionManifest,
        Self::WorkInstructionSelection,
        Self::WorkInstructions,
        Self::WorkInvocation,
        Self::WorkOperationEnvelope,
        Self::WorkOperationResult,
        Self::WorkProgressPrepare,
        Self::WorkProgressPreview,
        Self::WorkProgressRead,
        Self::WorkProgressSaveRequest,
        Self::WorkProgressSave,
        Self::WorkRecordBegin,
        Self::WorkRecordFinishRequest,
        Self::WorkRecordFinish,
        Self::WorkSkillBundle,
        Self::WorkSkillCatalog,
        Self::WorkSkillSelectionValidation,
        Self::WorkSkillSelection,
        Self::WorkSkillSnapshot,
        Self::WorkSourceImpact,
        Self::WorkArtifactMigrationAnalysis,
        Self::WorkArtifactMigrationRequest,
        Self::WorkSpecMigrationPrepareRequest,
        Self::WorkSpecMigrationPreviewRequest,
        Self::WorkSpecMigrationPreview,
        Self::WorkSpecMigrationPublication,
        Self::WorkSpecPrepareRequest,
        Self::WorkSpecPrepare,
        Self::WorkSpecReconciliationPrepareRequest,
        Self::WorkSpecReconciliationPreviewRequest,
        Self::WorkSpecReconciliationPreview,
        Self::WorkSpecReconciliationPublication,
        Self::WorkSpecTransaction,
        Self::WorkSpecUpdateRequest,
        Self::WorkSpecUpdate,
        Self::WorkSpecVerificationRequest,
        Self::WorkSpecVerification,
        Self::WorkTaskCollectionFingerprint,
        Self::WorkTaskCollectionProjection,
        Self::WorkTaskCollectionValidation,
        Self::WorkTaskDraftPrepare,
        Self::WorkTaskDraftRecovery,
        Self::WorkTaskDraftSave,
        Self::WorkTaskDraftSourceCheck,
        Self::WorkTaskDraftValidation,
        Self::WorkTaskDraft,
        Self::WorkTaskIndexValidation,
        Self::WorkTaskIndex,
        Self::WorkTaskItemValidation,
        Self::WorkTaskItem,
        Self::WorkTaskPlanningIndexValidation,
        Self::WorkTaskPlanningIndex,
        Self::WorkTaskSemanticRequest,
        Self::WorkWorkflowState,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkSourceSnapshot => "work-source-snapshot",
            Self::WorkSourceRead => "work-source-read",
            Self::WorkSourceValidation => "work-source-validation",
            Self::WorkAttemptAuthorization => "work-attempt-authorization",
            Self::WorkAttemptCloseRequest => "work-attempt-close-request",
            Self::WorkAttemptClose => "work-attempt-close",
            Self::WorkAttemptStartPrepareRequest => "work-attempt-start-prepare-request",
            Self::WorkAttemptStartPrepare => "work-attempt-start-prepare",
            Self::WorkAttemptStartRecovery => "work-attempt-start-recovery",
            Self::WorkAttemptStartRequest => "work-attempt-start-request",
            Self::WorkAttemptStart => "work-attempt-start",
            Self::WorkAttemptValidation => "work-attempt-validation",
            Self::WorkAttempt => "work-attempt",
            Self::WorkCliResult => "work-cli-result",
            Self::WorkCommandCorrectionRequest => "work-command-correction-request",
            Self::WorkCommandCorrection => "work-command-correction",
            Self::WorkCommandPreview => "work-command-preview",
            Self::WorkCommandResult => "work-command-result",
            Self::WorkCommandRunRequest => "work-command-run-request",
            Self::WorkCommandStarted => "work-command-started",
            Self::WorkContractCatalog => "work-contract-catalog",
            Self::WorkContractDescription => "work-contract-description",
            Self::WorkContractScaffold => "work-contract-scaffold",
            Self::WorkCorrectionCreateRequest => "work-correction-create-request",
            Self::WorkCorrectionCreate => "work-correction-create",
            Self::WorkCorrection => "work-correction",
            Self::WorkDelegationBuildRequest => "work-delegation-build-request",
            Self::WorkDelegationEnvelope => "work-delegation-envelope",
            Self::WorkDelegationValidation => "work-delegation-validation",
            Self::WorkDiscussionHandoffRequest => "work-discussion-handoff-request",
            Self::WorkDiscussionHandoff => "work-discussion-handoff",
            Self::WorkDiscussionProgress => "work-discussion-progress",
            Self::WorkError => "work-error",
            Self::WorkExecutePreflight => "work-execute-preflight",
            Self::WorkExecuteWorktreeSnapshot => "work-execute-worktree-snapshot",
            Self::WorkExecuteWorktree => "work-execute-worktree",
            Self::WorkExecutionDeviationPreview => "work-execution-deviation-preview",
            Self::WorkExecutionDeviationProposal => "work-execution-deviation-proposal",
            Self::WorkExecutionDeviationRecord => "work-execution-deviation-record",
            Self::WorkExecutionDeviationSemanticRequest => {
                "work-execution-deviation-semantic-request"
            }
            Self::WorkExecutionDeviation => "work-execution-deviation",
            Self::WorkExecutionIndex => "work-execution-index",
            Self::WorkExecutionRecoveryEvidence => "work-execution-recovery-evidence",
            Self::WorkExecutionRecoveryPrepareRequest => "work-execution-recovery-prepare-request",
            Self::WorkExecutionRecoveryPrepare => "work-execution-recovery-prepare",
            Self::WorkExecutionRecoveryRequest => "work-execution-recovery-request",
            Self::WorkExecutionRecovery => "work-execution-recovery",
            Self::WorkHandoffSourceValidation => "work-handoff-source-validation",
            Self::WorkHandoffValidation => "work-handoff-validation",
            Self::WorkHandoff => "work-handoff",
            Self::WorkHierarchySelectionValidation => "work-hierarchy-selection-validation",
            Self::WorkHierarchySelection => "work-hierarchy-selection",
            Self::WorkHierarchy => "work-hierarchy",
            Self::WorkInstructionCatalog => "work-instruction-catalog",
            Self::WorkInstructionSelectionManifest => "work-instruction-selection-manifest",
            Self::WorkInstructionSelection => "work-instruction-selection",
            Self::WorkInstructions => "work-instructions",
            Self::WorkInvocation => "work-invocation",
            Self::WorkOperationEnvelope => "work-operation-envelope",
            Self::WorkOperationResult => "work-operation-result",
            Self::WorkProgressPrepare => "work-progress-prepare",
            Self::WorkProgressPreview => "work-progress-preview",
            Self::WorkProgressRead => "work-progress-read",
            Self::WorkProgressSaveRequest => "work-progress-save-request",
            Self::WorkProgressSave => "work-progress-save",
            Self::WorkRecordBegin => "work-record-begin",
            Self::WorkRecordFinishRequest => "work-record-finish-request",
            Self::WorkRecordFinish => "work-record-finish",
            Self::WorkSkillBundle => "work-skill-bundle",
            Self::WorkSkillCatalog => "work-skill-catalog",
            Self::WorkSkillSelectionValidation => "work-skill-selection-validation",
            Self::WorkSkillSelection => "work-skill-selection",
            Self::WorkSkillSnapshot => "work-skill-snapshot",
            Self::WorkSourceImpact => "work-source-impact",
            Self::WorkArtifactMigrationAnalysis => "work-artifact-migration-analysis",
            Self::WorkArtifactMigrationRequest => "work-artifact-migration-request",
            Self::WorkSpecMigrationPrepareRequest => "work-spec-migration-prepare-request",
            Self::WorkSpecMigrationPreviewRequest => "work-spec-migration-preview-request",
            Self::WorkSpecMigrationPreview => "work-spec-migration-preview",
            Self::WorkSpecMigrationPublication => "work-spec-migration-publication",
            Self::WorkSpecPrepareRequest => "work-spec-prepare-request",
            Self::WorkSpecPrepare => "work-spec-prepare",
            Self::WorkSpecReconciliationPrepareRequest => {
                "work-spec-reconciliation-prepare-request"
            }
            Self::WorkSpecReconciliationPreviewRequest => {
                "work-spec-reconciliation-preview-request"
            }
            Self::WorkSpecReconciliationPreview => "work-spec-reconciliation-preview",
            Self::WorkSpecReconciliationPublication => "work-spec-reconciliation-publication",
            Self::WorkSpecTransaction => "work-spec-transaction",
            Self::WorkSpecUpdateRequest => "work-spec-update-request",
            Self::WorkSpecUpdate => "work-spec-update",
            Self::WorkSpecVerificationRequest => "work-spec-verification-request",
            Self::WorkSpecVerification => "work-spec-verification",
            Self::WorkTaskCollectionFingerprint => "work-task-collection-fingerprint",
            Self::WorkTaskCollectionProjection => "work-task-collection-projection",
            Self::WorkTaskCollectionValidation => "work-task-collection-validation",
            Self::WorkTaskDraftPrepare => "work-task-draft-prepare",
            Self::WorkTaskDraftRecovery => "work-task-draft-recovery",
            Self::WorkTaskDraftSave => "work-task-draft-save",
            Self::WorkTaskDraftSourceCheck => "work-task-draft-source-check",
            Self::WorkTaskDraftValidation => "work-task-draft-validation",
            Self::WorkTaskDraft => "work-task-draft",
            Self::WorkTaskIndexValidation => "work-task-index-validation",
            Self::WorkTaskIndex => "work-task-index",
            Self::WorkTaskItemValidation => "work-task-item-validation",
            Self::WorkTaskItem => "work-task-item",
            Self::WorkTaskPlanningIndexValidation => "work-task-planning-index-validation",
            Self::WorkTaskPlanningIndex => "work-task-planning-index",
            Self::WorkTaskSemanticRequest => "work-task-semantic-request",
            Self::WorkWorkflowState => "work-workflow-state",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn current_hierarchy_response_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-hierarchy-selection-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-hierarchy-selection-validation"
            ))
            .unwrap()
            .as_str(),
            "work-hierarchy-selection-validation"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-hierarchy/v1")).is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-hierarchy"))
                .unwrap()
                .as_str(),
            "work-hierarchy"
        );
    }

    #[test]
    fn current_hierarchy_selection_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-hierarchy-selection/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-hierarchy-selection"))
                .unwrap()
                .as_str(),
            "work-hierarchy-selection"
        );
    }

    #[test]
    fn current_skill_bundle_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-bundle/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-bundle"))
                .unwrap()
                .as_str(),
            "work-skill-bundle"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-catalog/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-catalog"))
                .unwrap()
                .as_str(),
            "work-skill-catalog"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-skill-selection-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-skill-selection-validation"
            ))
            .unwrap()
            .as_str(),
            "work-skill-selection-validation"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-snapshot/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-snapshot"))
                .unwrap()
                .as_str(),
            "work-skill-snapshot"
        );
    }

    #[test]
    fn current_skill_selection_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-selection/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-skill-selection"))
                .unwrap()
                .as_str(),
            "work-skill-selection"
        );
    }

    #[test]
    fn current_instruction_catalog_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-instruction-catalog/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-instruction-catalog"))
                .unwrap()
                .as_str(),
            "work-instruction-catalog"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-instruction-selection/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-instruction-selection"))
                .unwrap()
                .as_str(),
            "work-instruction-selection"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-instructions/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-instructions"))
                .unwrap()
                .as_str(),
            "work-instructions"
        );
    }

    #[test]
    fn current_instruction_selection_manifest_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-instruction-selection-manifest/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-instruction-selection-manifest"
            ))
            .unwrap()
            .as_str(),
            "work-instruction-selection-manifest"
        );
    }

    #[test]
    fn current_invocation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-invocation/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-invocation"))
                .unwrap()
                .as_str(),
            "work-invocation"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-operation-envelope/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-operation-envelope"))
                .unwrap()
                .as_str(),
            "work-operation-envelope"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-operation-result/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-operation-result"))
                .unwrap()
                .as_str(),
            "work-operation-result"
        );
    }

    #[test]
    fn current_task_item_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-item/v1")).is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-item"))
                .unwrap()
                .as_str(),
            "work-task-item"
        );
    }

    #[test]
    fn current_task_item_validation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-item-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-item-validation"))
                .unwrap()
                .as_str(),
            "work-task-item-validation"
        );
    }

    #[test]
    fn current_task_index_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-index/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-index"))
                .unwrap()
                .as_str(),
            "work-task-index"
        );
    }

    #[test]
    fn current_task_index_validation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-index-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-index-validation"))
                .unwrap()
                .as_str(),
            "work-task-index-validation"
        );
    }

    #[test]
    fn current_task_collection_diagnostics_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-diagnostics/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-diagnostics"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-fingerprint/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-fingerprint"
            ))
            .unwrap()
            .as_str(),
            "work-task-collection-fingerprint"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-projection/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-projection"
            ))
            .unwrap()
            .as_str(),
            "work-task-collection-projection"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-collection-validation"
            ))
            .unwrap()
            .as_str(),
            "work-task-collection-validation"
        );
    }

    #[test]
    fn current_task_planning_index_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-planning-index/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-planning-index"))
                .unwrap()
                .as_str(),
            "work-task-planning-index"
        );
    }

    #[test]
    fn current_task_planning_index_validation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-planning-index-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-planning-index-validation"
            ))
            .unwrap()
            .as_str(),
            "work-task-planning-index-validation"
        );
    }

    #[test]
    fn current_task_draft_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft"))
                .unwrap()
                .as_str(),
            "work-task-draft"
        );
    }

    #[test]
    fn current_task_draft_assembly_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-assembly/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-list-update/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-prepare/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-prepare"))
                .unwrap()
                .as_str(),
            "work-task-draft-prepare"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-recovery/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-recovery"))
                .unwrap()
                .as_str(),
            "work-task-draft-recovery"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-save/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-save"))
                .unwrap()
                .as_str(),
            "work-task-draft-save"
        );
    }

    #[test]
    fn current_task_draft_source_check_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-source-check/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-source-check"
            ))
            .unwrap()
            .as_str(),
            "work-task-draft-source-check"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-source-update/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-status/v1"))
                .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-draft-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-draft-validation"))
                .unwrap()
                .as_str(),
            "work-task-draft-validation"
        );
    }

    #[test]
    fn current_task_create_recovery_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-create-recovery/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-create/v1"))
                .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-prepare/v1"))
                .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-semantic-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-task-semantic-request"))
                .unwrap()
                .as_str(),
            "work-task-semantic-request"
        );
    }

    #[test]
    fn current_task_execution_validation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-execution-validation/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-task-execution-view/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_discussion_progress_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-discussion-progress/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-discussion-progress"))
                .unwrap()
                .as_str(),
            "work-discussion-progress"
        );
    }

    #[test]
    fn current_progress_prepare_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-prepare/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-prepare"))
                .unwrap()
                .as_str(),
            "work-progress-prepare"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-preview/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-preview"))
                .unwrap()
                .as_str(),
            "work-progress-preview"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-read/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-read"))
                .unwrap()
                .as_str(),
            "work-progress-read"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-progress-save-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-save-request"))
                .unwrap()
                .as_str(),
            "work-progress-save-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-save/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-progress-save"))
                .unwrap()
                .as_str(),
            "work-progress-save"
        );
    }

    #[test]
    fn current_discussion_handoff_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-discussion-handoff-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-discussion-handoff-request"
            ))
            .unwrap()
            .as_str(),
            "work-discussion-handoff-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-discussion-handoff/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-discussion-handoff"))
                .unwrap()
                .as_str(),
            "work-discussion-handoff"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-handoff-source-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-handoff-source-validation"
            ))
            .unwrap()
            .as_str(),
            "work-handoff-source-validation"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-handoff-validation/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-handoff-validation"))
                .unwrap()
                .as_str(),
            "work-handoff-validation"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-handoff/v1")).is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-handoff"))
                .unwrap()
                .as_str(),
            "work-handoff"
        );
    }

    #[test]
    fn current_delegation_build_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-delegation-build-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-delegation-build-request"
            ))
            .unwrap()
            .as_str(),
            "work-delegation-build-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-delegation-envelope/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-delegation-envelope"))
                .unwrap()
                .as_str(),
            "work-delegation-envelope"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-delegation-validation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-delegation-validation"))
                .unwrap()
                .as_str(),
            "work-delegation-validation"
        );
    }

    #[test]
    fn current_correction_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-correction/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-correction"))
                .unwrap()
                .as_str(),
            "work-correction"
        );
    }

    #[test]
    fn current_correction_create_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-correction-create-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-correction-create-request"
            ))
            .unwrap()
            .as_str(),
            "work-correction-create-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-correction-create/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-correction-create"))
                .unwrap()
                .as_str(),
            "work-correction-create"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-correction-validation/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_attempt_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt/v1")).is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt"))
                .unwrap()
                .as_str(),
            "work-attempt"
        );
    }

    #[test]
    fn current_attempt_authorization_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-authorization/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-authorization"))
                .unwrap()
                .as_str(),
            "work-attempt-authorization"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-validation/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-validation"))
                .unwrap()
                .as_str(),
            "work-attempt-validation"
        );
    }

    #[test]
    fn current_attempt_start_prepare_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-start-prepare-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-start-prepare-request"
            ))
            .unwrap()
            .as_str(),
            "work-attempt-start-prepare-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-start-prepare/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-start-prepare"))
                .unwrap()
                .as_str(),
            "work-attempt-start-prepare"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-start-recovery/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-start-recovery"
            ))
            .unwrap()
            .as_str(),
            "work-attempt-start-recovery"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-start-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-start-request"))
                .unwrap()
                .as_str(),
            "work-attempt-start-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-start/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-start"))
                .unwrap()
                .as_str(),
            "work-attempt-start"
        );
    }

    #[test]
    fn current_attempt_close_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-attempt-close-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-close-request"))
                .unwrap()
                .as_str(),
            "work-attempt-close-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-close/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-attempt-close"))
                .unwrap()
                .as_str(),
            "work-attempt-close"
        );
    }

    #[test]
    fn current_record_begin_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-record-begin/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-record-begin"))
                .unwrap()
                .as_str(),
            "work-record-begin"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-record-finish-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-record-finish-request"))
                .unwrap()
                .as_str(),
            "work-record-finish-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-record-finish/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-record-finish"))
                .unwrap()
                .as_str(),
            "work-record-finish"
        );
    }

    #[test]
    fn current_command_preview_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-preview/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-preview"))
                .unwrap()
                .as_str(),
            "work-command-preview"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-result/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-result"))
                .unwrap()
                .as_str(),
            "work-command-result"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-command-run-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-run-request"))
                .unwrap()
                .as_str(),
            "work-command-run-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-started/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-started"))
                .unwrap()
                .as_str(),
            "work-command-started"
        );
    }

    #[test]
    fn current_command_correction_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-command-correction-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-command-correction-request"
            ))
            .unwrap()
            .as_str(),
            "work-command-correction-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-correction/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-correction"))
                .unwrap()
                .as_str(),
            "work-command-correction"
        );
    }

    #[test]
    fn current_execution_index_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execution-index/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execution-index"))
                .unwrap()
                .as_str(),
            "work-execution-index"
        );
    }

    #[test]
    fn current_execution_index_validation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-index-validation/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_execute_preflight_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execute-preflight/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execute-preflight"))
                .unwrap()
                .as_str(),
            "work-execute-preflight"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execute-worktree-snapshot/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execute-worktree-snapshot"
            ))
            .unwrap()
            .as_str(),
            "work-execute-worktree-snapshot"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execute-worktree/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execute-worktree"))
                .unwrap()
                .as_str(),
            "work-execute-worktree"
        );
    }

    #[test]
    fn current_execution_deviation_authorization_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-authorization/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-preview/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-preview"
            ))
            .unwrap()
            .as_str(),
            "work-execution-deviation-preview"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-proposal/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-proposal"
            ))
            .unwrap()
            .as_str(),
            "work-execution-deviation-proposal"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-record/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-record"
            ))
            .unwrap()
            .as_str(),
            "work-execution-deviation-record"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-semantic-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation-semantic-request"
            ))
            .unwrap()
            .as_str(),
            "work-execution-deviation-semantic-request"
        );
    }

    #[test]
    fn current_execution_deviation_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-deviation/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execution-deviation"))
                .unwrap()
                .as_str(),
            "work-execution-deviation"
        );
    }

    #[test]
    fn current_execution_recovery_evidence_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-evidence/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-evidence"
            ))
            .unwrap()
            .as_str(),
            "work-execution-recovery-evidence"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-prepare-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-prepare-request"
            ))
            .unwrap()
            .as_str(),
            "work-execution-recovery-prepare-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-prepare/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-prepare"
            ))
            .unwrap()
            .as_str(),
            "work-execution-recovery-prepare"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-execution-recovery-request"
            ))
            .unwrap()
            .as_str(),
            "work-execution-recovery-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execution-recovery/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-execution-recovery"))
                .unwrap()
                .as_str(),
            "work-execution-recovery"
        );
    }

    #[test]
    fn current_spec_prepare_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-prepare-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-prepare-request"))
                .unwrap()
                .as_str(),
            "work-spec-prepare-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-prepare/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-prepare"))
                .unwrap()
                .as_str(),
            "work-spec-prepare"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-update-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-update-request"))
                .unwrap()
                .as_str(),
            "work-spec-update-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-update/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-update"))
                .unwrap()
                .as_str(),
            "work-spec-update"
        );
    }

    #[test]
    fn current_spec_verification_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-verification-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-verification-request"
            ))
            .unwrap()
            .as_str(),
            "work-spec-verification-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-verification/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-verification"))
                .unwrap()
                .as_str(),
            "work-spec-verification"
        );
    }

    #[test]
    fn current_spec_transaction_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-transaction/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-spec-transaction"))
                .unwrap()
                .as_str(),
            "work-spec-transaction"
        );
    }

    #[test]
    fn current_spec_migration_prepare_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-prepare-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-prepare-request"
            ))
            .unwrap()
            .as_str(),
            "work-spec-migration-prepare-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-preview-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-preview-request"
            ))
            .unwrap()
            .as_str(),
            "work-spec-migration-preview-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-preview/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-preview"
            ))
            .unwrap()
            .as_str(),
            "work-spec-migration-preview"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-publication/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-publication"
            ))
            .unwrap()
            .as_str(),
            "work-spec-migration-publication"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-migration-verification/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_artifact_migration_analysis_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-analysis/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-analysis"
            ))
            .unwrap()
            .as_str(),
            "work-artifact-migration-analysis"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-decisions/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-prepared/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-preview/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-request"
            ))
            .unwrap()
            .as_str(),
            "work-artifact-migration-request"
        );
    }

    #[test]
    fn current_artifact_migration_result_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-artifact-migration-result/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_spec_reconciliation_ledger_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-ledger/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_spec_reconciliation_prepare_request_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-prepare-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-prepare-request"
            ))
            .unwrap()
            .as_str(),
            "work-spec-reconciliation-prepare-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-preview-request/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-preview-request"
            ))
            .unwrap()
            .as_str(),
            "work-spec-reconciliation-preview-request"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-preview/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-preview"
            ))
            .unwrap()
            .as_str(),
            "work-spec-reconciliation-preview"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-publication/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-spec-reconciliation-publication"
            ))
            .unwrap()
            .as_str(),
            "work-spec-reconciliation-publication"
        );
    }

    #[test]
    fn current_source_impact_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-source-impact/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-source-impact"))
                .unwrap()
                .as_str(),
            "work-source-impact"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-workflow-state/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-workflow-state"))
                .unwrap()
                .as_str(),
            "work-workflow-state"
        );
    }

    #[test]
    fn current_cli_result_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-cli-result/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-cli-result"))
                .unwrap()
                .as_str(),
            "work-cli-result"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-error/v1")).is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-error"))
                .unwrap()
                .as_str(),
            "work-error"
        );
    }

    #[test]
    fn current_fingerprint_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-fingerprint/v1"))
                .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-paths/v1")).is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-specification-summary/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-transaction-workspace/v1"
            ))
            .is_err()
        );
    }

    #[test]
    fn current_contract_catalog_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-contract-catalog/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-contract-catalog"))
                .unwrap()
                .as_str(),
            "work-contract-catalog"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-contract-description/v1"
            ))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-contract-description"))
                .unwrap()
                .as_str(),
            "work-contract-description"
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!(
                "work-contract-registry-snapshot/v1"
            ))
            .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-contract-scaffold/v1"))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-contract-scaffold"))
                .unwrap()
                .as_str(),
            "work-contract-scaffold"
        );
    }

    #[test]
    fn current_command_tree_ids_reject_versioned_aliases() {
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-command-tree/v1"))
                .is_err()
        );
        assert!(
            serde_json::from_value::<PublicSchema>(serde_json::json!("work-migration-fixtures/v1"))
                .is_err()
        );
    }

    #[test]
    fn all_public_schema_literals_are_unique_and_round_trip() {
        let ids: HashSet<_> = PublicSchema::ALL
            .iter()
            .map(|schema| schema.as_str())
            .collect();
        assert_eq!(ids.len(), 108);
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
