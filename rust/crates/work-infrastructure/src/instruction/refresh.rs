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
#[cfg(test)]
use work_operations::canonical::sha256_hex;

use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::validate_transaction;

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::instruction::refresh_storage::discover_requirements;
use crate::routing_sources::RoutingSourceSession;
use crate::specification::storage::storage_path;
use crate::specification::storage::{
    execution_history_fingerprints, publish_journal, require_no_spec_update, write_journal,
};
use crate::transaction_storage::completion_marker;
use crate::transaction_storage::replace_journal;
use crate::writer_lock::LocalWriterLock;
use work_feature::plan::default_artifact_paths;

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
    fn default_paths(&self, id: &RequirementId) -> Vec<(String, String)> {
        default_artifact_paths(id)
            .into_iter()
            .map(|(kind, path)| (kind.to_owned(), path))
            .collect()
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
        .unwrap_or_else(|| default_artifact_paths(&id)[2].1.clone());
    let short = approved_sha256
        .chars()
        .take(12)
        .collect::<String>()
        .to_uppercase();
    let journal_relative = format!("{execution_dir}/.work-source-refresh-{short}.json");
    let marker_relative = format!("{journal_relative}.done");
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
    let history_sha256 = execution_history_fingerprints(project_root, &execution_dir)?;
    let (journal, approval) = transaction(&candidate, &request, &short, &history_sha256);
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
            .unwrap_or_else(|| default_artifact_paths(&id)[2].1.clone());
        let short = approval[..12].to_ascii_uppercase();
        let journal = format!("{execution}/.work-source-refresh-{short}.json");
        let single_operation = if storage_path(project_root, &journal)?.is_file()
            && !storage_path(project_root, &format!("{journal}.done"))?.is_file()
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
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use work_operations::execution::index::render_execution_index;
    use work_operations::plan::render_plan_value;

    fn copied_fixture(label: &str) -> PathBuf {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let source = repo.join("crates/work-infrastructure/fixtures/instruction-migration");
        let root = std::env::temp_dir().join(format!(
            "work-refresh-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(source.join(relative), target).unwrap();
        }
        root
    }

    #[test]
    fn refresh_preview_matches_python_migration_fixture() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = repo.join("crates/work-infrastructure/fixtures/instruction-migration");
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let preview = build_refresh(&root, &skill, "example").unwrap().preview;
        assert_eq!(preview["status"], "refreshable");
        assert_eq!(
            preview["approved_sha256"],
            "597c7d77447feecbc8808cb8b0bba6075f5189838112c07a4b66d8205ee7c6ea"
        );
        assert_eq!(preview["files"].as_array().unwrap().len(), 4);
        assert_eq!(
            source_impact(&root, &skill).unwrap()["affected_requirements"],
            1
        );
    }

    #[test]
    fn refresh_apply_and_idempotent_reapply() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let source = repo.join("crates/work-infrastructure/fixtures/instruction-migration");
        let root = std::env::temp_dir().join(format!(
            "work-refresh-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(source.join(relative), target).unwrap();
        }
        let history =
            root.join("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, b"historical evidence\n").unwrap();
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let preview = build_refresh(&root, &skill, "example").unwrap().preview;
        assert_eq!(preview["status"], "refreshable");
        assert_eq!(
            preview["affected"],
            json!({"plans":1,"task_items":1,"task_indexes":1,"execution_indexes":1})
        );
        let approval = preview["approved_sha256"].as_str().unwrap().to_owned();
        assert_eq!(
            apply_source_refresh(&root, &skill, "example", &"0".repeat(64), "apply")
                .unwrap_err()
                .reason_code,
            "source_refresh_approval_changed"
        );
        let applied = apply_source_refresh(&root, &skill, "example", &approval, "apply").unwrap();
        assert_eq!(applied["status"], "updated");
        assert!(
            root.join(applied["completion_marker"].as_str().unwrap())
                .is_file()
        );
        assert_eq!(fs::read(&history).unwrap(), b"historical evidence\n");
        let plan: Value = serde_json::from_slice(
            &fs::read(root.join("outputs/work/plans/example.json")).unwrap(),
        )
        .unwrap();
        assert!(plan["work_instruction_selection"]["routing_manifest"].is_object());
        let execution: Value = serde_json::from_slice(
            &fs::read(root.join("outputs/work/executions/example/index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            execution["instruction_selection_manifest"]["router_compatibility_revision"],
            3
        );
        assert_eq!(
            build_refresh(&root, &skill, "example").unwrap().preview["status"],
            "valid"
        );
        assert_eq!(
            apply_source_refresh(&root, &skill, "example", &approval, "apply").unwrap()["status"],
            "already_completed"
        );
    }

    #[test]
    fn batch_record_matches_python_reference_bytes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let source = repo.join("crates/work-infrastructure/fixtures/instruction-migration");
        let root = std::env::temp_dir().join(format!(
            "work-refresh-batch-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(source.join(relative), target).unwrap();
        }
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let preview = preview_source_refresh_all(&root, &skill).unwrap();
        assert_eq!(
            preview["approved_sha256"],
            "04cddcb31db07d99269a0ae787fb4ad8e6fd8a92ba362cf2a2335ccf2425dbe8"
        );
        let approved = preview["approved_sha256"].as_str().unwrap();
        let record = apply_source_refresh_all(&root, &skill, approved, "apply").unwrap();
        assert_eq!(record["status"], "updated");
        assert_eq!(record["semantics"], "recoverable_sequential");
        assert_eq!(record["completed_requirement_ids"], json!(["example"]));
        assert_eq!(record["publications"][0]["requirement_id"], "example");
        assert_eq!(
            record["publications"][0]["transaction_approval_sha256"],
            "7a50ed1c72286ce882b1912a30de5af1be07b92a1f481f37c78448ae392d322d"
        );
        let raw = fs::read(root.join(record["record_path"].as_str().unwrap())).unwrap();
        assert_eq!(
            sha256_hex(&raw),
            "8439de2f49c640ff266318160ef85fd386fb3181c9cae5b922b4f92d8301189d"
        );
        assert_eq!(
            apply_source_refresh_all(&root, &skill, approved, "apply").unwrap()["status"],
            "already_completed"
        );
    }

    #[test]
    fn batch_recovery_resumes_first_incomplete_requirement() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = copied_fixture("batch-resume");
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let single = build_refresh(&root, &skill, "example").unwrap().preview;
        let single_approval = single["approved_sha256"].as_str().unwrap();
        let batch_approval = "f".repeat(64);
        let record_path =
            root.join("outputs/work/transactions/pending/source-refresh-batch/FFFFFFFFFFFF.json");
        fs::create_dir_all(record_path.parent().unwrap()).unwrap();
        let record = json!({"schema":"work-source-refresh-batch-publication/v1",
            "status":"in_progress","semantics":"recoverable_sequential",
            "approved_sha256":batch_approval,
            "record_path":"outputs/work/transactions/pending/source-refresh-batch/FFFFFFFFFFFF.json",
            "requirements":[{"requirement_id":"alpha","approved_sha256":"a".repeat(64)},
                {"requirement_id":"example","approved_sha256":single_approval}],
            "completed_requirement_ids":["alpha"],
            "publications":[{"requirement_id":"alpha","status":"updated"}]});
        LocalFiles
            .create_new(&record_path, &render_batch(&record).unwrap())
            .unwrap();
        let resumed = apply_source_refresh_all(&root, &skill, &batch_approval, "recover").unwrap();
        assert_eq!(resumed["semantics"], "recoverable_sequential");
        assert_eq!(
            resumed["completed_requirement_ids"],
            json!(["alpha", "example"])
        );
        assert_eq!(resumed["publications"].as_array().unwrap().len(), 2);
        assert_eq!(resumed["publications"][0], record["publications"][0]);
        assert_eq!(resumed["publications"][1]["requirement_id"], "example");
        assert_eq!(
            apply_source_refresh_all(&root, &skill, &batch_approval, "apply").unwrap()["status"],
            "already_completed"
        );
    }

    #[test]
    fn active_attempt_blocks_refresh_without_publication() {
        let root = copied_fixture("active");
        let skill = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let execution = root.join("outputs/work/executions/example/index.json");
        let mut index: Value = serde_json::from_slice(&fs::read(&execution).unwrap()).unwrap();
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["overall_status"] = json!("in_progress");
        fs::write(&execution, render_execution_index(&index).unwrap()).unwrap();
        let before = fs::read(&execution).unwrap();
        let preview = build_refresh(&root, skill, "example").unwrap().preview;
        assert_eq!(preview["status"], "review_required");
        assert_eq!(preview["files"], json!([]));
        assert_eq!(
            preview["blocked"],
            json!([{"path":"outputs/work/executions/example/index.json","reason":"active_attempt_snapshot"}])
        );
        assert_eq!(fs::read(&execution).unwrap(), before);
    }

    #[test]
    fn changed_output_after_preview_rejects_approval() {
        let root = copied_fixture("output-drift");
        let skill = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let preview = build_refresh(&root, skill, "example").unwrap().preview;
        let plan_path = root.join("outputs/work/plans/example.json");
        let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan["summary"] = json!(format!(
            "{} External edit.",
            plan["summary"].as_str().unwrap()
        ));
        fs::write(&plan_path, render_plan_value(&plan).unwrap()).unwrap();
        let edited = fs::read(&plan_path).unwrap();
        assert_eq!(
            apply_source_refresh(
                &root,
                skill,
                "example",
                preview["approved_sha256"].as_str().unwrap(),
                "apply"
            )
            .unwrap_err()
            .reason_code,
            "source_refresh_approval_changed"
        );
        assert_eq!(fs::read(&plan_path).unwrap(), edited);
    }

    #[test]
    fn custom_execution_path_is_used_for_refresh_journal() {
        let root = copied_fixture("custom-path");
        let skill = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let plan_path = root.join("outputs/work/plans/example.json");
        let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan["artifacts"]["execution"] = json!("outputs/work/custom/runtime");
        fs::write(&plan_path, render_plan_value(&plan).unwrap()).unwrap();
        let execution = root.join("outputs/work/custom/runtime/index.json");
        fs::create_dir_all(execution.parent().unwrap()).unwrap();
        fs::rename(
            root.join("outputs/work/executions/example/index.json"),
            &execution,
        )
        .unwrap();
        let preview = build_refresh(&root, skill, "example").unwrap().preview;
        let approval = preview["approved_sha256"].as_str().unwrap();
        let result = apply_source_refresh(&root, skill, "example", approval, "apply").unwrap();
        assert!(
            result["journal"]
                .as_str()
                .unwrap()
                .starts_with("outputs/work/custom/runtime/")
        );
        assert!(
            root.join(result["completion_marker"].as_str().unwrap())
                .is_file()
        );
        assert_eq!(
            apply_source_refresh(&root, skill, "example", approval, "apply").unwrap()["status"],
            "already_completed"
        );
    }

    #[test]
    fn plan_only_refresh_publishes_only_plan() {
        let root = copied_fixture("plan-only");
        let skill = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let original: Value = serde_json::from_slice(
            &fs::read(root.join("outputs/work/plans/example.json")).unwrap(),
        )
        .unwrap();
        let mut plan = original;
        plan["requirement_id"] = json!("planonly");
        plan["artifacts"] = json!({"plan":"outputs/work/plans/planonly.json","task":"outputs/work/tasks/planonly/index.json","execution":"outputs/work/executions/planonly"});
        let plan_path = root.join("outputs/work/plans/planonly.json");
        fs::write(&plan_path, render_plan_value(&plan).unwrap()).unwrap();
        let before = fs::read(&plan_path).unwrap();
        let preview = build_refresh(&root, skill, "planonly").unwrap().preview;
        assert_eq!(
            preview["affected"],
            json!({"plans":1,"task_items":0,"task_indexes":0,"execution_indexes":0})
        );
        let result = apply_source_refresh(
            &root,
            skill,
            "planonly",
            preview["approved_sha256"].as_str().unwrap(),
            "apply",
        )
        .unwrap();
        assert_eq!(
            result["updated_files"],
            json!(["outputs/work/plans/planonly.json"])
        );
        assert_ne!(fs::read(&plan_path).unwrap(), before);
        assert!(
            root.join(result["completion_marker"].as_str().unwrap())
                .is_file()
        );
    }
}
