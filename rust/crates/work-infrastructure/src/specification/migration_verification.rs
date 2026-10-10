//! Read-only verification of published Migration evidence and installed bytes.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::specification::migration_publication::publication_paths;
use work_operations::derivation::fingerprint;
use work_operations::derivation::publication::completion_marker;
use work_operations::derivation::snapshot::decode_snapshot;
use work_operations::execution::index::render_execution_index;
use work_operations::specification::transaction::{render_transaction, validate_transaction};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::skill_catalog::SkillRootConfig;
use crate::specification::storage::storage_path;

fn fail(code: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, code, message, json!({}))
}

pub fn verify_published_transaction(
    root: &Path,
    journal_relative: &str,
    expected_request: Option<&Value>,
) -> Result<Value, WorkError> {
    verify_published_transaction_with_layout(root, journal_relative, expected_request)
}

pub fn verify_published_retained_transaction(
    root: &Path,
    execution: &str,
    journal_relative: &str,
    expected_request: Option<&Value>,
) -> Result<Value, WorkError> {
    crate::specification::storage::read_retained_journal(root, execution, journal_relative)
        .map_err(retained_verification_error)?;
    verify_published_transaction_with_layout(root, journal_relative, expected_request)
}

fn retained_verification_error(problem: WorkError) -> WorkError {
    if problem.reason_code == "journal_commit_evidence" {
        WorkError::new(
            problem.exit_code,
            "migration_verify_marker_mismatch",
            "The published marker differs from the approved journal.",
            problem.details,
        )
    } else {
        problem
    }
}

