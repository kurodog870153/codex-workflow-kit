//! Fingerprint evidence for execution recovery.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRecoveryEvidence {
    pub raw_sha256: String,
    pub size_bytes: u64,
}
