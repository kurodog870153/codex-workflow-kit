//! Public Execution validation, preview, publication, and recovery results.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;
use crate::task::item::TaskInputKind;

use super::attempt::AttemptStatus;
use super::deviation::{DeviationClassification, DeviationProposal};
use super::index::{ExecutionLock, ExecutionOverallStatus, ExecutionTaskStatus};
use super::recovery::ExecutionRecoveryEvidence;
use super::request::{AttemptStartRequest, CommandRunRequest, RecoveryTransaction};

/// Preserve the producer's JSON while checking its fixed public shape.
pub fn verified<T: DeserializeOwned>(value: Value) -> Value {
    let _: T = serde_json::from_value(value.clone()).expect("Execution result matches its model");
    value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLockStatus {
    Held,
    Released,
    RecordReserved,
    AttemptHeld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingDeviation {
    pub deviation_id: String,
    pub classification: DeviationClassification,
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptCloseResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub attempt_path: String,
    pub index_path: String,
    pub attempt_status: AttemptStatus,
    pub task_status: ExecutionTaskStatus,
    pub overall_status: ExecutionOverallStatus,
    pub pending_deviations: Vec<PendingDeviation>,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparedStatus {
    Prepared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptStartPrepareResponse {
    pub schema: PublicSchema,
    pub status: PreparedStatus,
    pub request: AttemptStartRequest,
    pub authorization_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStartStatus {
    Started,
    Recovered,
    AlreadyCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptStartResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub attempt_path: String,
    pub index_path: String,
    pub status: AttemptStartStatus,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptStartRecoveryResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub attempt_path: String,
    pub index_path: String,
    pub status: AttemptStartStatus,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidResult {
    Valid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptValidation {
    pub schema: PublicSchema,
    pub attempt_id: String,
    pub task_spec_id: String,
    pub task_id: String,
    pub status: AttemptStatus,
    pub record_count: usize,
    pub result: ValidResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCorrectionResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub record_id: String,
    pub index_path: String,
    pub correction_status: String,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCommandPreview {
    pub schema: PublicSchema,
    pub request: CommandRunRequest,
    pub task_id: String,
    pub attempt_id: String,
    pub record_id: String,
    pub working_directory: String,
    /// Execution environment entries are supplied by the selected command.
    pub execution: BTreeMap<String, Value>,
    pub invocation: CommandInvocation,
    pub receipt_prefix: String,
    pub sources: BTreeMap<String, String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub approved_sha256: Option<Nullable<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandInvocation {
    Direct {
        executable: String,
        executable_sha256: String,
        argv: Vec<String>,
    },
    WindowsBatch {
        launcher: String,
        launcher_sha256: String,
        script: String,
        script_sha256: String,
        arguments: Vec<String>,
        command_line: String,
        launcher_arguments: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCommandResult {
    pub schema: PublicSchema,
    pub approved_sha256: String,
    pub record_id: String,
    pub status: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub exit_code: Nullable<i64>,
    pub stdout_tail: String,
    pub stdout_truncated: bool,
    pub stderr_tail: String,
    pub stderr_truncated: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub receipt_prefix: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_finish_required: Option<Nullable<bool>>,
    /// The finish request differs by record kind and command result.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_finish_request: Option<Nullable<super::request::RecordFinishRequest>>,
}

pub type CommandPreview = PreparedCommandPreview;
pub type CommandResult = PreparedCommandResult;

/// Prepared receipt-directory shape; not registered or returned by current producers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedCommandPreview {
    pub schema: PublicSchema,
    pub request: CommandRunRequest,
    pub task_id: String,
    pub attempt_id: String,
    pub record_id: String,
    pub working_directory: String,
    /// Execution environment entries are supplied by the selected command.
    pub execution: BTreeMap<String, Value>,
    pub invocation: CommandInvocation,
    pub receipt_dir: String,
    pub sources: BTreeMap<String, String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub approved_sha256: Option<Nullable<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedCommandResult {
    pub schema: PublicSchema,
    pub approved_sha256: String,
    pub record_id: String,
    pub status: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub exit_code: Nullable<i64>,
    pub stdout_tail: String,
    pub stdout_truncated: bool,
    pub stderr_tail: String,
    pub stderr_truncated: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub receipt_dir: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_finish_required: Option<Nullable<bool>>,
    /// The finish request differs by record kind and command result.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_finish_request: Option<Nullable<super::request::RecordFinishRequest>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionCreateResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub correction_id: String,
    pub correction_path: String,
    pub index_path: String,
    pub affected_task_ids: Vec<String>,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightInput {
    pub id: String,
    pub kind: TaskInputKind,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightFile {
    pub id: String,
    pub action: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutePreflight {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub task_spec_id: String,
    pub task_id: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub task_path: String,
    pub execution_dir: String,
    pub index_path: String,
    pub task_status: ExecutionTaskStatus,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub dependencies: Nullable<Vec<String>>,
    pub confirmed_inputs: Vec<String>,
    pub inputs: Vec<PreflightInput>,
    pub files: Vec<PreflightFile>,
    pub task_collection_sha256: String,
    pub task_index_sha256: String,
    pub task_item_sha256: String,
    pub hierarchy_selection_sha256: String,
    pub task_instructions_sha256: String,
    pub execute_instructions_sha256: String,
    pub execute_instruction_selection: crate::instruction::InstructionSelection,
    pub execute_skill_selection: crate::skill::SkillSelection,
    pub index_sha256: String,
    pub eligibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRecord {
    pub index_status: String,
    pub worktree_status: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeSnapshot {
    pub schema: PublicSchema,
    pub records: Vec<WorktreeRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeClassification {
    TargetTask,
    CompletedDependency,
    Unrelated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeChange {
    pub index_status: String,
    pub worktree_status: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    pub path_classification: WorktreeClassification,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_task_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeCounts {
    pub staged: u64,
    pub unstaged: u64,
    pub untracked: u64,
    pub target_task: u64,
    pub completed_dependency: u64,
    pub unrelated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteWorktree {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub task_spec_id: String,
    pub task_id: String,
    pub task_collection_sha256: String,
    pub task_index_sha256: String,
    pub task_item_sha256: String,
    pub task_instructions_sha256: String,
    pub execute_instructions_sha256: String,
    pub task_status: ExecutionTaskStatus,
    pub task_path: String,
    pub index_sha256: String,
    pub execution_dir: String,
    pub snapshot_sha256: String,
    pub review_status: String,
    pub excluded_execution_change_count: u64,
    pub counts: WorktreeCounts,
    pub changes: Vec<WorktreeChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviationPreview {
    pub schema: PublicSchema,
    pub proposal: DeviationProposal,
    pub record_kind: super::attempt::RecordKind,
    pub action_validation: String,
    pub semantic_review: String,
    pub classification: DeviationClassification,
    pub blocking: bool,
    pub sources: BTreeMap<String, String>,
    pub preview_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviationRecordResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub deviation_id: String,
    pub attempt_path: String,
    pub classification: DeviationClassification,
    pub blocking: bool,
    pub record_status: String,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyExecutionRecoveryPrepare {
    pub schema: PublicSchema,
    pub status: PreparedStatus,
    pub request: super::request::LegacyExecutionRecoveryRequest,
    pub task_id: String,
    pub attempt_path: String,
    pub index_path: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub lock: Nullable<ExecutionLock>,
    pub attempt_status: AttemptStatus,
    pub evidence: BTreeMap<String, ExecutionRecoveryEvidence>,
    pub recovery_validation: String,
    pub recovery_authorized: bool,
}

pub type ExecutionRecoveryPrepare = PreparedExecutionRecoveryPrepare;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedExecutionRecoveryPrepare {
    pub schema: PublicSchema,
    pub status: PreparedStatus,
    pub request: super::request::PreparedExecutionRecoveryRequest,
    pub task_id: String,
    pub attempt_path: String,
    pub index_path: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub lock: Nullable<ExecutionLock>,
    pub attempt_status: AttemptStatus,
    pub evidence: BTreeMap<String, ExecutionRecoveryEvidence>,
    pub recovery_validation: String,
    pub recovery_authorized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecoveryResponse {
    pub schema: PublicSchema,
    pub transaction: RecoveryTransaction,
    pub task_id: String,
    pub attempt_id: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub attempt_path: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub correction_id: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub correction_path: Option<Nullable<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_path: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub affected_task_ids: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub record_id: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub attempt_status: Option<Nullable<AttemptStatus>>,
    pub lock_status: ExecutionLockStatus,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordBeginResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub base_record_id: String,
    pub record_id: String,
    pub record_kind: super::attempt::RecordKind,
    pub index_path: String,
    pub lock_status: ExecutionLockStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordFinishResponse {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub record_id: String,
    pub record_kind: super::attempt::RecordKind,
    pub attempt_path: String,
    pub index_path: String,
    pub record_status: String,
    pub lock_status: ExecutionLockStatus,
}

#[cfg(test)]
mod receipt_candidate_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prepared_recovery_response_binds_directory_and_isolates_legacy_shape() {
        let registry = crate::contract_data::registry_value();
        let mut legacy =
            registry["items"]["work-execution-recovery-prepare"]["description"]["example"].clone();
        let request = legacy["request"].as_object_mut().unwrap();
        request.remove("transaction_dir");
        request.remove("transaction_evidence_sha256");
        request.insert(
            "transaction_files".into(),
            json!([".work-record-begin-TASK-001-ATTEMPT-001-VAL-001.tmp"]),
        );
        let _: LegacyExecutionRecoveryPrepare = serde_json::from_value(legacy.clone()).unwrap();
        assert!(
            serde_json::from_value::<PreparedExecutionRecoveryPrepare>(legacy.clone()).is_err()
        );
        let mut candidate = legacy;
        candidate["request"]["transaction_dir"] = json!(
            "outputs/work/runtime/staging/example/record-begin/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        candidate["request"]["transaction_evidence_sha256"] = json!("b".repeat(64));
        let typed: PreparedExecutionRecoveryPrepare =
            serde_json::from_value(candidate.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), candidate);
        assert!(
            serde_json::from_value::<LegacyExecutionRecoveryPrepare>(candidate.clone()).is_err()
        );
        candidate["request"]["legacy_root"] = json!("execution");
        assert!(serde_json::from_value::<PreparedExecutionRecoveryPrepare>(candidate).is_err());
    }

    #[test]
    fn prepared_receipt_directory_shapes_reject_legacy_alias_and_preserve_current_contract() {
        let preview = json!({"schema":"work-command-preview",
            "request":{"schema":"work-command-run-request","timeout_seconds":60},
            "task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"CMD-001",
            "working_directory":"/project","execution":{"os":"linux"},
            "invocation":{"kind":"direct","executable":"/tool","executable_sha256":"a".repeat(64),"argv":["tool"]},
            "receipt_dir":"custom executions/TASK-001/ATTEMPT-001/receipts/CMD-001",
            "sources":{},"approved_sha256":"b".repeat(64)});
        let typed: PreparedCommandPreview = serde_json::from_value(preview.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), preview);
        assert!(serde_json::from_value::<LegacyCommandPreview>(preview.clone()).is_err());
        let mut legacy = preview.clone();
        let directory = legacy
            .as_object_mut()
            .unwrap()
            .remove("receipt_dir")
            .unwrap();
        legacy["receipt_prefix"] = directory;
        assert!(serde_json::from_value::<LegacyCommandPreview>(legacy.clone()).is_ok());
        assert!(serde_json::from_value::<PreparedCommandPreview>(legacy).is_err());
        let mut ambiguous = preview;
        ambiguous["receipt_prefix"] = json!("legacy");
        assert!(serde_json::from_value::<PreparedCommandPreview>(ambiguous).is_err());

        let result = json!({"schema":"work-command-result","approved_sha256":"b".repeat(64),
            "record_id":"CMD-001","status":"exited","exit_code":0,
            "stdout_tail":"","stdout_truncated":false,"stderr_tail":"","stderr_truncated":false,
            "receipt_dir":"custom executions/TASK-001/ATTEMPT-001/receipts/CMD-001",
            "record_finish_required":true});
        let typed: PreparedCommandResult = serde_json::from_value(result.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), result);
        assert!(serde_json::from_value::<LegacyCommandResult>(result.clone()).is_err());
        let mut legacy_result = result;
        let directory = legacy_result
            .as_object_mut()
            .unwrap()
            .remove("receipt_dir")
            .unwrap();
        legacy_result["receipt_prefix"] = directory;
        assert!(serde_json::from_value::<LegacyCommandResult>(legacy_result.clone()).is_ok());
        assert!(serde_json::from_value::<PreparedCommandResult>(legacy_result).is_err());
    }
}
