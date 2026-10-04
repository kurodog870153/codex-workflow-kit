//! Reviewed source refresh transaction construction.

use crate::instruction::refresh_build::RefreshCandidate;
use serde_json::Value;
use std::collections::BTreeMap;
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
};

use crate::error::{ExitCode, WorkError};

pub fn transaction(
    candidate: &RefreshCandidate,
    request: &Value,
    history: &BTreeMap<String, Vec<u8>>,
) -> Result<(Value, String), WorkError> {
    let mut source = candidate.before.clone();
    let mut after = candidate.after.clone();
    source.extend(candidate.source_evidence.clone());
    after.extend(candidate.source_evidence.clone());
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::SourceRefresh,
        order: PublicationOrder::Flat,
        request: request.clone(),
        artifacts: candidate.artifacts.clone(),
        affected_task_ids: Vec::new(),
        history: history.clone(),
        source,
        candidate: after,
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
