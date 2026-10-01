//! Reviewed instruction migration transaction construction.

use crate::instruction::migration_build::MigrationCandidate;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
};

use crate::error::{ExitCode, WorkError};

pub fn transaction(
    candidate: &MigrationCandidate,
    requirement_id: &str,
    approved_sha256: &str,
    history: &BTreeMap<String, Vec<u8>>,
) -> Result<(Value, String), WorkError> {
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::InstructionMigration,
        order: PublicationOrder::Flat,
        request: json!({"kind":"instruction_migration","requirement_id":requirement_id,
            "preview_fingerprint":approved_sha256}),
        artifacts: candidate.artifacts.clone(),
        affected_task_ids: Vec::new(),
        history: history.clone(),
        source: candidate.before.clone(),
        candidate: candidate.after.clone(),
    })
    .map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    Ok((derived.journal, derived.approval_sha256))
}
