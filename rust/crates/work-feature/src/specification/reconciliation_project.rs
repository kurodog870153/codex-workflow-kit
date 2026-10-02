//! Reconciliation artifact loading and validation through a read port.

use crate::error::{ExitCode, WorkError};
use crate::specification::reconciliation::preview_reconciliation;
use crate::specification::reconciliation_input::{
    validate_ledger_entries, validate_preview_fields,
};
use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::execution::attempt::validate_attempt_bytes;

pub trait ReconciliationArtifactRepository {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
}

pub struct ReconciliationSources {
    pub attempt_path: String,
    pub attempt_raw: Vec<u8>,
    pub attempt: Value,
    pub index: Value,
    pub ledger: Value,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn load_sources(
    repository: &impl ReconciliationArtifactRepository,
    request: &Value,
) -> Result<ReconciliationSources, WorkError> {
    validate_preview_fields(request)?;
    let attempt_path = request["attempt_path"].as_str().ok_or_else(|| {
        fail(
            "reconciliation_attempt_identity",
            "An Attempt path is required.",
        )
    })?;
    let raw = repository.read(attempt_path)?;
    let attempt = parse_json_contract(&raw)
        .map_err(|_| fail("invalid_json_contract", "The Attempt is not valid JSON."))?;
    validate_attempt_bytes(&attempt, &raw).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let execution_dir = attempt_path.split("/TASK-").next().ok_or_else(|| {
        fail(
            "reconciliation_attempt_identity",
            "The Attempt path is invalid.",
        )
    })?;
    let index_path = format!("{execution_dir}/index.json");
    let index_raw = repository.read(&index_path)?;
    let index = parse_json_contract(&index_raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The execution index is not valid JSON.",
        )
    })?;
    work_operations::execution::index::validate_execution_index(&index, &index_raw).map_err(
        |issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        },
    )?;
    let ledger_path = attempt_path
        .strip_suffix("attempt.json")
        .ok_or_else(|| {
            fail(
                "reconciliation_attempt_identity",
                "The Attempt path is invalid.",
            )
        })?
        .to_owned()
        + "reconciliation.json";
    let ledger = if repository.exists(&ledger_path)? {
        parse_json_contract(&repository.read(&ledger_path)?).map_err(|_| {
            fail(
                "reconciliation_ledger",
                "The existing reconciliation ledger is invalid.",
            )
        })?
    } else {
        json!({"schema":"work-spec-reconciliation-ledger/v1",
            "attempt_path":attempt_path,"entries":[]})
    };
    if ledger["schema"] != "work-spec-reconciliation-ledger/v1"
        || ledger["attempt_path"] != attempt_path
    {
        return Err(fail(
            "reconciliation_ledger",
            "The existing reconciliation ledger is invalid.",
        ));
    }
    validate_ledger_entries(&ledger)?;
    Ok(ReconciliationSources {
        attempt_path: attempt_path.to_owned(),
        attempt_raw: raw,
        attempt,
        index,
        ledger,
    })
}

pub fn preview_from_repository(
    repository: &impl ReconciliationArtifactRepository,
    request: &Value,
    migration_preview: impl FnOnce(&Value) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    let sources = load_sources(repository, request)?;
    let migration = if request["migration"].is_null() {
        None
    } else {
        Some(migration_preview(&request["migration"])?)
    };
    preview_reconciliation(
        request,
        &sources.attempt_path,
        &sources.attempt_raw,
        &sources.attempt,
        &sources.index,
        &sources.ledger,
        migration.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnusedRepository;
    impl ReconciliationArtifactRepository for UnusedRepository {
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("invalid request must stop before reading")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("invalid request must stop before inspecting paths")
        }
    }

    #[test]
    fn invalid_request_stops_before_artifact_ports() {
        let error = load_sources(&UnusedRepository, &json!({}))
            .err()
            .expect("request must fail");
        assert_eq!(error.reason_code, "invalid_object_fields");
    }

    #[test]
    fn invalid_request_stops_before_migration_preview() {
        let error = preview_from_repository(&UnusedRepository, &json!({}), |_| {
            panic!("invalid request must stop before migration preview")
        })
        .expect_err("request must fail");
        assert_eq!(error.reason_code, "invalid_object_fields");
    }
    #[test]
    fn legacy_migration_candidate_stops_before_attempt_and_preview_ports() {
        let request = json!({"schema":"work-spec-reconciliation-preview-request/v1", "attempt_path":"outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json", "choice":"all", "deviation_ids":[],
            "migration":{"schema":"work-spec-migration-preview-request/v1","sources":[],"semantic_decisions":[],"candidates":[{"path":"old.json","kind":"plan","content":{}}]}});
        let error = preview_from_repository(&UnusedRepository, &request, |_| {
            panic!("legacy migration must stop before callback")
        })
        .unwrap_err();
        assert_eq!(error.reason_code, "invalid_contract_value");
    }
}
