//! Read-only verification of published Migration evidence and installed bytes.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::specification::migration_publication::publication_paths;
use work_operations::derivation::fingerprint;
use work_operations::derivation::publication::{completion_marker, completion_marker_path};
use work_operations::derivation::snapshot::decode_snapshot;
use work_operations::execution::index::render_execution_index;
use work_operations::plan::render_plan_value;
use work_operations::specification::transaction::{render_transaction, validate_transaction};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::skill_catalog::SkillRootConfig;
use crate::specification::migration_reconciliation_publication::verify_final_chain;
use crate::specification::storage::storage_path;

fn fail(code: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, code, message, json!({}))
}

pub fn verify_published_transaction(
    root: &Path,
    journal_relative: &str,
    expected_request: Option<&Value>,
) -> Result<Value, WorkError> {
    let path = storage_path(root, journal_relative)?;
    if !path.is_file() {
        return Err(fail(
            "migration_verify_result_missing",
            "The Migration result journal is missing.",
        ));
    }
    let raw = LocalFiles.read_raw(&path)?;
    let journal: Value = serde_json::from_slice(&raw).map_err(|_| {
        fail(
            "migration_verify_result_invalid",
            "The Migration result journal is invalid JSON.",
        )
    })?;
    validate_transaction(&journal).map_err(|_| {
        fail(
            "migration_verify_result_invalid",
            "The Migration result journal has invalid evidence.",
        )
    })?;
    if render_transaction(&journal).ok().as_deref() != Some(raw.as_slice()) {
        return Err(fail(
            "migration_verify_result_invalid",
            "The Migration result journal is not canonical.",
        ));
    }
    if expected_request.is_some_and(|expected| journal["metadata"]["request"] != *expected) {
        return Err(fail(
            "migration_verify_request_mismatch",
            "The result journal does not bind to the supplied request.",
        ));
    }
    let files = journal["files"].as_array().expect("validated transaction");
    if journal["state"] != "published" || journal["published_count"] != files.len() {
        return Err(fail(
            "migration_verify_unpublished",
            "The Migration result has not been fully published.",
        ));
    }
    let marker_path = storage_path(root, &completion_marker_path(journal_relative))?;
    if !marker_path.is_file() || LocalFiles.read_raw(&marker_path)? != completion_marker(&raw) {
        return Err(fail(
            "migration_verify_marker_mismatch",
            "The Migration completion marker does not match the result journal.",
        ));
    }
    let mut installed = BTreeMap::new();
    for file in files {
        let relative = file["path"].as_str().expect("validated transaction path");
        let target = storage_path(root, relative)?;
        let current = if target.is_file() {
            Some(LocalFiles.read_raw(&target)?)
        } else {
            None
        };
        let after = file
            .get("after")
            .map(|snapshot| decode_snapshot(snapshot).expect("validated transaction snapshot"));
        if current != after {
            return Err(fail(
                "migration_verify_installed_mismatch",
                "Installed bytes differ from the published result.",
            ));
        }
        if let Some(bytes) = current {
            let hash = fingerprint::raw(&bytes);
            if journal["metadata"]["candidate_sha256"][relative] != hash {
                return Err(fail(
                    "migration_verify_result_invalid",
                    "The result candidate fingerprint is inconsistent.",
                ));
            }
            installed.insert(relative.to_owned(), hash);
        }
        if let Some(before) = file.get("before") {
            let bytes = decode_snapshot(before).expect("validated transaction snapshot");
            if journal["metadata"]["source_sha256"][relative] != fingerprint::raw(&bytes) {
                return Err(fail(
                    "migration_verify_result_invalid",
                    "The result source fingerprint is inconsistent.",
                ));
            }
        }
    }
    Ok(
        json!({"journal":journal_relative,"journal_sha256":fingerprint::raw(&raw),
        "installed_sha256":installed,"approval_sha256":journal["approval_sha256"]}),
    )
}

pub fn verify_semantic_migration(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    if request["schema"] != "work-spec-migration-preview-request/v1" {
        return Err(fail(
            "migration_verify_request_invalid",
            "A semantic Migration preview request is required.",
        ));
    }
    let paths = publication_paths(request, approved_sha256)?;
    let expected = json!({"migration":request,"preview_fingerprint":approved_sha256});
    let result = verify_published_transaction(root, &paths.journal, Some(&expected))?;
    let raw = LocalFiles.read_raw(&storage_path(root, &paths.journal)?)?;
    let journal: Value = serde_json::from_slice(&raw).expect("verified journal");
    for source in request["sources"].as_array().ok_or_else(|| {
        fail(
            "migration_verify_request_invalid",
            "Migration source evidence is missing.",
        )
    })? {
        let relative = source["path"].as_str().ok_or_else(|| {
            fail(
                "migration_verify_request_invalid",
                "A source path is missing.",
            )
        })?;
        if journal["metadata"]["source_sha256"][relative] != source["raw_sha256"] {
            return Err(fail(
                "migration_verify_request_mismatch",
                "The published source evidence differs from the request.",
            ));
        }
    }
    for candidate in request["candidates"].as_array().ok_or_else(|| {
        fail(
            "migration_verify_request_invalid",
            "Migration candidates are missing.",
        )
    })? {
        let relative = candidate["path"].as_str().ok_or_else(|| {
            fail(
                "migration_verify_request_invalid",
                "A candidate path is missing.",
            )
        })?;
        let bytes = match candidate["kind"].as_str() {
            Some("plan") => render_plan_value(&candidate["content"]),
            Some("task_index") => render_task(&candidate["content"], TaskDocumentKind::Index),
            Some("task_item") => render_task(&candidate["content"], TaskDocumentKind::Item),
            Some("execution_index") => render_execution_index(&candidate["content"]),
            _ => {
                return Err(fail(
                    "migration_verify_request_invalid",
                    "A candidate kind is invalid.",
                ));
            }
        }
        .map_err(|_| {
            fail(
                "migration_verify_request_invalid",
                "A candidate cannot be rendered.",
            )
        })?;
        if journal["metadata"]["candidate_sha256"][relative] != fingerprint::raw(&bytes)
            || result["installed_sha256"][relative] != fingerprint::raw(&bytes)
        {
            return Err(fail(
                "migration_verify_request_mismatch",
                "Installed candidate bytes differ from the reviewed request.",
            ));
        }
    }
    let plan = request["candidates"]
        .as_array()
        .expect("checked candidates")
        .iter()
        .find(|row| row["kind"] == "plan")
        .ok_or_else(|| {
            fail(
                "migration_verify_request_invalid",
                "A Plan candidate is required.",
            )
        })?;
    let requirement = plan["content"]["requirement_id"].as_str().ok_or_else(|| {
        fail(
            "migration_verify_request_invalid",
            "The requirement ID is missing.",
        )
    })?;
    let chain = verify_final_chain(root, skill_root, configs, requirement)?;
    Ok(
        json!({"schema":"work-spec-migration-verification/v1","status":"valid",
        "mode":"semantic","fingerprint":approved_sha256,"result":result,"final_chain":chain}),
    )
}
