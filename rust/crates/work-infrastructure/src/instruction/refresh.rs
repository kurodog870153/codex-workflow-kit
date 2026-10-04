//! Source-impact and refresh candidate construction.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::refresh::{
    RefreshImpactRepository, batch_preview, require_apply_operation, require_writable_approval,
    source_impact as build_source_impact,
};
use work_feature::instruction::refresh_batch::{
    existing_record, finish_record, new_record, require_new_operation,
};
use work_feature::instruction::refresh_publication::transaction;
use work_feature::ports::ArtifactStore;
use work_feature::ports::WriterLock;
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::publication::completion_marker;
use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::validate_transaction;

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::instruction::refresh_storage::discover_requirements;
use crate::routing_sources::RoutingSourceSession;
use crate::specification::storage::storage_path;
use crate::specification::storage::{
    execution_history_bytes, publish_journal, require_no_spec_update, write_journal,
};
use crate::transaction_storage::replace_journal;
use crate::writer_lock::LocalWriterLock;
use work_feature::artifact_paths::default_artifact_paths;

pub use work_feature::instruction::refresh_build::RefreshCandidate;
use work_feature::instruction::refresh_build::{
    RefreshRoutingRepository, RefreshSnapshotRepository, build_refresh as build_candidate,
};

struct LocalRefreshSnapshot<'a> {
    project_root: &'a Path,
}
impl RefreshSnapshotRepository for LocalRefreshSnapshot<'_> {
    fn discover_requirements(&self) -> Result<BTreeMap<String, Value>, WorkError> {
        discover_requirements(self.project_root)
    }
    fn default_paths(&self, id: &RequirementId) -> work_model::task::source::TaskArtifactPaths {
        default_artifact_paths(id)
    }
    fn source_evidence(&self, index: &Value) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
        let requirement = index["requirement_id"]
            .as_str()
            .ok_or_else(|| failure("invalid_requirement_id", "A Task requirement is required."))?;
        work_operations::task::source::validate_formal_context(index, requirement).map_err(
            |e| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    e.reason_code,
                    e.message,
                    e.details,
                )
            },
        )?;
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.to_path_buf(),
        };
        work_feature::task::source::verify_provenance(
            &paths,
            requirement,
            &serde_json::from_value(index["source"].clone()).expect("validated Task provenance"),
            &serde_json::from_value(index["artifacts"].clone())
                .expect("validated Task artifact paths"),
        )?;
        let mut evidence = BTreeMap::new();
        for relative in work_feature::task::source::evidence_paths(index)? {
            evidence.insert(relative.clone(), self.read(&relative)?);
        }
        Ok(evidence)
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(storage_path(self.project_root, relative)?.is_file())
    }
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        fs::read(storage_path(self.project_root, relative)?)
            .map_err(|_| failure("file_read_failed", "A formal artifact could not be read."))
    }
}
impl RefreshRoutingRepository for RoutingSourceSession {
    fn recheck_sources(&self) -> Result<(), WorkError> {
        RoutingSourceSession::recheck(self)
    }
}
fn failure(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}
pub fn build_refresh(
    project_root: &Path,
    skill_root: &Path,
    requirement_id: &str,
) -> Result<RefreshCandidate, WorkError> {
    let source = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let mut routing = RoutingSourceSession::new(skill_root.to_path_buf());
    build_candidate(
        &LocalRefreshSnapshot { project_root },
        &source,
        &mut routing,
        requirement_id,
    )
}

pub fn source_impact(project_root: &Path, skill_root: &Path) -> Result<Value, WorkError> {
    build_source_impact(&LocalRefreshImpact {
        project_root,
        skill_root,
    })
}

struct LocalRefreshImpact<'a> {
    project_root: &'a Path,
    skill_root: &'a Path,
}

