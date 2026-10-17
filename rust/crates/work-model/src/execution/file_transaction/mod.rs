//! Exact project-file publication requests and recovery evidence.
pub mod contracts;

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
        changes: Vec<FileChange>,
        baseline_confirmation: Option<FileBaselineConfirmation>,
    },
    Apply {
        attempt_id: String,
        changes: Vec<FileChange>,
        /// Exact read-time content hashes for every initial path; null means absent.
        expected_before: BTreeMap<String, Option<String>>,
        baseline_confirmation: Option<FileBaselineConfirmation>,
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

/// One simultaneous final-tree operation; paths refer to the initial tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum FileChange {
    Write {
        path: String,
        bytes: Vec<u8>,
    },
    Move {
        source: String,
        destination: String,
        bytes: Option<Vec<u8>>,
    },
}

/// Confirmation of a new current baseline, never approval of an unknown old write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileBaselineConfirmation {
    pub previous_transaction: String,
    pub before_sha256: String,
    pub evidence: String,
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
    /// None is legacy/unknown evidence; Some(empty) proves attribute absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos_xattrs: Option<BTreeMap<String, Vec<u8>>>,
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
    /// Dependency ranges, not recursive content snapshots.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directory_inputs: Vec<String>,
    pub changes: Vec<FileChange>,
    pub previous_transaction: Option<String>,
    pub baseline_confirmation: Option<FileBaselineConfirmation>,
    pub path_identities: BTreeMap<String, Option<String>>,
    pub before_metadata: BTreeMap<String, Option<FileMetadata>>,
    pub after_metadata: BTreeMap<String, Option<FileMetadata>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRecoveryPreview {
    pub schema: PublicSchema,
    pub manifest: RuntimeManifest,
    pub observed: BTreeMap<String, Option<FileState>>,
    pub observed_identities: BTreeMap<String, Option<String>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apply_requires_an_explicit_read_baseline() {
        let mut request = serde_json::json!({"schema":"work-file-transaction-request",
            "command":{"kind":"apply","attempt_id":"ATTEMPT-001",
                "changes":[{"kind":"write","path":"new.txt","bytes":[10]}]}});
        assert!(serde_json::from_value::<FileTransactionRequest>(request.clone()).is_err());
        request["command"]["expected_before"] = serde_json::json!({"new.txt":null});
        assert!(serde_json::from_value::<FileTransactionRequest>(request).is_ok());
    }
    #[test]
    fn legacy_metadata_bytes_remain_unchanged_and_unknown_xattrs_are_distinct_from_absence() {
        let legacy = include_str!("../../../fixtures/file-transaction/legacy-file-metadata.json");
        let snapshot: FileMetadata = serde_json::from_str(legacy).unwrap();
        assert!(snapshot.macos_xattrs.is_none());
        assert_eq!(serde_json::to_string(&snapshot).unwrap(), legacy);
        let mut absence = snapshot.clone();
        absence.macos_xattrs = Some(BTreeMap::new());
        assert_ne!(absence, snapshot);
        assert_eq!(
            serde_json::to_value(&absence).unwrap()["macos_xattrs"],
            serde_json::json!({})
        );
        absence
            .macos_xattrs
            .as_mut()
            .unwrap()
            .insert("com.work.binary".into(), vec![0, 10, 255]);
        assert_eq!(
            serde_json::from_slice::<FileMetadata>(&serde_json::to_vec(&absence).unwrap()).unwrap(),
            absence
        );
    }
}
