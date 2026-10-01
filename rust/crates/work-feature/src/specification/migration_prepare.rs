//! Prepare a revision migration request from a reviewed Specification candidate.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::snapshot::decode_snapshot;

pub trait MigrationPrepareRepository {
    fn read_execution(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
}

pub struct RevisionSemantic {
    pub semantic: Value,
    pub revision_raw: Vec<u8>,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn parse_revision_semantic(raw: &[u8]) -> Result<RevisionSemantic, WorkError> {
    let semantic = parse_json_contract(raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The migration preparation is invalid.",
        )
    })?;
    if semantic["schema"] != "work-spec-migration-prepare-request/v1"
        || semantic["mode"] != "revision"
    {
        return Err(fail(
            "migration_prepare_mode",
            "A revision migration is required.",
        ));
    }
    if semantic.as_object().is_none_or(|fields| {
        fields.keys().any(|field| {
            ![
                "schema",
                "mode",
                "requirement_id",
                "reason",
                "edits",
                "semantic_decisions",
            ]
            .contains(&field.as_str())
        })
    }) {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "Migration preparation has unknown fields.",
            json!({"location":"spec_migration_prepare"}),
        ));
    }
    let revision = json!({"schema":"work-spec-prepare-request/v1",
        "requirement_id":semantic["requirement_id"],"reason":semantic["reason"],
        "edits":semantic["edits"]});
    let _: work_model::specification::SpecMigrationPrepareRequest =
        serde_json::from_value(semantic.clone())
            .expect("validated migration request matches its model");
    let revision_raw = serde_json::to_vec(&revision).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The migration revision cannot be rendered.",
        )
    })?;
    Ok(RevisionSemantic {
        semantic,
        revision_raw,
    })
}

pub fn build_revision_request(
    repository: &impl MigrationPrepareRepository,
    semantic: &Value,
    prepared: &Value,
) -> Result<Value, WorkError> {
    let candidate = &prepared["request"];
    let transaction = &prepared["preview"]["transaction"];
    let artifacts = &candidate["plan"]["artifacts"];
    let plan_path = artifacts["plan"].as_str().ok_or_else(|| {
        fail(
            "spec_artifact_identity",
            "The candidate Plan path is missing.",
        )
    })?;
    let index_path = artifacts["task"].as_str().ok_or_else(|| {
        fail(
            "spec_artifact_identity",
            "The candidate TASK path is missing.",
        )
    })?;
    let execution_path = format!(
        "{}/index.json",
        artifacts["execution"].as_str().ok_or_else(|| fail(
            "spec_artifact_identity",
            "The candidate execution path is missing."
        ))?
    );
    let mut sources = Vec::new();
    for (path, digest) in transaction["metadata"]["source_sha256"]
        .as_object()
        .ok_or_else(|| {
            fail(
                "migration_source_missing",
                "Migration source evidence is missing.",
            )
        })?
    {
        sources.push(json!({"path":path,"raw_sha256":digest}));
    }
    let execution_raw = transaction["files"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["path"] == execution_path)
        .and_then(|row| row.get("after"))
        .map(|snapshot| {
            decode_snapshot(snapshot).map_err(|issue| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            })
        })
        .transpose()?
        .map_or_else(|| repository.read_execution(&execution_path), Ok)?;
    let execution = parse_json_contract(&execution_raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The candidate execution index is invalid.",
        )
    })?;
    let mut documents = vec![
        json!({"path":plan_path,"kind":"plan","content":candidate["plan"]}),
        json!({"path":index_path,"kind":"task_index","content":candidate["task_index"]}),
        json!({"path":execution_path,"kind":"execution_index","content":execution}),
    ];
    let directory = index_path.rsplit_once('/').map_or("", |(parent, _)| parent);
    for (id, item) in candidate["task_items"].as_object().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "TASK items are missing.",
        )
    })? {
        documents.push(json!({"path":format!("{directory}/tasks/{id}.json"),
            "kind":"task_item","task_id":id,"content":item}));
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecMigrationPreviewRequest,
    >(
        json!({"schema":"work-spec-migration-preview-request/v1",
        "sources":sources,"candidates":documents,
        "semantic_decisions":semantic["semantic_decisions"].as_array().cloned().unwrap_or_default()})
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnusedExecution;
    impl MigrationPrepareRepository for UnusedExecution {
        fn read_execution(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("invalid candidate must stop before reading execution")
        }
    }

    #[test]
    fn revision_candidate_requires_plan_path_before_execution_read() {
        let error = build_revision_request(
            &UnusedExecution,
            &json!({}),
            &json!({"request":{"plan":{"artifacts":{}}}}),
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "spec_artifact_identity");
    }
}
