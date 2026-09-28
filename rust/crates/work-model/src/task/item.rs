//! Formal TASK item and its fixed nested groups.

use serde::{Deserialize, Serialize};

use crate::common::{Nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;

use super::index::{TaskExecutionDefaults, TaskInstructionSource};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskItemInstructionSelection {
    pub selected_paths: Vec<String>,
    pub resolved_paths: Vec<String>,
    pub sources: Vec<TaskInstructionSource>,
    pub references: Vec<String>,
    pub instructions_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskTraceability {
    pub goal_ids: Vec<String>,
    pub deliverable_ids: Vec<String>,
    pub acceptance_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskInputKind {
    TaskOutput,
    ProjectState,
    UserProvided,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    pub id: String,
    pub kind: TaskInputKind,
    pub source: String,
    pub precondition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDecision {
    pub id: String,
    pub statement: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskFile {
    Create {
        id: String,
        path: String,
    },
    Modify {
        id: String,
        path: String,
    },
    Move {
        id: String,
        source: String,
        destination: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRisk {
    pub id: String,
    pub condition: String,
    pub impact: String,
    pub mitigation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskStep {
    pub id: String,
    pub action: String,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskCommand {
    Argv {
        id: String,
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<TaskExecutionDefaults>,
    },
    Shell {
        id: String,
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<TaskExecutionDefaults>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOperationKind {
    LocalState,
    ExternalState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOperation {
    pub id: String,
    pub kind: TaskOperationKind,
    pub action: String,
    pub target: String,
    pub validation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskValidation {
    Automated {
        id: String,
        command_ids: Vec<String>,
        pass_condition: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        acceptance_ids: Option<Vec<String>>,
    },
    Manual {
        id: String,
        confirmer: String,
        criteria: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        acceptance_ids: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskItem {
    pub schema: PublicSchema,
    pub id: String,
    pub title: String,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub skill_id: Nullable<String>,
    pub instruction_selection: TaskItemInstructionSelection,
    pub traceability: TaskTraceability,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<TaskInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<TaskDecision>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<TaskFile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risks: Option<Vec<TaskRisk>>,
    pub steps: Vec<TaskStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<TaskCommand>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations: Option<Vec<TaskOperation>>,
    pub validations: Vec<TaskValidation>,
}
