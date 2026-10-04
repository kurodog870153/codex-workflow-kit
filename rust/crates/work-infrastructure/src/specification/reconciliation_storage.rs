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
    ledger_transaction_with_history, require_approved_preview,
};
use work_feature::specification::reconciliation_semantic::{
    ReconciliationSemanticRepository, prepare_semantic_selection,
};
use work_operations::derivation::fingerprint;
use work_operations::specification::reconciliation_ledger::render_ledger;

use crate::files::LocalFiles;
use crate::skill_catalog::SkillRootConfig;
use crate::specification::migration::{prepare_revision_request, preview_migration};
use crate::specification::migration_publication::publish_migration_with_guard;
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
                if index["schema"] == "work-execution-index"
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
        let request = json!({"schema":"work-spec-migration-prepare-request",
            "mode":"revision","requirement_id":selection.requirement,
            "reason":selection.reason,"edits":selection.edits,"sources":semantic["sources"],
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
    >(json!({"schema":"work-spec-reconciliation-preview-request",
        "attempt_path":selection.attempt_path,"choice":selection.choice,
        "deviation_ids":selection.selected,"migration":migration}));
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
    let attempt_path = preview["attempt_path"]
        .as_str()
        .expect("validated Attempt path");
    let execution_dir = attempt_path
        .split("/TASK-")
        .next()
        .expect("validated execution path");
    let index_path = format!("{execution_dir}/index.json");
    let history = [attempt_path.to_owned(), index_path]
        .into_iter()
        .map(|path| {
            let raw = LocalFiles.read_raw(&storage_path(root, &path)?)?;
            Ok((path, raw))
        })
        .collect::<Result<BTreeMap<_, _>, WorkError>>()?;
    let transaction = ledger_transaction_with_history(
        &preview,
        approved_sha256,
        before.as_deref(),
        &after,
        history,
    )?;
    let execution_dir = transaction.execution_dir.as_str();
    let journal_path = transaction.journal_path.as_str();
    let marker_path = transaction.marker_path.as_str();
    let approval = &transaction.approval;
    require_no_spec_update(root, execution_dir, Some(journal_path))?;
    let writer = storage_path(root, &format!("{execution_dir}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&writer)?;
    require_no_spec_update(root, execution_dir, Some(journal_path))?;
    let fresh = preview_from_project(root, skill_root, configs, request)?;
    require_approved_preview(&fresh, request, approved_sha256, false)?;
    let current_before = target
        .is_file()
        .then(|| LocalFiles.read_raw(&target))
        .transpose()?;
    if current_before != before {
        return Err(fail(
            "reconciliation_approval_changed",
            "The reviewed ledger changed before publication.",
        ));
    }
    for (path, expected) in transaction.journal["metadata"]["history_sha256"]
        .as_object()
        .expect("derived history")
    {
        if json!(fingerprint::history(
            &LocalFiles.read_raw(&storage_path(root, path)?)?
        )) != *expected
        {
            return Err(fail(
                "reconciliation_approval_changed",
                "Attempt or Execution changed before ledger publication.",
            ));
        }
    }
    write_journal(root, journal_path, &transaction.journal)?;
    let published = publish_journal(root, journal_path, marker_path)?;
    if !fingerprint::verify_ledger(&LocalFiles.read_raw(&target)?, &fingerprint::ledger(&after)) {
        return Err(fail(
            "reconciliation_ledger_post_write",
            "The installed reconciliation ledger differs from approval.",
        ));
    }
    let publication = json!({"schema":"work-spec-migration-publication",
        "status":"updated","fingerprint":approved_sha256,
        "transaction_approval_sha256":approval,"journal":journal_path,
        "completion_marker":marker_path,"documents":[ledger_path],
        "publication_status":published["status"],"validator_results":[],"relationship_results":[]});
    Ok(work_model::specification::verified::<
        work_model::specification::SpecReconciliationPublication,
    >(
        json!({"schema":"work-spec-reconciliation-publication","status":"updated",
        "reconciliation_fingerprint":approved_sha256,"attempt_path":preview["attempt_path"],
        "attempt_sha256":preview["attempt_sha256"],
        "selected_deviation_ids":preview["selected_deviation_ids"],
        "retained_deviation_ids":preview["retained_deviation_ids"],
        "ledger_path":ledger_path,"ledger_sha256":fingerprint::ledger(&after),
        "publication":publication,"deviation_classifications":preview["deviation_classifications"]}),
    ))
}

struct LedgerRecoverySources<'a> {
    root: &'a Path,
    ledger_path: &'a str,
    before: Option<Vec<u8>>,
}
impl ReconciliationArtifactRepository for LedgerRecoverySources<'_> {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        if relative == self.ledger_path {
            return self
                .before
                .clone()
                .ok_or_else(|| fail("reconciliation_ledger", "The original ledger is absent."));
        }
        LocalFiles.read_raw(&storage_path(self.root, relative)?)
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        if relative == self.ledger_path {
            return Ok(self.before.is_some());
        }
        Ok(storage_path(self.root, relative)?.is_file())
    }
}