impl RefreshImpactRepository for LocalRefreshImpact<'_> {
    fn requirement_ids(&self) -> Result<Vec<String>, WorkError> {
        Ok(discover_requirements(self.project_root)?
            .into_keys()
            .collect())
    }

    fn build_candidate(&self, requirement_id: &str) -> Result<RefreshCandidate, WorkError> {
        build_refresh(self.project_root, self.skill_root, requirement_id)
    }
}

fn publication(
    status: &str,
    requirement_id: &str,
    approved_sha256: &str,
    approval: &Value,
    journal_relative: &str,
    marker_relative: &str,
    updated: &[Value],
) -> Value {
    let result = json!({"schema":"work-source-refresh-publication/v1","status":status,
        "requirement_id":requirement_id,"approved_sha256":approved_sha256,
        "transaction_approval_sha256":approval,"journal":journal_relative,
        "completion_marker":marker_relative,"updated_files":updated});
    let _: work_model::source::SourceRefreshPublication = serde_json::from_value(result.clone())
        .expect("source refresh publication matches its model");
    result
}

pub fn apply_source_refresh(
    project_root: &Path,
    skill_root: &Path,
    requirement_id: &str,
    approved_sha256: &str,
    operation: &str,
) -> Result<Value, WorkError> {
    let id = RequirementId::from_str(requirement_id)
        .map_err(|_| failure("invalid_requirement_id", "The requirement ID is invalid."))?;
    let discovered = discover_requirements(project_root)?;
    let execution_dir = discovered
        .get(requirement_id)
        .and_then(|paths| paths["execution"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| default_artifact_paths(&id).execution);
    let journal_relative = work_operations::derivation::publication::journal_path(
        &execution_dir,
        work_operations::derivation::publication::JournalKind::SourceRefresh(approved_sha256),
    );
    let marker_relative =
        work_operations::derivation::publication::completion_marker_path(&journal_relative);
    let journal_path = storage_path(project_root, &journal_relative)?;
    let marker_path = storage_path(project_root, &marker_relative)?;
    let request = json!({"kind":"source_refresh","requirement_id":requirement_id,
        "preview_fingerprint":approved_sha256});
    if marker_path.is_file() || operation == "recover" {
        let raw = fs::read(&journal_path).map_err(|_| {
            WorkError::new(
                ExitCode::InternalError,
                "internal_error",
                "An unexpected internal error occurred.",
                json!({}),
            )
        })?;
        let journal = parse_json_contract(&raw).map_err(|_| {
            failure(
                "invalid_contract_value",
                "The source refresh journal is invalid.",
            )
        })?;
        validate_transaction(&journal).map_err(|_| {
            failure(
                "invalid_contract_value",
                "The source refresh journal is invalid.",
            )
        })?;
        if marker_path.is_file() {
            let marker = fs::read(&marker_path).map_err(|_| {
                failure(
                    "source_refresh_marker_conflict",
                    "The refresh completion marker cannot be read.",
                )
            })?;
            if marker != completion_marker(&raw) {
                return Err(failure(
                    "source_refresh_marker_conflict",
                    "The refresh completion marker does not match its journal.",
                ));
            }
        }
        if journal["metadata"]["request"] != request {
            return Err(failure(
                if marker_path.is_file() {
                    "source_refresh_completed_request_changed"
                } else {
                    "source_refresh_recovery_request_changed"
                },
                "The refresh belongs to a different request.",
            ));
        }
        for (path, expected) in journal["metadata"]["candidate_sha256"].as_object().unwrap() {
            if journal["metadata"]["source_sha256"][path] == *expected
                && work_operations::derivation::fingerprint::raw(
                    &LocalFiles.read_raw(&storage_path(project_root, path)?)?,
                ) != *expected
            {
                return Err(failure(
                    "source_refresh_source_changed",
                    "The immutable Source proof changed after approval.",
                ));
            }
        }
        let updated = journal["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["path"].clone())
            .collect::<Vec<_>>();
        if marker_path.is_file() {
            return Ok(publication(
                "already_completed",
                requirement_id,
                approved_sha256,
                &journal["approval_sha256"],
                &journal_relative,
                &marker_relative,
                &updated,
            ));
        }
        require_no_spec_update(project_root, &execution_dir, Some(&journal_relative))?;
        let lock = storage_path(
            project_root,
            &format!("{execution_dir}/.work-state-writer.lock"),
        )?;
        LocalWriterLock.require_idle(&lock)?;
        let _guard = LocalWriterLock.acquire(&lock)?;
        let published = publish_journal(project_root, &journal_relative, &marker_relative)?;
        let status = if published["status"] == "already_published" {
            "already_completed"
        } else {
            "updated"
        };
        return Ok(publication(
            status,
            requirement_id,
            approved_sha256,
            &journal["approval_sha256"],
            &journal_relative,
            &marker_relative,
            &updated,
        ));
    }
    require_apply_operation(operation)?;
    let candidate = build_refresh(project_root, skill_root, requirement_id)?;
    require_writable_approval(&candidate, approved_sha256)?;
    let history = execution_history_bytes(project_root, &execution_dir)?;
    let (journal, approval) = transaction(&candidate, &request, &history)?;
    fs::create_dir_all(storage_path(project_root, &execution_dir)?).map_err(|_| {
        failure(
            "file_write_failed",
            "The source refresh directory cannot be created.",
        )
    })?;
    require_no_spec_update(project_root, &execution_dir, None)?;
    let lock = storage_path(
        project_root,
        &format!("{execution_dir}/.work-state-writer.lock"),
    )?;
    LocalWriterLock.require_idle(&lock)?;
    let _guard = LocalWriterLock.acquire(&lock)?;
    write_journal(project_root, &journal_relative, &journal)?;
    let published = publish_journal(project_root, &journal_relative, &marker_relative)?;
    let status = if published["status"] == "already_published" {
        "already_completed"
    } else {
        "updated"
    };
    let updated = candidate
        .after
        .keys()
        .map(|path| json!(path))
        .collect::<Vec<_>>();
    Ok(publication(
        status,
        requirement_id,
        approved_sha256,
        &json!(approval),
        &journal_relative,
        &marker_relative,
        &updated,
    ))
}

pub fn preview_source_refresh_all(
    project_root: &Path,
    skill_root: &Path,
) -> Result<Value, WorkError> {
    let impact = source_impact(project_root, skill_root)?;
    batch_preview(&impact)
}

struct OrderedBatch<'a> {
    value: &'a Value,
    path: Vec<String>,
}

impl Serialize for OrderedBatch<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(object) = self.value.as_object() {
            let order: &[&str] = if self.path.is_empty() {
                &[
                    "schema",
                    "status",
                    "semantics",
                    "approved_sha256",
                    "record_path",
                    "requirements",
                    "completed_requirement_ids",
                    "publications",
                ]
            } else if self.path.last().is_some_and(|part| part == "requirements") {
                &["requirement_id", "approved_sha256"]
            } else if self.path.last().is_some_and(|part| part == "publications") {
                &[
                    "schema",
                    "status",
                    "requirement_id",
                    "approved_sha256",
                    "transaction_approval_sha256",
                    "journal",
                    "completion_marker",
                    "updated_files",
                ]
            } else {
                &[]
            };
            let mut output = serializer.serialize_map(Some(object.len()))?;
            for key in order.iter().copied().chain(
                object
                    .keys()
                    .filter(|key| !order.contains(&key.as_str()))
                    .map(String::as_str),
            ) {
                if let Some(value) = object.get(key) {
                    let mut path = self.path.clone();
                    path.push(key.into());
                    output.serialize_entry(key, &OrderedBatch { value, path })?;
                }
            }
            output.end()
        } else if let Some(array) = self.value.as_array() {
            let mut output = serializer.serialize_seq(Some(array.len()))?;
            for value in array {
                output.serialize_element(&OrderedBatch {
                    value,
                    path: self.path.clone(),
                })?;
            }
            output.end()
        } else {
            self.value.serialize(serializer)
        }
    }
}

