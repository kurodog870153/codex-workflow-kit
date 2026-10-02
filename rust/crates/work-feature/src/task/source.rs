//! Resolve immutable Source bytes and independently confirmed Task choices.
use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::ports::{SnapshotBytes, SourceSnapshotReader};
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use serde_json::{Value, json};
use work_model::task::draft::PlanningSource;
pub fn validate_context(
    snapshots: &impl SourceSnapshotReader,
    instructions: &impl InstructionSourceRepository,
    skills: &impl SkillSnapshotRepository,
    paths: &impl ArtifactPathRepository,
    roots: &[SkillRoot],
    requirement: &str,
    value: &Value,
) -> Result<(PlanningSource, SnapshotBytes), WorkError> {
    let context = work_operations::task::source::validate_planning_source(value, requirement)
        .map_err(|e| WorkError::new(ExitCode::Contract, e.reason_code, e.message, e.details))?;
    let id = requirement.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_requirement_id",
            "A portable requirement ID is required.",
            json!({}),
        )
    })?;
    paths.validate_paths(&id, &context.artifacts)?;
    let snapshot =
        snapshots.read_snapshot_at(&id, &context.snapshot.source_id, &context.artifacts.source)?;
    work_operations::source_snapshot::validate(&snapshot.manifest, Some(&snapshot.bytes)).map_err(
        |e| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                e.reason_code,
                e.message,
                json!({}),
            )
        },
    )?;
    if snapshot.manifest != context.snapshot {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_snapshot_mismatch",
            "Task provenance must exactly match the immutable Snapshot.",
            json!({}),
        ));
    }
    crate::hierarchy::validate_selection(instructions, &value["hierarchy_selection"])?;
    crate::skill::validate_selection(skills, roots, &value["skill_selection"])?;
    Ok((context, snapshot))
}

pub fn verify_provenance(
    repository: &(impl ArtifactPathRepository + SourceSnapshotReader),
    requirement: &str,
    source: &work_model::task::source::TaskProvenance,
    artifacts: &work_model::task::source::TaskArtifactPaths,
) -> Result<String, WorkError> {
    let id = requirement.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_requirement_id",
            "A portable requirement ID is required.",
            json!({}),
        )
    })?;
    repository.validate_paths(&id, artifacts)?;
    match source {
        work_model::task::source::TaskProvenance::Snapshot { manifest } => {
            let saved = repository.read_snapshot_at(&id, &manifest.source_id, &artifacts.source)?;
            work_operations::source_snapshot::validate(&saved.manifest, Some(&saved.bytes))
                .map_err(|e| {
                    WorkError::new(
                        ExitCode::ArtifactIntegrity,
                        e.reason_code,
                        e.message,
                        json!({}),
                    )
                })?;
            if saved.manifest != *manifest || manifest.requirement_id != requirement {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "source_snapshot_mismatch",
                    "Task must bind the exact stored same-requirement Snapshot.",
                    json!({}),
                ));
            }
        }
        work_model::task::source::TaskProvenance::Migration {
            sources,
            approval_sha256,
        } => {
            if sources.is_empty()
                || *approval_sha256
                    != work_operations::derivation::fingerprint::migration_source_approval(
                        requirement,
                        sources,
                    )
            {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "migration_source_approval_mismatch",
                    "Migration must retain its complete approved evidence set.",
                    json!({}),
                ));
            }
            let mut paths = std::collections::HashSet::new();
            for source in sources {
                if !work_operations::task::source::valid_relative_path(&source.path)
                    || !paths.insert(work_operations::canonical::portable_path_identity(
                        &source.path,
                    ))
                    || source.raw.len() as u64 != source.size
                    || !work_operations::derivation::fingerprint::verify_raw(
                        &source.raw,
                        &source.raw_sha256,
                    )
                {
                    return Err(WorkError::new(
                        ExitCode::ArtifactIntegrity,
                        "migration_source_drift",
                        "Original migration bytes must remain intact.",
                        json!({"path":source.path}),
                    ));
                }
            }
        }
    }
    Ok(work_operations::derivation::fingerprint::task_provenance(
        source,
    ))
}
pub fn evidence_paths(collection: &Value) -> Result<Vec<String>, WorkError> {
    let source: work_model::task::source::TaskProvenance =
        serde_json::from_value(collection["source"].clone()).map_err(|_| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_task_source",
                "Task provenance is invalid.",
                json!({}),
            )
        })?;
    match source {
        work_model::task::source::TaskProvenance::Snapshot { manifest } => {
            let root = collection["artifacts"]["source"].as_str().ok_or_else(|| {
                WorkError::new(
                    ExitCode::Contract,
                    "invalid_artifact_path",
                    "A Source root is required.",
                    json!({}),
                )
            })?;
            let directory = format!("{root}/{}", manifest.source_id.as_str());
            Ok(vec![
                format!("{directory}/manifest.json"),
                format!("{directory}/manifest.json.done"),
                format!("{directory}/{}", manifest.content.path.as_str()),
            ])
        }
        work_model::task::source::TaskProvenance::Migration { .. } => Ok(vec![]),
    }
}
