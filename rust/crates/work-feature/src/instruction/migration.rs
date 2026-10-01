//! Instruction migration approval and eligibility decisions.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_operations::derivation::fingerprint;

use crate::error::{ExitCode, WorkError};
use crate::instruction::migration_build::MigrationCandidate;

pub struct MigrationDecision {
    pub preview: Value,
    pub before: BTreeMap<String, Vec<u8>>,
    pub after: BTreeMap<String, Vec<u8>>,
}

pub fn decide_migration(
    requirement_id: &str,
    mut before: BTreeMap<String, Vec<u8>>,
    mut after: BTreeMap<String, Vec<u8>>,
    excluded: Vec<Value>,
    mut counts: Value,
) -> Result<MigrationDecision, WorkError> {
    if !excluded.is_empty() {
        before.clear();
        after.clear();
        counts = json!({"plans":0,"task_items":0,"task_indexes":0,"execution_indexes":0});
    }
    let files = after
        .iter()
        .map(|(path, raw)| {
            json!({"path":path,
                "before_sha256":fingerprint::raw(&before[path]),"after_sha256":fingerprint::raw(raw)})
        })
        .collect::<Vec<_>>();
    let evidence = json!({"requirement_id":requirement_id,"router_compatibility_revision":3,
        "excluded":excluded,"files":files});
    let approval = fingerprint::structured(&evidence).map_err(|_| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "invalid_contract_value",
            "The migration preview cannot be fingerprinted.",
            json!({}),
        )
    })?;
    let status = if !excluded.is_empty() {
        "review_required"
    } else if !files.is_empty() {
        "migration_required"
    } else {
        "current"
    };
    let preview =
        work_model::instruction::verified::<work_model::instruction::InstructionMigrationPreview>(
            json!({"schema":"work-instruction-migration-preview/v1","status":status,
        "requirement_id":requirement_id,"router_compatibility_revision":3,
        "affected":counts,"excluded":excluded,"files":files,"approved_sha256":approval}),
        );
    Ok(MigrationDecision {
        preview,
        before,
        after,
    })
}

pub fn require_writable_approval(
    candidate: &MigrationCandidate,
    approved_sha256: &str,
) -> Result<(), WorkError> {
    if candidate.preview["status"] != "migration_required" {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "instruction_migration_not_writable",
            "The instruction migration preview is not writable.",
            json!({}),
        ));
    }
    if candidate.preview["approved_sha256"] != approved_sha256 {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "instruction_migration_approval_changed",
            "The approved instruction migration fingerprint changed.",
            json!({}),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_artifact_blocks_all_migration_writes() {
        let before = BTreeMap::from([("plan.json".into(), b"before".to_vec())]);
        let after = BTreeMap::from([("plan.json".into(), b"after".to_vec())]);
        let decision = decide_migration(
            "example",
            before,
            after,
            vec![json!({"path":"execution/index.json","reason":"active_attempt_snapshot"})],
            json!({"plans":1,"task_items":0,"task_indexes":0,"execution_indexes":0}),
        )
        .unwrap();
        assert_eq!(decision.preview["status"], "review_required");
        assert_eq!(decision.preview["affected"]["plans"], 0);
        assert!(decision.before.is_empty());
        assert!(decision.after.is_empty());
    }
}
