//! Complete migration transaction preparation over artifact ports.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use work_operations::canonical::sha256_hex;
use work_operations::execution::index::render_execution_index;
use work_operations::plan::render_plan_value;
use work_operations::specification::transaction::{
    approval_sha256, derived_transaction_id, encode_snapshot, validate_transaction,
};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

pub trait MigrationTransactionRepository {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
    fn history_sha256(&self, execution_dir: &str) -> Result<BTreeMap<String, String>, WorkError>;
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn candidate_bytes(row: &Value) -> Result<Vec<u8>, WorkError> {
    match row["kind"].as_str() {
        Some("plan") => render_plan_value(&row["content"]),
        Some("task_index") => render_task(&row["content"], TaskDocumentKind::Index),
        Some("task_item") => render_task(&row["content"], TaskDocumentKind::Item),
        Some("execution_index") => render_execution_index(&row["content"]),
        _ => {
            return Err(fail(
                "migration_candidate_set_incomplete",
                "Unknown migration candidate kind.",
            ));
        }
    }
    .map_err(|_| {
        fail(
            "invalid_contract_value",
            "A migration candidate cannot be rendered.",
        )
    })
}

pub fn migration_transaction(
    repository: &impl MigrationTransactionRepository,
    request: &Value,
    preview: &Value,
) -> Result<Value, WorkError> {
    migration_transaction_with_additional(repository, request, preview, &BTreeMap::new())
}

pub fn migration_transaction_with_additional(
    repository: &impl MigrationTransactionRepository,
    request: &Value,
    preview: &Value,
    additional: &BTreeMap<String, Vec<u8>>,
) -> Result<Value, WorkError> {
    if preview["writable_ready"] != true {
        return Err(fail(
            "migration_not_writable",
            "The migration preview is not ready for publication.",
        ));
    }
    let mut sources = BTreeMap::new();
    for evidence in request["sources"]
        .as_array()
        .ok_or_else(|| fail("migration_source_missing", "Migration sources are missing."))?
    {
        let path = evidence["path"].as_str().ok_or_else(|| {
            fail(
                "migration_source_missing",
                "A migration source path is missing.",
            )
        })?;
        let raw = repository.read(path)?;
        if evidence["raw_sha256"] != sha256_hex(&raw) {
            return Err(fail(
                "migration_source_changed",
                "Migration source bytes changed.",
            ));
        }
        if sources.insert(path.to_owned(), raw).is_some() {
            return Err(fail(
                "migration_source_duplicate",
                "Migration source paths must be unique.",
            ));
        }
    }
    let mut candidates = BTreeMap::new();
    let mut artifacts = None;
    let mut task_ids = Vec::new();
    for row in request["candidates"].as_array().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "Migration candidates are missing.",
        )
    })? {
        let path = row["path"].as_str().ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "A migration candidate path is missing.",
            )
        })?;
        if row["kind"] == "plan" {
            artifacts = Some(row["content"]["artifacts"].clone());
        }
        if row["kind"] == "task_item" {
            task_ids.push(
                row["task_id"]
                    .as_str()
                    .ok_or_else(|| {
                        fail(
                            "migration_candidate_set_incomplete",
                            "A migration TASK ID is missing.",
                        )
                    })?
                    .to_owned(),
            );
        }
        if candidates
            .insert(path.to_owned(), candidate_bytes(row)?)
            .is_some()
        {
            return Err(fail(
                "migration_candidate_duplicate",
                "Migration candidate paths must be unique.",
            ));
        }
    }
    for (path, raw) in additional {
        if candidates.contains_key(path) {
            return Err(fail(
                "migration_additional_candidate_duplicate",
                "An additional candidate duplicates a migration candidate.",
            ));
        }
        if repository.exists(path)? {
            sources.insert(path.clone(), repository.read(path)?);
        }
        candidates.insert(path.clone(), raw.clone());
    }
    task_ids.sort();
    let artifacts = artifacts.ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "A migration Plan is missing.",
        )
    })?;
    let execution = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "migration_execution_directory",
            "An execution directory is required.",
        )
    })?;
    let mut files = Vec::new();
    for path in sources
        .keys()
        .chain(candidates.keys())
        .collect::<BTreeSet<_>>()
    {
        let before = sources.get(path);
        let after = candidates.get(path);
        let mut file = json!({"phase":if after.is_none() {50} else {10},
            "path":path,"operation":if before.is_none() {"add"} else if after.is_none() {"remove"} else {"replace"}});
        if let Some(raw) = before {
            file["before"] = encode_snapshot(raw);
        }
        if let Some(raw) = after {
            file["after"] = encode_snapshot(raw);
        }
        files.push(file);
    }
    files.sort_by_key(|row| {
        (
            row["phase"].as_u64().unwrap(),
            row["path"].as_str().unwrap().to_owned(),
        )
    });
    let source_hashes = sources
        .iter()
        .map(|(path, raw)| (path.clone(), sha256_hex(raw)))
        .collect::<BTreeMap<_, _>>();
    let candidate_hashes = candidates
        .iter()
        .map(|(path, raw)| (path.clone(), sha256_hex(raw)))
        .collect::<BTreeMap<_, _>>();
    let metadata = json!({"request":{"migration":request,"preview_fingerprint":preview["fingerprint"]},
        "artifacts":artifacts,"affected_task_ids":task_ids,
        "history_sha256":repository.history_sha256(execution)?,
        "source_sha256":source_hashes,"candidate_sha256":candidate_hashes});
    let files = Value::Array(files);
    let approval = approval_sha256(&files, &metadata);
    let id = derived_transaction_id("MIGRATION", &approval).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let transaction = json!({"schema":"work-spec-transaction/v1","transaction_id":id,
        "approval_sha256":approval,"state":"prepared","published_count":0,
        "metadata":metadata,"files":files});
    validate_transaction(&transaction).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    Ok(transaction)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnusedRepository;

    impl MigrationTransactionRepository for UnusedRepository {
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("rejected preview must not read files")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("rejected preview must not inspect files")
        }
        fn history_sha256(&self, _: &str) -> Result<BTreeMap<String, String>, WorkError> {
            panic!("rejected preview must not inspect history")
        }
    }

    #[test]
    fn rejected_preview_stops_before_repository_calls() {
        let error = migration_transaction(
            &UnusedRepository,
            &json!({}),
            &json!({"writable_ready":false}),
        )
        .expect_err("preview must be rejected");
        assert_eq!(error.reason_code, "migration_not_writable");
    }
}
