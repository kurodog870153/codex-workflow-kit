//! Reconciliation of a closed Attempt with a durable ledger transaction.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{ArtifactStore, WriterLock};
use work_feature::specification::reconciliation_input::unique_execution_source;
use work_feature::specification::reconciliation_project::{
    ReconciliationArtifactRepository, preview_from_repository,
};
use work_feature::specification::reconciliation_publication::{
    ledger_transaction, require_approved_preview,
};
use work_feature::specification::reconciliation_semantic::{
    ReconciliationSemanticRepository, prepare_semantic_selection,
};
use work_operations::canonical::sha256_hex;
use work_operations::specification::reconciliation_ledger::render_ledger;

use crate::files::LocalFiles;
use crate::skill_catalog::SkillRootConfig;
use crate::specification::migration::{prepare_revision_request, preview_migration};
use crate::specification::migration_publication::publish_migration_with_additional;
use crate::specification::storage::{
    publish_journal, require_no_spec_update, storage_path, write_journal,
};
use crate::writer_lock::LocalWriterLock;

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn execution_source(root: &Path, requirement: &str) -> Result<String, WorkError> {
    let mut pending = vec!["outputs/work".to_owned()];
    let mut matches = Vec::new();
    while let Some(relative) = pending.pop() {
        let directory = storage_path(root, &relative)?;
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&directory).map_err(|_| {
            fail(
                "file_read_failed",
                "The managed Work directory cannot be inspected.",
            )
        })? {
            let entry = entry.map_err(|_| {
                fail(
                    "file_read_failed",
                    "The managed Work directory cannot be inspected.",
                )
            })?;
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            let child = format!("{relative}/{}", entry.file_name().to_string_lossy());
            if path.is_dir() {
                pending.push(child);
            } else if path.is_file() && entry.file_name() == "index.json" {
                let Ok(raw) = LocalFiles.read_raw(&storage_path(root, &child)?) else {
                    continue;
                };
                let Ok(index) = serde_json::from_slice::<Value>(&raw) else {
                    continue;
                };
                if index["schema"] == "work-execution-index/v1"
                    && index["requirement_id"] == requirement
                {
                    matches.push(relative.clone());
                }
            }
        }
    }
    unique_execution_source(matches)
}

pub struct LocalSemanticReconciliation<'a> {
    pub root: &'a Path,
}
impl ReconciliationSemanticRepository for LocalSemanticReconciliation<'_> {
    fn execution_source(&self, requirement: &str) -> Result<String, WorkError> {
        execution_source(self.root, requirement)
    }
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&storage_path(self.root, relative)?)
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(storage_path(self.root, relative)?.is_file())
    }
}

pub fn prepare_from_semantic(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    semantic: &Value,
    date: &str,
) -> Result<Value, WorkError> {
    let selection = prepare_semantic_selection(&LocalSemanticReconciliation { root }, semantic)?;
    let migration = if selection.choice == "retain_only" {
        Value::Null
    } else {
        let request = json!({"schema":"work-spec-migration-prepare-request/v1",
            "mode":"revision","requirement_id":selection.requirement,
            "reason":selection.reason,"edits":selection.edits,
            "semantic_decisions":selection.decisions});
        prepare_revision_request(
            root,
            skill_root,
            configs,
            &serde_json::to_vec(&request).map_err(|_| {
                fail(
                    "invalid_json_contract",
                    "The semantic reconciliation request is invalid.",
                )
            })?,
            date,
        )?
    };
    let request = work_model::specification::verified::<
        work_model::specification::SpecReconciliationPreviewRequest,
    >(
        json!({"schema":"work-spec-reconciliation-preview-request/v1",
        "attempt_path":selection.attempt_path,"choice":selection.choice,
        "deviation_ids":selection.selected,"migration":migration}),
    );
    let preview = preview_from_project(root, skill_root, configs, &request)?;
    Ok(json!({"request":request,"preview":preview,"output_file":null}))
}

pub struct LocalReconciliationArtifacts<'a> {
    pub root: &'a Path,
}
impl ReconciliationArtifactRepository for LocalReconciliationArtifacts<'_> {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&storage_path(self.root, relative)?)
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(storage_path(self.root, relative)?.is_file())
    }
}

