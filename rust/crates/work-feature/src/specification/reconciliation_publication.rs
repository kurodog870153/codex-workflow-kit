//! Approval checks shared by reconciliation publication modes.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
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
    ledger_transaction_with_history(preview, approved_sha256, before, after, BTreeMap::new())
}

pub fn ledger_transaction_with_history(
    preview: &Value,
    approved_sha256: &str,
    before: Option<&[u8]>,
    after: &[u8],
    history: BTreeMap<String, Vec<u8>>,
) -> Result<LedgerPublication, WorkError> {
    ledger_transaction_with_layout(preview, approved_sha256, before, after, history)
}

fn ledger_transaction_with_layout(
    preview: &Value,
    approved_sha256: &str,
    before: Option<&[u8]>,
    after: &[u8],
    history: BTreeMap<String, Vec<u8>>,
) -> Result<LedgerPublication, WorkError> {
    let ledger_path = preview["ledger_path"].as_str().unwrap();
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::Reconciliation,
        order: PublicationOrder::Flat,
        request: json!({"reconciliation_fingerprint":approved_sha256,
            "attempt_path":preview["attempt_path"]}),
        artifacts: json!({}),
        affected_task_ids: Vec::new(),
        history,
        source: before.map_or_else(BTreeMap::new, |raw| {
            BTreeMap::from([(ledger_path.to_owned(), raw.to_vec())])
        }),
        candidate: BTreeMap::from([(ledger_path.to_owned(), after.to_vec())]),
    })
    .map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let journal = derived.journal;
    let approval = derived.approval_sha256;
    let execution_dir = preview["attempt_path"]
        .as_str()
        .unwrap()
        .rsplit_once("/TASK-")
        .ok_or_else(|| {
            fail(
                "reconciliation_attempt_identity",
                "A complete Attempt path is required.",
            )
        })?
        .0
        .to_owned();
    use work_operations::derivation::publication::{self, JournalKind};
    let kind = JournalKind::SpecificationMigration(approved_sha256);
    let journal_path = {
        publication::retained_journal_path(&execution_dir, kind).map_err(|_| {
            fail(
                "reconciliation_attempt_identity",
                "Complete approval and execution identities are required.",
            )
        })?
    };
    let marker_path = {
        work_operations::specification::transaction::verify_retained_journal_layout(
            &execution_dir,
            &journal_path,
            &journal,
        )
        .map_err(|_| {
            fail(
                "reconciliation_attempt_identity",
                "The reviewed Attempt must bind its exact journal scope.",
            )
        })?;
        publication::retained_journal_marker(&journal_path).map_err(|_| {
            fail(
                "reconciliation_attempt_identity",
                "The retained journal path is invalid.",
            )
        })?
    };
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
    fn retained_ledger_binds_complete_attempt_in_custom_execution_scope() {
        let execution = "custom/TASK-folder/例";
        let mut preview = json!({"ledger_path":format!("{execution}/TASK-001/ATTEMPT-001/reconciliation.json"),
            "attempt_path":format!("{execution}/TASK-001/ATTEMPT-001/attempt.json")});
        let transaction = ledger_transaction_with_layout(
            &preview,
            &"a".repeat(64),
            None,
            b"ledger\n",
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(transaction.execution_dir, execution);
        assert_eq!(
            transaction.journal_path,
            format!("{execution}/journals/specification-migration/AAAAAAAAAAAA/journal.json")
        );
        assert_eq!(
            transaction.marker_path,
            format!("{execution}/journals/specification-migration/AAAAAAAAAAAA/committed.sha256")
        );
        assert_eq!(transaction.journal["metadata"]["artifacts"], json!({}));
        for attempt in [
            format!("{execution}/TASK-000/ATTEMPT-001/attempt.json"),
            format!("{execution}/TASK-001/ATTEMPT-000/attempt.json"),
            format!("{execution}/TASK-001/ATTEMPT-001/foreign.json"),
        ] {
            preview["attempt_path"] = json!(attempt);
            assert!(
                ledger_transaction_with_layout(
                    &preview,
                    &"a".repeat(64),
                    None,
                    b"ledger\n",
                    BTreeMap::new()
                )
                .is_err()
            );
        }
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