pub fn recover_ledger_only(
    root: &Path,
    _skill_root: &Path,
    _configs: &[SkillRootConfig],
    request: &Value,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    work_feature::specification::reconciliation_input::validate_preview_fields(request)?;
    if request["choice"] != "retain_only" {
        return Err(fail(
            "reconciliation_migration_required",
            "Ledger-only recovery requires retain-only approval.",
        ));
    }
    if !work_operations::protocol::valid_sha256(approved_sha256) {
        return Err(fail(
            "reconciliation_approval_changed",
            "The approved reconciliation fingerprint is invalid.",
        ));
    }
    let attempt_path = request["attempt_path"]
        .as_str()
        .expect("validated Attempt path");
    let execution = attempt_path
        .rsplit_once("/TASK-")
        .ok_or_else(|| {
            fail(
                "reconciliation_attempt_identity",
                "An execution directory is required.",
            )
        })?
        .0;
    let journal_path = work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationMigration(
            approved_sha256,
        ),
    );
    let marker = work_operations::derivation::publication::completion_marker_path(&journal_path);
    let writer = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&writer)?;
    require_no_spec_update(root, execution, Some(&journal_path))?;
    let raw = LocalFiles.read_raw(&storage_path(root, &journal_path)?)?;
    let journal = work_operations::canonical::parse_json_contract(&raw).map_err(|_| {
        fail(
            "reconciliation_recovery_journal",
            "The ledger recovery journal is invalid.",
        )
    })?;
    work_operations::specification::transaction::validate_transaction(&journal).map_err(|_| {
        fail(
            "reconciliation_recovery_journal",
            "The ledger recovery journal has invalid evidence.",
        )
    })?;
    if work_operations::specification::transaction::render_transaction(&journal)
        .ok()
        .as_deref()
        != Some(raw.as_slice())
    {
        return Err(fail(
            "reconciliation_recovery_journal",
            "The ledger recovery journal must be canonical.",
        ));
    }
    let ledger_path = format!(
        "{}/reconciliation.json",
        attempt_path
            .strip_suffix("/attempt.json")
            .ok_or_else(|| fail(
                "reconciliation_attempt_identity",
                "An Attempt filename is required."
            ))?
    );
    let files = journal["files"].as_array().expect("validated files");
    if files.len() != 1 || files[0]["path"] != ledger_path {
        return Err(fail(
            "reconciliation_recovery_request_changed",
            "Ledger-only recovery may publish only its approved ledger.",
        ));
    }
    let before = files[0]
        .get("before")
        .map(work_operations::derivation::snapshot::decode_snapshot)
        .transpose()
        .map_err(|_| {
            fail(
                "reconciliation_recovery_journal",
                "Original ledger evidence is invalid.",
            )
        })?;
    let sources = LedgerRecoverySources {
        root,
        ledger_path: &ledger_path,
        before: before.clone(),
    };
    let preview = preview_from_repository(&sources, request, |_| {
        Err(fail(
            "reconciliation_migration_required",
            "Ledger-only recovery cannot invoke migration.",
        ))
    })?;
    require_approved_preview(&preview, request, approved_sha256, false)?;
    let after = render_ledger(&preview["ledger"])
        .map_err(|message| fail("reconciliation_ledger", message))?;
    let history = [attempt_path.to_owned(), format!("{execution}/index.json")]
        .into_iter()
        .map(|path| {
            Ok((
                path.clone(),
                LocalFiles.read_raw(&storage_path(root, &path)?)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, WorkError>>()?;
    let candidate = ledger_transaction_with_history(
        &preview,
        approved_sha256,
        before.as_deref(),
        &after,
        history,
    )?;
    let mut expected = candidate.journal.clone();
    expected["state"] = journal["state"].clone();
    expected["published_count"] = journal["published_count"].clone();
    if expected != journal {
        return Err(fail(
            "reconciliation_recovery_request_changed",
            "Recovery requires the identical approved ledger, Attempt and Execution evidence.",
        ));
    }
    let publication = publish_journal(root, &journal_path, &marker)?;
    if LocalFiles.read_raw(&storage_path(root, &ledger_path)?)? != after {
        return Err(fail(
            "reconciliation_ledger_post_write",
            "Recovered ledger bytes differ from approval.",
        ));
    }
    Ok(
        json!({"schema":"work-spec-reconciliation-publication","status":"recovered","reconciliation_fingerprint":approved_sha256,"attempt_path":attempt_path,"attempt_sha256":preview["attempt_sha256"],
        "selected_deviation_ids":preview["selected_deviation_ids"],"retained_deviation_ids":preview["retained_deviation_ids"],"ledger_path":ledger_path,"ledger_sha256":fingerprint::ledger(&after),"deviation_classifications":preview["deviation_classifications"],
        "publication":{"schema":"work-spec-migration-publication","status":"recovered","fingerprint":approved_sha256,"transaction_approval_sha256":candidate.approval,"journal":journal_path,"completion_marker":marker,"documents":[ledger_path],"publication_status":publication["status"],"validator_results":[],"relationship_results":[]}}),
    )
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
    let publication = publish_migration_with_guard(
        root,
        skill_root,
        configs,
        &request["migration"],
        "apply",
        migration_fingerprint,
        &additional,
        || {
            let fresh = preview_from_project(root, skill_root, configs, request)?;
            require_approved_preview(&fresh, request, approved_sha256, true)
        },
    )?;
    if !fingerprint::verify_ledger(
        &LocalFiles.read_raw(&storage_path(root, ledger_path)?)?,
        &fingerprint::ledger(&ledger_raw),
    ) {
        return Err(fail(
            "reconciliation_ledger_post_write",
            "The installed reconciliation ledger differs from approval.",
        ));
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecReconciliationPublication,
    >(
        json!({"schema":"work-spec-reconciliation-publication","status":"updated",
        "reconciliation_fingerprint":approved_sha256,"attempt_path":preview["attempt_path"],
        "attempt_sha256":preview["attempt_sha256"],
        "selected_deviation_ids":preview["selected_deviation_ids"],
        "retained_deviation_ids":preview["retained_deviation_ids"],
        "ledger_path":ledger_path,"ledger_sha256":fingerprint::ledger(&ledger_raw),
        "publication":publication,"deviation_classifications":preview["deviation_classifications"]}),
    ))
}

struct MigrationRecoverySources<'a> {
    root: &'a Path,
    before: &'a BTreeMap<String, Vec<u8>>,
    ledger_path: &'a str,
}
impl ReconciliationArtifactRepository for MigrationRecoverySources<'_> {
    fn read(&self, path: &str) -> Result<Vec<u8>, WorkError> {
        if let Some(raw) = self.before.get(path) {
            return Ok(raw.clone());
        }
        LocalFiles.read_raw(&storage_path(self.root, path)?)
    }
    fn exists(&self, path: &str) -> Result<bool, WorkError> {
        if path == self.ledger_path {
            return Ok(self.before.contains_key(path));
        }
        Ok(self.before.contains_key(path) || storage_path(self.root, path)?.is_file())
    }
}

pub fn recover_with_migration(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    work_feature::specification::reconciliation_input::validate_preview_fields(request)?;
    if request["migration"].is_null() || !work_operations::protocol::valid_sha256(approved_sha256) {
        return Err(fail(
            "reconciliation_migration_required",
            "Migration recovery requires an approved migration request.",
        ));
    }
    let attempt_path = request["attempt_path"].as_str().ok_or_else(|| {
        fail(
            "reconciliation_attempt_identity",
            "An Attempt path is required.",
        )
    })?;
    let execution = attempt_path
        .rsplit_once("/TASK-")
        .ok_or_else(|| {
            fail(
                "reconciliation_attempt_identity",
                "An execution directory is required.",
            )
        })?
        .0;
    let ledger_path = format!(
        "{}/reconciliation.json",
        attempt_path
            .strip_suffix("/attempt.json")
            .ok_or_else(|| fail(
                "reconciliation_attempt_identity",
                "An Attempt filename is required."
            ))?
    );
    let mut matches = Vec::new();
    for entry in fs::read_dir(storage_path(root, execution)?).map_err(|_| {
        fail(
            "reconciliation_recovery_journal",
            "The recovery directory cannot be read.",
        )
    })? {
        let entry = entry.map_err(|_| {
            fail(
                "reconciliation_recovery_journal",
                "The recovery directory entry cannot be read.",
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(".work-spec-migration-")
            || !name.ends_with(".json")
            || entry.path().is_symlink()
        {
            continue;
        }
        let raw = LocalFiles.read_raw(&storage_path(root, &format!("{execution}/{name}"))?)?;
        let Ok(journal) = work_operations::canonical::parse_json_contract(&raw) else {
            continue;
        };
        if journal["metadata"]["request"]["migration"] != request["migration"] {
            continue;
        }
        work_operations::specification::transaction::validate_transaction(&journal).map_err(
            |_| {
                fail(
                    "reconciliation_recovery_journal",
                    "The recovery journal evidence is invalid.",
                )
            },
        )?;
        if work_operations::specification::transaction::render_transaction(&journal)
            .ok()
            .as_deref()
            != Some(raw.as_slice())
        {
            return Err(fail(
                "reconciliation_recovery_journal",
                "The recovery journal must be canonical.",
            ));
        }
        let migration_approval = journal["metadata"]["request"]["preview_fingerprint"]
            .as_str()
            .ok_or_else(|| {
                fail(
                    "reconciliation_recovery_journal",
                    "The migration approval is missing.",
                )
            })?;
        let paths = work_feature::specification::migration_publication::publication_paths(
            &request["migration"],
            migration_approval,
        )?;
        if paths.journal != format!("{execution}/{name}") {
            return Err(fail(
                "reconciliation_recovery_journal",
                "The journal namespace differs from its approval.",
            ));
        }
        matches.push(journal);
    }
    if matches.len() != 1 {
        return Err(fail(
            "reconciliation_recovery_journal",
            "Recovery requires one matching approved transaction.",
        ));
    }
    let journal = &matches[0];
    let mut before = BTreeMap::new();
    for file in journal["files"].as_array().ok_or_else(|| {
        fail(
            "reconciliation_recovery_journal",
            "Journal files are missing.",
        )
    })? {
        if let Some(snapshot) = file.get("before").filter(|v| !v.is_null()) {
            let raw =
                work_operations::derivation::snapshot::decode_snapshot(snapshot).map_err(|_| {
                    fail(
                        "reconciliation_recovery_journal",
                        "The original snapshot is invalid.",
                    )
                })?;
            before.insert(
                file["path"]
                    .as_str()
                    .ok_or_else(|| {
                        fail(
                            "reconciliation_recovery_journal",
                            "A journal path is invalid.",
                        )
                    })?
                    .to_owned(),
                raw,
            );
        }
    }
    let reviewed_preview = || {
        let preview = preview_from_repository(
            &MigrationRecoverySources {
                root,
                before: &before,
                ledger_path: &ledger_path,
            },
            request,
            |migration| {
                crate::specification::migration::preview_migration_with_baseline(
                    root,
                    skill_root,
                    configs,
                    migration,
                    Some(&before),
                )
            },
        )?;
        require_approved_preview(&preview, request, approved_sha256, true)?;
        Ok::<_, WorkError>(preview)
    };
    let preview = reviewed_preview()?;
    let ledger_raw = render_ledger(&preview["ledger"])
        .map_err(|message| fail("reconciliation_ledger", message))?;
    let migration_approval = preview["migration_preview"]["fingerprint"]
        .as_str()
        .ok_or_else(|| {
            fail(
                "reconciliation_recovery_journal",
                "The migration approval is missing.",
            )
        })?;
    let publication = publish_migration_with_guard(
        root,
        skill_root,
        configs,
        &request["migration"],
        "recover",
        migration_approval,
        &BTreeMap::from([(ledger_path.clone(), ledger_raw.clone())]),
        || reviewed_preview().map(|_| ()),
    )?;
    if LocalFiles.read_raw(&storage_path(root, &ledger_path)?)? != ledger_raw {
        return Err(fail(
            "reconciliation_ledger_post_write",
            "Recovered ledger bytes differ from approval.",
        ));
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecReconciliationPublication,
    >(json!({
        "schema":"work-spec-reconciliation-publication", "status":"recovered", "reconciliation_fingerprint":approved_sha256,
        "attempt_path":attempt_path,"attempt_sha256":preview["attempt_sha256"],"selected_deviation_ids":preview["selected_deviation_ids"],
        "retained_deviation_ids":preview["retained_deviation_ids"],"ledger_path":ledger_path,"ledger_sha256":fingerprint::ledger(&ledger_raw),
        "publication":publication,"deviation_classifications":preview["deviation_classifications"]})))
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
            "schema": "work-spec-reconciliation-preview-request",
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
    fn closed_attempt_preview_and_ledger_publication_match_current_contract() {
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
        let correction = json!({
            "schema":"work-correction",
            "correction_id":"ATTEMPT-001-CORRECTION-001",
            "created_at":"2026-09-26T10:05+08:00",
            "target_attempt_id":"ATTEMPT-001",
            "task_collection_sha256":attempt["task_collection_sha256"],
            "task_index_sha256":attempt["task_index_sha256"],
            "task_item_sha256":attempt["task_item_sha256"],
            "task_instructions_sha256":attempt["task_instructions_sha256"],
            "execute_instructions_sha256":attempt["execute_instructions_sha256"],
            "field":"records[0].outcome",
            "correct_value":"passed",
            "reason":"Historical correction evidence."
        });
        work_operations::execution::correction::validate_correction(&correction).unwrap();
        let correction_raw =
            work_operations::execution::correction::render_correction(&correction).unwrap();
        let correction_path = root
            .join(request["attempt_path"].as_str().unwrap())
            .parent()
            .unwrap()
            .join("corrections/ATTEMPT-001-CORRECTION-001.json");
        fs::create_dir_all(correction_path.parent().unwrap()).unwrap();
        fs::write(&correction_path, &correction_raw).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let semantic: Value =
            serde_json::from_slice(&fs::read(fixture.join("semantic-request.json")).unwrap())
                .unwrap();
        let prepared = prepare_from_semantic(
            &root,
            &repo.join("../skills/work"),
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
        let preview =
            preview_from_project(&root, &repo.join("../skills/work"), &[], &request).unwrap();
        assert_eq!(preview, expected);
        let index_path = root.join("outputs/work/executions/example/index.json");
        let index_raw = fs::read(&index_path).unwrap();
        let mut changed_index: Value = serde_json::from_slice(&index_raw).unwrap();
        changed_index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-002");
        fs::write(
            &index_path,
            work_operations::execution::index::render_execution_index(&changed_index).unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepare_from_semantic(
                &root,
                &repo.join("../skills/work"),
                &[],
                &semantic,
                "2026-09-26",
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_attempt_position"
        );
        assert_eq!(
            preview_from_project(&root, &repo.join("../skills/work"), &[], &request)
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
                &repo.join("../skills/work"),
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
        let error =
            preview_from_project(&root, &repo.join("../skills/work"), &[], &invalid).unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Contract);
        assert_eq!(error.reason_code, "invalid_contract_value");
        assert_eq!(error.details["location"], "contract");
        let published = publish_ledger_only(
            &root,
            &repo.join("../skills/work"),
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
        assert_eq!(fs::read(&correction_path).unwrap(), correction_raw);
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
            preview_from_project(&root, &repo.join("../skills/work"), &[], &request)
                .unwrap_err()
                .reason_code,
            "reconciliation_nothing_pending"
        );
        let mut corrupted: Value =
            serde_json::from_slice(&fs::read(&ledger_path).unwrap()).unwrap();
        corrupted["entries"][0]["attempt_sha256"] = json!("invalid");
        fs::write(&ledger_path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        let error =
            preview_from_project(&root, &repo.join("../skills/work"), &[], &request).unwrap_err();
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
            &repo.join("../skills/work"),
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
                &repo.join("../skills/work"),
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
            &repo.join("../skills/work"),
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
                &repo.join("../skills/work"),
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
                &repo.join("../skills/work"),
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
            &repo.join("../skills/work"),
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
            &repo.join("../skills/work"),
            &[],
            &semantic,
            "2026-09-26",
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "reconciliation_attempt_missing");
        assert_eq!(error.exit_code, ExitCode::ArtifactIntegrity);
    }

    #[test]
    fn incorporated_deviation_preview_matches_current_contract_migration() {
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
        let attempt_before = fs::read(root.join(attempt)).unwrap();
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let actual =
            preview_from_project(&root, &repo.join("../skills/work"), &[], &request).unwrap();
        let semantic: Value =
            serde_json::from_slice(&fs::read(fixture.join("semantic-request.json")).unwrap())
                .unwrap();
        let prepared = prepare_from_semantic(
            &root,
            &repo.join("../skills/work"),
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
            &repo.join("../skills/work"),
            &[],
            &request,
            expected["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(fs::read(root.join(attempt)).unwrap(), attempt_before);
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
        let recovered = recover_with_migration(
            &root,
            &repo.join("../skills/work"),
            &[],
            &request,
            expected["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(recovered["status"], "recovered");
        assert_eq!(fs::read(root.join(ledger_path)).unwrap(), ledger_raw);
        assert_eq!(
            fs::read(root.join(journal_path)).unwrap(),
            fs::read(fixture.join(journal_path)).unwrap()
        );
    }
    #[test]
    fn migration_recovery_preserves_one_approved_set_at_each_write_boundary() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration");
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let publication: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        let journal_path = publication["publication"]["journal"].as_str().unwrap();
        let marker = publication["publication"]["completion_marker"]
            .as_str()
            .unwrap();
        let original: Value =
            serde_json::from_slice(&fs::read(fixture.join(journal_path)).unwrap()).unwrap();
        let files = original["files"].as_array().unwrap();
        let attempt_path = request["attempt_path"].as_str().unwrap();
        let attempt_raw = fs::read(fixture.join(attempt_path)).unwrap();
        let approval = preview["fingerprint"].as_str().unwrap();
        for count in 0..=files.len() {
            let root = std::env::temp_dir().join(format!(
                "work-reconcile-set-{count}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for source in request["migration"]["sources"].as_array().unwrap() {
                let path = source["path"].as_str().unwrap();
                fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
                fs::copy(fixture.join(path), root.join(path)).unwrap();
            }
            fs::create_dir_all(root.join(attempt_path).parent().unwrap()).unwrap();
            fs::write(root.join(attempt_path), &attempt_raw).unwrap();
            fs::write(root.join("src.txt"), b"source\n").unwrap();
            let mut journal = original.clone();
            journal["state"] = json!("prepared");
            journal["published_count"] = json!(0);
            write_journal(&root, journal_path, &journal).unwrap();
            for file in files.iter().take(count) {
                let path = file["path"].as_str().unwrap();
                let after =
                    work_operations::derivation::snapshot::decode_snapshot(&file["after"]).unwrap();
                fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
                fs::write(root.join(path), after).unwrap();
            }
            let journal_before = fs::read(root.join(journal_path)).unwrap();
            let mut drifted: Value = serde_json::from_slice(&attempt_raw).unwrap();
            drifted["ended_at"] = json!("2026-09-26T10:01+08:00");
            let drifted_raw =
                work_operations::execution::attempt::render_attempt(&drifted).unwrap();
            fs::write(root.join(attempt_path), &drifted_raw).unwrap();
            assert_eq!(
                recover_with_migration(
                    &root,
                    &repo.join("../skills/work"),
                    &[],
                    &request,
                    approval
                )
                .unwrap_err()
                .reason_code,
                "reconciliation_approval_changed"
            );
            assert_eq!(fs::read(root.join(journal_path)).unwrap(), journal_before);
            assert_eq!(fs::read(root.join(attempt_path)).unwrap(), drifted_raw);
            assert!(!root.join(marker).exists());
            fs::write(root.join(attempt_path), &attempt_raw).unwrap();
            let recovered = recover_with_migration(
                &root,
                &repo.join("../skills/work"),
                &[],
                &request,
                approval,
            )
            .unwrap();
            assert_eq!(recovered["publication"]["publication_status"], "published");
            for file in files {
                assert_eq!(
                    fs::read(root.join(file["path"].as_str().unwrap())).unwrap(),
                    work_operations::derivation::snapshot::decode_snapshot(&file["after"]).unwrap()
                );
            }
            assert_eq!(fs::read(root.join(attempt_path)).unwrap(), attempt_raw);
            assert_eq!(
                recover_with_migration(
                    &root,
                    &repo.join("../skills/work"),
                    &[],
                    &request,
                    approval
                )
                .unwrap()["publication"]["publication_status"],
                "already_published"
            );
        }
    }

    #[test]
    fn retain_only_recovers_before_or_after_ledger_write_and_rejects_attempt_drift() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow");
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let publication: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        let attempt_path = request["attempt_path"].as_str().unwrap();
        let index_path = "outputs/work/executions/example/index.json";
        let journal_path = publication["publication"]["journal"].as_str().unwrap();
        let marker_path = publication["publication"]["completion_marker"]
            .as_str()
            .unwrap();
        let ledger_path = publication["ledger_path"].as_str().unwrap();
        let approved = preview["fingerprint"].as_str().unwrap();
        let attempt_raw = fs::read(fixture.join(attempt_path)).unwrap();
        let index_raw = fs::read(fixture.join(index_path)).unwrap();
        let ledger_raw = fs::read(fixture.join(ledger_path)).unwrap();
        for written in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "work-retain-recovery-{written}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for (path, raw) in [(attempt_path, &attempt_raw), (index_path, &index_raw)] {
                fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
                fs::write(root.join(path), raw).unwrap();
            }
            let mut journal: Value =
                serde_json::from_slice(&fs::read(fixture.join(journal_path)).unwrap()).unwrap();
            journal["state"] = json!("prepared");
            journal["published_count"] = json!(0);
            write_journal(&root, journal_path, &journal).unwrap();
            if written {
                fs::write(root.join(ledger_path), &ledger_raw).unwrap();
            }
            let before = fs::read(root.join(journal_path)).unwrap();
            let mut changed: Value = serde_json::from_slice(&attempt_raw).unwrap();
            changed["ended_at"] = json!("2026-09-26T10:01+08:00");
            let changed_raw =
                work_operations::execution::attempt::render_attempt(&changed).unwrap();
            fs::write(root.join(attempt_path), &changed_raw).unwrap();
            assert_eq!(
                recover_ledger_only(&root, &repo.join("../skills/work"), &[], &request, approved)
                    .unwrap_err()
                    .reason_code,
                "reconciliation_approval_changed"
            );
            assert_eq!(fs::read(root.join(journal_path)).unwrap(), before);
            assert_eq!(fs::read(root.join(attempt_path)).unwrap(), changed_raw);
            assert!(!root.join(marker_path).exists());
            fs::write(root.join(attempt_path), &attempt_raw).unwrap();
            let recovered =
                recover_ledger_only(&root, &repo.join("../skills/work"), &[], &request, approved)
                    .unwrap();
            assert_eq!(recovered["status"], "recovered");
            assert_eq!(recovered["publication"]["publication_status"], "published");
            assert_eq!(fs::read(root.join(ledger_path)).unwrap(), ledger_raw);
            assert_eq!(fs::read(root.join(attempt_path)).unwrap(), attempt_raw);
            assert_eq!(fs::read(root.join(index_path)).unwrap(), index_raw);
            assert_eq!(
                recover_ledger_only(&root, &repo.join("../skills/work"), &[], &request, approved)
                    .unwrap()["publication"]["publication_status"],
                "already_published"
            );
        }
    }
}
