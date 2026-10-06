//! Semantic inputs for formal TASK generation and Specification.

use serde::{Deserialize, Serialize};

use super::index::TaskExecutionDefaults;
use super::item::{TaskInputKind, TaskOperationKind};

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
    pub acceptance_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticTaskCandidate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_criteria: Option<Vec<super::source::TaskAcceptance>>,
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
