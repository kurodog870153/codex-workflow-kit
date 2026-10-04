//! Instruction router migration preview over formal artifacts.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::migration::require_writable_approval;
use work_feature::instruction::migration_publication::transaction;
use work_feature::ports::WriterLock;
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::publication::completion_marker;
use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::validate_transaction;

use crate::routing_sources::RoutingSourceSession;
use crate::specification::storage::storage_path;
use crate::specification::storage::{
    execution_history_bytes, publish_journal, require_no_spec_update, write_journal,
};
use crate::writer_lock::LocalWriterLock;
use work_feature::artifact_paths::default_artifact_paths;

pub use work_feature::instruction::migration_build::MigrationCandidate;
use work_feature::instruction::migration_build::{
    MigrationSnapshotRepository, build_migration as build_candidate,
};

fn failure(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

struct LocalMigrationSnapshot<'a> {
    project_root: &'a Path,
}
impl MigrationSnapshotRepository for LocalMigrationSnapshot<'_> {
    fn artifact_paths(
        &self,
        id: &RequirementId,
    ) -> Result<work_model::task::source::TaskArtifactPaths, WorkError> {
        let discovered =
            crate::instruction::refresh_storage::discover_requirements(self.project_root)?;
        discovered
            .get(id.as_str())
            .map(|paths| {
                serde_json::from_value(paths.clone()).map_err(|_| {
                    failure("invalid_artifact_paths", "Task artifact paths are invalid.")
                })
            })
            .unwrap_or_else(|| Ok(default_artifact_paths(id)))
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
pub fn build_migration(
    project_root: &Path,
    skill_root: &Path,
    requirement_id: &str,
) -> Result<MigrationCandidate, WorkError> {
    let mut routing = RoutingSourceSession::new(skill_root.to_path_buf());
    let candidate = build_candidate(
        &LocalMigrationSnapshot { project_root },
        &mut routing,
        requirement_id,
    )?;
    routing.recheck()?;
    Ok(candidate)
}

pub fn apply_migration(
    project_root: &Path,
    skill_root: &Path,
    requirement_id: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let id = RequirementId::from_str(requirement_id)
        .map_err(|_| failure("invalid_requirement_id", "The requirement ID is invalid."))?;
    let paths = LocalMigrationSnapshot { project_root }.artifact_paths(&id)?;
    let execution_dir = &paths.execution;
    let journal_relative = work_operations::derivation::publication::journal_path(
        execution_dir,
        work_operations::derivation::publication::JournalKind::InstructionMigration(
            approved_sha256,
        ),
    );
    let marker_relative =
        work_operations::derivation::publication::completion_marker_path(&journal_relative);
    let journal_path = storage_path(project_root, &journal_relative)?;
    let marker_path = storage_path(project_root, &marker_relative)?;
    if marker_path.is_file() {
        let raw = fs::read(&journal_path).map_err(|_| {
            failure(
                "instruction_migration_marker_conflict",
                "The migration journal cannot be read.",
            )
        })?;
        let journal = parse_json_contract(&raw).map_err(|_| {
            failure(
                "instruction_migration_marker_conflict",
                "The migration journal is invalid.",
            )
        })?;
        validate_transaction(&journal).map_err(|_| {
            failure(
                "instruction_migration_marker_conflict",
                "The migration journal is invalid.",
            )
        })?;
        let marker = fs::read(&marker_path).map_err(|_| {
            failure(
                "instruction_migration_marker_conflict",
                "The migration completion marker cannot be read.",
            )
        })?;
        if marker != completion_marker(&raw) {
            return Err(failure(
                "instruction_migration_marker_conflict",
                "The migration completion marker does not match its journal.",
            ));
        }
        for (path, expected) in journal["metadata"]["candidate_sha256"].as_object().unwrap() {
            if journal["metadata"]["source_sha256"][path] == *expected
                && work_operations::derivation::fingerprint::raw(
                    &fs::read(storage_path(project_root, path)?).map_err(|_| {
                        failure("file_read_failed", "Source evidence cannot be read.")
                    })?,
                ) != *expected
            {
                return Err(failure(
                    "instruction_migration_source_changed",
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
        return Ok(work_model::instruction::verified::<
            work_model::instruction::InstructionMigrationPublication,
        >(
            json!({"schema":"work-instruction-migration-publication/v1",
            "status":"already_completed","requirement_id":requirement_id,
            "approved_sha256":approved_sha256,"transaction_approval_sha256":journal["approval_sha256"],
            "journal":journal_relative,"completion_marker":marker_relative,"updated_files":updated}),
        ));
    }
    let candidate = build_migration(project_root, skill_root, requirement_id)?;
    require_writable_approval(&candidate, approved_sha256)?;
    let history = execution_history_bytes(project_root, execution_dir)?;
    let (journal, approval) = transaction(&candidate, requirement_id, approved_sha256, &history)?;
    require_no_spec_update(project_root, execution_dir, None)?;
    let lock = storage_path(
        project_root,
        &format!("{execution_dir}/.work-state-writer.lock"),
    )?;
    LocalWriterLock.require_idle(&lock)?;
    let _guard = LocalWriterLock.acquire(&lock)?;
    write_journal(project_root, &journal_relative, &journal)?;
    let published = publish_journal(project_root, &journal_relative, &marker_relative)?;
    let updated = candidate.after.keys().cloned().collect::<Vec<_>>();
    Ok(work_model::instruction::verified::<
        work_model::instruction::InstructionMigrationPublication,
    >(
        json!({"schema":"work-instruction-migration-publication/v1",
        "status":if published["status"] == "already_published" {"already_completed"} else {"updated"},
        "requirement_id":requirement_id,"approved_sha256":approved_sha256,
        "transaction_approval_sha256":approval,"journal":journal_relative,
        "completion_marker":marker_relative,"updated_files":updated}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-update/item-goal");
        let root = std::env::temp_dir().join(format!(
            "work-task-migration-{label}-{}-{}",
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
    fn task_only_migration_binds_immutable_source_and_custom_paths() {
        let (root, skill) = fixture_root("custom-paths");
        let old = "outputs/work/tasks/example/index.json";
        let index_raw = fs::read(root.join(old)).unwrap();
        let mut index: Value = serde_json::from_slice(&index_raw).unwrap();
        let task = "outputs/work/custom/tasks/example/index.json";
        let execution = "outputs/work/custom/executions/example";
        index["artifacts"]["task"] = json!(task);
        index["artifacts"]["execution"] = json!(execution);
        fs::create_dir_all(root.join("outputs/work/custom/tasks/example/tasks")).unwrap();
        fs::rename(
            root.join("outputs/work/tasks/example/tasks/TASK-001.json"),
            root.join("outputs/work/custom/tasks/example/tasks/TASK-001.json"),
        )
        .unwrap();
        fs::rename(root.join(old), root.join(task)).unwrap();
        let rendered = work_operations::task::ordering::render_task(
            &index,
            work_operations::task::ordering::TaskDocumentKind::Index,
        )
        .unwrap();
        fs::write(root.join(task), rendered).unwrap();
        fs::create_dir_all(root.join(execution)).unwrap();
        // Migration rebinds Execute to the current Task collection before publication.
        fs::rename(
            root.join("outputs/work/executions/example/index.json"),
            root.join(format!("{execution}/index.json")),
        )
        .unwrap();
        let candidate = build_migration(&root, &skill, "example").unwrap();
        assert_eq!(candidate.artifacts["task"], task);
        assert_eq!(candidate.artifacts["execution"], execution);
        assert_eq!(candidate.preview["status"], "migration_required");
        assert!(!candidate.source_evidence.is_empty());
        assert!(
            !candidate.preview["affected"]
                .as_object()
                .unwrap()
                .contains_key("plans")
        );
        for path in candidate.source_evidence.keys() {
            assert!(!candidate.after.contains_key(path));
        }
        let approval = candidate.preview["approved_sha256"].as_str().unwrap();
        assert_eq!(
            apply_migration(&root, &skill, "example", &"0".repeat(64))
                .unwrap_err()
                .reason_code,
            "instruction_migration_approval_changed"
        );
        let published = apply_migration(&root, &skill, "example", approval).unwrap();
        assert_eq!(published["status"], "updated");
        assert!(
            published["journal"]
                .as_str()
                .unwrap()
                .starts_with(execution)
        );
        for (path, raw) in &candidate.source_evidence {
            assert_eq!(fs::read(root.join(path)).unwrap(), *raw);
        }
        assert_eq!(
            apply_migration(&root, &skill, "example", approval).unwrap()["status"],
            "already_completed"
        );
        assert_eq!(
            build_migration(&root, &skill, "example").unwrap().preview["status"],
            "current"
        );
        let source = candidate
            .source_evidence
            .keys()
            .find(|p| p.ends_with("source.txt"))
            .unwrap();
        fs::write(root.join(source), b"changed").unwrap();
        assert_eq!(
            apply_migration(&root, &skill, "example", approval)
                .unwrap_err()
                .reason_code,
            "instruction_migration_source_changed"
        );
        assert!(build_migration(&root, &skill, "example").is_err());
    }

    #[test]
    fn missing_task_has_integrity_reason() {
        let root = std::env::temp_dir();
        let skill = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        assert_eq!(
            build_migration(&root, skill, "missing-task-test")
                .err()
                .unwrap()
                .reason_code,
            "instruction_migration_task_missing"
        );
    }
}