fn verify_published_transaction_with_layout(
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
    {
        let execution = journal["metadata"]["artifacts"]["execution"]
            .as_str()
            .ok_or_else(|| {
                fail(
                    "journal_layout_metadata",
                    "The journal must identify its execution scope.",
                )
            })?;
        crate::specification::storage::read_retained_journal(root, execution, journal_relative)
            .map_err(retained_verification_error)?;
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
    let marker_relative = {
        work_operations::derivation::publication::retained_journal_marker(journal_relative)
            .map_err(|_| {
                fail(
                    "journal_layout_identity",
                    "The retained journal path is invalid.",
                )
            })?
    };
    let marker_path = storage_path(root, &marker_relative)?;
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
    for (relative, expected) in journal["metadata"]["history_sha256"]
        .as_object()
        .expect("verified history metadata")
    {
        let changed = files.iter().find(|file| file["path"] == *relative);
        let raw = if let Some(before) = changed.and_then(|file| file.get("before")) {
            decode_snapshot(before).expect("verified snapshot")
        } else {
            LocalFiles
                .read_raw(&storage_path(root, relative)?)
                .map_err(|_| {
                    fail(
                        "migration_verify_history_mismatch",
                        "An immutable history record is missing.",
                    )
                })?
        };
        if json!(fingerprint::history(&raw)) != *expected {
            return Err(fail(
                "migration_verify_history_mismatch",
                "Immutable history bytes differ from approved publication evidence.",
            ));
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
    if request["schema"] != "work-spec-migration-preview-request" {
        return Err(fail(
            "migration_verify_request_invalid",
            "A semantic Migration preview request is required.",
        ));
    }
    let _: work_model::specification::SpecMigrationPreviewRequest = serde_json::from_value(request.clone()).map_err(|_| fail("migration_verify_request_invalid", "Verification accepts only current Task and Execution candidates with exact raw evidence."))?;
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
    let task = request["candidates"]
        .as_array()
        .expect("checked candidates")
        .iter()
        .find(|row| row["kind"] == "task_index")
        .ok_or_else(|| {
            fail(
                "migration_verify_request_invalid",
                "A Task index candidate is required.",
            )
        })?;
    let _requirement = task["content"]["requirement_id"].as_str().ok_or_else(|| {
        fail(
            "migration_verify_request_invalid",
            "The requirement ID is missing.",
        )
    })?;
    let mut baseline = BTreeMap::new();
    for file in journal["files"].as_array().expect("verified journal files") {
        if let Some(before) = file.get("before") {
            baseline.insert(
                file["path"].as_str().expect("verified path").to_owned(),
                decode_snapshot(before).expect("verified snapshot"),
            );
        }
    }
    let preview = crate::specification::migration::preview_migration_with_baseline(
        root,
        skill_root,
        configs,
        request,
        Some(&baseline),
    )?;
    work_feature::specification::migration_publication::require_approved_preview(
        &preview,
        approved_sha256,
    )?;
    let installed = request["candidates"]
        .as_array()
        .expect("checked candidates")
        .iter()
        .map(|candidate| {
            let path = candidate["path"].as_str().expect("checked path");
            (path.to_owned(), result["installed_sha256"][path].clone())
        })
        .collect::<BTreeMap<_, _>>();
    let chain = json!({"status":"valid", "installed_sha256":installed});
    Ok(
        json!({"schema":"work-spec-migration-verification","status":"valid",
        "mode":"semantic","fingerprint":approved_sha256,"result":result,"final_chain":chain}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use work_operations::derivation::transaction::{
        PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
    };

    fn test_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "work-migration-verify-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn published_verification_rejects_history_drift_without_writes() {
        let root = test_root("history");
        let history_path = "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json";
        let history_raw = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/cases/execution/handoff/closed-stopped/project")
                .join(history_path),
        )
        .unwrap();
        fs::create_dir_all(root.join(history_path).parent().unwrap()).unwrap();
        fs::write(root.join(history_path), &history_raw).unwrap();
        fs::write(root.join("artifact.json"), b"original").unwrap();
        let transaction = TransactionDeriver::derive(TransactionInput {
            kind: TransactionKind::Migration,
            order: PublicationOrder::Flat,
            request: json!({"migration":{},"preview_fingerprint":"a".repeat(64)}),
            artifacts: json!({"execution":"outputs/work/executions/example"}),
            affected_task_ids: vec![],
            history: BTreeMap::from([(history_path.into(), history_raw)]),
            source: BTreeMap::from([("artifact.json".into(), b"original".to_vec())]),
            candidate: BTreeMap::from([("artifact.json".into(), b"reviewed".to_vec())]),
        })
        .unwrap()
        .journal;
        let relative = work_operations::derivation::publication::journal_path(
            "outputs/work/executions/example",
            work_operations::derivation::publication::JournalKind::SpecificationMigration(
                &"a".repeat(64),
            ),
        );
        let marker_relative =
            work_operations::derivation::publication::completion_marker_path(&relative);
        fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
        crate::specification::storage::write_journal(&root, &relative, &transaction).unwrap();
        crate::specification::storage::publish_journal(&root, &relative, &marker_relative).unwrap();
        assert!(verify_published_transaction(&root, &relative, None).is_ok());
        let journal = fs::read(root.join(&relative)).unwrap();
        let marker = fs::read(root.join(&marker_relative)).unwrap();
        fs::write(root.join(history_path), b"history drift").unwrap();
        assert_eq!(
            verify_published_transaction(&root, &relative, None)
                .unwrap_err()
                .reason_code,
            "migration_verify_history_mismatch"
        );
        assert_eq!(fs::read(root.join(&relative)).unwrap(), journal);
        assert_eq!(fs::read(root.join(&marker_relative)).unwrap(), marker);
        assert_eq!(fs::read(root.join("artifact.json")).unwrap(), b"reviewed");
        assert_eq!(fs::read(root.join(history_path)).unwrap(), b"history drift");
    }

    #[test]
    fn semantic_verification_reads_reviewed_custom_task_and_snapshot_routes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project");
        let root = test_root("custom");
        let mut request: Value = serde_json::from_slice(&fs::read(repo.join("crates/work-infrastructure/fixtures/cases/specification/migration/reconstruction/input/semantic-request.json")).unwrap()).unwrap();
        let index: Value = serde_json::from_slice(
            &fs::read(fixture.join("outputs/work/tasks/example/index.json")).unwrap(),
        )
        .unwrap();
        request["task_context"]["source"] = index["source"].clone();
        request["task_context"]["artifacts"] = json!({"source":"custom/sources/example","task":"custom/tasks/example/index.json","execution":"custom/executions/example"});
        let mut paths = work_feature::task::source::evidence_paths(&index).unwrap();
        paths.extend([
            "outputs/work/tasks/example/index.json".into(),
            "outputs/work/tasks/example/tasks/TASK-001.json".into(),
            "outputs/work/executions/example/index.json".into(),
        ]);
        let mut sources = Vec::new();
        for path in paths {
            let raw = fs::read(fixture.join(&path)).unwrap();
            let destination = path.replacen("outputs/work/", "custom/", 1);
            fs::create_dir_all(root.join(&destination).parent().unwrap()).unwrap();
            fs::write(root.join(&destination), &raw).unwrap();
            sources.push(json!({"path":destination,"raw_sha256":fingerprint::raw(&raw)}));
        }
        request["sources"] = json!(sources);
        let skill =
            crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap();
        let candidate = crate::specification::reconstruction::prepare_reconstruction_request(
            &root,
            &skill,
            &[],
            &serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        let preview =
            crate::specification::migration::preview_migration(&root, &skill, &[], &candidate)
                .unwrap();
        let approved = preview["fingerprint"].as_str().unwrap();
        let published = crate::specification::migration_publication::publish_migration(
            &root,
            &skill,
            &[],
            &candidate,
            "apply",
            approved,
        )
        .unwrap();
        let journal = root.join(published["journal"].as_str().unwrap());
        let before = fs::read(&journal).unwrap();
        assert_eq!(
            verify_semantic_migration(&root, &skill, &[], &candidate, approved).unwrap()["status"],
            "valid"
        );
        let source_path = root.join("custom/sources/example/SRC-001/source.txt");
        let mut source = fs::read(&source_path).unwrap();
        source[0] ^= 1;
        fs::write(&source_path, &source).unwrap();
        assert_eq!(
            verify_semantic_migration(&root, &skill, &[], &candidate, approved)
                .unwrap_err()
                .reason_code,
            "migration_verify_installed_mismatch"
        );
        assert_eq!(fs::read(&journal).unwrap(), before);
        assert_eq!(fs::read(&source_path).unwrap(), source);
        assert!(!root.join("outputs/work/plans").exists());
    }
}
