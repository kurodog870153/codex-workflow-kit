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
use work_operations::derivation::publication::completion_marker;
use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::validate_transaction;

use crate::routing_sources::RoutingSourceSession;
use crate::specification::storage::storage_path;
use crate::specification::storage::{
    execution_history_bytes, publish_journal, require_no_spec_update, write_journal,
};
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

    #[test]
    fn missing_plan_has_python_reason() {
        let root = std::env::temp_dir();
        let skill = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        assert_eq!(
            build_migration(&root, skill, "missing-plan-test")
                .err()
                .unwrap()
                .reason_code,
            "instruction_migration_plan_missing"
        );
    }
}
