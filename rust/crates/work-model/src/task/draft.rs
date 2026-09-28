//! TASK planning index, saved discussion, and semantic candidate shapes.

use serde::{Deserialize, Serialize};

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

use super::index::TaskExecutionDefaults;
use super::item::{TaskInputKind, TaskOperationKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningTaskStatus {
    Planned,
    InProgress,
    Refined,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedDraftStatus {
    InProgress,
    Refined,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningSource {
    pub plan_sha256: String,
    pub hierarchy_selection_sha256: String,
    pub skill_selection_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInstructionSelection {
    pub selected_paths: Vec<String>,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftReference {
    pub save_revision: u64,
    pub revision: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningTask {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub scope: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub dependencies: Vec<String>,
    pub status: PlanningTaskStatus,
    pub boundary_revision: u64,
    pub instructions_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_ref: Option<DraftReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_selection: Option<DraftInstructionSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPlanningIndex {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub revision: u64,
    pub source: PlanningSource,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub current_task_id: Nullable<String>,
    pub tasks: Vec<PlanningTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_task_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftDecision {
    pub statement: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateFileAction {
    Create,
    Modify,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateCommandMode {
    Argv,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateValidationKind {
    Automated,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateReferenceKind {
    Inputs,
    Decisions,
    Files,
    Risks,
    Commands,
    Operations,
    Validations,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateInput {
    pub key: String,
    pub kind: TaskInputKind,
    pub precondition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_position: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDecision {
    pub key: String,
    pub statement: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateFile {
    pub key: String,
    pub action: CandidateFileAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateRisk {
    pub key: String,
    pub condition: String,
    pub impact: String,
    pub mitigation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateReference {
    pub kind: CandidateReferenceKind,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateStep {
    pub key: String,
    pub action: String,
    pub references: Vec<CandidateReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateCommand {
    pub key: String,
    pub mode: CandidateCommandMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<TaskExecutionDefaults>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateOperation {
    pub key: String,
    pub kind: TaskOperationKind,
    pub action: String,
    pub target: String,
    pub validation_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateValidation {
    pub key: String,
    pub kind: CandidateValidationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_positions: Option<Vec<u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTaskCandidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<CandidateInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<CandidateDecision>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<CandidateFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risks: Option<Vec<CandidateRisk>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<CandidateStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<CandidateCommand>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations: Option<Vec<CandidateOperation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validations: Option<Vec<CandidateValidation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraft {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub task_id: String,
    pub revision: u64,
    pub boundary_revision: u64,
    pub source: PlanningSource,
    pub instructions_sha256: String,
    pub status: SavedDraftStatus,
    pub notes: Vec<String>,
    pub confirmed_decisions: Vec<DraftDecision>,
    pub tentative: Vec<String>,
    pub open_questions: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub next_discussion_point: Nullable<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_candidate: Option<SemanticTaskCandidate>,
}
