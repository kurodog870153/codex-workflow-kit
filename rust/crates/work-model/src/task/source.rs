//! Exact provenance and independently confirmed Task planning choices.
use crate::source::snapshot::SourceSnapshot;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskAcceptance {
    pub id: String,
    pub criterion: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskArtifactPaths {
    pub source: String,
    pub task: String,
    pub execution: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationSourceEvidence {
    pub path: String,
    pub raw_sha256: String,
    pub size: u64,
    pub raw: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskProvenance {
    Snapshot {
        manifest: SourceSnapshot,
    },
    Migration {
        sources: Vec<MigrationSourceEvidence>,
        approval_sha256: String,
    },
}
