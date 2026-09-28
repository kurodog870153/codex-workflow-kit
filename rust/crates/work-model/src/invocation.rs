//! Explicit invocation shape.

use serde::{Deserialize, Serialize};

use crate::schema::PublicSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationMode {
    Plan,
    Task,
    Execute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationEntryKind {
    Workflow,
    ProgressResume,
    TaskPlanning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationEntry {
    pub kind: InvocationEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub schema: PublicSchema,
    pub mode: InvocationMode,
    pub request: String,
    pub entry: InvocationEntry,
}
