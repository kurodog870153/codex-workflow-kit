//! Approval checks shared by reconciliation publication modes.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};
use work_operations::canonical::sha256_hex;
use work_operations::specification::transaction::{
    approval_sha256, derived_transaction_id, encode_snapshot,
};

pub struct LedgerPublication {
    pub journal: Value,
    pub approval: String,
    pub execution_dir: String,
    pub journal_path: String,
    pub marker_path: String,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn require_approved_preview(
    preview: &Value,
    request: &Value,
    approved_sha256: &str,
    requires_migration: bool,
) -> Result<(), WorkError> {
    if preview["fingerprint"] != approved_sha256 {
        return Err(fail(
            "reconciliation_approval_changed",
            "The approved reconciliation fingerprint changed.",
        ));
    }
    if preview["publication_ready"] != true {
        return Err(fail(
            "reconciliation_not_publishable",
            "This reconciliation does not have a writable candidate set.",
        ));
    }
    if requires_migration && request["migration"].is_null() {
        return Err(fail(
            "reconciliation_migration_required",
            "A migration candidate set is required.",
        ));
    }
    if !requires_migration && !request["migration"].is_null() {
        return Err(fail(
            "reconciliation_migration_required",
            "Selected deviations require migration publication.",
        ));
    }
    Ok(())
}

pub fn requires_migration(request: &Value) -> bool {
    !request["migration"].is_null()
}

pub fn ledger_transaction(
    preview: &Value,
    approved_sha256: &str,
    before: Option<&[u8]>,
    after: &[u8],
) -> Result<LedgerPublication, WorkError> {
    let ledger_path = preview["ledger_path"].as_str().unwrap();
    let mut file = json!({"phase":10,"path":ledger_path,
        "operation":if before.is_some() {"replace"} else {"add"},
        "after":encode_snapshot(after)});
    if let Some(raw) = before {
        file["before"] = encode_snapshot(raw);
    }
    let files = json!([file]);
    let metadata = json!({"request":{"reconciliation_fingerprint":approved_sha256,
        "attempt_path":preview["attempt_path"]},"artifacts":{},"affected_task_ids":[],
        "history_sha256":{},
        "source_sha256":before.map_or_else(|| json!({}),
            |raw| json!({ledger_path:sha256_hex(raw)})),
        "candidate_sha256":{ledger_path:sha256_hex(after)}});
    let approval = approval_sha256(&files, &metadata);
    let transaction_id = derived_transaction_id("RECONCILIATION", &approval).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let journal = json!({"schema":"work-spec-transaction/v1","transaction_id":transaction_id,
        "approval_sha256":approval,"state":"prepared","published_count":0,
        "metadata":metadata,"files":files});
    let execution_dir = preview["attempt_path"]
        .as_str()
        .unwrap()
        .split("/TASK-")
        .next()
        .unwrap()
        .to_owned();
    let journal_path = format!(
        "{execution_dir}/.work-spec-migration-{}.json",
        approved_sha256[..12].to_ascii_uppercase()
    );
    let marker_path = format!("{journal_path}.done");
    Ok(LedgerPublication {
        journal,
        approval,
        execution_dir,
        journal_path,
        marker_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_failure_precedes_readiness_failure() {
        let preview = json!({"fingerprint":"old","publication_ready":false});
        let request = json!({"migration":null});
        let error = require_approved_preview(&preview, &request, "new", false)
            .expect_err("fingerprint must fail first");
        assert_eq!(error.reason_code, "reconciliation_approval_changed");
    }

    #[test]
    fn publication_mode_requires_matching_migration_shape() {
        let preview = json!({"fingerprint":"same","publication_ready":true});
        let request = json!({"migration":null});
        let error = require_approved_preview(&preview, &request, "same", true)
            .expect_err("migration is required");
        assert_eq!(error.reason_code, "reconciliation_migration_required");
    }

    #[test]
    fn ledger_transaction_keeps_add_operation_and_execution_directory() {
        let preview = json!({"ledger_path":"outputs/work/e/TASK-001/ATTEMPT-001/reconciliation.json",
            "attempt_path":"outputs/work/e/TASK-001/ATTEMPT-001/attempt.json"});
        let transaction = ledger_transaction(&preview, &"a".repeat(64), None, b"ledger\n").unwrap();
        assert_eq!(transaction.journal["files"][0]["operation"], "add");
        assert_eq!(transaction.execution_dir, "outputs/work/e");
        assert_eq!(transaction.journal["approval_sha256"], transaction.approval);
    }
}
