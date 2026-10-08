//! Deterministic Attempt-start recovery decisions over prepared evidence.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};

pub fn require_staging_recovery_binding(
    request: &Value,
    manifest: &work_model::runtime::RuntimeManifest,
    binding: &work_operations::execution::recovery::ExecutionStagingBinding,
) -> Result<(), WorkError> {
    work_operations::execution::requests::validate_staging_recovery_request(request, false)
        .map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
    if request["transaction_dir"] != binding.transaction_dir
        || request["transaction_files"] != json!(binding.transaction_files)
        || request["attempt_id"] != manifest.business_identity["attempt_id"]
        || request["transaction"]
            .as_str()
            .map(|name| name.replace('_', "-"))
            != Some(manifest.operation.clone())
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_transaction_changed",
            "The request requires the identical transaction, operation, Attempt and complete observed inventory.",
            json!({}),
        ));
    }
    if request["transaction_evidence_sha256"]
        != work_operations::execution::recovery::execution_staging_evidence_sha256(binding)
            .map_err(|issue| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            })?
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_evidence_changed",
            "The transaction's bytes and SHA/size evidence changed after review.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn require_staging_recovery_evidence(
    reviewed: &work_operations::execution::recovery::ExecutionStagingBinding,
    current: &work_operations::execution::recovery::ExecutionStagingBinding,
) -> Result<(), WorkError> {
    if reviewed != current {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_evidence_changed",
            "Recovery bytes, hashes, sizes or inventory changed before publication.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn command_correction_identity(
    index: &Value,
    task_id: &str,
    attempt_id: &str,
) -> Result<(String, String), WorkError> {
    let record_id = index["lock"]["record_id"]
        .as_str()
        .filter(|id| id.starts_with("CMD-"))
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_command_lock_required",
                "command_correction recovery requires a reserved CMD record.",
                json!({}),
            )
        })?;
    let safe_record = record_id.replace('#', "-retry-");
    let name = format!(".work-command-correction-{task_id}-{attempt_id}-{safe_record}.tmp");
    Ok((record_id.to_owned(), name))
}

pub fn require_recovery_file_set(request: &Value, inventory: &[String]) -> Result<(), WorkError> {
    if request["transaction_files"] != json!(inventory) {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_file_set_changed",
            "The preserved transaction file set differs from the approved request.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn require_recovery_artifact_paths(
    contract: &Value,
    task_path: &str,
    execution_dir: &str,
) -> Result<(), WorkError> {
    if contract["artifacts"]["task"] != task_path
        || contract["artifacts"]["execution"] != execution_dir
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_artifact_paths",
            "Explicit paths do not match formal TASK artifacts.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn validate_attempt_start_inventory(
    inventory: &[String],
    lock_name: &str,
    started_name: &str,
) -> Result<(), WorkError> {
    if inventory
        .iter()
        .any(|name| name != lock_name && name != started_name)
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "attempt_start_recovery_mixed_transactions",
            "Attempt-start recovery requires only its own preserved files.",
            json!({"files":inventory}),
        ));
    }
    Ok(())
}

pub fn attempt_start_stage(
    current: &[u8],
    base: &[u8],
    locked: &[u8],
    started: &[u8],
) -> Result<u8, WorkError> {
    let stage = if current == base {
        0
    } else if current == locked {
        1
    } else if current == started {
        2
    } else {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "attempt_start_recovery_index_mismatch",
            "The current index is not a deterministic Attempt-start stage.",
            json!({}),
        ));
    };
    Ok(stage)
}

