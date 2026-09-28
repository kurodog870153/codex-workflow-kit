//! Source refresh batch approval and publication state decisions.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn existing_record(mut record: Value, approved_sha256: &str) -> Result<Value, WorkError> {
    if record["approved_sha256"] != approved_sha256 {
        return Err(fail(
            "source_refresh_batch_record_changed",
            "The batch record approval does not match.",
        ));
    }
    if record["status"] != "in_progress" {
        record["status"] = json!("already_completed");
    }
    Ok(record)
}

pub fn require_new_operation(operation: &str) -> Result<(), WorkError> {
    if operation == "recover" {
        return Err(fail(
            "source_refresh_batch_record_missing",
            "Batch recovery requires its progress record.",
        ));
    }
    if operation != "apply" {
        return Err(fail(
            "source_refresh_batch_operation",
            "Use apply or recover for batch refresh.",
        ));
    }
    Ok(())
}

pub fn new_record(
    preview: &Value,
    approved_sha256: &str,
    relative: &str,
) -> Result<Value, WorkError> {
    if preview["status"] != "refreshable" {
        return Err(fail(
            "source_refresh_batch_not_writable",
            "The batch refresh preview is not writable.",
        ));
    }
    if preview["approved_sha256"] != approved_sha256 {
        return Err(fail(
            "source_refresh_batch_approval_changed",
            "The approved batch refresh fingerprint changed.",
        ));
    }
    let requirements = preview["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["status"] == "refreshable")
        .map(|row| {
            json!({"requirement_id":row["requirement_id"],
                "approved_sha256":row["approved_sha256"]})
        })
        .collect::<Vec<_>>();
    Ok(json!({"schema":"work-source-refresh-batch-publication/v1",
        "status":"in_progress","semantics":"recoverable_sequential",
        "approved_sha256":approved_sha256,"record_path":relative,
        "requirements":requirements,"completed_requirement_ids":[],"publications":[]}))
}

pub fn finish_record(record: &mut Value) {
    let publications = record["publications"].as_array().unwrap();
    let status = if !publications.is_empty()
        && publications
            .iter()
            .all(|row| row["status"] == "already_completed")
    {
        "already_completed"
    } else {
        "updated"
    };
    record["status"] = json!(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_recovery_record_keeps_original_reason() {
        let error = require_new_operation("recover").expect_err("record is required");
        assert_eq!(error.reason_code, "source_refresh_batch_record_missing");
    }

    #[test]
    fn batch_record_contains_only_refreshable_requirements() {
        let preview = json!({"status":"refreshable","approved_sha256":"same",
            "requirements":[{"requirement_id":"a","status":"valid","approved_sha256":"a"},
                {"requirement_id":"b","status":"refreshable","approved_sha256":"b"}]});
        let record = new_record(&preview, "same", "record.json").unwrap();
        assert_eq!(record["requirements"].as_array().unwrap().len(), 1);
        assert_eq!(record["requirements"][0]["requirement_id"], "b");
    }
}
