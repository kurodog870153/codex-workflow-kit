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

pub fn validate_semantic_request(value: &Value) -> Result<(), WorkError> {
    let request: work_model::specification::SpecMigrationPrepareRequest =
        serde_json::from_value(value.clone()).map_err(|cause| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_contract_value",
                "Migration requires reviewed Task decisions and exact raw source fingerprints.",
                json!({"cause":cause.to_string()}),
            )
        })?;
    if request
        .requirement_id
        .parse::<work_operations::identifiers::RequirementId>()
        .is_err()
        || request.sources.is_empty()
    {
        return Err(fail(
            "migration_source_missing",
            "A valid requirement and reviewed raw sources are required.",
        ));
    }
    let mut paths = std::collections::BTreeSet::new();
    for source in &request.sources {
        if !work_operations::task::source::valid_relative_path(&source.path)
            || !work_operations::protocol::valid_sha256(&source.raw_sha256)
            || !paths.insert(work_operations::canonical::portable_path_identity(
                &source.path,
            ))
        {
            return Err(fail(
                "migration_source_invalid",
                "Reviewed source paths must be unique with exact SHA-256 fingerprints.",
            ));
        }
    }
    let decisions = value["semantic_decisions"].as_array().ok_or_else(|| {
        fail(
            "migration_decisions_missing",
            "Explicit reviewed semantic decisions are required.",
        )
    })?;
    let mut ids = std::collections::BTreeSet::new();
    for decision in decisions {
        if decision["id"]
            .as_str()
            .is_none_or(|id| id.trim().is_empty() || !ids.insert(id))
            || decision["resolution"].is_null()
        {
            return Err(fail(
                "migration_semantic_decisions_unresolved",
                "Every semantic decision requires a unique ID and reviewed resolution.",
            ));
        }
    }
    let allowed = match request.mode.as_str() {
        "revision" => vec![
            "schema",
            "mode",
            "requirement_id",
            "sources",
            "reason",
            "edits",
            "semantic_decisions",
        ],
        "reconstruction" => vec![
            "schema",
            "mode",
            "requirement_id",
            "sources",
            "task_context",
            "task_title",
            "task_summary",
            "execution_defaults",
            "tasks",
            "semantic_decisions",
        ],
        _ => {
            return Err(fail(
                "migration_prepare_mode",
                "Migration mode must be revision or reconstruction.",
            ));
        }
    };
    if value
        .as_object()
        .unwrap()
        .keys()
        .any(|field| !allowed.contains(&field.as_str()))
    {
        return Err(fail(
            "migration_prepare_fields",
            "Migration fields do not match the selected mode.",
        ));
    }
    if request.mode == "reconstruction"
        && (request.task_context.is_none()
            || value["tasks"].as_array().is_none_or(Vec::is_empty)
            || ["task_title", "task_summary"].iter().any(|field| {
                value[field]
                    .as_str()
                    .is_none_or(|text| text.trim().is_empty())
            }))
    {
        return Err(fail(
            "migration_decisions_missing",
            "Complete Task context, title, summary and semantic tasks are required.",
        ));
    }
    work_operations::derivation::legacy_layout::validate_offline_review(value)
        .map_err(|code| fail(code, "Layout migration requires explicit offline deployment review and the complete exact raw inventory."))?;
    Ok(())
}

pub fn parse_revision_semantic(raw: &[u8]) -> Result<RevisionSemantic, WorkError> {
    let semantic = parse_json_contract(raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The migration preparation is invalid.",
        )
    })?;
    if semantic["schema"] != "work-spec-migration-prepare-request" || semantic["mode"] != "revision"
    {
        return Err(fail(
            "migration_prepare_mode",
            "A revision migration is required.",
        ));
    }
    validate_semantic_request(&semantic)?;
    let revision = json!({"schema":"work-spec-prepare-request",
        "requirement_id":semantic["requirement_id"],"reason":semantic["reason"],
        "edits":semantic["edits"]});
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
    let artifacts = &candidate["task_index"]["artifacts"];
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
    let reviewed = semantic["sources"].as_array().ok_or_else(|| {
        fail(
            "migration_source_missing",
            "Reviewed raw source fingerprints are required.",
        )
    })?;
    if sources.iter().any(|source| !reviewed.contains(source)) {
        return Err(fail(
            "migration_source_review_incomplete",
            "Every current revision source must be included in the reviewed evidence.",
        ));
    }
    sources = reviewed.clone();
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
    >(json!({"schema":"work-spec-migration-preview-request",
        "sources":sources,"candidates":documents,
        "semantic_decisions":semantic["semantic_decisions"].as_array().cloned().unwrap_or_default()})))
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
    fn revision_candidate_requires_task_path_before_execution_read() {
        let error = build_revision_request(
            &UnusedExecution,
            &json!({}),
            &json!({"request":{"task_index":{"artifacts":{}}}}),
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "spec_artifact_identity");
    }
}