pub fn require_started_attempt(stage: u8, attempt_exists: bool) -> Result<(), WorkError> {
    if stage == 2 && !attempt_exists {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "attempt_start_recovery_attempt_missing",
            "A started index requires the matching Attempt artifact.",
            json!({}),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_recovery_binds_full_directory_inventory_and_attempt_then_rechecks_evidence() {
        use work_model::runtime::*;
        use work_operations::derivation::fingerprint;
        use work_operations::execution::recovery::*;
        let raw = b"prepared".to_vec();
        let payloads = std::collections::BTreeMap::from([("index.json.tmp".into(), raw.clone())]);
        let manifest=build_execution_staging_manifest(ExecutionStagingInput {
            canonical_root:"/project",requirement:&"example".parse().unwrap(),execution_dir:"custom execution",
            operation:work_operations::derivation::publication::RuntimeOperation::RecordBegin,
            approval_sha256:&"a".repeat(64),business_identity:json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"VAL-001"}),
            targets:vec![RuntimeTarget {path:"custom execution/index.json".into(),before:None,
                after:Some(RuntimeBytes {sha256:fingerprint::raw(&raw),bytes:raw})}],payloads:&payloads,
        }).unwrap();
        let mut observed = payloads;
        observed.insert(
            "transaction.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        );
        let binding = execution_staging_binding(
            &manifest,
            "/project",
            &"example".parse().unwrap(),
            "custom execution",
            "TASK-001",
            &observed,
        )
        .unwrap();
        let request = json!({"schema":"work-execution-recovery-request","transaction":"record_begin","attempt_id":"ATTEMPT-001",
            "transaction_dir":binding.transaction_dir,"transaction_files":binding.transaction_files,
            "transaction_evidence_sha256":execution_staging_evidence_sha256(&binding).unwrap()});
        require_staging_recovery_binding(&request, &manifest, &binding).unwrap();
        require_staging_recovery_evidence(&binding, &binding).unwrap();
        let mut stale = request.clone();
        stale["transaction_evidence_sha256"] = json!("f".repeat(64));
        assert_eq!(
            require_staging_recovery_binding(&stale, &manifest, &binding)
                .unwrap_err()
                .reason_code,
            "execution_recovery_evidence_changed"
        );
        for (field, value) in [
            ("attempt_id", json!("ATTEMPT-002")),
            (
                "transaction_dir",
                json!(format!(
                    "outputs/work/runtime/staging/other/record-begin/{}",
                    "a".repeat(64)
                )),
            ),
            ("transaction_files", json!(["transaction.json"])),
        ] {
            let mut changed = request.clone();
            changed[field] = value;
            assert_eq!(
                require_staging_recovery_binding(&changed, &manifest, &binding)
                    .unwrap_err()
                    .reason_code,
                "execution_recovery_transaction_changed"
            );
        }
        for change in 0..4 {
            let mut current = binding.clone();
            match change {
                0 => current
                    .transaction_files
                    .push("transaction.json.tmp".into()),
                1 => {
                    current
                        .evidence
                        .get_mut("index.json.tmp")
                        .unwrap()
                        .raw_sha256 = "b".repeat(64)
                }
                2 => {
                    current
                        .evidence
                        .get_mut("index.json.tmp")
                        .unwrap()
                        .size_bytes += 1
                }
                _ => current.missing_files.push("index.json.tmp".into()),
            }
            assert_eq!(
                require_staging_recovery_evidence(&binding, &current)
                    .unwrap_err()
                    .reason_code,
                "execution_recovery_evidence_changed"
            );
            if change == 1 || change == 2 {
                assert_eq!(
                    require_staging_recovery_binding(&request, &manifest, &current)
                        .unwrap_err()
                        .reason_code,
                    "execution_recovery_evidence_changed"
                );
            }
        }
    }

    #[test]
    fn mixed_transaction_is_rejected_before_recovery() {
        let error =
            validate_attempt_start_inventory(&["other.tmp".to_owned()], "lock.tmp", "started.tmp")
                .expect_err("mixed transaction must fail");
        assert_eq!(
            error.reason_code,
            "attempt_start_recovery_mixed_transactions"
        );
    }

    #[test]
    fn started_index_requires_attempt_artifact() {
        let stage = attempt_start_stage(b"started", b"base", b"locked", b"started").unwrap();
        let error = require_started_attempt(stage, false).expect_err("missing Attempt must fail");
        assert_eq!(error.reason_code, "attempt_start_recovery_attempt_missing");
    }

    #[test]
    fn correction_identity_uses_reserved_command_record() {
        let (record, name) = command_correction_identity(
            &json!({"lock":{"record_id":"CMD-001#2"}}),
            "TASK-001",
            "ATTEMPT-001",
        )
        .unwrap();
        assert_eq!(record, "CMD-001#2");
        assert_eq!(
            name,
            ".work-command-correction-TASK-001-ATTEMPT-001-CMD-001-retry-2.tmp"
        );
    }

    #[test]
    fn approved_file_set_requires_exact_inventory() {
        let request = json!({"transaction_files":["one.tmp"]});
        let error = require_recovery_file_set(&request, &["two.tmp".to_owned()])
            .expect_err("changed inventory must fail");
        assert_eq!(error.reason_code, "execution_recovery_file_set_changed");
    }
}
