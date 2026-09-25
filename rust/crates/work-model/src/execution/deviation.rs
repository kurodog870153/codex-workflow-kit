//! Deviation proposal, approval, and persistent record.

use crate::schema::PublicSchema;
use crate::task::index::TaskExecutionDefaults;
use crate::task::item::{TaskCommand, TaskOperation, TaskValidation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplacementCommand {
    Argv {
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<TaskExecutionDefaults>,
    },
    Shell {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<TaskExecutionDefaults>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeviationAction {
    ReplaceCommand {
        record_id: String,
        replacement: ReplacementCommand,
    },
    AddCommand {
        after_record_id: String,
        command: TaskCommand,
    },
    AddValidation {
        validation: TaskValidation,
    },
    SkipRecord {
        record_id: String,
        reason: String,
    },
    AdjustOperation {
        operation: TaskOperation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviationImpact {
    pub summary: String,
    pub requirement_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_changed: Option<bool>,
    pub acceptance_criteria_changed: bool,
    pub deliverables_changed: bool,
    pub safety_boundary_changed: bool,
    pub external_side_effect_boundary_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviationProposal {
    pub schema: PublicSchema,
    pub task_id: String,
    pub attempt_id: String,
    pub anchor_record_id: String,
    pub task_basis: Vec<String>,
    pub gap: String,
    pub action: DeviationAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiable_files: Option<Vec<String>>,
    pub impact: DeviationImpact,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalAuthorization {
    pub schema: DeviationAuthorizationSchema,
    pub preview_sha256: String,
    pub action: DeviationAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiable_files: Option<Vec<String>>,
    pub authorization_evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviationAuthorizationSchema {
    #[serde(rename = "work-execution-deviation-authorization/v1")]
    V1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviationOutcome {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviationDecision {
    pub outcome: DeviationOutcome,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationStatus {
    Pending,
    Incorporated,
    Declined,
    NotNeeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionDeviation {
    pub schema: PublicSchema,
    pub deviation_id: String,
    pub approved_preview_sha256: String,
    pub proposal: DeviationProposal,
    pub supplemental_authorization: SupplementalAuthorization,
    pub decision: DeviationDecision,
    pub reconciliation_status: ReconciliationStatus,
}