fn render_batch(value: &Value) -> Result<Vec<u8>, WorkError> {
    let mut raw = serde_json::to_vec_pretty(&OrderedBatch {
        value,
        path: Vec::new(),
    })
    .map_err(|_| {
        failure(
            "invalid_contract_value",
            "The batch record cannot be rendered.",
        )
    })?;
    raw.push(b'\n');
    Ok(raw)
}

pub fn apply_source_refresh_all(
    project_root: &Path,
    skill_root: &Path,
    approved_sha256: &str,
    operation: &str,
) -> Result<Value, WorkError> {
    let short = approved_sha256
        .chars()
        .take(12)
        .collect::<String>()
        .to_uppercase();
    let relative = format!("outputs/work/transactions/pending/source-refresh-batch/{short}.json");
    let path = storage_path(project_root, &relative)?;
    let mut record = if path.is_file() {
        let raw = fs::read(&path).map_err(|_| {
            failure(
                "file_read_failed",
                "The batch progress record cannot be read.",
            )
        })?;
        let record = parse_json_contract(&raw).map_err(|_| {
            failure(
                "invalid_contract_value",
                "The batch progress record is invalid.",
            )
        })?;
        let record = existing_record(record, approved_sha256)?;
        if record["status"] != "in_progress" {
            return Ok(record);
        }
        record
    } else {
        require_new_operation(operation)?;
        let preview = preview_source_refresh_all(project_root, skill_root)?;
        let record = new_record(&preview, approved_sha256, &relative)?;
        fs::create_dir_all(path.parent().unwrap()).map_err(|_| {
            failure(
                "file_write_failed",
                "The batch progress directory cannot be created.",
            )
        })?;
        LocalFiles.create_new(&path, &render_batch(&record)?)?;
        record
    };
    let requirements = record["requirements"].as_array().unwrap().clone();
    let completed = record["publications"].as_array().unwrap().len();
    for requirement in requirements.iter().skip(completed) {
        let requirement_id = requirement["requirement_id"].as_str().unwrap();
        let approval = requirement["approved_sha256"].as_str().unwrap();
        let discovered = discover_requirements(project_root)?;
        let id = RequirementId::from_str(requirement_id)
            .map_err(|_| failure("invalid_requirement_id", "The requirement ID is invalid."))?;
        let execution = discovered
            .get(requirement_id)
            .and_then(|paths| paths["execution"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| default_artifact_paths(&id).execution);
        let journal = work_operations::derivation::publication::journal_path(
            &execution,
            work_operations::derivation::publication::JournalKind::SourceRefresh(approval),
        );
        let single_operation = if storage_path(project_root, &journal)?.is_file()
            && !storage_path(
                project_root,
                &work_operations::derivation::publication::completion_marker_path(&journal),
            )?
            .is_file()
        {
            "recover"
        } else {
            "apply"
        };
        let publication = apply_source_refresh(
            project_root,
            skill_root,
            requirement_id,
            approval,
            single_operation,
        )?;
        record["publications"]
            .as_array_mut()
            .unwrap()
            .push(publication);
        record["completed_requirement_ids"]
            .as_array_mut()
            .unwrap()
            .push(json!(requirement_id));
        replace_journal(&path, &render_batch(&record)?)?;
    }
    finish_record(&mut record);
    replace_journal(&path, &render_batch(&record)?)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_operations::derivation::snapshot::decode_snapshot;
    use work_operations::task::ordering::{TaskDocumentKind, render_task};

    fn fixture_root(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-update/item-goal");
        let root = std::env::temp_dir().join(format!(
            "work-task-refresh-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        (root, repo.join("../skills/work"))
    }

    #[test]
    fn refresh_task_and_execution_preserves_source_and_recovers_partial_publication() {
        let (root, skill) = fixture_root("recover");
        let sources = ["manifest.json", "manifest.json.done", "source.txt"].map(|name| {
            let path = root.join("outputs/work/sources/example/SRC-001").join(name);
            let raw = fs::read(&path).unwrap();
            (path, raw)
        });
        let candidate = build_refresh(&root, &skill, "example").unwrap();
        assert_eq!(candidate.preview["status"], "refreshable");
        assert!(candidate.preview["affected"].get("plans").is_none());
        assert_eq!(candidate.source_evidence.len(), 3);
        let approval = candidate.preview["approved_sha256"].as_str().unwrap();
        assert_eq!(
            build_refresh(&root, &skill, "example").unwrap().preview["approved_sha256"],
            approval
        );
        assert_eq!(
            apply_source_refresh(&root, &skill, "example", &"0".repeat(64), "apply")
                .unwrap_err()
                .reason_code,
            "source_refresh_approval_changed"
        );
        let request = json!({"kind":"source_refresh", "requirement_id":"example", "preview_fingerprint":approval});
        let (mut journal, _) = transaction(&candidate, &request, &BTreeMap::new()).unwrap();
        assert!(
            journal["files"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| !row["path"].as_str().unwrap().contains("/sources/"))
        );
        let first = &journal["files"][0];
        fs::write(
            root.join(first["path"].as_str().unwrap()),
            decode_snapshot(&first["after"]).unwrap(),
        )
        .unwrap();
        journal["state"] = json!("publishing");
        journal["published_count"] = json!(1);
        let path = work_operations::derivation::publication::journal_path(
            "outputs/work/executions/example",
            work_operations::derivation::publication::JournalKind::SourceRefresh(approval),
        );
        write_journal(&root, &path, &journal).unwrap();
        let result = apply_source_refresh(&root, &skill, "example", approval, "recover").unwrap();
        assert_eq!(result["status"], "updated");
        assert_eq!(
            apply_source_refresh(&root, &skill, "example", approval, "apply").unwrap()["status"],
            "already_completed"
        );
        for (path, raw) in &sources {
            assert_eq!(fs::read(path).unwrap(), *raw);
        }
        assert!(!root.join("outputs/work/plans").exists());
        let index_raw = fs::read(root.join("outputs/work/tasks/example/index.json")).unwrap();
        let index: Value = serde_json::from_slice(&index_raw).unwrap();
        assert_eq!(index["source"]["kind"], "snapshot");
        assert_eq!(
            render_task(&index, TaskDocumentKind::Index).unwrap(),
            index_raw
        );
        fs::write(&sources[2].0, b"drift").unwrap();
        assert!(build_refresh(&root, &skill, "example").is_err());
        assert_eq!(
            apply_source_refresh(&root, &skill, "example", approval, "recover")
                .unwrap_err()
                .reason_code,
            "source_refresh_source_changed"
        );
    }

    #[test]
    fn batch_refresh_discovers_task_without_plan_and_keeps_source_immutable() {
        let (root, skill) = fixture_root("batch");
        let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
        let raw = fs::read(&source).unwrap();
        let impact = source_impact(&root, &skill).unwrap();
        assert_eq!(impact["affected_requirements"], 1);
        let preview = preview_source_refresh_all(&root, &skill).unwrap();
        assert_eq!(preview["status"], "refreshable");
        let approval = preview["approved_sha256"].as_str().unwrap();
        let published = apply_source_refresh_all(&root, &skill, approval, "apply").unwrap();
        assert_eq!(published["completed_requirement_ids"], json!(["example"]));
        assert_eq!(
            apply_source_refresh_all(&root, &skill, approval, "recover").unwrap()["status"],
            "already_completed"
        );
        assert_eq!(fs::read(source).unwrap(), raw);
        assert!(!root.join("outputs/work/plans").exists());
    }
}
