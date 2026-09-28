//! Instruction router migration preview over formal artifacts.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::migration::require_writable_approval;
use work_feature::instruction::migration_publication::transaction;
use work_feature::ports::WriterLock;
use work_operations::canonical::parse_json_contract;
use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::validate_transaction;

use crate::routing_sources::RoutingSourceSession;
use crate::specification::storage::storage_path;
use crate::specification::storage::{
    execution_history_fingerprints, publish_journal, require_no_spec_update, write_journal,
};
use crate::transaction_storage::completion_marker;
use crate::writer_lock::LocalWriterLock;
use work_feature::plan::default_artifact_paths;

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
    let paths = default_artifact_paths(&id);
    let execution_dir = &paths[2].1;
    let short = approved_sha256
        .chars()
        .take(12)
        .collect::<String>()
        .to_uppercase();
    let journal_relative = format!("{execution_dir}/.work-instruction-migration-{short}.json");
    let marker_relative = format!("{journal_relative}.done");
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
    let history_sha256 = execution_history_fingerprints(project_root, execution_dir)?;
    let (journal, approval) = transaction(
        &candidate,
        requirement_id,
        approved_sha256,
        &short,
        &history_sha256,
    );
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
    use std::time::{SystemTime, UNIX_EPOCH};
    use work_operations::execution::index::render_execution_index;

    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let destination = target.join(entry.file_name());
            if path.is_dir() {
                copy_tree(&path, &destination);
            } else {
                fs::copy(path, destination).unwrap();
            }
        }
    }

    fn fixture() -> (std::path::PathBuf, std::path::PathBuf) {
        let source = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/instruction-migration"
        ));
        let root = std::env::temp_dir().join(format!(
            "work-migration-{}-{}",
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
        (source, root)
    }

    #[test]
    fn missing_plan_has_python_reason() {
        let root = std::env::temp_dir();
        let skill = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        assert_eq!(
            build_migration(&root, skill, "missing-plan-test")
                .err()
                .unwrap()
                .reason_code,
            "instruction_migration_plan_missing"
        );
    }

    #[test]
    fn preview_and_apply_match_python_migration_fixture() {
        let (source, root) = fixture();
        let history =
            root.join("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, b"history\n").unwrap();
        let skill = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let expected: Value =
            serde_json::from_slice(&fs::read(source.join("expected.json")).unwrap()).unwrap();
        let candidate = build_migration(&root, skill, "example").unwrap();
        assert_eq!(candidate.preview, expected);
        let approval = expected["approved_sha256"].as_str().unwrap();
        assert_eq!(
            apply_migration(&root, skill, "example", &"0".repeat(64))
                .unwrap_err()
                .reason_code,
            "instruction_migration_approval_changed"
        );
        let publication = apply_migration(&root, skill, "example", approval).unwrap();
        assert_eq!(publication["status"], "updated");
        assert_eq!(fs::read(&history).unwrap(), b"history\n");
        assert_eq!(
            build_migration(&root, skill, "example").unwrap().preview["status"],
            "current"
        );
        assert_eq!(
            apply_migration(&root, skill, "example", approval).unwrap()["status"],
            "already_completed"
        );
    }

    #[test]
    fn active_attempt_preview_requires_review_without_changing_artifacts() {
        let (_source, root) = fixture();
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
        let history =
            root.join("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, b"active snapshot\n").unwrap();
        let tracked = [
            root.join("outputs/work/plans/example.json"),
            root.join("outputs/work/tasks/example/index.json"),
            execution,
            history,
        ];
        let before = tracked
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let preview = build_migration(&root, skill, "example").unwrap().preview;
        assert_eq!(preview["status"], "review_required");
        assert_eq!(preview["files"], json!([]));
        for (path, original) in tracked.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn migration_and_refresh_reject_instruction_source_drift_after_preview() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        for (mode, changed_source, expected_reason) in [
            (
                "migration",
                "references/instruction-loading.md",
                "instruction_migration_approval_changed",
            ),
            (
                "refresh",
                "references/workflows/plan.md",
                "source_refresh_approval_changed",
            ),
        ] {
            let (_, root) = fixture();
            let skill = root.join(format!("{mode}-skill"));
            copy_tree(
                &repo.join("crates/work-infrastructure/legacy-work-skill/references"),
                &skill.join("references"),
            );
            let preview = if mode == "migration" {
                build_migration(&root, &skill, "example").unwrap().preview
            } else {
                crate::instruction::refresh::build_refresh(&root, &skill, "example")
                    .unwrap()
                    .preview
            };
            let approval = preview["approved_sha256"].as_str().unwrap();
            let source = skill.join(changed_source);
            let mut raw = fs::read(&source).unwrap();
            raw.extend_from_slice(b"\nDrift.\n");
            fs::write(&source, raw).unwrap();
            let error = if mode == "migration" {
                apply_migration(&root, &skill, "example", approval).unwrap_err()
            } else {
                crate::instruction::refresh::apply_source_refresh(
                    &root, &skill, "example", approval, "apply",
                )
                .unwrap_err()
            };
            assert_eq!(error.reason_code, expected_reason);
        }
    }
}
