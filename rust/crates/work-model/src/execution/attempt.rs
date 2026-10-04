//! Attempt authorization, records, and completion evidence.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::deviation::ExecutionDeviation;
use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

/// The existing authorization validator accepts identity rows with producer-specific fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationScopeEntry {
    pub id: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReapprovalCondition {
    ScopeExpansion,
    SourceOrWorktreeDrift,
    FailureDivergence,
    Retry,
    Recovery,
    UnknownResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptAuthorization {
    pub schema: PublicSchema,
    pub task_id: String,
    pub commands: Vec<AuthorizationScopeEntry>,
    pub validations: Vec<AuthorizationScopeEntry>,
    pub modifiable_files: Vec<String>,
    pub working_directories: Vec<String>,
    pub external_operations: Vec<AuthorizationScopeEntry>,
    /// Existing stored authorizations admit producer-specific deviation payloads.
    pub allowed_deviations: Vec<Value>,
    pub reapproval_conditions: Vec<ReapprovalCondition>,
    pub authorization_evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    InProgress,
    Completed,
    Stopped,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalType {
    SpecificationDefect,
    InstructionsChanged,
    ValidationFailed,
    UnexpectedChange,
    ExternalOperationFailed,
    UserStopped,
    Environment,
    ExternalService,
    Permission,
    RequiredInput,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedRecord {
    pub source_attempt_id: String,
    pub record_id: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum BareCommand {
    Argv { argv: Vec<String> },
    Shell { script: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCorrection {
    pub original_command: BareCommand,
    pub actual_command: BareCommand,
    pub reason: String,
    pub authorization_evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkippedStatus {
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Command,
    Operation,
    Validation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkippedRecord {
    pub id: String,
    pub kind: RecordKind,
    pub status: SkippedStatus,
    pub reason: String,
    pub deviation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRecord {
    pub id: String,
    pub kind: RecordKind,
    pub exit_code: i64,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<CommandCorrection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationOutcome {
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub id: String,
    pub kind: RecordKind,
    pub outcome: OperationOutcome,
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOutcome {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRecord {
    pub id: String,
    pub kind: RecordKind,
    pub outcome: ValidationOutcome,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttemptRecord {
    Skipped(SkippedRecord),
    Command(CommandRecord),
    Operation(OperationRecord),
    Validation(ValidationRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallResultStatus {
    CompleteSuccess,
    PartialSuccess,
    Failure,
    UncertainResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverallResult {
    pub status: OverallResultStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_effective: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub schema: PublicSchema,
    pub attempt_id: String,
    pub task_spec_id: String,
    pub task_id: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub status: AttemptStatus,
    pub task_collection_sha256: String,
    pub task_index_sha256: String,
    pub task_item_sha256: String,
    pub task_instructions_sha256: String,
    pub execute_instructions_sha256: String,
    pub hierarchy_selection_sha256: String,
    pub execute_skill_selection_sha256: String,
    pub authorization: AttemptAuthorization,
    pub authorization_sha256: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continued_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried_records: Option<Vec<CarriedRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_deviations: Option<Vec<ExecutionDeviation>>,
    pub records: Vec<AttemptRecord>,
    pub acceptance_results: Vec<super::acceptance::AcceptanceProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_result: Option<OverallResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_type: Option<FinalType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closing_authorization_evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}
