//! Exact project-file publication requests and recovery evidence.
use crate::runtime::RuntimeManifest;
use crate::schema::PublicSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTransactionRequest {
    pub schema: PublicSchema,
    pub command: FileTransactionCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FileTransactionCommand {
    Prepare {
        attempt_id: String,
        staged_files: BTreeMap<String, String>,
    },
    Apply {
        preview: Box<FileTransactionPreview>,
        approved_sha256: String,
        authorization_evidence: String,
    },
    RecoveryPrepare {
        transaction_identity: String,
    },
    Restore {
        transaction_identity: String,
        approved_sha256: String,
        authorization_evidence: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTransactionPreview {
    pub schema: PublicSchema,
    pub manifest: RuntimeManifest,
}

/// Supported mode, ownership and security snapshots, including explicit absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileMetadata {
    pub readonly: bool,
    pub unix_mode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_uid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_gid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_security: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileState {
    pub bytes: Vec<u8>,
    pub metadata: FileMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTransactionBinding {
    pub task_id: String,
    pub attempt_id: String,
    pub source_sha256: BTreeMap<String, String>,
    pub staged_files: BTreeMap<String, String>,
    pub before_metadata: BTreeMap<String, Option<FileMetadata>>,
    pub after_metadata: BTreeMap<String, Option<FileMetadata>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecoveryPreview {
    pub schema: PublicSchema,
    pub manifest: RuntimeManifest,
    pub observed: BTreeMap<String, Option<FileState>>,
    pub approval_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTransactionResult {
    pub schema: PublicSchema,
    pub transaction_identity: String,
    pub phase: crate::runtime::RuntimePhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileAuthorization {
    pub transaction_identity: String,
    pub publication_sha256: String,
    pub authorization_evidence: String,
    pub recovery: Option<FileRecoveryPreview>,
}