pub fn preview_from_project(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
) -> Result<Value, WorkError> {
    preview_from_repository(
        &LocalReconciliationArtifacts { root },
        request,
        |migration| preview_migration(root, skill_root, configs, migration),
    )
}

pub fn publish_ledger_only(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let preview = preview_from_project(root, skill_root, configs, request)?;
    require_approved_preview(&preview, request, approved_sha256, false)?;
    let ledger_path = preview["ledger_path"].as_str().unwrap();
    let target = storage_path(root, ledger_path)?;
    let before = target
        .is_file()
        .then(|| LocalFiles.read_raw(&target))
        .transpose()?;
    let after = render_ledger(&preview["ledger"])
        .map_err(|message| fail("reconciliation_ledger", message))?;
    let transaction = ledger_transaction(&preview, approved_sha256, before.as_deref(), &after)?;
    let execution_dir = transaction.execution_dir.as_str();
    let journal_path = transaction.journal_path.as_str();
    let marker_path = transaction.marker_path.as_str();
    let approval = &transaction.approval;
    require_no_spec_update(root, execution_dir, Some(journal_path))?;
    let writer = storage_path(root, &format!("{execution_dir}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&writer)?;
    require_no_spec_update(root, execution_dir, Some(journal_path))?;
    write_journal(root, journal_path, &transaction.journal)?;
    let published = publish_journal(root, journal_path, marker_path)?;
    if sha256_hex(&LocalFiles.read_raw(&target)?) != sha256_hex(&after) {
        return Err(fail(
            "reconciliation_ledger_post_write",
            "The installed reconciliation ledger differs from approval.",
        ));
    }
    let publication = json!({"schema":"work-spec-migration-publication/v1",
        "status":"updated","fingerprint":approved_sha256,
        "transaction_approval_sha256":approval,"journal":journal_path,
        "completion_marker":marker_path,"documents":[ledger_path],
        "publication_status":published["status"],"validator_results":[],"relationship_results":[]});
    Ok(work_model::specification::verified::<
        work_model::specification::SpecReconciliationPublication,
    >(
        json!({"schema":"work-spec-reconciliation-publication/v1","status":"updated",
        "reconciliation_fingerprint":approved_sha256,"attempt_path":preview["attempt_path"],
        "attempt_sha256":preview["attempt_sha256"],
        "selected_deviation_ids":preview["selected_deviation_ids"],
        "retained_deviation_ids":preview["retained_deviation_ids"],
        "ledger_path":ledger_path,"ledger_sha256":sha256_hex(&after),
        "publication":publication,"deviation_classifications":preview["deviation_classifications"]}),
    ))
}

pub fn publish_with_migration(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let preview = preview_from_project(root, skill_root, configs, request)?;
    require_approved_preview(&preview, request, approved_sha256, true)?;
    let ledger_path = preview["ledger_path"].as_str().unwrap();
    let ledger_raw = render_ledger(&preview["ledger"])
        .map_err(|message| fail("reconciliation_ledger", message))?;
    let additional = BTreeMap::from([(ledger_path.to_owned(), ledger_raw.clone())]);
    let migration_fingerprint = preview["migration_preview"]["fingerprint"]
        .as_str()
        .ok_or_else(|| {
            fail(
                "reconciliation_not_publishable",
                "The migration fingerprint is missing.",
            )
        })?;
    let publication = publish_migration_with_additional(
        root,
        skill_root,
        configs,
        &request["migration"],
        "apply",
        migration_fingerprint,
        &additional,
    )?;
    if sha256_hex(&LocalFiles.read_raw(&storage_path(root, ledger_path)?)?)
        != sha256_hex(&ledger_raw)
    {
        return Err(fail(
            "reconciliation_ledger_post_write",
            "The installed reconciliation ledger differs from approval.",
        ));
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecReconciliationPublication,
    >(
        json!({"schema":"work-spec-reconciliation-publication/v1","status":"updated",
        "reconciliation_fingerprint":approved_sha256,"attempt_path":preview["attempt_path"],
        "attempt_sha256":preview["attempt_sha256"],
        "selected_deviation_ids":preview["selected_deviation_ids"],
        "retained_deviation_ids":preview["retained_deviation_ids"],
        "ledger_path":ledger_path,"ledger_sha256":sha256_hex(&ledger_raw),
        "publication":publication,"deviation_classifications":preview["deviation_classifications"]}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use work_feature::specification::reconciliation_input::validate_preview_fields;

    #[test]
    fn retain_only_preview_rejects_candidate_ids_or_migration() {
        let mut request = json!({
            "schema": "work-spec-reconciliation-preview-request/v1",
            "attempt_path": "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
            "choice": "retain_only",
            "deviation_ids": [],
            "migration": null
        });
        validate_preview_fields(&request).unwrap();
        request["deviation_ids"] = json!(["DEVIATION-001"]);
        assert_eq!(
            validate_preview_fields(&request).unwrap_err().reason_code,
            "invalid_contract_value"
        );
        request["deviation_ids"] = json!([]);
        request["migration"] = json!({});
        assert_eq!(
            validate_preview_fields(&request).unwrap_err().reason_code,
            "invalid_contract_value"
        );
    }

    #[test]
    fn closed_attempt_preview_and_ledger_publication_match_python() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow");
        let root = std::env::temp_dir().join(format!(
            "work-reconcile-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        for relative in [
            request["attempt_path"].as_str().unwrap(),
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let migration_fixture = fixture.join("with-migration");
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(migration_fixture.join(relative), destination).unwrap();
        }
        let attempt_raw = fs::read(root.join(request["attempt_path"].as_str().unwrap())).unwrap();
        let attempt: Value = serde_json::from_slice(&attempt_raw).unwrap();
        assert_eq!(attempt["status"], "completed");
        assert_eq!(attempt["records"][0]["status"], "skipped");
        assert_eq!(
            attempt["execution_deviations"][0]["proposal"]["action"]["kind"],
            "skip_record"
        );
        assert_eq!(
            attempt["execution_deviations"][0]["supplemental_authorization"]["preview_sha256"],
            attempt["execution_deviations"][0]["approved_preview_sha256"]
        );
        let rendered = work_operations::execution::attempt::render_attempt(&attempt).unwrap();
        assert!(
            rendered == attempt_raw,
            "attempt byte difference: {:?}",
            String::from_utf8_lossy(&rendered)
                .lines()
                .zip(String::from_utf8_lossy(&attempt_raw).lines())
                .enumerate()
                .find(|(_, (left, right))| left != right)
                .map(|(line, (left, right))| (line, left.to_owned(), right.to_owned()))
        );
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let semantic: Value =
            serde_json::from_slice(&fs::read(fixture.join("semantic-request.json")).unwrap())
                .unwrap();
        let prepared = prepare_from_semantic(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &semantic,
            "2026-09-26",
        )
        .unwrap();
        let prepared_request: Value =
            serde_json::from_slice(&fs::read(fixture.join("prepared-request.json")).unwrap())
                .unwrap();
        assert_eq!(prepared["request"], prepared_request);
        assert_eq!(prepared["preview"], expected);
        let preview = preview_from_project(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request,
        )
        .unwrap();
        assert_eq!(preview, expected);
        let index_path = root.join("outputs/work/executions/example/index.json");
        let index_raw = fs::read(&index_path).unwrap();
        let mut changed_index: Value = serde_json::from_slice(&index_raw).unwrap();
        changed_index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-002");
        fs::write(&index_path, serde_json::to_vec(&changed_index).unwrap()).unwrap();
        assert_eq!(
            prepare_from_semantic(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &semantic,
                "2026-09-26",
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_attempt_position"
        );
        assert_eq!(
            preview_from_project(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &request
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_attempt_not_latest"
        );
        fs::write(&index_path, index_raw).unwrap();
        let duplicate = root.join("outputs/work/duplicate/index.json");
        fs::create_dir_all(duplicate.parent().unwrap()).unwrap();
        fs::copy(&index_path, &duplicate).unwrap();
        assert_eq!(
            prepare_from_semantic(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &semantic,
                "2026-09-26",
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_execution_source_ambiguous"
        );
        fs::remove_file(duplicate).unwrap();
        let mut invalid = request.clone();
        invalid["deviation_ids"] = json!(["DEVIATION-001"]);
        let error = preview_from_project(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &invalid,
        )
        .unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Contract);
        assert_eq!(error.reason_code, "invalid_contract_value");
        assert_eq!(error.details["location"], "contract");
        let published = publish_ledger_only(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request,
            expected["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        let reference: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        assert_eq!(published, reference);
        assert_eq!(
            published["retained_deviation_ids"],
            json!(["DEVIATION-001"])
        );
        let current_index: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        assert_eq!(current_index["overall_status"], "completed");
        assert_eq!(
            fs::read(root.join(request["attempt_path"].as_str().unwrap())).unwrap(),
            attempt_raw
        );
        for relative in [
            published["ledger_path"].as_str().unwrap(),
            published["publication"]["journal"].as_str().unwrap(),
        ] {
            assert_eq!(
                fs::read(root.join(relative)).unwrap(),
                fs::read(fixture.join(relative)).unwrap(),
                "artifact {relative}"
            );
        }
        let ledger_path = root.join(published["ledger_path"].as_str().unwrap());
        assert_eq!(
            preview_from_project(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &request
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_nothing_pending"
        );
        let mut corrupted: Value =
            serde_json::from_slice(&fs::read(&ledger_path).unwrap()).unwrap();
        corrupted["entries"][0]["attempt_sha256"] = json!("invalid");
        fs::write(&ledger_path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        let error = preview_from_project(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request,
        )
        .unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Contract);
        assert_eq!(error.reason_code, "invalid_contract_value");
        assert_eq!(error.details["location"], "entries[0].attempt_sha256");
    }

    #[test]
    fn retain_only_prepare_requires_only_execution_sources() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow");
        let semantic: Value =
            serde_json::from_slice(&fs::read(fixture.join("semantic-request.json")).unwrap())
                .unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "work-reconcile-execution-only-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/executions/example/index.json",
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let prepared = prepare_from_semantic(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &semantic,
            "2026-09-26",
        )
        .unwrap();
        assert_eq!(prepared["preview"], expected);
        assert!(prepared["request"]["migration"].is_null());
        let mut omitted_defaults = semantic.clone();
        for key in [
            "deviation_positions",
            "reason",
            "edits",
            "semantic_decisions",
        ] {
            omitted_defaults.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(
            prepare_from_semantic(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &omitted_defaults,
                "2026-09-26"
            )
            .unwrap()["request"],
            prepared["request"]
        );
        let mut invalid = semantic.clone();
        invalid["deviation_positions"] = json!([2, 1]);
        let error = prepare_from_semantic(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &invalid,
            "2026-09-26",
        )
        .unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Contract);
        assert_eq!(error.reason_code, "invalid_contract_value");
        assert_eq!(error.details["location"], "contract");
        invalid = semantic.clone();
        invalid["edits"] = json!([{"target":{"artifact":"task_item","task_id":"TASK-001"},
            "field":"goal","after":"changed"}]);
        assert_eq!(
            prepare_from_semantic(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &invalid,
                "2026-09-26"
            )
            .unwrap_err()
            .reason_code,
            "invalid_contract_value"
        );
        for (field, value) in [
            (
                "attempt_path",
                json!("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json"),
            ),
            ("plan_path", json!("outputs/work/plans/example.json")),
            ("deviation_ids", json!(["DEVIATION-001"])),
        ] {
            invalid = semantic.clone();
            invalid[field] = value;
            let error = prepare_from_semantic(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &invalid,
                "2026-09-26",
            )
            .unwrap_err();
            assert_eq!(error.reason_code, "invalid_object_fields", "{field}");
            assert_eq!(error.details["unknown"], json!([field]), "{field}");
        }
        invalid = semantic.clone();
        invalid.as_object_mut().unwrap().remove("task_position");
        let error = prepare_from_semantic(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &invalid,
            "2026-09-26",
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["missing"], json!(["task_position"]));
        let missing_root = root.join("missing-attempt");
        let index_relative = "outputs/work/executions/example/index.json";
        let missing_index = missing_root.join(index_relative);
        fs::create_dir_all(missing_index.parent().unwrap()).unwrap();
        fs::copy(fixture.join(index_relative), missing_index).unwrap();
        let error = prepare_from_semantic(
            &missing_root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &semantic,
            "2026-09-26",
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "reconciliation_attempt_missing");
        assert_eq!(error.exit_code, ExitCode::ArtifactIntegrity);
    }

    #[test]
    fn incorporated_deviation_preview_matches_python_migration() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration");
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "work-reconcile-migration-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for source in request["migration"]["sources"].as_array().unwrap() {
            let relative = source["path"].as_str().unwrap();
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let attempt = request["attempt_path"].as_str().unwrap();
        let destination = root.join(attempt);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(attempt), destination).unwrap();
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let actual = preview_from_project(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request,
        )
        .unwrap();
        let semantic: Value =
            serde_json::from_slice(&fs::read(fixture.join("semantic-request.json")).unwrap())
                .unwrap();
        let prepared = prepare_from_semantic(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &semantic,
            "2026-09-26",
        )
        .unwrap();
        assert_eq!(prepared["request"], request);
        assert_eq!(prepared["preview"], expected);
        fn first_difference(left: &Value, right: &Value, path: &str) -> Option<String> {
            if left == right {
                return None;
            }
            if let (Some(left), Some(right)) = (left.as_str(), right.as_str()) {
                let difference = left
                    .lines()
                    .zip(right.lines())
                    .enumerate()
                    .find(|(_, (actual, expected))| actual != expected);
                return Some(format!(
                    "{path}: first line={difference:?}, line counts={}/{}",
                    left.lines().count(),
                    right.lines().count()
                ));
            }
            if let (Some(left), Some(right)) = (left.as_object(), right.as_object()) {
                for key in left.keys().chain(right.keys()) {
                    if let Some(difference) = first_difference(
                        left.get(key).unwrap_or(&Value::Null),
                        right.get(key).unwrap_or(&Value::Null),
                        &format!("{path}/{key}"),
                    ) {
                        return Some(difference);
                    }
                }
            }
            if let (Some(left), Some(right)) = (left.as_array(), right.as_array()) {
                for position in 0..left.len().max(right.len()) {
                    if let Some(difference) = first_difference(
                        left.get(position).unwrap_or(&Value::Null),
                        right.get(position).unwrap_or(&Value::Null),
                        &format!("{path}/{position}"),
                    ) {
                        return Some(difference);
                    }
                }
            }
            Some(format!("{path}: actual={} expected={}", left, right))
        }
        assert!(
            actual == expected,
            "{}",
            first_difference(&actual, &expected, "").unwrap_or_default()
        );
        let published = publish_with_migration(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request,
            expected["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        let reference: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        assert_eq!(published, reference);
        for relative in [
            published["ledger_path"].as_str().unwrap(),
            published["publication"]["journal"].as_str().unwrap(),
        ] {
            let actual = fs::read(root.join(relative)).unwrap();
            let reference = fs::read(fixture.join(relative)).unwrap();
            let difference = String::from_utf8_lossy(&actual)
                .lines()
                .zip(String::from_utf8_lossy(&reference).lines())
                .enumerate()
                .find(|(_, (left, right))| left != right)
                .map(|(line, (left, right))| (line, left.to_owned(), right.to_owned()));
            assert!(
                actual == reference,
                "artifact {relative}, first difference {difference:?}"
            );
        }
    }

    #[test]
    fn incorporated_deviation_recovers_after_ledger_write_before_journal_progress() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration");
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "work-reconcile-recovery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for source in request["migration"]["sources"].as_array().unwrap() {
            let relative = source["path"].as_str().unwrap();
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let attempt_path = request["attempt_path"].as_str().unwrap();
        let destination = root.join(attempt_path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(attempt_path), destination).unwrap();
        let publication: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        let journal_path = publication["publication"]["journal"].as_str().unwrap();
        let mut journal: Value =
            serde_json::from_slice(&fs::read(fixture.join(journal_path)).unwrap()).unwrap();
        journal["state"] = json!("prepared");
        journal["published_count"] = json!(0);
        write_journal(&root, journal_path, &journal).unwrap();
        let ledger_path = expected["ledger_path"].as_str().unwrap();
        let ledger_raw = render_ledger(&expected["ledger"]).unwrap();
        fs::write(root.join(ledger_path), &ledger_raw).unwrap();
        let recovered = publish_migration_with_additional(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request["migration"],
            "recover",
            expected["migration_preview"]["fingerprint"]
                .as_str()
                .unwrap(),
            &BTreeMap::from([(ledger_path.to_owned(), ledger_raw.clone())]),
        )
        .unwrap();
        assert_eq!(recovered["status"], "recovered");
        assert_eq!(fs::read(root.join(ledger_path)).unwrap(), ledger_raw);
        assert_eq!(
            fs::read(root.join(journal_path)).unwrap(),
            fs::read(fixture.join(journal_path)).unwrap()
        );
    }
}
