//! Deterministic Attempt-start recovery decisions over prepared evidence.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};

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
