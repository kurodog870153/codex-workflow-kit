//! Execution index, TASK rows, and exclusive transaction locks.

use serde::{Deserialize, Serialize};

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

use super::attempt::CommandCorrection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTaskStatus {
    Pending,
    InProgress,
    PendingRetry,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionOverallStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatusReasonKind {
    Attempt,
    Correction,
    TaskChange,
    InstructionAudit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionStatusReason {
    pub kind: ExecutionStatusReasonKind,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTask {
    pub id: String,
    pub status: ExecutionTaskStatus,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub task_item_sha256: String,
    pub instructions_sha256: String,
    pub acceptance_results: Vec<super::acceptance::AcceptanceProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_attempt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_correction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<ExecutionStatusReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionLock {
    SpecUpdate {
        record: String,
    },
    Execution {
        task_id: String,
        attempt_id: String,
        execute_instructions_sha256: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        record_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_authorization_evidence: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_correction: Option<CommandCorrection>,
    },
    Correction {
        task_id: String,
        attempt_id: String,
        correction_id: String,
        execute_instructions_sha256: String,
        invalidates_completion: bool,
        affected_task_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionIndex {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub title: String,
    pub task_spec_id: String,
    pub task_collection_sha256: String,
    pub task_index_sha256: String,
    pub task_instructions_sha256: String,
    pub hierarchy_selection_sha256: String,
    pub skill_selection_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_selection_manifest: Option<crate::instruction::InstructionSelectionManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_task_instruction_audit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<ExecutionLock>,
    pub overall_status: ExecutionOverallStatus,
    pub tasks: Vec<ExecutionTask>,
    pub acceptance_results: Vec<super::acceptance::AcceptanceProgress>,
}
