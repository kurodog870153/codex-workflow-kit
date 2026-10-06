//! Caller and generated Execution requests.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable};
use crate::schema::PublicSchema;

use super::attempt::{AttemptAuthorization, AttemptStatus, BareCommand, FinalType};
use super::deviation::{DeviationAction, DeviationImpact, DeviationProposal};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptCloseRequest {
    pub schema: PublicSchema,
    pub status: AttemptStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_type: Option<FinalType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedRecordChoice {
    pub position: u64,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviationAnchorKind {
    Command,
    Validation,
    Operation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedDeviationChoice {
    pub anchor_kind: DeviationAnchorKind,
    pub anchor_position: u64,
    pub action: SemanticDeviationAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptStartPrepareRequest {
    pub command_positions: Vec<u64>,
    pub validation_positions: Vec<u64>,
    pub modifiable_files: Vec<String>,
    pub external_operation_positions: Vec<u64>,
    pub allowed_deviations: Vec<AllowedDeviationChoice>,
    pub authorization_evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried_records: Option<Vec<CarriedRecordChoice>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptStartContinuation {
    pub source_attempt_id: String,
    pub carried_records: Vec<ContinuationCarriedRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationCarriedRecord {
    pub record_id: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptStartRequest {
    pub schema: PublicSchema,
    pub worktree_snapshot_sha256: String,
    pub authorization: AttemptAuthorization,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<AttemptStartContinuation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCorrectionRequest {
    pub schema: PublicSchema,
    pub actual_command: BareCommand,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRunRequest {
    pub schema: PublicSchema,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionCreateRequest {
    pub schema: PublicSchema,
    pub target_attempt_id: String,
    pub field: String,
    pub correct_value: String,
    pub reason: String,
    pub invalidates_completion: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviationSemanticRequest {
    pub gap: String,
    pub action: SemanticDeviationAction,
    pub modifiable_files: Vec<String>,
    pub impact: DeviationImpact,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticDeviationAction {
    ReplaceCommand { replacement: SemanticCommand },
    AddCommand { command: SemanticCommand },
    AddValidation { validation: SemanticValidation },
    SkipRecord { reason: String },
    AdjustOperation { operation: SemanticOperation },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCommandMode {
    Argv,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticCommand {
    pub mode: SemanticCommandMode,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub argv: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub script: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution: Option<Nullable<crate::task::index::TaskExecutionDefaults>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticValidationKind {
    Automated,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticValidation {
    pub kind: SemanticValidationKind,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub command_positions: Option<Nullable<Vec<u64>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub pass_condition: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirmer: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub criteria: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub acceptance_positions: Option<Nullable<Vec<u64>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOperation {
    pub kind: crate::task::item::TaskOperationKind,
    pub action: String,
    pub target: String,
    pub validation_position: u64,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub command_position: Option<Nullable<u64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryTransaction {
    RecordBegin,
    RecordFinish,
    AttemptClose,
    Correction,
    DeviationRecord,
    CommandCorrection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecoveryPrepareRequest {
    pub schema: PublicSchema,
    pub transaction: RecoveryTransaction,
    pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyExecutionRecoveryRequest {
    pub schema: PublicSchema,
    pub transaction: RecoveryTransaction,
    pub attempt_id: String,
    pub transaction_files: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub authorization_evidence: Option<Nullable<String>>,
}

pub type ExecutionRecoveryRequest = PreparedExecutionRecoveryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedExecutionRecoveryRequest {
    pub schema: PublicSchema,
    pub transaction: RecoveryTransaction,
    pub attempt_id: String,
    pub transaction_dir: String,
    pub transaction_files: Vec<String>,
    pub transaction_evidence_sha256: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub authorization_evidence: Option<Nullable<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordFinishOutcome {
    Passed,
    Failed,
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordFinishData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RecordFinishOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordFinishRequest {
    pub schema: PublicSchema,
    pub record: RecordFinishData,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub modified_files: Option<Nullable<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_evidence: Option<String>,
}

pub type ExecutionDeviationProposal = DeviationProposal;
pub type AuthorizedDeviationAction = DeviationAction;

/// Assert that a request accepted by an existing validator has its public shape.
pub fn verified<T: DeserializeOwned>(value: &Value) {
    let _: T =
        serde_json::from_value(value.clone()).expect("validated Execution request matches model");
}

#[cfg(test)]
mod staging_request_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn candidate_recovery_request_requires_directory_and_rejects_unknown_fields() {
        let request = json!({"schema":"work-execution-recovery-request","transaction":"record_finish",
            "attempt_id":"ATTEMPT-001","transaction_dir":"outputs/work/runtime/staging/example/record-finish/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "transaction_files":["attempt.json.tmp","index.json.tmp","transaction.json"],"transaction_evidence_sha256":"b".repeat(64)});
        let typed: PreparedExecutionRecoveryRequest =
            serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), request);
        assert!(serde_json::from_value::<LegacyExecutionRecoveryRequest>(request.clone()).is_err());
        let mut missing = request.clone();
        missing.as_object_mut().unwrap().remove("transaction_dir");
        assert!(
            serde_json::from_value::<PreparedExecutionRecoveryRequest>(missing.clone()).is_err()
        );
        missing
            .as_object_mut()
            .unwrap()
            .remove("transaction_evidence_sha256");
        assert!(serde_json::from_value::<LegacyExecutionRecoveryRequest>(missing).is_ok());
        let mut unknown = request;
        unknown["execution_dir"] = json!("fallback");
        assert!(serde_json::from_value::<PreparedExecutionRecoveryRequest>(unknown).is_err());
    }
}
