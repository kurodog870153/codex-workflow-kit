//! Specification command flows across semantic selection and migration preview.

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::specification::reconciliation_project::{
    ReconciliationArtifactRepository, preview_from_repository,
};
use work_feature::specification::reconciliation_publication::requires_migration;
use work_feature::specification::reconciliation_semantic::{
    ReconciliationSemanticRepository, prepare_semantic_selection,
};

pub fn preview_reconciliation(
    repository: &impl ReconciliationArtifactRepository,
    request: &Value,
    migration_preview: impl FnOnce(&Value) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    preview_from_repository(repository, request, migration_preview)
}

pub fn apply_reconciliation(
    request: &Value,
    approved_sha256: &str,
    publish_ledger: impl FnOnce(&Value, &str) -> Result<Value, WorkError>,
    publish_migration: impl FnOnce(&Value, &str) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    if requires_migration(request) {
        publish_migration(request, approved_sha256)
    } else {
        publish_ledger(request, approved_sha256)
    }
}

pub fn recover_reconciliation(
    request: &Value,
    approved_sha256: &str,
    recover_ledger: impl FnOnce(&Value, &str) -> Result<Value, WorkError>,
    recover_migration: impl FnOnce(&Value, &str) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    work_feature::specification::reconciliation_input::validate_preview_fields(request)?;
    if requires_migration(request) {
        recover_migration(request, approved_sha256)
    } else {
        recover_ledger(request, approved_sha256)
    }
}

pub fn prepare_reconciliation(
    repository: &impl ReconciliationSemanticRepository,
    semantic: &Value,
    date: &str,
    prepare_revision: impl FnOnce(&[u8], &str) -> Result<Value, WorkError>,
    preview: impl FnOnce(&Value) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    let selection = prepare_semantic_selection(repository, semantic)?;
    let migration = if selection.choice == "retain_only" {
        Value::Null
    } else {
        let request = json!({"schema":"work-spec-migration-prepare-request/v1",
            "mode":"revision","requirement_id":selection.requirement,
            "reason":selection.reason,"edits":selection.edits,"sources":semantic["sources"],
            "semantic_decisions":selection.decisions});
        prepare_revision(
            &serde_json::to_vec(&request).map_err(|_| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "invalid_json_contract",
                    "The semantic reconciliation request is invalid.",
                    json!({}),
                )
            })?,
            date,
        )?
    };
    let request = json!({"schema":"work-spec-reconciliation-preview-request/v1",
        "attempt_path":selection.attempt_path,"choice":selection.choice,
        "deviation_ids":selection.selected,"migration":migration});
    let result = preview(&request)?;
    Ok(json!({"request":request,"preview":result,"output_file":null}))
}

pub fn verify(report: Value) -> Result<Value, WorkError> {
    if report["verified"] == false {
        Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "specification_verification_failed",
            "Specification verification found incomplete or changed evidence; review the report.",
            report,
        ))
    } else {
        Ok(report)
    }
}

pub fn update(update: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    update()
}

pub fn prepare(prepare: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    prepare()
}

pub fn migration(
    operation: &str,
    preview: impl FnOnce() -> Result<Value, WorkError>,
    publish: impl FnOnce(&str) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    if operation == "semantic-preview" {
        preview()
    } else {
        publish(if operation == "semantic-recover" {
            "recover"
        } else {
            "apply"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Unused;
    impl ReconciliationSemanticRepository for Unused {
        fn execution_source(&self, _: &str) -> Result<String, WorkError> {
            panic!("invalid semantic input must stop before discovery")
        }
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("invalid semantic input must stop before reading")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("invalid semantic input must stop before path inspection")
        }
    }

    #[test]
    fn invalid_semantic_input_stops_before_flow_ports() {
        let error = prepare_reconciliation(
            &Unused,
            &json!({}),
            "2026-01-01",
            |_, _| panic!("migration must not run"),
            |_| panic!("preview must not run"),
        )
        .expect_err("semantic input must fail");
        assert_eq!(error.reason_code, "invalid_object_fields");
    }

    #[test]
    fn apply_flow_selects_migration_port() {
        let result = apply_reconciliation(
            &json!({"migration":{}}),
            "approved",
            |_, _| panic!("ledger-only port must not run"),
            |_, approval| Ok(json!({"approved":approval})),
        )
        .unwrap();
        assert_eq!(result["approved"], "approved");
    }
}
