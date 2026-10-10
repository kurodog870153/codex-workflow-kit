//! Project-relative execution index repository.

use std::fs;
use std::path::PathBuf;

#[cfg(test)]
use crate::writer_lock::WriterLock;
use serde_json::{Value, json};
use work_feature::artifact_paths::ArtifactPathRepository;
use work_feature::error::{ExitCode, WorkError};
use work_feature::execution::command_publication::{
    approved_receipt, build_command_request, command_completion,
};
use work_feature::execution::recovery::{
    attempt_start_stage, command_correction_identity, require_recovery_artifact_paths,
    require_recovery_file_set, require_started_attempt, validate_attempt_start_inventory,
};
use work_feature::execution::{
    AttemptClosePublication, AttemptCloseRepository, AttemptStartPublication,
    AttemptStartRepository, CommandCorrectionPublication, CommandCorrectionRepository,
    CommandPrepareRepository, CommandProjectRequest, CommandProjectSources, CorrectionPublication,
    CorrectionRepository, DeviationRecordPublication, DeviationRecordRepository,
    ExecutionIndexRepository, ExecutionProjectTarget, ExecutionWorktreeRepository,
    RecordBeginPublication, RecordBeginRepository, RecordFinishPublication, RecordFinishRepository,
    RecoveryPrepareRepository, RuntimeExecutionReadiness, inspect_worktree,
    prepare_execute_preflight_from_project, recheck_command_from_project,
};
use work_feature::instruction::InstructionSourceRepository;
use work_feature::ports::SourceSnapshotReader;
use work_feature::ports::{ArtifactStore, CommandRunner, Git};
#[cfg(test)]
use work_feature::ports::{CommandRequest, CommandStatus};
use work_feature::skill::SkillSnapshotRepository;
use work_feature::task::TaskCollectionRepository;
use work_feature::task::{load_collection, load_task_execution_context};
use work_operations::canonical::{parse_json_contract, portable_path_identity};
use work_operations::execution::attempt_start::{
    build_attempt_candidate, recovery_base_index, recovery_candidate, validate_attempt_namespace,
};
#[cfg(windows)]
use work_operations::execution::command_run::windows_batch_command_line;
use work_operations::execution::command_run::{build_command_started, command_json_bytes};
use work_operations::execution::correction::{build_correction_candidates, render_correction};
use work_operations::execution::preflight::FileState;
use work_operations::execution::recovery::{
    AttemptCloseRecovery, CommandCorrectionRecovery, RecordBeginRecovery, RecordFinishRecovery,
    validate_attempt_close_recovery, validate_command_correction_recovery,
    validate_deviation_record_recovery, validate_record_begin_recovery,
    validate_record_finish_recovery, validate_recovery_direction,
};
use work_operations::execution::requests::{
    validate_attempt_start_request, validate_recovery_request,
};
use work_operations::execution::worktree::inspect_records;
use work_operations::execution::{
    build_execution_lock, start_index, validate_authorization_scope, validate_execution_identity,
};

use crate::files::{LocalFiles, resolve_project_path};
use crate::git::{LocalGit, parse_porcelain_v1_z};
use crate::process::LocalCommandRunner;
use crate::specification::storage::{require_no_spec_update, storage_path};
use crate::writer_lock::LocalWriterLock;

#[derive(Debug, Clone)]
pub struct LocalExecutionStorage {
    pub project_root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandReceiptStage {
    StartedWritten,
    StartedVerified,
    CommandReturned,
    FinishedWritten,
    FinishedVerified,
}

/// Exact observed evidence; missing payloads remain explicit rather than disappearing from pending.
#[derive(Debug, Clone)]
pub struct RuntimeExecutionInventory {
    pub transaction_dir: String,
    pub manifest: work_model::runtime::RuntimeManifest,
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeExecutionStep {
    pub target: String,
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
    pub prepared_file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeExecutionPlan {
    pub manifest: work_model::runtime::RuntimeManifest,
    pub payloads: std::collections::BTreeMap<String, Vec<u8>>,
    pub steps: Vec<RuntimeExecutionStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeExecutionStage {
    Prepared,
    StepWritten(usize),
    StepVerified(usize),
    ProgressWritten(usize),
    PublishedVerified,
    BeforeCleanup,
    Cleaned,
}

pub struct RecordBeginRecoveryInput<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub task: &'a Value,
    pub attempt: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
}

pub struct CommandCorrectionRecoveryInput<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub task: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
}

pub struct DeviationRecordRecoveryInput<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
}

pub struct CorrectionRecoveryInput<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub correction_id: &'a str,
    pub collection: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
}

pub struct AttemptStartRecoveryRequest<'a> {
    pub target: ExecutionProjectTarget<'a>,
    pub request: &'a Value,
    pub confirmed_inputs: &'a [String],
    pub started_at: &'a str,
}

struct RecoveryIndexView<'a> {
    storage: &'a LocalExecutionStorage,
    index_raw: &'a [u8],
}

impl ExecutionIndexRepository for RecoveryIndexView<'_> {
    fn read_index(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        if !relative_path.ends_with("/index.json") {
            return Err(WorkError::new(
                ExitCode::Contract,
                "invalid_execution_index_path",
                "Recovery preflight requires the execution index.",
                json!({}),
            ));
        }
        Ok(self.index_raw.to_vec())
    }
    fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError> {
        ExecutionIndexRepository::inspect_path(self.storage, relative_path)
    }
    fn check_execution_ready(
        &self,
        _task_path: &str,
        _execution_dir: &str,
    ) -> Result<(), WorkError> {
        Ok(())
    }
    fn check_execution_task_layout(
        &self,
        execution_dir: &str,
        task_id: &str,
    ) -> Result<(), WorkError> {
        self.storage
            .check_execution_task_layout(execution_dir, task_id)
    }
}

pub struct RecordFinishRecoveryInput<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub task: &'a Value,
    pub attempt: &'a Value,
    pub index: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
    pub authorization_evidence: Option<&'a str>,
}

pub struct AttemptCloseRecoveryInput<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub task: &'a Value,
    pub attempt: &'a Value,
    pub index: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
}

impl LocalExecutionStorage {
    fn attempt_start_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &AttemptStartPublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        self.validate_runtime_context(context)?;
        let manifest = work_operations::execution::attempt_start::build_attempt_start_staging(
            work_operations::execution::attempt_start::AttemptStartStagingInput {
                canonical_root: context
                    .writer()
                    .canonical_project_root
                    .to_str()
                    .unwrap_or(""),
                requirement: &context.writer().requirement_id,
                execution_dir: publication.execution_dir,
                task_id: publication.task_id,
                attempt_id: publication.attempt_id,
                index_before: publication.index_before,
                locked_index: publication.locked_index,
                attempt: publication.attempt,
                started_index: publication.started_index,
                expected_snapshot: publication.expected_snapshot,
            },
        )
        .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        let index = format!("{}/index.json", publication.execution_dir);
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![
                RuntimeExecutionStep {
                    target: index.clone(),
                    before: Some(publication.index_before.to_vec()),
                    after: publication.locked_index.to_vec(),
                    prepared_file: Some("index.locked.json.tmp".into()),
                },
                RuntimeExecutionStep {
                    target: format!(
                        "{}/{}/{}/attempt.json",
                        publication.execution_dir, publication.task_id, publication.attempt_id
                    ),
                    before: None,
                    after: publication.attempt.to_vec(),
                    prepared_file: None,
                },
                RuntimeExecutionStep {
                    target: index,
                    before: Some(publication.locked_index.to_vec()),
                    after: publication.started_index.to_vec(),
                    prepared_file: Some("index.started.json.tmp".into()),
                },
            ],
        })
    }

    fn publish_attempt_start_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &AttemptStartPublication<'_>,
    ) -> Result<(), WorkError> {
        let plan = self.attempt_start_staging_plan(context, publication)?;
        let records =
            work_operations::execution::worktree::runtime_worktree_records(&self.git_status()?);
        let actual = inspect_records(
            &records,
            publication.execution_dir,
            publication.task_id,
            &json!({}),
            &[],
        );
        if actual["snapshot_sha256"] != publication.expected_snapshot {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_worktree_snapshot_changed",
                "The Git worktree snapshot changed after review.",
                json!({"expected":publication.expected_snapshot,"actual":actual["snapshot_sha256"]}),
            ));
        }
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        let row = original["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == publication.task_id)
            .ok_or_else(transaction_conflict)?;
        validate_attempt_namespace(
            &self.attempt_names(publication.execution_dir, publication.task_id)?,
            row["status"].as_str().unwrap_or(""),
            row["latest_attempt"].as_str(),
            publication.attempt_id,
            false,
        )
        .map_err(recovery_rule)?;
        self.run_execution_staging_scoped(context, &plan, false, &LocalFiles, |_| Ok(()))
    }

    fn recover_attempt_start_staging_scoped<H, S, P, T>(
        &self,
        writer: &work_feature::execution::ExecutionWriterContext,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: AttemptStartRecoveryRequest<'_>,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        validate_attempt_start_request(input.request).map_err(recovery_rule)?;
        self.validate_runtime_context(writer)?;
        let target = input.target;
        if target.task_path != writer.target().task_path
            || target.execution_dir != writer.target().execution_dir
            || target.task_id != writer.target().task_id
        {
            return Err(transaction_conflict());
        }
        let pending = self.runtime_execution_inventory(writer)?;
        if pending.len() != 1 || pending[0].manifest.operation != "attempt-start" {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "attempt_start_recovery_transaction_required",
                "Recovery requires exactly one verified Attempt-start transaction.",
                json!({}),
            ));
        }
        let frozen = &pending[0].manifest;
        let attempt_id = frozen.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let index_path = format!("{}/index.json", target.execution_dir);
        let attempt_path = format!(
            "{}/{}/{attempt_id}/attempt.json",
            target.execution_dir, target.task_id
        );
        let base_raw = frozen
            .targets
            .iter()
            .find(|item| item.path == index_path)
            .and_then(|item| item.before.as_ref())
            .ok_or_else(transaction_conflict)?
            .bytes
            .clone();
        let attempt_raw = frozen
            .targets
            .iter()
            .find(|item| item.path == attempt_path)
            .and_then(|item| item.after.as_ref())
            .ok_or_else(transaction_conflict)?
            .bytes
            .clone();
        let base = parse_json_contract(&base_raw).map_err(|_| transaction_conflict())?;
        let preserved = parse_json_contract(&attempt_raw).map_err(|_| transaction_conflict())?;
        writer.check_execution_index(&base, &base_raw)?;
        let view = RecoveryIndexView {
            storage: self,
            index_raw: &base_raw,
        };
        let preflight =
            prepare_execute_preflight_from_project(sources, &view, target, input.confirmed_inputs)?;
        let context = load_task_execution_context(
            sources.instructions,
            sources.skills,
            sources.paths,
            sources.task_repository,
            sources.skill_roots,
            target.task_path,
            target.task_id,
        )?;
        work_feature::task::recheck_task_execution_context(
            sources.paths,
            sources.task_repository,
            &context,
        )?;
        let task = context.contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == target.task_id)
            .ok_or_else(transaction_conflict)?;
        validate_authorization_scope(
            &input.request["authorization"],
            task,
            &context.contract["execution_defaults"],
        )
        .map_err(recovery_rule)?;
        let adapter = RuntimeExecutionStorage {
            storage: self,
            context: writer,
        };
        let worktree =
            inspect_worktree(&adapter, &preflight, &context.validation, &context.contract)?;
        if worktree["snapshot_sha256"] != input.request["worktree_snapshot_sha256"]
            || frozen.business_identity["expected_snapshot"]
                != input.request["worktree_snapshot_sha256"]
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_worktree_snapshot_changed",
                "The Git worktree snapshot changed after review.",
                json!({}),
            ));
        }
        let source = if let Some(id) = input.request["continuation"]["source_attempt_id"].as_str() {
            let raw = self.read_attempt(&format!(
                "{}/{}/{id}/attempt.json",
                target.execution_dir, target.task_id
            ))?;
            let value = parse_json_contract(&raw).map_err(|_| transaction_conflict())?;
            work_operations::execution::attempt::validate_attempt_bytes(&value, &raw)
                .map_err(recovery_rule)?;
            Some(value)
        } else {
            None
        };
        let candidate = build_attempt_candidate(
            &preflight,
            &base,
            input.request,
            attempt_id,
            preserved["started_at"]
                .as_str()
                .ok_or_else(transaction_conflict)?,
            source.as_ref(),
        )
        .map_err(recovery_rule)?;
        if work_operations::execution::attempt::render_attempt(&candidate).map_err(recovery_rule)?
            != attempt_raw
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_recovery_attempt_mismatch",
                "The frozen Attempt differs from the authorized recovery candidate.",
                json!({}),
            ));
        }
        let row = base["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == target.task_id)
            .ok_or_else(transaction_conflict)?;
        validate_attempt_namespace(
            &self.attempt_names(target.execution_dir, target.task_id)?,
            row["status"].as_str().unwrap_or(""),
            row["latest_attempt"].as_str(),
            attempt_id,
            true,
        )
        .map_err(recovery_rule)?;
        let mut locked = base.clone();
        locked["lock"] = build_execution_lock(
            target.task_id,
            attempt_id,
            candidate["execute_instructions_sha256"]
                .as_str()
                .unwrap_or(""),
        );
        let locked_raw = work_operations::execution::index::render_execution_index(&locked)
            .map_err(|_| transaction_conflict())?;
        let started_raw = work_operations::execution::index::render_execution_index(
            &start_index(&locked, target.task_id, attempt_id).map_err(recovery_rule)?,
        )
        .map_err(|_| transaction_conflict())?;
        let plan = self.attempt_start_staging_plan(
            writer,
            &AttemptStartPublication {
                execution_dir: target.execution_dir,
                task_id: target.task_id,
                attempt_id,
                index_before: &base_raw,
                locked_index: &locked_raw,
                attempt: &attempt_raw,
                started_index: &started_raw,
                expected_snapshot: input.request["worktree_snapshot_sha256"]
                    .as_str()
                    .unwrap_or(""),
            },
        )?;
        let mut expected = plan.manifest.clone();
        expected.phase = frozen.phase;
        expected.published_count = frozen.published_count;
        if &expected != frozen {
            return Err(transaction_conflict());
        }
        work_feature::task::recheck_task_execution_context(
            sources.paths,
            sources.task_repository,
            &context,
        )?;
        self.run_execution_staging_scoped(writer, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::AttemptStartRecoveryResponse,
        >(
            json!({"schema":"work-attempt-start-recovery","task_id":target.task_id,"attempt_id":attempt_id,
                "attempt_path":attempt_path,"index_path":index_path,"status":"recovered","lock_status":"held"}),
        ))
    }

    fn record_begin_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &RecordBeginPublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == publication.task_id)
            .ok_or_else(transaction_conflict)?;
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        let attempt =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        validate_execution_identity(
            &context.task_context().collection,
            &context.task_context().validation,
            &original,
            &attempt,
            publication.task_id,
        )
        .map_err(recovery_rule)?;
        let manifest = work_operations::execution::build_record_begin_staging(
            work_operations::execution::RecordBeginStagingInput {
                canonical_root: context
                    .writer()
                    .canonical_project_root
                    .to_str()
                    .unwrap_or(""),
                requirement: &context.writer().requirement_id,
                execution_dir: publication.execution_dir,
                task_id: publication.task_id,
                attempt_id: publication.attempt_id,
                record_id: publication.record_id,
                task,
                index_before: publication.index_before,
                attempt_before: publication.attempt_before,
                index_after: publication.index_after,
            },
        )
        .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![RuntimeExecutionStep {
                target: format!("{}/index.json", publication.execution_dir),
                before: Some(publication.index_before.to_vec()),
                after: publication.index_after.to_vec(),
                prepared_file: Some("index.json.tmp".into()),
            }],
        })
    }

    fn reviewed_execution_staging(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
        operation: &str,
    ) -> Result<RuntimeExecutionInventory, WorkError> {
        work_operations::execution::requests::validate_staging_recovery_request(request, false)
            .map_err(recovery_rule)?;
        let pending = self.runtime_execution_inventory(context)?;
        if pending.len() != 1 || pending[0].manifest.operation != operation {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "execution_recovery_transaction_required",
                "Recovery requires one matching verified transaction.",
                json!({}),
            ));
        }
        let inventory = pending.into_iter().next().unwrap();
        let binding = work_operations::execution::recovery::execution_staging_binding(
            &inventory.manifest,
            context
                .writer()
                .canonical_project_root
                .to_str()
                .unwrap_or(""),
            &context.writer().requirement_id,
            context.target().execution_dir,
            context.target().task_id,
            &inventory.files,
        )
        .map_err(recovery_rule)?;
        work_feature::execution::recovery::require_staging_recovery_binding(
            request,
            &inventory.manifest,
            &binding,
        )?;
        Ok(inventory)
    }

    fn recover_record_begin_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
    ) -> Result<Value, WorkError> {
        let pending = self.reviewed_execution_staging(context, request, "record-begin")?;
        let manifest = &pending.manifest;
        let execution = context.target().execution_dir;
        let index_path = format!("{execution}/index.json");
        let task_id = context.target().task_id;
        let attempt_id = manifest.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let record_id = manifest.business_identity["record_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let index = manifest
            .targets
            .iter()
            .find(|target| target.path == index_path)
            .ok_or_else(transaction_conflict)?;
        let attempt = manifest
            .targets
            .iter()
            .find(|target| {
                target.path == format!("{execution}/{task_id}/{attempt_id}/attempt.json")
            })
            .and_then(|target| target.before.as_ref())
            .ok_or_else(transaction_conflict)?;
        let plan = self.record_begin_staging_plan(
            context,
            &RecordBeginPublication {
                execution_dir: execution,
                task_id,
                attempt_id,
                record_id,
                index_before: &index
                    .before
                    .as_ref()
                    .ok_or_else(transaction_conflict)?
                    .bytes,
                attempt_before: &attempt.bytes,
                index_after: &index.after.as_ref().ok_or_else(transaction_conflict)?.bytes,
            },
        )?;
        let mut expected = plan.manifest.clone();
        expected.phase = manifest.phase;
        expected.published_count = manifest.published_count;
        if &expected != manifest {
            return Err(transaction_conflict());
        }
        self.run_execution_staging_scoped(context, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","transaction":"record_begin","task_id":task_id,
                "attempt_id":attempt_id,"record_id":record_id,"lock_status":"record_reserved","status":"recovered"}),
        ))
    }

    fn command_correction_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &CommandCorrectionPublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == publication.task_id)
            .ok_or_else(transaction_conflict)?;
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        let attempt =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        validate_execution_identity(
            &context.task_context().collection,
            &context.task_context().validation,
            &original,
            &attempt,
            publication.task_id,
        )
        .map_err(recovery_rule)?;
        let manifest =
            work_operations::execution::command_correction::build_command_correction_staging(
                work_operations::execution::command_correction::CommandCorrectionStagingInput {
                    canonical_root: context
                        .writer()
                        .canonical_project_root
                        .to_str()
                        .unwrap_or(""),
                    requirement: &context.writer().requirement_id,
                    execution_dir: publication.execution_dir,
                    task_id: publication.task_id,
                    attempt_id: publication.attempt_id,
                    record_id: publication.record_id,
                    task,
                    index_before: publication.index_before,
                    attempt_before: publication.attempt_before,
                    index_after: publication.index_after,
                },
            )
            .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![RuntimeExecutionStep {
                target: format!("{}/index.json", publication.execution_dir),
                before: Some(publication.index_before.to_vec()),
                after: publication.index_after.to_vec(),
                prepared_file: Some("index.json.tmp".into()),
            }],
        })
    }

    fn recover_command_correction_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
    ) -> Result<Value, WorkError> {
        let pending = self.reviewed_execution_staging(context, request, "command-correction")?;
        let manifest = &pending.manifest;
        let execution = context.target().execution_dir;
        let index_path = format!("{execution}/index.json");
        let task_id = context.target().task_id;
        let attempt_id = manifest.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let record_id = manifest.business_identity["record_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let index = manifest
            .targets
            .iter()
            .find(|target| target.path == index_path)
            .ok_or_else(transaction_conflict)?;
        let attempt = manifest
            .targets
            .iter()
            .find(|target| {
                target.path == format!("{execution}/{task_id}/{attempt_id}/attempt.json")
            })
            .and_then(|target| target.before.as_ref())
            .ok_or_else(transaction_conflict)?;
        let plan = self.command_correction_staging_plan(
            context,
            &CommandCorrectionPublication {
                execution_dir: execution,
                task_id,
                attempt_id,
                record_id,
                index_before: &index
                    .before
                    .as_ref()
                    .ok_or_else(transaction_conflict)?
                    .bytes,
                attempt_before: &attempt.bytes,
                index_after: &index.after.as_ref().ok_or_else(transaction_conflict)?.bytes,
            },
        )?;
        let mut expected = plan.manifest.clone();
        expected.phase = manifest.phase;
        expected.published_count = manifest.published_count;
        if &expected != manifest {
            return Err(transaction_conflict());
        }
        self.run_execution_staging_scoped(context, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","transaction":"command_correction","task_id":task_id,
                "attempt_id":attempt_id,"record_id":record_id,"lock_status":"record_reserved","status":"recovered"}),
        ))
    }

    fn record_finish_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &RecordFinishPublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == publication.task_id)
            .ok_or_else(transaction_conflict)?;
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        let attempt =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        validate_execution_identity(
            &context.task_context().collection,
            &context.task_context().validation,
            &original,
            &attempt,
            publication.task_id,
        )
        .map_err(recovery_rule)?;
        let manifest = work_operations::execution::record_finish::build_record_finish_staging(
            work_operations::execution::record_finish::RecordFinishStagingInput {
                canonical_root: context
                    .writer()
                    .canonical_project_root
                    .to_str()
                    .unwrap_or(""),
                requirement: &context.writer().requirement_id,
                execution_dir: publication.execution_dir,
                task_id: publication.task_id,
                attempt_id: publication.attempt_id,
                record_id: publication.record_id,
                task,
                request: publication.request,
                index_before: publication.index_before,
                index_after: publication.index_after,
                attempt_before: publication.attempt_before,
                attempt_after: publication.attempt_after,
            },
        )
        .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![
                RuntimeExecutionStep {
                    target: format!(
                        "{}/{}/{}/attempt.json",
                        publication.execution_dir, publication.task_id, publication.attempt_id
                    ),
                    before: Some(publication.attempt_before.to_vec()),
                    after: publication.attempt_after.to_vec(),
                    prepared_file: Some("attempt.json.tmp".into()),
                },
                RuntimeExecutionStep {
                    target: format!("{}/index.json", publication.execution_dir),
                    before: Some(publication.index_before.to_vec()),
                    after: publication.index_after.to_vec(),
                    prepared_file: Some("index.json.tmp".into()),
                },
            ],
        })
    }

    fn recover_record_finish_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
    ) -> Result<Value, WorkError> {
        let pending = self.reviewed_execution_staging(context, request, "record-finish")?;
        let manifest = &pending.manifest;
        let execution = context.target().execution_dir;
        let task_id = context.target().task_id;
        let attempt_id = manifest.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let record_id = manifest.business_identity["record_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let index_path = format!("{execution}/index.json");
        let attempt_path = format!("{execution}/{task_id}/{attempt_id}/attempt.json");
        let index = manifest
            .targets
            .iter()
            .find(|target| target.path == index_path)
            .ok_or_else(transaction_conflict)?;
        let attempt = manifest
            .targets
            .iter()
            .find(|target| target.path == attempt_path)
            .ok_or_else(transaction_conflict)?;
        let publication = RecordFinishPublication {
            execution_dir: execution,
            task_id,
            attempt_id,
            record_id,
            request: &manifest.business_identity["record_finish_request"],
            index_before: &index
                .before
                .as_ref()
                .ok_or_else(transaction_conflict)?
                .bytes,
            index_after: &index.after.as_ref().ok_or_else(transaction_conflict)?.bytes,
            attempt_before: &attempt
                .before
                .as_ref()
                .ok_or_else(transaction_conflict)?
                .bytes,
            attempt_after: &attempt
                .after
                .as_ref()
                .ok_or_else(transaction_conflict)?
                .bytes,
        };
        let plan = self.record_finish_staging_plan(context, &publication)?;
        let mut expected = plan.manifest.clone();
        expected.phase = manifest.phase;
        expected.published_count = manifest.published_count;
        if &expected != manifest {
            return Err(transaction_conflict());
        }
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == task_id)
            .ok_or_else(transaction_conflict)?;
        let before =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        let after =
            parse_json_contract(publication.attempt_after).map_err(|_| transaction_conflict())?;
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        validate_record_finish_recovery(RecordFinishRecovery {
            index: &original,
            index_raw: publication.index_before,
            attempt: &before,
            attempt_raw: publication.attempt_before,
            prepared_attempt: Some((&after, publication.attempt_after)),
            task,
            authorization_evidence: request["authorization_evidence"].as_str(),
        })
        .map_err(recovery_rule)?;
        self.run_execution_staging_scoped(context, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","transaction":"record_finish","task_id":task_id,"attempt_id":attempt_id,
                "record_id":record_id,"attempt_path":attempt_path,"index_path":index_path,"lock_status":"attempt_held","status":"recovered"}),
        ))
    }

    fn attempt_close_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &AttemptClosePublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == publication.task_id)
            .ok_or_else(transaction_conflict)?;
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        let attempt =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        validate_execution_identity(
            &context.task_context().collection,
            &context.task_context().validation,
            &original,
            &attempt,
            publication.task_id,
        )
        .map_err(recovery_rule)?;
        let manifest = work_operations::execution::attempt_close::build_attempt_close_staging(
            work_operations::execution::attempt_close::AttemptCloseStagingInput {
                canonical_root: context
                    .writer()
                    .canonical_project_root
                    .to_str()
                    .unwrap_or(""),
                requirement: &context.writer().requirement_id,
                execution_dir: publication.execution_dir,
                task_id: publication.task_id,
                attempt_id: publication.attempt_id,
                task,
                index_before: publication.index_before,
                index_after: publication.index_after,
                attempt_before: publication.attempt_before,
                attempt_after: publication.attempt_after,
            },
        )
        .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![
                RuntimeExecutionStep {
                    target: format!(
                        "{}/{}/{}/attempt.json",
                        publication.execution_dir, publication.task_id, publication.attempt_id
                    ),
                    before: Some(publication.attempt_before.to_vec()),
                    after: publication.attempt_after.to_vec(),
                    prepared_file: Some("attempt.json.tmp".into()),
                },
                RuntimeExecutionStep {
                    target: format!("{}/index.json", publication.execution_dir),
                    before: Some(publication.index_before.to_vec()),
                    after: publication.index_after.to_vec(),
                    prepared_file: Some("index.json.tmp".into()),
                },
            ],
        })
    }

    fn recover_attempt_close_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
    ) -> Result<Value, WorkError> {
        let pending = self.reviewed_execution_staging(context, request, "attempt-close")?;
        let manifest = &pending.manifest;
        let execution = context.target().execution_dir;
        let task_id = context.target().task_id;
        let attempt_id = manifest.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let index_path = format!("{execution}/index.json");
        let attempt_path = format!("{execution}/{task_id}/{attempt_id}/attempt.json");
        let index = manifest
            .targets
            .iter()
            .find(|target| target.path == index_path)
            .ok_or_else(transaction_conflict)?;
        let attempt = manifest
            .targets
            .iter()
            .find(|target| target.path == attempt_path)
            .ok_or_else(transaction_conflict)?;
        let publication = AttemptClosePublication {
            execution_dir: execution,
            task_id,
            attempt_id,
            index_before: &index
                .before
                .as_ref()
                .ok_or_else(transaction_conflict)?
                .bytes,
            index_after: &index.after.as_ref().ok_or_else(transaction_conflict)?.bytes,
            attempt_before: &attempt
                .before
                .as_ref()
                .ok_or_else(transaction_conflict)?
                .bytes,
            attempt_after: &attempt
                .after
                .as_ref()
                .ok_or_else(transaction_conflict)?
                .bytes,
        };
        let plan = self.attempt_close_staging_plan(context, &publication)?;
        let mut expected = plan.manifest.clone();
        expected.phase = manifest.phase;
        expected.published_count = manifest.published_count;
        if &expected != manifest {
            return Err(transaction_conflict());
        }
        let after =
            parse_json_contract(publication.attempt_after).map_err(|_| transaction_conflict())?;
        if after["status"] == "completed" {
            super::file_transaction_storage::require_retained_publication(context, attempt_id)?;
        }
        self.run_execution_staging_scoped(context, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","transaction":"attempt_close","task_id":task_id,"attempt_id":attempt_id,
                "attempt_status":after["status"],"attempt_path":attempt_path,"index_path":index_path,"lock_status":"released","status":"recovered"}),
        ))
    }

    fn deviation_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &DeviationRecordPublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        let index_path = format!("{}/index.json", publication.execution_dir);
        let index_raw = publication
            .sources
            .get(&index_path)
            .ok_or_else(transaction_conflict)?;
        let index = parse_json_contract(index_raw).map_err(|_| transaction_conflict())?;
        let attempt =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        validate_execution_identity(
            &context.task_context().collection,
            &context.task_context().validation,
            &index,
            &attempt,
            publication.task_id,
        )
        .map_err(recovery_rule)?;
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == publication.task_id)
            .ok_or_else(transaction_conflict)?;
        let sources = publication
            .sources
            .iter()
            .map(|(path, raw)| (path.clone(), raw.clone()))
            .collect();
        let manifest = work_operations::execution::deviation::build_deviation_staging(
            work_operations::execution::deviation::DeviationStagingInput {
                canonical_root: context
                    .writer()
                    .canonical_project_root
                    .to_str()
                    .unwrap_or(""),
                requirement: &context.writer().requirement_id,
                execution_dir: publication.execution_dir,
                task_id: publication.task_id,
                attempt_id: publication.attempt_id,
                record_id: publication.record_id,
                task,
                index_before: index_raw,
                attempt_before: publication.attempt_before,
                attempt_after: publication.attempt_after,
                sources: &sources,
            },
        )
        .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![RuntimeExecutionStep {
                target: format!(
                    "{}/{}/{}/attempt.json",
                    publication.execution_dir, publication.task_id, publication.attempt_id
                ),
                before: Some(publication.attempt_before.to_vec()),
                after: publication.attempt_after.to_vec(),
                prepared_file: Some("attempt.json.tmp".into()),
            }],
        })
    }

    fn recover_deviation_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
    ) -> Result<Value, WorkError> {
        let pending = self.reviewed_execution_staging(context, request, "deviation-record")?;
        let manifest = &pending.manifest;
        let execution = context.target().execution_dir;
        let task_id = context.target().task_id;
        let attempt_id = manifest.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let record_id = manifest.business_identity["record_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let path = format!("{execution}/{task_id}/{attempt_id}/attempt.json");
        let attempt = manifest
            .targets
            .iter()
            .find(|target| target.path == path)
            .ok_or_else(transaction_conflict)?;
        let sources: std::collections::HashMap<String, Vec<u8>> =
            serde_json::from_value(manifest.business_identity["publication_sources"].clone())
                .map_err(|_| transaction_conflict())?;
        let plan = self.deviation_staging_plan(
            context,
            &DeviationRecordPublication {
                execution_dir: execution,
                task_id,
                attempt_id,
                record_id,
                sources: &sources,
                attempt_before: &attempt
                    .before
                    .as_ref()
                    .ok_or_else(transaction_conflict)?
                    .bytes,
                attempt_after: &attempt
                    .after
                    .as_ref()
                    .ok_or_else(transaction_conflict)?
                    .bytes,
            },
        )?;
        let mut expected = plan.manifest.clone();
        expected.phase = manifest.phase;
        expected.published_count = manifest.published_count;
        if &expected != manifest {
            return Err(transaction_conflict());
        }
        self.run_execution_staging_scoped(context, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","status":"recovered","transaction":"deviation_record",
                "task_id":task_id,"attempt_id":attempt_id,"record_id":record_id,"lock_status":"record_reserved"}),
        ))
    }

    fn correction_staging_plan(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        publication: &CorrectionPublication<'_>,
    ) -> Result<RuntimeExecutionPlan, WorkError> {
        let original =
            parse_json_contract(publication.index_before).map_err(|_| transaction_conflict())?;
        let attempt =
            parse_json_contract(publication.attempt_before).map_err(|_| transaction_conflict())?;
        validate_execution_identity(
            &context.task_context().collection,
            &context.task_context().validation,
            &original,
            &attempt,
            publication.task_id,
        )
        .map_err(recovery_rule)?;
        let current_name = format!("{}.json", publication.correction_id);
        let names = self
            .correction_names(
                publication.execution_dir,
                publication.task_id,
                publication.attempt_id,
            )?
            .into_iter()
            .filter(|name| name != &current_name)
            .collect::<Vec<_>>();
        let expected = work_operations::derivation::identity::next_correction_id(
            publication.attempt_id,
            &names,
        )
        .map_err(|_| {
            runtime_scan_error(
                "correction_create_invalid_existing_name",
                publication.execution_dir,
            )
        })?;
        if expected != publication.correction_id {
            return Err(runtime_scan_error(
                "correction_staging_next_id_mismatch",
                publication.execution_dir,
            ));
        }
        let manifest = work_operations::execution::correction::build_correction_staging(
            work_operations::execution::correction::CorrectionStagingInput {
                canonical_root: context
                    .writer()
                    .canonical_project_root
                    .to_str()
                    .unwrap_or(""),
                requirement: &context.writer().requirement_id,
                execution_dir: publication.execution_dir,
                task_id: publication.task_id,
                attempt_id: publication.attempt_id,
                correction_id: publication.correction_id,
                collection: &context.task_context().collection,
                index_before: publication.index_before,
                attempt_before: publication.attempt_before,
                artifact: publication.artifact,
                locked_index: publication.locked_index,
                final_index: publication.final_index,
            },
        )
        .map_err(recovery_rule)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(&manifest)
            .map_err(recovery_rule)?;
        let index = format!("{}/index.json", publication.execution_dir);
        Ok(RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![
                RuntimeExecutionStep {
                    target: index.clone(),
                    before: Some(publication.index_before.to_vec()),
                    after: publication.locked_index.to_vec(),
                    prepared_file: Some("index.locked.json.tmp".into()),
                },
                RuntimeExecutionStep {
                    target: format!(
                        "{}/{}/{}/corrections/{}.json",
                        publication.execution_dir,
                        publication.task_id,
                        publication.attempt_id,
                        publication.correction_id
                    ),
                    before: None,
                    after: publication.artifact.to_vec(),
                    prepared_file: Some("correction.json.tmp".into()),
                },
                RuntimeExecutionStep {
                    target: index,
                    before: Some(publication.locked_index.to_vec()),
                    after: publication.final_index.to_vec(),
                    prepared_file: Some("index.json.tmp".into()),
                },
            ],
        })
    }

    fn recover_correction_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        request: &Value,
    ) -> Result<Value, WorkError> {
        let pending = self.reviewed_execution_staging(context, request, "correction")?;
        let manifest = &pending.manifest;
        let execution = context.target().execution_dir;
        let task_id = context.target().task_id;
        let attempt_id = manifest.business_identity["attempt_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let correction_id = manifest.business_identity["correction_id"]
            .as_str()
            .ok_or_else(transaction_conflict)?;
        let index_path = format!("{execution}/index.json");
        let correction_path =
            format!("{execution}/{task_id}/{attempt_id}/corrections/{correction_id}.json");
        let index = manifest
            .targets
            .iter()
            .find(|target| target.path == index_path)
            .ok_or_else(transaction_conflict)?;
        let artifact = manifest
            .targets
            .iter()
            .find(|target| target.path == correction_path)
            .and_then(|target| target.after.as_ref())
            .ok_or_else(transaction_conflict)?;
        let attempt = manifest
            .targets
            .iter()
            .find(|target| {
                target.path == format!("{execution}/{task_id}/{attempt_id}/attempt.json")
            })
            .and_then(|target| target.before.as_ref())
            .ok_or_else(transaction_conflict)?;
        let payloads = work_operations::execution::recovery::execution_staging_payloads(manifest)
            .map_err(recovery_rule)?;
        let plan = self.correction_staging_plan(
            context,
            &CorrectionPublication {
                execution_dir: execution,
                task_id,
                attempt_id,
                correction_id,
                index_before: &index
                    .before
                    .as_ref()
                    .ok_or_else(transaction_conflict)?
                    .bytes,
                attempt_before: &attempt.bytes,
                artifact: &artifact.bytes,
                locked_index: payloads
                    .get("index.locked.json.tmp")
                    .ok_or_else(transaction_conflict)?,
                final_index: &index.after.as_ref().ok_or_else(transaction_conflict)?.bytes,
            },
        )?;
        let mut expected = plan.manifest.clone();
        expected.phase = manifest.phase;
        expected.published_count = manifest.published_count;
        if &expected != manifest {
            return Err(transaction_conflict());
        }
        self.run_execution_staging_scoped(context, &plan, true, &LocalFiles, |_| Ok(()))?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","transaction":"correction","task_id":task_id,"attempt_id":attempt_id,
                "correction_id":correction_id,"correction_path":correction_path,"index_path":index_path,
                "affected_task_ids":manifest.business_identity["affected_task_ids"],"lock_status":"released","status":"recovered"}),
        ))
    }

    pub fn publish_execution_staging_candidate(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        plan: &RuntimeExecutionPlan,
    ) -> Result<(), WorkError> {
        self.with_runtime_execution_writer(context, |_| {
            self.run_execution_staging_scoped(context, plan, false, &LocalFiles, |_| Ok(()))
        })
    }

    pub fn recover_execution_staging_candidate(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        plan: &RuntimeExecutionPlan,
    ) -> Result<(), WorkError> {
        self.with_runtime_execution_writer(context, |_| {
            self.run_execution_staging_scoped(context, plan, true, &LocalFiles, |_| Ok(()))
        })
    }

    fn run_execution_staging_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        plan: &RuntimeExecutionPlan,
        recover: bool,
        store: &impl ArtifactStore,
        after_stage: impl FnMut(RuntimeExecutionStage) -> Result<(), WorkError>,
    ) -> Result<(), WorkError> {
        self.run_execution_staging_bytes(context, plan, recover, store, after_stage)
            .map_err(|mut failure| {
                let root = &context.writer().canonical_project_root;
                let preserved = (|| -> Result<_, WorkError> {
                    self.validate_runtime_context(context)?;
                    let directory = crate::transaction_storage::runtime_manifest_directory(
                        root,
                        &plan.manifest,
                    )?;
                    let path = crate::files::resolve_runtime_path(
                        root,
                        &format!(
                            "{}/transaction.json",
                            directory
                                .strip_prefix(root)
                                .map_err(|_| transaction_conflict())?
                                .to_string_lossy()
                                .replace('\\', "/")
                        ),
                    )?;
                    let raw = store.read_raw(&path)?;
                    let stored: work_model::runtime::RuntimeManifest =
                        serde_json::from_slice(&raw).map_err(|_| transaction_conflict())?;
                    work_operations::execution::recovery::validate_execution_staging_manifest(
                        &stored,
                    )
                    .map_err(recovery_rule)?;
                    let mut expected = plan.manifest.clone();
                    expected.phase = stored.phase;
                    expected.published_count = stored.published_count;
                    if expected != stored
                        || serde_json::to_vec(&stored).map_err(|_| transaction_conflict())? != raw
                    {
                        return Err(transaction_conflict());
                    }
                    Ok((directory, stored))
                })();
                if let Ok((directory, stored)) = preserved {
                    let mut details = failure.details.as_object().cloned().unwrap_or_default();
                    details.insert("recovery_required".into(), json!(true));
                    details.insert(
                        "transaction_dir".into(),
                        json!(
                            directory
                                .strip_prefix(root)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/")
                        ),
                    );
                    details.insert("transaction_stage".into(), json!(stored.phase));
                    details.insert("published_count".into(), json!(stored.published_count));
                    failure.details = json!(details);
                }
                failure
            })
    }

    fn run_execution_staging_bytes(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        plan: &RuntimeExecutionPlan,
        recover: bool,
        store: &impl ArtifactStore,
        mut after_stage: impl FnMut(RuntimeExecutionStage) -> Result<(), WorkError>,
    ) -> Result<(), WorkError> {
        use crate::transaction_storage::{
            cleanup_runtime_transaction, prepare_runtime_transaction, read_runtime_manifest,
            replace_checked, runtime_manifest_directory, update_runtime_progress,
        };
        use work_model::runtime::RuntimePhase as P;
        self.validate_runtime_context(context)?;
        require_no_spec_update(&self.project_root, context.target().execution_dir, None)?;
        let root = &context.writer().canonical_project_root;
        let manifest = &plan.manifest;
        work_operations::execution::recovery::validate_execution_staging_manifest(manifest)
            .map_err(recovery_rule)?;
        if manifest.canonical_root != root.to_str().unwrap_or("")
            || manifest.requirement_id != context.writer().requirement_id.as_str()
            || manifest.execution_dir != context.target().execution_dir
            || manifest.business_identity["task_id"] != context.target().task_id
        {
            return Err(runtime_scan_error(
                "runtime_manifest_context_mismatch",
                &manifest.execution_dir,
            ));
        }
        let directory = runtime_manifest_directory(root, manifest)?;
        let relative = directory
            .strip_prefix(root)
            .map_err(|_| {
                runtime_scan_error("runtime_manifest_context_mismatch", &manifest.execution_dir)
            })?
            .to_str()
            .ok_or_else(|| {
                runtime_scan_error("runtime_manifest_context_mismatch", &manifest.execution_dir)
            })?
            .replace('\\', "/");
        let states = runtime_execution_plan_states(plan)?;
        for (path, raw) in &context.task_context().sources {
            let absolute = crate::files::resolve_runtime_path(root, path)?;
            if store.read_raw(&absolute)? != *raw {
                return Err(runtime_scan_error("execution_staging_source_changed", path));
            }
        }
        if let Some(value) = manifest.business_identity.get("publication_sources") {
            let sources: std::collections::BTreeMap<String, Vec<u8>> =
                serde_json::from_value(value.clone()).map_err(|_| {
                    runtime_scan_error("execution_staging_source_changed", &relative)
                })?;
            for (path, raw) in sources {
                let absolute = crate::files::resolve_runtime_path(root, &path)?;
                if let Some(target) = manifest.targets.iter().find(|target| target.path == path) {
                    if target.before.as_ref().map(|evidence| &evidence.bytes) != Some(&raw) {
                        return Err(runtime_scan_error(
                            "execution_staging_source_changed",
                            &path,
                        ));
                    }
                } else if store.read_raw(&absolute)? != raw {
                    return Err(runtime_scan_error(
                        "execution_staging_source_changed",
                        &path,
                    ));
                }
            }
        }
        for file in &manifest.inventory {
            let raw = plan
                .payloads
                .get(&file.path)
                .ok_or_else(|| runtime_scan_error("runtime_inventory_incomplete", &relative))?;
            if raw.len() as u64 != file.size_bytes
                || !work_operations::derivation::fingerprint::verify_raw(raw, &file.sha256)
            {
                return Err(runtime_scan_error(
                    "runtime_inventory_hash_mismatch",
                    &relative,
                ));
            }
        }
        if plan.payloads.len() != manifest.inventory.len() {
            return Err(runtime_scan_error("runtime_inventory_foreign", &relative));
        }
        let observed = read_runtime_execution_targets(store, root, manifest)?;
        let stage = states.iter().rposition(|state| *state == observed);
        if !recover {
            self.require_runtime_execution_ready(context, None)?;
            if manifest.phase != P::Prepared
                || manifest.published_count != 0
                || observed != states[0]
            {
                return Err(runtime_scan_error(
                    "execution_staging_source_changed",
                    &relative,
                ));
            }
            prepare_runtime_transaction(store, root, manifest, &plan.payloads)?;
            after_stage(RuntimeExecutionStage::Prepared)?;
        } else if !directory.exists() {
            self.require_runtime_execution_ready(context, None)?;
            if stage != Some(plan.steps.len()) {
                return Err(runtime_scan_error(
                    "runtime_cleanup_identity_missing",
                    &relative,
                ));
            }
            let mut finished = manifest.clone();
            finished.phase = P::Cleaning;
            finished.published_count = finished.targets.len();
            cleanup_runtime_transaction(store, root, &finished)?;
            return Ok(());
        } else {
            self.require_runtime_execution_ready(context, Some(&relative))?;
            read_runtime_manifest(store, root, manifest)?;
        }
        let (mut stage, partial_step) = if let Some(stage) = stage {
            (stage, None)
        } else if recover {
            let candidate = (0..plan.steps.len())
                .find(|index| {
                    let step = &plan.steps[*index];
                    step.before.is_none()
                        && observed
                            .get(&step.target)
                            .and_then(|raw| raw.as_ref())
                            .is_some_and(|raw| step.after.starts_with(raw))
                        && states[*index].iter().all(|(path, raw)| {
                            path == &step.target || observed.get(path) == Some(raw)
                        })
                })
                .ok_or_else(|| {
                    runtime_scan_error("execution_staging_formal_conflict", &relative)
                })?;
            (candidate + 1, Some(candidate))
        } else {
            return Err(runtime_scan_error(
                "execution_staging_formal_conflict",
                &relative,
            ));
        };
        let control =
            crate::files::resolve_runtime_path(root, &format!("{relative}/transaction.json.tmp"))?;
        let adoption = if control.exists() {
            let current = read_runtime_manifest(store, root, manifest)?;
            let raw = store.read_raw(&control)?;
            let count = manifest
                .targets
                .iter()
                .filter(|target| {
                    states[stage].get(&target.path)
                        == Some(&target.after.as_ref().map(|bytes| bytes.bytes.clone()))
                })
                .count();
            let candidates =
                work_operations::execution::recovery::execution_control_candidates(&current, &raw)
                    .map_err(|_| {
                        runtime_scan_error("runtime_control_temporary_invalid", &relative)
                    })?;
            let next = candidates
                .into_iter()
                .find(|next| {
                    next.published_count <= count
                        && (!matches!(next.phase, P::PublishedVerified | P::Cleaning)
                            || stage == plan.steps.len())
                })
                .ok_or_else(|| {
                    runtime_scan_error("runtime_control_temporary_invalid", &relative)
                })?;
            let encoded = serde_json::to_vec(&next)
                .map_err(|_| runtime_scan_error("runtime_control_temporary_invalid", &relative))?;
            if encoded != raw && !recover {
                return Err(runtime_scan_error(
                    "runtime_control_temporary_invalid",
                    &relative,
                ));
            }
            Some((current, raw, encoded))
        } else {
            None
        };
        if let Some(candidate) = partial_step {
            let step = &plan.steps[candidate];
            let target = crate::files::resolve_runtime_path(root, &step.target)?;
            crate::transaction_storage::complete_write(&target, &step.after)?;
            after_stage(RuntimeExecutionStage::StepWritten(candidate))?;
            if store.read_raw(&target)? != step.after {
                return Err(runtime_scan_error(
                    "execution_staging_readback",
                    &step.target,
                ));
            }
            after_stage(RuntimeExecutionStage::StepVerified(candidate))?;
        }
        if recover {
            let current = read_runtime_manifest(store, root, manifest)?;
            if !matches!(current.phase, P::PublishedVerified | P::Cleaning) {
                for (file, raw) in &plan.payloads {
                    let path =
                        crate::files::resolve_runtime_path(root, &format!("{relative}/{file}"))?;
                    crate::transaction_storage::complete_write(&path, raw)?;
                    if store.read_raw(&path)? != *raw {
                        return Err(runtime_scan_error(
                            "runtime_inventory_hash_mismatch",
                            &relative,
                        ));
                    }
                }
            }
        }
        if let Some((current, raw, encoded)) = adoption {
            if encoded != raw {
                crate::transaction_storage::complete_write(&control, &encoded)?;
                if store.read_raw(&control)? != encoded {
                    return Err(runtime_scan_error(
                        "runtime_control_temporary_invalid",
                        &relative,
                    ));
                }
            }
            replace_checked(
                store,
                &directory.join("transaction.json"),
                &serde_json::to_vec(&current)
                    .map_err(|_| runtime_scan_error("runtime_manifest_invalid", &relative))?,
                &encoded,
                &control,
                true,
            )?;
        }
        let current = read_runtime_manifest(store, root, manifest)?;
        if current.phase == P::Cleaning {
            if stage != plan.steps.len() {
                return Err(runtime_scan_error(
                    "execution_staging_formal_conflict",
                    &relative,
                ));
            }
            after_stage(RuntimeExecutionStage::BeforeCleanup)?;
            cleanup_runtime_transaction(store, root, manifest)?;
            after_stage(RuntimeExecutionStage::Cleaned)?;
            return Ok(());
        }
        while stage < plan.steps.len() {
            let step = &plan.steps[stage];
            let target = crate::files::resolve_runtime_path(root, &step.target)?;
            crate::files::require_runtime_same_filesystem(&directory, &target)?;
            if let Some(before) = &step.before {
                let file = step.prepared_file.as_ref().ok_or_else(|| {
                    runtime_scan_error("execution_staging_plan_invalid", &step.target)
                })?;
                let temporary =
                    crate::files::resolve_runtime_path(root, &format!("{relative}/{file}"))?;
                replace_checked(store, &target, before, &step.after, &temporary, true)?;
            } else {
                store.create_directories(target.parent().ok_or_else(|| {
                    runtime_scan_error("execution_staging_plan_invalid", &step.target)
                })?)?;
                crate::files::resolve_runtime_path(root, &step.target)?;
                store.create_new(&target, &step.after)?;
            }
            after_stage(RuntimeExecutionStage::StepWritten(stage))?;
            if store.read_raw(&target)? != step.after {
                return Err(runtime_scan_error(
                    "execution_staging_readback",
                    &step.target,
                ));
            }
            after_stage(RuntimeExecutionStage::StepVerified(stage))?;
            let actual = read_runtime_execution_targets(store, root, manifest)?;
            if actual != states[stage + 1] {
                return Err(runtime_scan_error(
                    "execution_staging_formal_conflict",
                    &relative,
                ));
            }
            let count = manifest
                .targets
                .iter()
                .filter(|target| {
                    actual.get(&target.path)
                        == Some(&target.after.as_ref().map(|bytes| bytes.bytes.clone()))
                })
                .count();
            update_runtime_progress(store, root, manifest, count, P::Publishing)?;
            after_stage(RuntimeExecutionStage::ProgressWritten(stage))?;
            stage += 1;
        }
        if read_runtime_execution_targets(store, root, manifest)? != *states.last().unwrap() {
            return Err(runtime_scan_error("execution_staging_readback", &relative));
        }
        // Recreate any rename-consumed prepared file from the frozen full proof before verified cleanup.
        for (file, raw) in &plan.payloads {
            let path = crate::files::resolve_runtime_path(root, &format!("{relative}/{file}"))?;
            if !path.exists() {
                store.create_new(&path, raw)?;
            }
            if store.read_raw(&path)? != *raw {
                return Err(runtime_scan_error(
                    "runtime_inventory_hash_mismatch",
                    &relative,
                ));
            }
        }
        update_runtime_progress(
            store,
            root,
            manifest,
            manifest.targets.len(),
            P::PublishedVerified,
        )?;
        after_stage(RuntimeExecutionStage::PublishedVerified)?;
        after_stage(RuntimeExecutionStage::BeforeCleanup)?;
        cleanup_runtime_transaction(store, root, manifest)?;
        after_stage(RuntimeExecutionStage::Cleaned)?;
        Ok(())
    }

    /// Read-only candidate scan is scoped to the verified requirement, including spec operations.
    pub fn runtime_execution_inventory(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
    ) -> Result<Vec<RuntimeExecutionInventory>, WorkError> {
        self.validate_runtime_context(context)?;
        let root = &context.writer().canonical_project_root;
        let execution = context.target().execution_dir;
        self.runtime_execution_inventory_scope(root, &context.writer().requirement_id, execution)
    }

    pub fn require_runtime_execution_inventory_ready(
        &self,
        context: &work_feature::execution::ExecutionInventoryContext,
    ) -> Result<(), WorkError> {
        let pending = self.runtime_execution_inventory_scope(
            &context.writer().canonical_project_root,
            &context.writer().requirement_id,
            context.execution_dir(),
        )?;
        if let Some(item) = pending.first() {
            return Err(runtime_scan_error(
                "runtime_execution_transaction_present",
                &item.transaction_dir,
            ));
        }
        Ok(())
    }

    /// Read-only inventory of the current execution and retained journal staging.
    pub fn retained_requirement_inventory(
        &self,
        context: &work_feature::ports::RequirementWriterContext,
        execution: &str,
    ) -> Result<Vec<RuntimeExecutionInventory>, WorkError> {
        self.runtime_execution_inventory_scope(
            &context.canonical_project_root,
            &context.requirement_id,
            execution,
        )
    }

    fn runtime_execution_inventory_scope(
        &self,
        root: &std::path::Path,
        requirement: &work_operations::identifiers::RequirementId,
        execution: &str,
    ) -> Result<Vec<RuntimeExecutionInventory>, WorkError> {
        if self
            .project_root
            .canonicalize()
            .map_err(|_| runtime_scan_error("execution_writer_root", execution))?
            != root
        {
            return Err(runtime_scan_error(
                "execution_writer_root_mismatch",
                execution,
            ));
        }
        let formal = crate::files::resolve_runtime_path(root, execution)?;
        if formal.exists() {
            for entry in fs::read_dir(&formal)
                .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", execution))?
            {
                let entry = entry
                    .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", execution))?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| runtime_scan_error("runtime_inventory_foreign", execution))?;
                if name.starts_with(".work-") && name.ends_with(".tmp") {
                    crate::files::resolve_runtime_path(root, &format!("{execution}/{name}"))?;
                    return Err(runtime_scan_error(
                        "legacy_execution_transaction_present",
                        &format!("{execution}/{name}"),
                    ));
                }
            }
        }
        let namespace = format!("outputs/work/runtime/staging/{}", requirement.as_str());
        let directory = crate::files::resolve_runtime_path(root, &namespace)?;
        if !directory.exists() {
            return Ok(Vec::new());
        }
        if !directory.is_dir() {
            return Err(runtime_scan_error("runtime_inventory_foreign", &namespace));
        }
        let mut result = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", &namespace))?
        {
            let entry = entry
                .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", &namespace))?;
            let operation = entry.file_name();
            let operation = operation
                .to_str()
                .ok_or_else(|| runtime_scan_error("runtime_operation_invalid", &namespace))?;
            let relative = format!("{namespace}/{operation}");
            let path = crate::files::resolve_runtime_path(root, &relative)?;
            if !path.is_dir() {
                return Err(runtime_scan_error("runtime_inventory_foreign", &relative));
            }
            if operation == "project-files" {
                super::file_transaction_storage::check_retained(
                    root,
                    requirement.as_str(),
                    execution,
                )?;
                continue;
            }
            if !matches!(
                operation,
                "attempt-start"
                    | "record-begin"
                    | "command-correction"
                    | "record-finish"
                    | "deviation-record"
                    | "attempt-close"
                    | "correction"
                    | "specification-update"
                    | "specification-migration"
                    | "specification-migration-item"
                    | "specification-migration-reconcile"
                    | "instruction-migration"
                    | "source-refresh"
                    | "source-capture"
            ) {
                return Err(runtime_scan_error("runtime_operation_invalid", &relative));
            }
            for transaction in fs::read_dir(&path)
                .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", &relative))?
            {
                let transaction = transaction
                    .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", &relative))?;
                let name = transaction.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| runtime_scan_error("runtime_identity_invalid", &relative))?;
                let relative = format!("{relative}/{name}");
                let path = crate::files::resolve_runtime_path(root, &relative)?;
                if !path.is_dir() {
                    return Err(runtime_scan_error("runtime_inventory_foreign", &relative));
                }
                if operation == "source-capture" {
                    return Err(runtime_scan_error(
                        "runtime_source_capture_pending",
                        &relative,
                    ));
                }
                let manifest_path = crate::files::resolve_runtime_path(
                    root,
                    &format!("{relative}/transaction.json"),
                )?;
                let raw = LocalFiles
                    .read_raw(&manifest_path)
                    .map_err(|_| runtime_scan_error("runtime_manifest_missing", &relative))?;
                let manifest: work_model::runtime::RuntimeManifest =
                    serde_json::from_slice(&raw)
                        .map_err(|_| runtime_scan_error("runtime_manifest_invalid", &relative))?;
                let actual =
                    crate::transaction_storage::runtime_manifest_directory(root, &manifest)?;
                if actual != path
                    || manifest.requirement_id != requirement.as_str()
                    || manifest.execution_dir != execution
                    || manifest.operation != operation
                {
                    return Err(runtime_scan_error(
                        "runtime_manifest_context_mismatch",
                        &relative,
                    ));
                }
                if serde_json::to_vec(&manifest)
                    .map_err(|_| runtime_scan_error("runtime_manifest_invalid", &relative))?
                    != raw
                {
                    return Err(runtime_scan_error(
                        "runtime_manifest_noncanonical",
                        &relative,
                    ));
                }
                if matches!(
                    operation,
                    "attempt-start"
                        | "record-begin"
                        | "command-correction"
                        | "record-finish"
                        | "deviation-record"
                        | "attempt-close"
                        | "correction"
                ) && manifest
                    .targets
                    .iter()
                    .any(|target| !target.path.starts_with(&format!("{execution}/")))
                {
                    return Err(runtime_scan_error(
                        "runtime_target_context_mismatch",
                        &relative,
                    ));
                }
                let expected = manifest
                    .inventory
                    .iter()
                    .map(|file| (file.path.clone(), file))
                    .collect::<std::collections::BTreeMap<_, _>>();
                if is_retained_journal_operation(&manifest.operation) {
                    let prepared =
                        work_operations::derivation::transaction::restore_journal_staging(
                            &manifest,
                        )
                        .map_err(|_| {
                            runtime_scan_error("runtime_journal_evidence_invalid", &relative)
                        })?;
                    let journal_path = manifest.business_identity["journal_path"]
                        .as_str()
                        .ok_or_else(|| {
                            runtime_scan_error("runtime_journal_evidence_invalid", &relative)
                        })?;
                    let journal: Value = serde_json::from_slice(&prepared.prepared_journal)
                        .map_err(|_| {
                            runtime_scan_error("runtime_journal_evidence_invalid", &relative)
                        })?;
                    work_operations::specification::transaction::verify_retained_journal_layout(
                        execution,
                        journal_path,
                        &journal,
                    )
                    .map_err(|_| {
                        runtime_scan_error("runtime_journal_context_mismatch", &relative)
                    })?;
                    let request = &journal["metadata"]["request"];
                    for claim in [
                        request.get("requirement_id"),
                        request["task_index"].get("requirement_id"),
                    ] {
                        if claim.is_some_and(|value| value != requirement.as_str()) {
                            return Err(runtime_scan_error(
                                "runtime_journal_requirement_mismatch",
                                &relative,
                            ));
                        }
                    }
                }
                let mut files = std::collections::BTreeMap::new();
                collect_runtime_inventory_files(
                    root, &relative, "", &expected, &manifest, &mut files,
                )?;
                let missing_files = expected
                    .keys()
                    .filter(|file| !files.contains_key(*file))
                    .cloned()
                    .collect();
                result.push(RuntimeExecutionInventory {
                    transaction_dir: relative,
                    manifest,
                    files,
                    missing_files,
                });
            }
        }
        result.sort_by(|a, b| a.transaction_dir.cmp(&b.transaction_dir));
        Ok(result)
    }

    /// Recovery may exempt only its fully verified transaction, never an operation or requirement.
    pub fn require_runtime_execution_ready(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        reviewed_transaction: Option<&str>,
    ) -> Result<(), WorkError> {
        let inventory = self.runtime_execution_inventory(context)?;
        if let Some(pending) = inventory
            .iter()
            .find(|entry| Some(entry.transaction_dir.as_str()) != reviewed_transaction)
        {
            return Err(runtime_scan_error(
                "runtime_execution_transaction_present",
                &pending.transaction_dir,
            ));
        }
        if reviewed_transaction.is_some_and(|reviewed| {
            !inventory
                .iter()
                .any(|entry| entry.transaction_dir == reviewed)
        }) {
            return Err(runtime_scan_error(
                "runtime_recovery_transaction_missing",
                reviewed_transaction.unwrap(),
            ));
        }
        Ok(())
    }

    fn receipt_paths_observed(
        &self,
        paths: &work_operations::derivation::publication::CommandReceiptPaths,
    ) -> Result<bool, WorkError> {
        let root = self
            .project_root
            .canonicalize()
            .map_err(|_| transaction_conflict())?;
        for relative in [&paths.legacy_started, &paths.legacy_finished] {
            let path = crate::files::resolve_runtime_path(&root, relative)?;
            if path.symlink_metadata().is_ok() {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "legacy_command_receipt_present",
                    "Legacy command evidence requires reviewed offline handling; never run this instance again.",
                    json!({"path":relative}),
                ));
            }
        }
        let directory = crate::files::resolve_runtime_path(&root, &paths.directory)?;
        // Even an empty allocated directory is retained evidence of an interrupted start.
        if directory.symlink_metadata().is_ok() {
            if !directory.is_dir() {
                return Err(transaction_io(
                    "command_run_receipt_directory_invalid",
                    &directory,
                ));
            }
            for entry in fs::read_dir(&directory)
                .map_err(|_| transaction_io("command_run_receipt_probe_failed", &directory))?
            {
                let entry = entry
                    .map_err(|_| transaction_io("command_run_receipt_probe_failed", &directory))?;
                let name = entry.file_name();
                let name = name.to_str().ok_or_else(|| {
                    transaction_io("command_run_receipt_probe_failed", &entry.path())
                })?;
                crate::files::resolve_runtime_path(&root, &format!("{}/{name}", paths.directory))?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub fn receipt_directory_exists(
        &self,
        execution: &str,
        task: &str,
        attempt: &str,
        record: &str,
    ) -> Result<bool, WorkError> {
        let paths = work_operations::derivation::publication::command_receipt_paths(
            execution, task, attempt, record,
        )
        .map_err(|_| transaction_conflict())?;
        self.receipt_paths_observed(&paths)
    }

    pub fn run_prepared_command_with_receipts(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        approval: &str,
        recheck: impl FnOnce() -> Result<(Value, String), WorkError>,
        runner: &impl CommandRunner,
    ) -> Result<Value, WorkError> {
        self.with_runtime_execution_writer(context, |_| {
            self.run_receipts_scoped(context, approval, recheck, runner, |_| Ok(()))
        })
    }

    fn run_receipts_scoped(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        approval: &str,
        recheck: impl FnOnce() -> Result<(Value, String), WorkError>,
        runner: &impl CommandRunner,
        mut after_stage: impl FnMut(CommandReceiptStage) -> Result<(), WorkError>,
    ) -> Result<Value, WorkError> {
        self.validate_runtime_context(context)?;
        let target = context.target();
        require_no_spec_update(&self.project_root, target.execution_dir, None)?;
        let root = self
            .project_root
            .canonicalize()
            .map_err(|_| transaction_conflict())?;
        let index_path = crate::files::resolve_runtime_path(
            &root,
            &format!("{}/index.json", target.execution_dir),
        )?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let index = parse_json_contract(&index_raw).map_err(|_| transaction_conflict())?;
        context.check_execution_index(&index, &index_raw)?;
        let (preview, evidence) = recheck()?;
        let paths = work_feature::execution::command_publication::approved_receipt_paths(
            &preview,
            target.execution_dir,
            approval,
        )?;
        if preview["task_id"] != target.task_id {
            return Err(transaction_conflict());
        }
        if self.receipt_paths_observed(&paths)? {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_already_started",
                "This record already has execution evidence; never run it again.",
                json!({"receipt_dir":paths.directory}),
            ));
        }
        let command = build_command_request(&preview)?;
        let directory = crate::files::resolve_runtime_path(&root, &paths.directory)?;
        let started_path = crate::files::resolve_runtime_path(&root, &paths.started)?;
        let finished_path = crate::files::resolve_runtime_path(&root, &paths.finished)?;
        crate::files::require_runtime_same_filesystem(&directory, &index_path)?;
        crate::files::require_runtime_same_filesystem(&directory, &finished_path)?;
        fs::create_dir_all(directory.parent().expect("receipt parent"))
            .map_err(|_| command_interrupted(&paths.directory))?;
        fs::create_dir(&directory).map_err(|_| command_interrupted(&paths.directory))?;
        let started = command_json_bytes(&build_command_started(&preview, &evidence));
        LocalFiles
            .create_new(&started_path, &started)
            .map_err(|_| command_interrupted(&paths.directory))?;
        after_stage(CommandReceiptStage::StartedWritten)?;
        let checked = crate::files::resolve_runtime_path(&root, &paths.started)?;
        if LocalFiles.read_raw(&checked)? != started {
            return Err(command_interrupted(&paths.directory));
        }
        after_stage(CommandReceiptStage::StartedVerified)?;
        let outcome = runner.run(&command);
        after_stage(CommandReceiptStage::CommandReturned)?;
        let completion =
            work_feature::execution::command_publication::command_completion_with_receipts(
                &preview, &outcome,
            );
        let finished = command_json_bytes(&completion.finished);
        let checked = crate::files::resolve_runtime_path(&root, &paths.finished)?;
        LocalFiles
            .create_new(&checked, &finished)
            .map_err(|_| command_interrupted(&paths.directory))?;
        after_stage(CommandReceiptStage::FinishedWritten)?;
        let checked = crate::files::resolve_runtime_path(&root, &paths.finished)?;
        if LocalFiles.read_raw(&checked)? != finished {
            return Err(command_interrupted(&paths.directory));
        }
        after_stage(CommandReceiptStage::FinishedVerified)?;
        completion.into_result()
    }

    fn recover_attempt_start_from_project_scoped<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: AttemptStartRecoveryRequest<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        require_execution_writer_context(runtime_held)?;
        validate_attempt_start_request(input.request).map_err(recovery_rule)?;
        let target = input.target;
        require_no_spec_update(&self.project_root, target.execution_dir, None)?;
        #[cfg(test)]
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", target.execution_dir),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&writer_path)?)
        };
        require_no_spec_update(&self.project_root, target.execution_dir, None)?;
        let index_path = storage_path(
            &self.project_root,
            &format!("{}/index.json", target.execution_dir),
        )?;
        let current_raw = LocalFiles.read_raw(&index_path)?;
        let current = parse_json_contract(&current_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The execution index is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::index::validate_execution_index(&current, &current_raw)
            .map_err(recovery_rule)?;
        let inventory = self.temporary_names(target.execution_dir)?;
        let attempt_id =
            recovery_candidate(&inventory, &current, target.task_id).map_err(recovery_rule)?;
        let lock_name = format!(
            ".work-attempt-start-{}-{attempt_id}-lock.tmp",
            target.task_id
        );
        let started_name = format!(
            ".work-attempt-start-{}-{attempt_id}-started.tmp",
            target.task_id
        );
        validate_attempt_start_inventory(&inventory, &lock_name, &started_name)?;
        let continuation = input
            .request
            .get("continuation")
            .filter(|value| !value.is_null());
        let original_status = if continuation.is_some() {
            "pending_retry"
        } else {
            "pending"
        };
        let source_attempt_id = continuation.and_then(|value| value["source_attempt_id"].as_str());
        let base = recovery_base_index(
            &current,
            target.task_id,
            &attempt_id,
            original_status,
            source_attempt_id,
        )
        .map_err(recovery_rule)?;
        let base_raw =
            work_operations::execution::index::render_execution_index(&base).expect("JSON index");
        let view = RecoveryIndexView {
            storage: self,
            index_raw: &base_raw,
        };
        let mut preflight =
            prepare_execute_preflight_from_project(sources, &view, target, input.confirmed_inputs)?;
        let context = load_task_execution_context(
            sources.instructions,
            sources.skills,
            sources.paths,
            sources.task_repository,
            sources.skill_roots,
            target.task_path,
            target.task_id,
        )?;
        work_feature::task::recheck_task_execution_context(
            sources.paths,
            sources.task_repository,
            &context,
        )?;
        let task = context.contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == target.task_id)
            .ok_or_else(|| {
                WorkError::new(
                    ExitCode::Contract,
                    "attempt_start_task_not_found",
                    "The target TASK is missing.",
                    json!({}),
                )
            })?;
        validate_authorization_scope(
            &input.request["authorization"],
            task,
            &context.contract["execution_defaults"],
        )
        .map_err(recovery_rule)?;
        let worktree = inspect_worktree(self, &preflight, &context.validation, &context.contract)?;
        if worktree["snapshot_sha256"] != input.request["worktree_snapshot_sha256"] {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_worktree_snapshot_changed",
                "The Git worktree snapshot changed after review.",
                json!({}),
            ));
        }
        preflight["snapshot_sha256"] = worktree["snapshot_sha256"].clone();
        let names = self.attempt_names(target.execution_dir, target.task_id)?;
        let latest = base["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == target.task_id)
            .and_then(|row| row["latest_attempt"].as_str());
        validate_attempt_namespace(&names, original_status, latest, &attempt_id, true)
            .map_err(recovery_rule)?;
        let attempt_relative = format!(
            "{}/{}/{attempt_id}/attempt.json",
            target.execution_dir, target.task_id
        );
        let attempt_path = storage_path(&self.project_root, &attempt_relative)?;
        let existing_attempt = if attempt_path.is_file() {
            let raw = LocalFiles.read_raw(&attempt_path)?;
            let value = parse_json_contract(&raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The Attempt is invalid.",
                    json!({}),
                )
            })?;
            work_operations::execution::attempt::validate_attempt_bytes(&value, &raw)
                .map_err(recovery_rule)?;
            Some((value, raw))
        } else {
            None
        };
        let source_attempt = if let Some(source_id) = source_attempt_id {
            let path = format!(
                "{}/{}/{source_id}/attempt.json",
                target.execution_dir, target.task_id
            );
            let raw = self.read_attempt(&path)?;
            let value = parse_json_contract(&raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The source Attempt is invalid.",
                    json!({}),
                )
            })?;
            work_operations::execution::attempt::validate_attempt_bytes(&value, &raw)
                .map_err(recovery_rule)?;
            Some(value)
        } else {
            None
        };
        let started_at = existing_attempt
            .as_ref()
            .and_then(|(value, _)| value["started_at"].as_str())
            .unwrap_or(input.started_at);
        let candidate = build_attempt_candidate(
            &preflight,
            &base,
            input.request,
            &attempt_id,
            started_at,
            source_attempt.as_ref(),
        )
        .map_err(recovery_rule)?;
        let candidate_raw = work_operations::execution::attempt::render_attempt(&candidate)
            .map_err(recovery_rule)?;
        if existing_attempt
            .as_ref()
            .is_some_and(|(_, raw)| *raw != candidate_raw)
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_recovery_attempt_mismatch",
                "The existing Attempt differs from deterministic recovery evidence.",
                json!({}),
            ));
        }
        let mut locked = base.clone();
        locked["lock"] = build_execution_lock(
            target.task_id,
            &attempt_id,
            candidate["execute_instructions_sha256"]
                .as_str()
                .unwrap_or(""),
        );
        let locked_raw =
            work_operations::execution::index::render_execution_index(&locked).expect("JSON index");
        work_operations::execution::index::validate_execution_index(&locked, &locked_raw)
            .map_err(recovery_rule)?;
        let started = start_index(&locked, target.task_id, &attempt_id).map_err(recovery_rule)?;
        let started_raw = work_operations::execution::index::render_execution_index(&started)
            .expect("JSON index");
        work_operations::execution::index::validate_execution_index(&started, &started_raw)
            .map_err(recovery_rule)?;
        let stage = attempt_start_stage(&current_raw, &base_raw, &locked_raw, &started_raw)?;
        let lock_temp = storage_path(
            &self.project_root,
            &format!("{}/{}", target.execution_dir, lock_name),
        )?;
        let started_temp = storage_path(
            &self.project_root,
            &format!("{}/{}", target.execution_dir, started_name),
        )?;
        if lock_temp.is_file() && LocalFiles.read_raw(&lock_temp)? != locked_raw
            || started_temp.is_file() && LocalFiles.read_raw(&started_temp)? != started_raw
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_recovery_prepared_mismatch",
                "A preserved Attempt-start index is not the canonical target.",
                json!({}),
            ));
        }
        require_started_attempt(stage, existing_attempt.is_some())?;
        work_feature::task::recheck_task_execution_context(
            sources.paths,
            sources.task_repository,
            &context,
        )?;
        if stage == 0 {
            if !lock_temp.is_file() {
                LocalFiles.create_new(&lock_temp, &locked_raw)?;
            }
            install_recovery_file(
                &lock_temp,
                &index_path,
                &locked_raw,
                &base_raw,
                "attempt_start_lock_installed",
            )?;
        }
        if existing_attempt.is_none() {
            let parent = attempt_path.parent().expect("Attempt parent");
            LocalFiles.create_directories(parent)?;
            LocalFiles.create_new(&attempt_path, &candidate_raw)?;
            if LocalFiles.read_raw(&attempt_path)? != candidate_raw {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "attempt_start_recovery_attempt_mismatch",
                    "The recovered Attempt bytes differ from the candidate.",
                    json!({}),
                ));
            }
        }
        if stage < 2 {
            if !started_temp.is_file() {
                LocalFiles.create_new(&started_temp, &started_raw)?;
            }
            install_recovery_file(
                &started_temp,
                &index_path,
                &started_raw,
                &locked_raw,
                "attempt_start_index_synchronized",
            )?;
        }
        for temporary in [&lock_temp, &started_temp] {
            if temporary.is_file() {
                fs::remove_file(temporary).map_err(|_| {
                    transaction_io("attempt_start_recovery_cleanup_failed", temporary)
                })?;
            }
        }
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::AttemptStartRecoveryResponse,
        >(
            json!({"schema":"work-attempt-start-recovery","task_id":target.task_id,
            "attempt_id":attempt_id,"attempt_path":attempt_relative,
            "index_path":format!("{}/index.json", target.execution_dir),
            "status":"recovered","lock_status":"held"}),
        ))
    }

    fn recover_correction_scoped(
        &self,
        input: CorrectionRecoveryInput<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        #[cfg(test)]
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&writer_path)?)
        };
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let index_path = storage_path(
            &self.project_root,
            &format!("{}/index.json", input.execution_dir),
        )?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{}/{}/{}/attempt.json",
                input.execution_dir, input.task_id, input.attempt_id
            ),
        )?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_path)?;
        if index_raw != input.index_before || attempt_raw != input.attempt_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed after validation.",
                json!({}),
            ));
        }
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The execution index is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::index::validate_execution_index(&index, &index_raw)
            .map_err(recovery_rule)?;
        let attempt = parse_json_contract(&attempt_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Attempt is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::attempt::validate_attempt_bytes(&attempt, &attempt_raw)
            .map_err(recovery_rule)?;
        if attempt["status"] == "in_progress"
            || attempt["attempt_id"] != input.attempt_id
            || attempt["task_id"] != input.task_id
        {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "correction_recovery_attempt_not_closed",
                "Correction recovery requires the matching closed Attempt.",
                json!({}),
            ));
        }
        let prefix = format!(
            "{}/.work-correction-{}-{}",
            input.execution_dir, input.task_id, input.correction_id
        );
        let artifact_temp = storage_path(&self.project_root, &format!("{prefix}-artifact.tmp"))?;
        let lock_temp = storage_path(&self.project_root, &format!("{prefix}-lock.tmp"))?;
        let index_temp = storage_path(&self.project_root, &format!("{prefix}-index.tmp"))?;
        let correction_relative = format!(
            "{}/{}/{}/corrections/{}.json",
            input.execution_dir, input.task_id, input.attempt_id, input.correction_id
        );
        let correction_path = storage_path(&self.project_root, &correction_relative)?;
        let artifact_raw = if artifact_temp.is_file() {
            LocalFiles.read_raw(&artifact_temp)?
        } else if correction_path.is_file() {
            LocalFiles.read_raw(&correction_path)?
        } else {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "correction_recovery_artifact_missing",
                "The approved Correction content is not preserved.",
                json!({}),
            ));
        };
        let artifact = parse_json_contract(&artifact_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Correction is invalid.",
                json!({}),
            )
        })?;
        if render_correction(&artifact).map_err(recovery_rule)? != artifact_raw
            || artifact["correction_id"] != input.correction_id
            || artifact["target_attempt_id"] != input.attempt_id
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_recovery_artifact_identity_mismatch",
                "The preserved Correction is not the canonical transaction artifact.",
                json!({}),
            ));
        }
        for field in [
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
            "task_instructions_sha256",
            "execute_instructions_sha256",
        ] {
            if artifact[field] != attempt[field] {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "correction_recovery_fingerprint_mismatch",
                    "The Correction fingerprints do not match the target Attempt.",
                    json!({"field":field}),
                ));
            }
        }
        let current_has_lock = index["lock"]["kind"] == "correction";
        let (base_index, transaction_lock) = if current_has_lock {
            let mut base = index.clone();
            base.as_object_mut().expect("index object").remove("lock");
            (base, index["lock"].clone())
        } else {
            if !lock_temp.is_file() {
                return Err(WorkError::new(
                    ExitCode::WorkflowState,
                    "correction_recovery_lock_target_missing",
                    "The canonical Correction lock target is not preserved.",
                    json!({}),
                ));
            }
            let raw = LocalFiles.read_raw(&lock_temp)?;
            let locked = parse_json_contract(&raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The Correction lock index is invalid.",
                    json!({}),
                )
            })?;
            work_operations::execution::index::validate_execution_index(&locked, &raw)
                .map_err(recovery_rule)?;
            (index.clone(), locked["lock"].clone())
        };
        if transaction_lock["kind"] != "correction"
            || transaction_lock["task_id"] != input.task_id
            || transaction_lock["attempt_id"] != input.attempt_id
            || transaction_lock["correction_id"] != input.correction_id
            || transaction_lock["execute_instructions_sha256"]
                != attempt["execute_instructions_sha256"]
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_recovery_lock_identity_mismatch",
                "The Correction lock does not match the preserved transaction.",
                json!({}),
            ));
        }
        let request = json!({"schema":"work-correction-create-request",
            "target_attempt_id":input.attempt_id,"field":artifact["field"],
            "correct_value":artifact["correct_value"],"reason":artifact["reason"],
            "invalidates_completion":transaction_lock["invalidates_completion"]});
        let candidates = build_correction_candidates(
            input.collection,
            &base_index,
            &attempt,
            input.task_id,
            &request,
            input.correction_id,
            artifact["created_at"].as_str().unwrap_or(""),
        )
        .map_err(recovery_rule)?;
        let locked_raw =
            work_operations::execution::index::render_execution_index(&candidates.locked_index)
                .expect("JSON index");
        let final_raw =
            work_operations::execution::index::render_execution_index(&candidates.final_index)
                .expect("JSON index");
        if transaction_lock["affected_task_ids"] != json!(candidates.affected_task_ids)
            || current_has_lock && index_raw != locked_raw
            || !current_has_lock && LocalFiles.read_raw(&lock_temp)? != locked_raw
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_recovery_lock_bytes_mismatch",
                "The Correction lock is not the unique canonical target.",
                json!({}),
            ));
        }
        if !index_temp.is_file() || LocalFiles.read_raw(&index_temp)? != final_raw {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_recovery_index_target_mismatch",
                "The canonical final index target is not preserved.",
                json!({}),
            ));
        }
        if correction_path.is_file() && LocalFiles.read_raw(&correction_path)? != artifact_raw
            || artifact_temp.is_file() && LocalFiles.read_raw(&artifact_temp)? != artifact_raw
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_recovery_existing_artifact_mismatch",
                "The preserved Correction bytes conflict.",
                json!({}),
            ));
        }
        if !current_has_lock {
            install_recovery_file(
                &lock_temp,
                &index_path,
                &locked_raw,
                &index_raw,
                "recovery_lock_installed",
            )?;
        }
        if !correction_path.is_file() {
            fs::create_dir_all(correction_path.parent().expect("correction parent")).map_err(
                |_| transaction_io("correction_recovery_directory_failed", &correction_path),
            )?;
            fs::hard_link(&artifact_temp, &correction_path).map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "correction_recovery_artifact_install_failed",
                    "The preserved Correction could not be installed exclusively.",
                    json!({"recovery_required":true,"transaction_stage":"recovery_lock_installed"}),
                )
            })?;
        }
        if artifact_temp.is_file() {
            fs::remove_file(&artifact_temp).map_err(|_| {
                transaction_io("correction_recovery_cleanup_failed", &artifact_temp)
            })?;
        }
        install_recovery_file(
            &index_temp,
            &index_path,
            &final_raw,
            &locked_raw,
            "recovery_index_synchronized",
        )?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","transaction":"correction",
            "task_id":input.task_id,"attempt_id":input.attempt_id,
            "correction_id":input.correction_id,"correction_path":correction_relative,
            "index_path":format!("{}/index.json", input.execution_dir),
            "affected_task_ids":candidates.affected_task_ids,
            "lock_status":"released","status":"recovered"}),
        ))
    }

    fn recover_deviation_record_scoped(
        &self,
        input: DeviationRecordRecoveryInput<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        #[cfg(test)]
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&writer_path)?)
        };
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let index_path = storage_path(
            &self.project_root,
            &format!("{}/index.json", input.execution_dir),
        )?;
        let attempt_relative = format!(
            "{}/{}/{}/attempt.json",
            input.execution_dir, input.task_id, input.attempt_id
        );
        let attempt_path = storage_path(&self.project_root, &attempt_relative)?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_path)?;
        if index_raw != input.index_before || attempt_raw != input.attempt_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed after validation.",
                json!({}),
            ));
        }
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The execution index JSON is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::index::validate_execution_index(&index, &index_raw)
            .map_err(recovery_rule)?;
        let record_id = index["lock"]["record_id"].as_str().ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_record_lock_required",
                "deviation_record recovery requires a reserved record.",
                json!({}),
            )
        })?;
        let attempt = parse_json_contract(&attempt_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Attempt JSON is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::attempt::validate_attempt_bytes(&attempt, &attempt_raw)
            .map_err(recovery_rule)?;
        let safe_record = record_id.replace('#', "-retry-");
        let temporary = storage_path(
            &self.project_root,
            &format!(
                "{}/.work-deviation-record-{}-{}-{safe_record}-attempt.tmp",
                input.execution_dir, input.task_id, input.attempt_id
            ),
        )?;
        if temporary.is_file() {
            let prepared_raw = LocalFiles.read_raw(&temporary)?;
            let prepared = parse_json_contract(&prepared_raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The prepared Attempt JSON is invalid.",
                    json!({}),
                )
            })?;
            validate_deviation_record_recovery(&attempt, &attempt_raw, &prepared, &prepared_raw)
                .map_err(recovery_rule)?;
            install_recovery_file(
                &temporary,
                &attempt_path,
                &prepared_raw,
                &attempt_raw,
                "deviation_record_attempt_update",
            )?;
            let installed_raw = LocalFiles.read_raw(&attempt_path)?;
            let installed = parse_json_contract(&installed_raw).map_err(|_| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "execution_recovery_write_mismatch",
                    "The recovered Attempt is invalid.",
                    json!({}),
                )
            })?;
            work_operations::execution::attempt::validate_attempt_bytes(&installed, &installed_raw)
                .map_err(recovery_rule)?;
        } else if attempt["execution_deviations"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "execution_recovery_deviation_missing",
                "No recorded or prepared execution deviation establishes recovery.",
                json!({}),
            ));
        }
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","status":"recovered",
            "transaction":"deviation_record","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":record_id,
            "lock_status":"record_reserved"}),
        ))
    }

    fn recover_command_correction_scoped(
        &self,
        input: CommandCorrectionRecoveryInput<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        #[cfg(test)]
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&writer_path)?)
        };
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let index_path = storage_path(
            &self.project_root,
            &format!("{}/index.json", input.execution_dir),
        )?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{}/{}/{}/attempt.json",
                input.execution_dir, input.task_id, input.attempt_id
            ),
        )?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_path)?;
        if index_raw != input.index_before || attempt_raw != input.attempt_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed after validation.",
                json!({}),
            ));
        }
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The execution index JSON is invalid.",
                json!({}),
            )
        })?;
        let (record_id, name) =
            command_correction_identity(&index, input.task_id, input.attempt_id)?;
        let temporary = storage_path(
            &self.project_root,
            &format!("{}/{}", input.execution_dir, name),
        )?;
        if !temporary.is_file() {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "execution_recovery_command_correction_file_required",
                "command_correction recovery requires its prepared index file.",
                json!({"expected":name}),
            ));
        }
        let prepared_raw = LocalFiles.read_raw(&temporary)?;
        let prepared = parse_json_contract(&prepared_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The prepared index JSON is invalid.",
                json!({}),
            )
        })?;
        validate_command_correction_recovery(CommandCorrectionRecovery {
            index: &index,
            index_raw: &index_raw,
            prepared: &prepared,
            prepared_raw: &prepared_raw,
            task: input.task,
        })
        .map_err(recovery_rule)?;
        if LocalFiles.read_raw(&attempt_path)? != attempt_raw {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed before publication.",
                json!({}),
            ));
        }
        install_recovery_file(
            &temporary,
            &index_path,
            &prepared_raw,
            &index_raw,
            "command_correction_lock_update",
        )?;
        let installed_raw = LocalFiles.read_raw(&index_path)?;
        let installed = parse_json_contract(&installed_raw).map_err(|_| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_write_mismatch",
                "The recovered index is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::index::validate_execution_index(&installed, &installed_raw)
            .map_err(recovery_rule)?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","status":"recovered",
            "transaction":"command_correction","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":record_id,
            "lock_status":"record_reserved"}),
        ))
    }

    fn run_prepared_command_scoped(
        &self,
        execution_dir: &str,
        approved_sha256: &str,
        recheck: impl FnOnce() -> Result<(Value, String), WorkError>,
        runner: &impl CommandRunner,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution_dir}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        let (preview, evidence) = recheck()?;
        let receipt = approved_receipt(&preview, execution_dir, approved_sha256)?;
        let started_path = storage_path(&self.project_root, &format!("{receipt}.started.json"))?;
        let finished_path = storage_path(&self.project_root, &format!("{receipt}.finished.json"))?;
        if started_path.exists() || finished_path.exists() {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_already_started",
                "This record already has execution evidence; never run it again.",
                json!({"receipt":receipt}),
            ));
        }
        let command = build_command_request(&preview)?;
        let started = build_command_started(&preview, &evidence);
        LocalFiles
            .create_new(&started_path, &command_json_bytes(&started))
            .map_err(|_| command_interrupted(receipt))?;
        let outcome = runner.run(&command);
        let completion = command_completion(&preview, &outcome);
        LocalFiles
            .create_new(&finished_path, &command_json_bytes(&completion.finished))
            .map_err(|_| command_interrupted(receipt))?;
        completion.into_result()
    }

    fn recover_attempt_close_scoped(
        &self,
        input: AttemptCloseRecoveryInput<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = input.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        let index_path = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                input.task_id, input.attempt_id
            ),
        )?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_path)?;
        if index_raw != input.index_before || attempt_raw != input.attempt_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed after validation.",
                json!({}),
            ));
        }
        let mut pending = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("execution_recovery_file_read_failed", &directory))?
        {
            let entry = entry
                .map_err(|_| transaction_io("execution_recovery_file_read_failed", &directory))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-") && name.ends_with(".tmp") && entry.path().is_file() {
                pending.push(name);
            }
        }
        pending.sort();
        validate_recovery_direction(
            "attempt_close",
            input.task_id,
            input.attempt_id,
            &pending,
            input.index,
            input.attempt,
        )
        .map_err(recovery_rule)?;
        let prefix = format!(
            "{execution}/.work-attempt-close-{}-{}",
            input.task_id, input.attempt_id
        );
        let attempt_temp = storage_path(&self.project_root, &format!("{prefix}-attempt.tmp"))?;
        let index_temp = storage_path(&self.project_root, &format!("{prefix}-index.tmp"))?;
        let prepared = if attempt_temp.exists() {
            let raw = LocalFiles.read_raw(&attempt_temp)?;
            let value = parse_json_contract(&raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The prepared Attempt JSON is invalid.",
                    json!({}),
                )
            })?;
            Some((value, raw))
        } else {
            None
        };
        let target = validate_attempt_close_recovery(AttemptCloseRecovery {
            index: input.index,
            index_raw: &index_raw,
            attempt: input.attempt,
            attempt_raw: &attempt_raw,
            prepared_attempt: prepared
                .as_ref()
                .map(|(value, raw)| (value, raw.as_slice())),
            task: input.task,
        })
        .map_err(recovery_rule)?;
        prepare_recovery_file(&index_temp, &target.index)?;
        if let Some(expected) = target.attempt {
            install_recovery_file(
                &attempt_temp,
                &attempt_path,
                &expected,
                &attempt_raw,
                "attempt_close_attempt_update",
            )?;
        }
        install_recovery_file(
            &index_temp,
            &index_path,
            &target.index,
            &index_raw,
            "attempt_close_index_update",
        )?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","status":"recovered",
            "transaction":"attempt_close","task_id":input.task_id,
            "attempt_id":input.attempt_id,"attempt_status":target.status,
            "attempt_path":format!("{execution}/{}/{}/attempt.json", input.task_id, input.attempt_id),
            "index_path":format!("{execution}/index.json"),"lock_status":"released"}),
        ))
    }

    fn recover_record_finish_scoped(
        &self,
        input: RecordFinishRecoveryInput<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = input.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        let index_path = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                input.task_id, input.attempt_id
            ),
        )?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_path)?;
        if index_raw != input.index_before || attempt_raw != input.attempt_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed after validation.",
                json!({}),
            ));
        }
        let mut pending = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("execution_recovery_file_read_failed", &directory))?
        {
            let entry = entry
                .map_err(|_| transaction_io("execution_recovery_file_read_failed", &directory))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-") && name.ends_with(".tmp") && entry.path().is_file() {
                pending.push(name);
            }
        }
        pending.sort();
        validate_recovery_direction(
            "record_finish",
            input.task_id,
            input.attempt_id,
            &pending,
            input.index,
            input.attempt,
        )
        .map_err(recovery_rule)?;
        let record_id = input.index["lock"]["record_id"].as_str().unwrap_or("");
        let safe_record = record_id.replace('#', "-retry-");
        let prefix = format!(
            "{execution}/.work-record-finish-{}-{}-{safe_record}",
            input.task_id, input.attempt_id
        );
        let attempt_temp = storage_path(&self.project_root, &format!("{prefix}-attempt.tmp"))?;
        let index_temp = storage_path(&self.project_root, &format!("{prefix}-index.tmp"))?;
        let prepared = if attempt_temp.exists() {
            let raw = LocalFiles.read_raw(&attempt_temp)?;
            let value = parse_json_contract(&raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The prepared Attempt JSON is invalid.",
                    json!({}),
                )
            })?;
            Some((value, raw))
        } else {
            None
        };
        let target = validate_record_finish_recovery(RecordFinishRecovery {
            index: input.index,
            index_raw: &index_raw,
            attempt: input.attempt,
            attempt_raw: &attempt_raw,
            prepared_attempt: prepared
                .as_ref()
                .map(|(value, raw)| (value, raw.as_slice())),
            task: input.task,
            authorization_evidence: input.authorization_evidence,
        })
        .map_err(recovery_rule)?;
        prepare_recovery_file(&index_temp, &target.index)?;
        if let Some(expected) = target.attempt {
            install_recovery_file(
                &attempt_temp,
                &attempt_path,
                &expected,
                &attempt_raw,
                "record_finish_attempt_update",
            )?;
        }
        install_recovery_file(
            &index_temp,
            &index_path,
            &target.index,
            &index_raw,
            "record_finish_index_update",
        )?;
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","status":"recovered",
            "transaction":"record_finish","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":target.record_id,
            "attempt_path":format!("{execution}/{}/{}/attempt.json", input.task_id, input.attempt_id),
            "index_path":format!("{execution}/index.json"),"lock_status":"attempt_held"}),
        ))
    }

    fn recover_record_begin_scoped(
        &self,
        input: RecordBeginRecoveryInput<'_>,
        runtime_held: bool,
    ) -> Result<Value, WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = storage_path(&self.project_root, input.execution_dir)?;
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let index_path = storage_path(
            &self.project_root,
            &format!("{}/index.json", input.execution_dir),
        )?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{}/{}/{}/attempt.json",
                input.execution_dir, input.task_id, input.attempt_id
            ),
        )?;
        let prefix = format!(".work-record-begin-{}-{}-", input.task_id, input.attempt_id);
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&execution)
            .map_err(|_| transaction_io("execution_recovery_file_read_failed", &execution))?
        {
            let entry = entry
                .map_err(|_| transaction_io("execution_recovery_file_read_failed", &execution))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".tmp") && entry.path().is_file() {
                candidates.push(name);
            }
        }
        candidates.sort();
        if candidates.len() != 1 {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "execution_recovery_record_begin_file_required",
                "record_begin recovery requires exactly one prepared index file.",
                json!({"files":candidates}),
            ));
        }
        let name = &candidates[0];
        let temporary = storage_path(
            &self.project_root,
            &format!("{}/{name}", input.execution_dir),
        )?;
        let index_raw = LocalFiles.read_raw(&index_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_path)?;
        if index_raw != input.index_before || attempt_raw != input.attempt_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed after validation.",
                json!({}),
            ));
        }
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The execution index JSON is invalid.",
                json!({}),
            )
        })?;
        let prepared_raw = LocalFiles.read_raw(&temporary)?;
        let prepared = parse_json_contract(&prepared_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The prepared index JSON is invalid.",
                json!({}),
            )
        })?;
        let record_id = validate_record_begin_recovery(RecordBeginRecovery {
            index: &index,
            index_raw: &index_raw,
            prepared: &prepared,
            prepared_raw: &prepared_raw,
            attempt: input.attempt,
            task: input.task,
            task_id: input.task_id,
            attempt_id: input.attempt_id,
            temporary_name: name,
        })
        .map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        if LocalFiles.read_raw(&index_path)? != index_raw
            || LocalFiles.read_raw(&attempt_path)? != attempt_raw
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_source_changed",
                "A recovery source changed before publication.",
                json!({}),
            ));
        }
        LocalFiles.replace(&temporary, &index_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "execution_recovery_replace_failed",
                "The prepared record-begin index could not be installed.",
                json!({"path":temporary.to_string_lossy(),"recovery_required":true,
                "transaction_stage":"record_begin_lock_update"}),
            )
        })?;
        let installed_raw = LocalFiles.read_raw(&index_path)?;
        let installed = parse_json_contract(&installed_raw).map_err(|_| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_write_mismatch",
                "The recovered index is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::index::validate_execution_index(&installed, &installed_raw)
            .map_err(|issue| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            })?;
        if installed_raw != prepared_raw {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_write_mismatch",
                "The installed index differs from the prepared bytes.",
                json!({}),
            ));
        }
        Ok(work_model::execution::response::verified::<
            work_model::execution::response::ExecutionRecoveryResponse,
        >(
            json!({"schema":"work-execution-recovery","status":"recovered",
            "transaction":"record_begin","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":record_id,
            "lock_status":"record_reserved"}),
        ))
    }

    fn publish_attempt_start_scoped(
        &self,
        publication: &AttemptStartPublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        let task = publication.task_id;
        let attempt = publication.attempt_id;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let index = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!("{execution}/{task}/{attempt}/attempt.json"),
        )?;
        let lock_temp = storage_path(
            &self.project_root,
            &format!("{execution}/.work-attempt-start-{task}-{attempt}-lock.tmp"),
        )?;
        let started_temp = storage_path(
            &self.project_root,
            &format!("{execution}/.work-attempt-start-{task}-{attempt}-started.tmp"),
        )?;
        if lock_temp.exists() || started_temp.exists() || attempt_path.exists() {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_transaction_present",
                "Attempt-start transaction evidence already exists.",
                json!({}),
            ));
        }
        let records = self.git_status()?;
        let records = if runtime_held {
            work_operations::execution::worktree::runtime_worktree_records(&records)
        } else {
            records
        };
        let actual_snapshot = inspect_records(&records, execution, task, &json!({}), &[]);
        if actual_snapshot["snapshot_sha256"] != publication.expected_snapshot {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_worktree_snapshot_changed",
                "The Git worktree snapshot changed after review.",
                json!({"expected":publication.expected_snapshot,
                    "actual":actual_snapshot["snapshot_sha256"]}),
            ));
        }
        if LocalFiles.read_raw(&index)? != publication.index_before {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_start_index_changed",
                "The execution index changed before Attempt start.",
                json!({}),
            ));
        }
        let operation = || -> Result<(), WorkError> {
            LocalFiles.create_new(&lock_temp, publication.locked_index)?;
            if LocalFiles.read_raw(&index)? != publication.index_before {
                return Err(transaction_conflict());
            }
            LocalFiles.replace(&lock_temp, &index)?;
            if LocalFiles.read_raw(&index)? != publication.locked_index {
                return Err(transaction_conflict());
            }
            let parent = attempt_path.parent().ok_or_else(|| {
                transaction_io("attempt_start_attempt_directory_failed", &attempt_path)
            })?;
            LocalFiles
                .create_directories(parent)
                .map_err(|_| transaction_io("attempt_start_attempt_directory_failed", parent))?;
            LocalFiles.create_new(&attempt_path, publication.attempt)?;
            if LocalFiles.read_raw(&attempt_path)? != publication.attempt {
                return Err(transaction_conflict());
            }
            LocalFiles.create_new(&started_temp, publication.started_index)?;
            if LocalFiles.read_raw(&index)? != publication.locked_index {
                return Err(transaction_conflict());
            }
            LocalFiles.replace(&started_temp, &index)?;
            if LocalFiles.read_raw(&index)? != publication.started_index {
                return Err(transaction_conflict());
            }
            Ok(())
        };
        operation().map_err(|mut failure| {
            let stage = if started_temp.exists() {
                "started_index_prepared"
            } else if attempt_path.exists() {
                "attempt_created"
            } else if LocalFiles.read_raw(&index).ok().as_deref() == Some(publication.locked_index)
            {
                "lock_installed"
            } else if lock_temp.exists() {
                "lock_index_prepared"
            } else {
                "not_started"
            };
            if stage != "not_started" {
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("attempt_id".into(), json!(attempt));
                details.insert("recovery_required".into(), json!(true));
                details.insert("transaction_stage".into(), json!(stage));
                failure.details = json!(details);
            }
            failure
        })
    }

    fn publish_deviation_record_scoped(
        &self,
        publication: &DeviationRecordPublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        let mut pending = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("deviation_record_transaction_scan_failed", &directory))?
        {
            let entry = entry.map_err(|_| {
                transaction_io("deviation_record_transaction_scan_failed", &directory)
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-deviation-record-")
                && name.ends_with(".tmp")
                && entry.path().is_file()
            {
                pending.push(name);
            }
        }
        pending.sort();
        if !pending.is_empty() {
            return Err(WorkError::new(
                ExitCode::LockConflict,
                "deviation_record_transaction_present",
                "A deviation-record transaction already requires recovery.",
                json!({"files":pending,"recovery_required":true}),
            ));
        }
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                publication.task_id, publication.attempt_id
            ),
        )?;
        let check_sources = || -> Result<(), WorkError> {
            if LocalFiles.read_raw(&attempt_path)? != publication.attempt_before {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "deviation_record_source_changed",
                    "The Attempt changed before recording the deviation.",
                    json!({}),
                ));
            }
            for (path, raw) in publication.sources {
                if LocalFiles.read_raw(&storage_path(&self.project_root, path)?)? != *raw {
                    return Err(WorkError::new(
                        ExitCode::ArtifactIntegrity,
                        "deviation_record_source_changed",
                        "A deviation source changed before publication.",
                        json!({"path":path}),
                    ));
                }
            }
            Ok(())
        };
        check_sources()?;
        let safe_record = publication.record_id.replace('#', "-retry-");
        let temporary = storage_path(
            &self.project_root,
            &format!(
                "{execution}/.work-deviation-record-{}-{}-{safe_record}-attempt.tmp",
                publication.task_id, publication.attempt_id
            ),
        )?;
        LocalFiles
            .create_new(&temporary, publication.attempt_after)
            .map_err(|_| transaction_io("deviation_record_prepare_failed", &temporary))?;
        if let Err(mut failure) = check_sources() {
            failure.details =
                json!({"recovery_required":true,"transaction_stage":"attempt_update_prepared"});
            return Err(failure);
        }
        LocalFiles.replace(&temporary, &attempt_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "deviation_record_replace_failed",
                "The prepared deviation record could not be installed.",
                json!({"recovery_required":true,"transaction_stage":"attempt_update_prepared"}),
            )
        })?;
        if LocalFiles.read_raw(&attempt_path)? != publication.attempt_after {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "deviation_record_write_mismatch",
                "The installed Attempt differs from the prepared deviation record.",
                json!({}),
            ));
        }
        Ok(())
    }

    fn publish_correction_scoped(
        &self,
        publication: &CorrectionPublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let writer_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&writer_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("correction_create_transaction_scan_failed", &directory))?
        {
            let entry = entry.map_err(|_| {
                transaction_io("correction_create_transaction_scan_failed", &directory)
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-") && name.ends_with(".tmp") && entry.path().is_file() {
                return Err(WorkError::new(
                    ExitCode::LockConflict,
                    "correction_create_transaction_present",
                    "An execution transaction already requires recovery.",
                    json!({"file":name}),
                ));
            }
        }
        let index_path = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                publication.task_id, publication.attempt_id
            ),
        )?;
        if LocalFiles.read_raw(&index_path)? != publication.index_before
            || LocalFiles.read_raw(&attempt_path)? != publication.attempt_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_create_source_changed",
                "Execution state changed before Correction publication.",
                json!({}),
            ));
        }
        let prefix = format!(
            "{execution}/.work-correction-{}-{}",
            publication.task_id, publication.correction_id
        );
        let artifact_temp = storage_path(&self.project_root, &format!("{prefix}-artifact.tmp"))?;
        let lock_temp = storage_path(&self.project_root, &format!("{prefix}-lock.tmp"))?;
        let index_temp = storage_path(&self.project_root, &format!("{prefix}-index.tmp"))?;
        for (path, bytes) in [
            (&artifact_temp, publication.artifact),
            (&lock_temp, publication.locked_index),
            (&index_temp, publication.final_index),
        ] {
            LocalFiles.create_new(path, bytes).map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "correction_create_prepare_failed",
                    "A Correction transaction file could not be prepared.",
                    json!({"path":path.to_string_lossy(),"recovery_required":true,
                    "transaction_stage":"artifact_prepared"}),
                )
            })?;
        }
        if LocalFiles.read_raw(&index_path)? != publication.index_before
            || LocalFiles.read_raw(&attempt_path)? != publication.attempt_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_create_source_changed",
                "Execution state changed during Correction preparation.",
                json!({"recovery_required":true,"transaction_stage":"index_prepared"}),
            ));
        }
        LocalFiles.replace(&lock_temp, &index_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "correction_create_lock_install_failed",
                "The prepared Correction lock could not be installed.",
                json!({"recovery_required":true,"transaction_stage":"lock_prepared"}),
            )
        })?;
        if LocalFiles.read_raw(&index_path)? != publication.locked_index {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_create_lock_mismatch",
                "The installed Correction lock differs from the prepared bytes.",
                json!({"recovery_required":true,"transaction_stage":"lock_installed"}),
            ));
        }
        let correction_relative = format!(
            "{execution}/{}/{}/corrections/{}.json",
            publication.task_id, publication.attempt_id, publication.correction_id
        );
        let correction_path = storage_path(&self.project_root, &correction_relative)?;
        fs::create_dir_all(correction_path.parent().expect("correction parent"))
            .map_err(|_| WorkError::new(ExitCode::IoFailure,
                "correction_create_directory_failed",
                "The Correction directory could not be created.",
                json!({"recovery_required":true,"transaction_stage":"correction_directory_prepared"})))?;
        fs::hard_link(&artifact_temp, &correction_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "correction_create_artifact_install_failed",
                "The prepared Correction artifact could not be installed exclusively.",
                json!({"recovery_required":true,"transaction_stage":"lock_installed"}),
            )
        })?;
        if LocalFiles.read_raw(&correction_path)? != publication.artifact {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_create_artifact_mismatch",
                "The installed Correction artifact differs from the prepared bytes.",
                json!({"recovery_required":true,"transaction_stage":"correction_installed"}),
            ));
        }
        fs::remove_file(&artifact_temp).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "correction_create_cleanup_failed",
                "The installed Correction transaction marker could not be removed.",
                json!({"recovery_required":true,"transaction_stage":"correction_installed"}),
            )
        })?;
        LocalFiles.replace(&index_temp, &index_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "correction_create_index_install_failed",
                "The corrected execution index could not be installed.",
                json!({"recovery_required":true,"transaction_stage":"correction_installed"}),
            )
        })?;
        if LocalFiles.read_raw(&index_path)? != publication.final_index {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "correction_create_index_mismatch",
                "The corrected execution index differs from the prepared bytes.",
                json!({"recovery_required":true,"transaction_stage":"index_synchronized"}),
            ));
        }
        Ok(())
    }

    fn publish_command_correction_scoped(
        &self,
        publication: &CommandCorrectionPublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        let mut pending = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("command_correction_transaction_scan_failed", &directory))?
        {
            let entry = entry.map_err(|_| {
                transaction_io("command_correction_transaction_scan_failed", &directory)
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-command-correction-")
                && name.ends_with(".tmp")
                && entry.path().is_file()
            {
                pending.push(name);
            }
        }
        pending.sort();
        if !pending.is_empty() {
            return Err(WorkError::new(
                ExitCode::LockConflict,
                "command_correction_transaction_present",
                "A command-correction transaction already requires recovery.",
                json!({"files":pending,"recovery_required":true}),
            ));
        }
        let index_path = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                publication.task_id, publication.attempt_id
            ),
        )?;
        if LocalFiles.read_raw(&index_path)? != publication.index_before
            || LocalFiles.read_raw(&attempt_path)? != publication.attempt_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "command_correction_source_changed",
                "Execution state changed before command correction.",
                json!({}),
            ));
        }
        let safe_record = publication.record_id.replace('#', "-retry-");
        let temporary = storage_path(
            &self.project_root,
            &format!(
                "{execution}/.work-command-correction-{}-{}-{safe_record}.tmp",
                publication.task_id, publication.attempt_id
            ),
        )?;
        LocalFiles
            .create_new(&temporary, publication.index_after)
            .map_err(|_| transaction_io("command_correction_prepare_failed", &temporary))?;
        if LocalFiles.read_raw(&index_path)? != publication.index_before
            || LocalFiles.read_raw(&attempt_path)? != publication.attempt_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "command_correction_source_changed",
                "Execution state changed during command correction.",
                json!({"recovery_required":true,"transaction_stage":"lock_update_prepared"}),
            ));
        }
        LocalFiles.replace(&temporary, &index_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "command_correction_replace_failed",
                "The prepared command correction could not be installed.",
                json!({"recovery_required":true,"transaction_stage":"lock_update_prepared"}),
            )
        })?;
        if LocalFiles.read_raw(&index_path)? != publication.index_after {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "command_correction_write_mismatch",
                "The stored execution index does not match the prepared bytes.",
                json!({}),
            ));
        }
        Ok(())
    }

    fn publish_record_begin_scoped(
        &self,
        publication: &RecordBeginPublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let index_path = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_path = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                publication.task_id, publication.attempt_id
            ),
        )?;
        let safe_record = publication.record_id.replace('#', "-retry-");
        let temporary = storage_path(
            &self.project_root,
            &format!(
                "{execution}/.work-record-begin-{}-{}-{safe_record}.tmp",
                publication.task_id, publication.attempt_id
            ),
        )?;
        let execution_path = storage_path(&self.project_root, execution)?;
        let mut pending = Vec::new();
        for entry in fs::read_dir(&execution_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "record_begin_transaction_scan_failed",
                "The execution transaction directory could not be scanned.",
                json!({"path":execution_path.to_string_lossy()}),
            )
        })? {
            let entry = entry.map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "record_begin_transaction_scan_failed",
                    "The execution transaction directory could not be scanned.",
                    json!({"path":execution_path.to_string_lossy()}),
                )
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-record-begin-")
                && name.ends_with(".tmp")
                && entry.path().is_file()
            {
                pending.push(name);
            }
        }
        pending.sort();
        if !pending.is_empty() {
            return Err(WorkError::new(
                ExitCode::LockConflict,
                "record_begin_transaction_present",
                "A record-begin transaction already requires recovery.",
                json!({"files":pending,"recovery_required":true}),
            ));
        }
        if LocalFiles.read_raw(&index_path)? != publication.index_before
            || LocalFiles.read_raw(&attempt_path)? != publication.attempt_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "record_begin_index_changed",
                "The execution state changed before record begin.",
                json!({}),
            ));
        }
        LocalFiles
            .create_new(&temporary, publication.index_after)
            .map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "record_begin_prepare_failed",
                    "The record-begin index update could not be prepared.",
                    json!({"path":temporary.to_string_lossy()}),
                )
            })?;
        if LocalFiles.read_raw(&index_path)? != publication.index_before
            || LocalFiles.read_raw(&attempt_path)? != publication.attempt_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "record_begin_index_changed",
                "The execution index changed during record begin.",
                json!({"path":temporary.to_string_lossy(),"recovery_required":true,
                    "transaction_stage":"lock_update_prepared"}),
            ));
        }
        LocalFiles.replace(&temporary, &index_path).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "record_begin_replace_failed",
                "The prepared record-begin index could not be installed.",
                json!({"path":temporary.to_string_lossy(),"recovery_required":true,
                    "transaction_stage":"lock_update_prepared"}),
            )
        })?;
        if LocalFiles.read_raw(&index_path)? != publication.index_after {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "record_begin_write_mismatch",
                "The stored execution index does not match the prepared bytes.",
                json!({}),
            ));
        }
        Ok(())
    }

    fn publish_record_finish_scoped(
        &self,
        publication: &RecordFinishPublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        let mut pending = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("record_finish_transaction_scan_failed", &directory))?
        {
            let entry = entry
                .map_err(|_| transaction_io("record_finish_transaction_scan_failed", &directory))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-record-finish-")
                && name.ends_with(".tmp")
                && entry.path().is_file()
            {
                pending.push(name);
            }
        }
        pending.sort();
        if !pending.is_empty() {
            return Err(WorkError::new(
                ExitCode::LockConflict,
                "record_finish_transaction_present",
                "A record-finish transaction already requires recovery.",
                json!({"files":pending,"recovery_required":true}),
            ));
        }
        let attempt = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                publication.task_id, publication.attempt_id
            ),
        )?;
        let index = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let safe_record = publication.record_id.replace('#', "-retry-");
        let prefix = format!(
            "{execution}/.work-record-finish-{}-{}-{safe_record}",
            publication.task_id, publication.attempt_id
        );
        let attempt_temp = storage_path(&self.project_root, &format!("{prefix}-attempt.tmp"))?;
        let index_temp = storage_path(&self.project_root, &format!("{prefix}-index.tmp"))?;
        if LocalFiles.read_raw(&attempt)? != publication.attempt_before
            || LocalFiles.read_raw(&index)? != publication.index_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "record_finish_source_changed",
                "A transaction source changed before replacement.",
                json!({}),
            ));
        }
        let publish = || -> Result<(), WorkError> {
            LocalFiles
                .create_new(&attempt_temp, publication.attempt_after)
                .map_err(|_| transaction_io("record_finish_prepare_failed", &attempt_temp))?;
            if LocalFiles.read_raw(&attempt)? != publication.attempt_before
                || LocalFiles.read_raw(&index)? != publication.index_before
            {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "record_finish_source_changed",
                    "A transaction source changed before replacement.",
                    json!({}),
                ));
            }
            LocalFiles
                .replace(&attempt_temp, &attempt)
                .map_err(|_| transaction_io("record_finish_replace_failed", &attempt_temp))?;
            if LocalFiles.read_raw(&attempt)? != publication.attempt_after {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "record_finish_write_mismatch",
                    "The installed record-finish bytes do not match the prepared bytes.",
                    json!({}),
                ));
            }
            LocalFiles
                .create_new(&index_temp, publication.index_after)
                .map_err(|_| transaction_io("record_finish_prepare_failed", &index_temp))?;
            if LocalFiles.read_raw(&index)? != publication.index_before {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "record_finish_source_changed",
                    "A transaction source changed before replacement.",
                    json!({}),
                ));
            }
            LocalFiles
                .replace(&index_temp, &index)
                .map_err(|_| transaction_io("record_finish_replace_failed", &index_temp))?;
            if LocalFiles.read_raw(&index)? != publication.index_after {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "record_finish_write_mismatch",
                    "The installed record-finish bytes do not match the prepared bytes.",
                    json!({}),
                ));
            }
            Ok(())
        };
        publish().map_err(|mut failure| {
            let stage = if index_temp.exists() {
                "lock_update_prepared"
            } else if LocalFiles.read_raw(&attempt).ok().as_deref()
                == Some(publication.attempt_after)
            {
                "attempt_updated"
            } else if attempt_temp.exists() {
                "attempt_update_prepared"
            } else {
                "not_started"
            };
            if stage != "not_started" {
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("recovery_required".into(), json!(true));
                details.insert("transaction_stage".into(), json!(stage));
                failure.details = json!(details);
            }
            failure
        })
    }

    fn publish_attempt_close_scoped(
        &self,
        publication: &AttemptClosePublication<'_>,
        runtime_held: bool,
    ) -> Result<(), WorkError> {
        require_execution_writer_context(runtime_held)?;
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        #[cfg(test)]
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        #[cfg(test)]
        let _writer = if runtime_held {
            None
        } else {
            Some(LocalWriterLock.acquire(&lock_path)?)
        };
        require_no_spec_update(&self.project_root, execution, None)?;
        let directory = storage_path(&self.project_root, execution)?;
        let mut pending = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("attempt_close_transaction_scan_failed", &directory))?
        {
            let entry = entry
                .map_err(|_| transaction_io("attempt_close_transaction_scan_failed", &directory))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-") && name.ends_with(".tmp") && entry.path().is_file() {
                pending.push(name);
            }
        }
        pending.sort();
        if !pending.is_empty() {
            return Err(WorkError::new(
                ExitCode::LockConflict,
                "attempt_close_transaction_present",
                "An execution transaction already requires recovery.",
                json!({"files":pending,"recovery_required":true}),
            ));
        }
        let attempt = storage_path(
            &self.project_root,
            &format!(
                "{execution}/{}/{}/attempt.json",
                publication.task_id, publication.attempt_id
            ),
        )?;
        let index = storage_path(&self.project_root, &format!("{execution}/index.json"))?;
        let attempt_temp = storage_path(
            &self.project_root,
            &format!(
                "{execution}/.work-attempt-close-{}-{}-attempt.tmp",
                publication.task_id, publication.attempt_id
            ),
        )?;
        let index_temp = storage_path(
            &self.project_root,
            &format!(
                "{execution}/.work-attempt-close-{}-{}-index.tmp",
                publication.task_id, publication.attempt_id
            ),
        )?;
        if LocalFiles.read_raw(&attempt)? != publication.attempt_before
            || LocalFiles.read_raw(&index)? != publication.index_before
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "attempt_close_source_changed",
                "A source artifact changed during Attempt close.",
                json!({}),
            ));
        }
        let publish = || -> Result<(), WorkError> {
            LocalFiles
                .create_new(&attempt_temp, publication.attempt_after)
                .map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "attempt_close_prepare_failed",
                        "The attempt-close update could not be prepared.",
                        json!({"path":attempt_temp.to_string_lossy()}),
                    )
                })?;
            if LocalFiles.read_raw(&attempt)? != publication.attempt_before
                || LocalFiles.read_raw(&index)? != publication.index_before
            {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "attempt_close_source_changed",
                    "A source artifact changed during Attempt close.",
                    json!({}),
                ));
            }
            LocalFiles.replace(&attempt_temp, &attempt).map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "attempt_close_replace_failed",
                    "The prepared attempt-close update could not be installed.",
                    json!({}),
                )
            })?;
            if LocalFiles.read_raw(&attempt)? != publication.attempt_after {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "attempt_close_stored_bytes_mismatch",
                    "The installed attempt-close bytes do not match the prepared bytes.",
                    json!({}),
                ));
            }
            LocalFiles
                .create_new(&index_temp, publication.index_after)
                .map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "attempt_close_prepare_failed",
                        "The attempt-close update could not be prepared.",
                        json!({"path":index_temp.to_string_lossy()}),
                    )
                })?;
            if LocalFiles.read_raw(&index)? != publication.index_before {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "attempt_close_source_changed",
                    "A source artifact changed during Attempt close.",
                    json!({}),
                ));
            }
            LocalFiles.replace(&index_temp, &index).map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "attempt_close_replace_failed",
                    "The prepared attempt-close update could not be installed.",
                    json!({}),
                )
            })?;
            if LocalFiles.read_raw(&index)? != publication.index_after {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "attempt_close_stored_bytes_mismatch",
                    "The installed attempt-close bytes do not match the prepared bytes.",
                    json!({}),
                ));
            }
            Ok(())
        };
        publish().map_err(|mut failure| {
            let stage = if index_temp.exists() {
                "index_update_prepared"
            } else if LocalFiles.read_raw(&attempt).ok().as_deref()
                == Some(publication.attempt_after)
            {
                "attempt_updated"
            } else if attempt_temp.exists() {
                "attempt_update_prepared"
            } else {
                "not_started"
            };
            if stage != "not_started" {
                let mut details = failure.details.as_object().cloned().unwrap_or_default();
                details.insert("recovery_required".into(), json!(true));
                details.insert("transaction_stage".into(), json!(stage));
                failure.details = json!(details);
            }
            failure
        })
    }

    fn validate_runtime_context(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
    ) -> Result<(), WorkError> {
        let root = self.project_root.canonicalize().map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "execution_writer_root",
                "The project root could not be resolved.",
                json!({}),
            )
        })?;
        if root != context.writer().canonical_project_root {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_root_mismatch",
                "Writer context belongs to another project.",
                json!({}),
            ));
        }
        Ok(())
    }

    /// Candidate scope for all verified execution publication and recovery adapters.
    pub fn with_runtime_execution_writer<T>(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        write: impl FnOnce(&work_model::runtime::RuntimeOwner) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        self.validate_runtime_context(context)?;
        crate::writer_lock::require_no_legacy_locks(
            context.writer(),
            Some(context.target().execution_dir),
        )?;
        work_feature::ports::with_runtime_writer(
            &LocalWriterLock,
            context.writer(),
            work_model::runtime::LockClass::Execution,
            write,
        )
    }

    pub fn recover_attempt_start_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: AttemptStartRecoveryRequest<'_>,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        self.recover_attempt_start_from_project_scoped(sources, input, false)
    }

    pub fn recover_execution_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        target: ExecutionProjectTarget<'_>,
        request: &Value,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        require_execution_writer_context(false)?;
        require_legacy_recovery_input()?;
        self.recover_legacy_execution_from_project(sources, target, request)
    }

    pub(crate) fn recover_legacy_execution_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        target: ExecutionProjectTarget<'_>,
        request: &Value,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        work_operations::execution::requests::validate_legacy_recovery_request(request, false)
            .map_err(recovery_rule)?;
        self.check_recovery_idle(target.execution_dir)?;
        let inventory = self.temporary_names(target.execution_dir)?;
        require_recovery_file_set(request, &inventory)?;
        let context = load_task_execution_context(
            sources.instructions,
            sources.skills,
            sources.paths,
            sources.task_repository,
            sources.skill_roots,
            target.task_path,
            target.task_id,
        )?;
        require_recovery_artifact_paths(&context.contract, target.task_path, target.execution_dir)?;
        let index_path = format!("{}/index.json", target.execution_dir);
        let index_raw = self.read_index(&index_path)?;
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The execution index is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::index::validate_execution_index(&index, &index_raw)
            .map_err(recovery_rule)?;
        let attempt_id = request["attempt_id"].as_str().unwrap_or("");
        let attempt_path = format!(
            "{}/{}/{attempt_id}/attempt.json",
            target.execution_dir, target.task_id
        );
        let attempt_raw = self.read_attempt(&attempt_path)?;
        let attempt = parse_json_contract(&attempt_raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Attempt is invalid.",
                json!({}),
            )
        })?;
        work_operations::execution::attempt::validate_attempt_bytes(&attempt, &attempt_raw)
            .map_err(recovery_rule)?;
        validate_execution_identity(
            &context.collection,
            &context.validation,
            &index,
            &attempt,
            target.task_id,
        )
        .map_err(recovery_rule)?;
        let transaction = request["transaction"].as_str().unwrap_or("");
        let direction = validate_recovery_direction(
            transaction,
            target.task_id,
            attempt_id,
            &inventory,
            &index,
            &attempt,
        )
        .map_err(recovery_rule)?;
        let task = context.contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == target.task_id)
            .ok_or_else(|| {
                WorkError::new(
                    ExitCode::Contract,
                    "execution_recovery_task_identity",
                    "Unknown TASK ID.",
                    json!({}),
                )
            })?;
        work_feature::task::recheck_task_execution_context(
            sources.paths,
            sources.task_repository,
            &context,
        )?;
        match transaction {
            "record_begin" => self.recover_record_begin(RecordBeginRecoveryInput {
                execution_dir: target.execution_dir,
                task_id: target.task_id,
                attempt_id,
                task,
                attempt: &attempt,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
            }),
            "command_correction" => {
                self.recover_command_correction(CommandCorrectionRecoveryInput {
                    execution_dir: target.execution_dir,
                    task_id: target.task_id,
                    attempt_id,
                    task,
                    index_before: &index_raw,
                    attempt_before: &attempt_raw,
                })
            }
            "record_finish" => self.recover_record_finish(RecordFinishRecoveryInput {
                execution_dir: target.execution_dir,
                task_id: target.task_id,
                attempt_id,
                task,
                attempt: &attempt,
                index: &index,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
                authorization_evidence: request["authorization_evidence"].as_str(),
            }),
            "attempt_close" => self.recover_attempt_close(AttemptCloseRecoveryInput {
                execution_dir: target.execution_dir,
                task_id: target.task_id,
                attempt_id,
                task,
                attempt: &attempt,
                index: &index,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
            }),
            "deviation_record" => self.recover_deviation_record(DeviationRecordRecoveryInput {
                execution_dir: target.execution_dir,
                task_id: target.task_id,
                attempt_id,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
            }),
            "correction" => {
                let full = load_collection(
                    sources.instructions,
                    sources.skills,
                    sources.paths,
                    sources.task_repository,
                    sources.skill_roots,
                    target.task_path,
                )?;
                work_feature::task::recheck_task_execution_context(
                    sources.paths,
                    sources.task_repository,
                    &context,
                )?;
                let correction_id = direction["correction_id"].as_str().unwrap_or("");
                self.recover_correction(CorrectionRecoveryInput {
                    execution_dir: target.execution_dir,
                    task_id: target.task_id,
                    attempt_id,
                    correction_id,
                    collection: &full["collection_contract"],
                    index_before: &index_raw,
                    attempt_before: &attempt_raw,
                })
            }
            _ => unreachable!("validated recovery transaction"),
        }
    }

    pub fn recover_correction(
        &self,
        input: CorrectionRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        self.recover_correction_scoped(input, false)
    }

    pub fn recover_deviation_record(
        &self,
        input: DeviationRecordRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        self.recover_deviation_record_scoped(input, false)
    }

    pub fn run_command_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: CommandProjectRequest<'_>,
        approved_sha256: &str,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        self.run_prepared_command(
            input.execution_dir,
            approved_sha256,
            || recheck_command_from_project(sources, self, input),
            &LocalCommandRunner,
        )
    }

    pub fn recover_command_correction(
        &self,
        input: CommandCorrectionRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        self.recover_command_correction_scoped(input, false)
    }

    pub fn resolve_command_invocation(
        &self,
        argv: &[String],
        cwd: &std::path::Path,
    ) -> Result<Value, WorkError> {
        let executable = argv
            .first()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                WorkError::new(
                    ExitCode::WorkflowState,
                    "command_run_argv",
                    "argv cannot contain NUL bytes.",
                    json!({}),
                )
            })?;
        if argv.iter().any(|value| value.contains('\0')) {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_argv",
                "argv cannot contain NUL bytes.",
                json!({}),
            ));
        }
        let selected = if executable.contains(['/', '\\']) {
            let path = PathBuf::from(executable);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        } else {
            let located = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|entry| {
                    if entry.is_absolute() {
                        entry
                    } else {
                        cwd.join(entry)
                    }
                })
                .map(|entry| entry.join(executable))
                .find(|path| is_runnable_file(path));
            located.ok_or_else(|| {
                WorkError::new(
                    ExitCode::WorkflowState,
                    "command_run_executable",
                    "The approved executable is not available on PATH.",
                    json!({}),
                )
            })?
        };
        let selected = std::path::absolute(&selected).map_err(|_| {
            WorkError::new(
                ExitCode::WorkflowState,
                "command_run_executable",
                "The selected executable is not runnable.",
                json!({}),
            )
        })?;
        let resolved = selected.canonicalize().map_err(|_| {
            WorkError::new(
                ExitCode::WorkflowState,
                "command_run_executable",
                "The selected executable is not runnable.",
                json!({}),
            )
        })?;
        let batch = [&selected, &resolved].iter().any(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
                })
        });
        if batch {
            #[cfg(windows)]
            {
                let system_root = std::env::var_os("SystemRoot").ok_or_else(|| {
                    WorkError::new(
                        ExitCode::WorkflowState,
                        "command_run_launcher",
                        "SystemRoot is required to select the fixed cmd.exe launcher.",
                        json!({}),
                    )
                })?;
                let launcher = PathBuf::from(system_root).join("System32/cmd.exe");
                if !launcher.is_file()
                    || launcher.is_symlink()
                    || selected.is_symlink()
                    || !selected.is_file()
                {
                    return Err(WorkError::new(
                        ExitCode::WorkflowState,
                        "command_run_launcher",
                        "The fixed launcher or batch script is not a regular file.",
                        json!({}),
                    ));
                }
                let arguments = argv[1..].to_vec();
                let (command_line, launcher_arguments) =
                    windows_batch_command_line(&selected.to_string_lossy(), &arguments).map_err(
                        |issue| {
                            WorkError::new(
                                ExitCode::WorkflowState,
                                issue.reason_code,
                                issue.message,
                                issue.details,
                            )
                        },
                    )?;
                return Ok(json!({"kind":"windows_batch",
                    "launcher":launcher.to_string_lossy(),
                    "launcher_sha256":work_operations::derivation::fingerprint::raw(&LocalFiles.read_raw(&launcher)?),
                    "script":selected.to_string_lossy(),
                    "script_sha256":work_operations::derivation::fingerprint::raw(&LocalFiles.read_raw(&selected)?),
                    "arguments":arguments,"command_line":command_line,
                    "launcher_arguments":launcher_arguments}));
            }
            #[cfg(not(windows))]
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_argv_only",
                "Batch files can only be prepared on Windows.",
                json!({}),
            ));
        }
        if !is_runnable_file(&selected) {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_executable",
                "The selected executable is not runnable.",
                json!({}),
            ));
        }
        Ok(
            json!({"kind":"direct","executable":selected.to_string_lossy(),
            "executable_sha256":work_operations::derivation::fingerprint::raw(&LocalFiles.read_raw(&selected)?),
            "argv":argv}),
        )
    }

    pub fn run_prepared_command(
        &self,
        execution_dir: &str,
        approved_sha256: &str,
        recheck: impl FnOnce() -> Result<(Value, String), WorkError>,
        runner: &impl CommandRunner,
    ) -> Result<Value, WorkError> {
        self.run_prepared_command_scoped(execution_dir, approved_sha256, recheck, runner, false)
    }

    pub fn recover_attempt_close(
        &self,
        input: AttemptCloseRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        self.recover_attempt_close_scoped(input, false)
    }

    pub fn recover_record_finish(
        &self,
        input: RecordFinishRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        self.recover_record_finish_scoped(input, false)
    }

    pub fn recover_record_begin(
        &self,
        input: RecordBeginRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        self.recover_record_begin_scoped(input, false)
    }
}

/// Complete candidate execution adapter. A verified TASK context supplies identity to every writer.
pub struct RuntimeExecutionStorage<'a> {
    pub storage: &'a LocalExecutionStorage,
    pub context: &'a work_feature::execution::ExecutionWriterContext,
}

impl RuntimeExecutionStorage<'_> {
    fn check_index(&self, raw: &[u8]) -> Result<(), WorkError> {
        let index = parse_json_contract(raw).map_err(|_| transaction_conflict())?;
        self.context.check_execution_index(&index, raw)
    }

    fn write<T>(
        &self,
        execution: &str,
        write: impl FnOnce() -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        if execution != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The publication target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage
            .with_runtime_execution_writer(self.context, |_| write())
    }
    pub fn recover_attempt_start_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: AttemptStartRecoveryRequest<'_>,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        self.write(input.target.execution_dir, || {
            {
                self.storage
                    .recover_attempt_start_staging_scoped(self.context, sources, input)
            }
        })
    }
    pub fn recover_correction(
        &self,
        input: CorrectionRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_legacy_recovery_input()?;
        self.write(input.execution_dir, || {
            self.check_index(input.index_before)?;
            self.storage.recover_correction_scoped(input, true)
        })
    }
    pub fn recover_deviation_record(
        &self,
        input: DeviationRecordRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_legacy_recovery_input()?;
        self.write(input.execution_dir, || {
            self.check_index(input.index_before)?;
            self.storage.recover_deviation_record_scoped(input, true)
        })
    }
    pub fn recover_command_correction(
        &self,
        input: CommandCorrectionRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_legacy_recovery_input()?;
        self.write(input.execution_dir, || {
            self.check_index(input.index_before)?;
            self.storage.recover_command_correction_scoped(input, true)
        })
    }
    pub fn run_prepared_command(
        &self,
        execution_dir: &str,
        approved_sha256: &str,
        recheck: impl FnOnce() -> Result<(Value, String), WorkError>,
        runner: &impl CommandRunner,
    ) -> Result<Value, WorkError> {
        self.write(execution_dir, || {
            self.storage
                .run_receipts_scoped(self.context, approved_sha256, recheck, runner, |_| Ok(()))
        })
    }
    pub fn recover_attempt_close(
        &self,
        input: AttemptCloseRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_legacy_recovery_input()?;
        self.write(input.execution_dir, || {
            self.check_index(input.index_before)?;
            self.storage.recover_attempt_close_scoped(input, true)
        })
    }
    pub fn recover_record_finish(
        &self,
        input: RecordFinishRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_legacy_recovery_input()?;
        self.write(input.execution_dir, || {
            self.check_index(input.index_before)?;
            self.storage.recover_record_finish_scoped(input, true)
        })
    }
    pub fn recover_record_begin(
        &self,
        input: RecordBeginRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_legacy_recovery_input()?;
        self.write(input.execution_dir, || {
            self.check_index(input.index_before)?;
            self.storage.recover_record_begin_scoped(input, true)
        })
    }
    pub fn recover_execution_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        target: ExecutionProjectTarget<'_>,
        request: &Value,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        let _ = sources;
        self.write(target.execution_dir, || {
            if target.task_path != self.context.target().task_path
                || target.task_id != self.context.target().task_id
            {
                return Err(transaction_conflict());
            }
            match request["transaction"].as_str() {
                Some("record_begin") => self
                    .storage
                    .recover_record_begin_staging_scoped(self.context, request),
                Some("command_correction") => self
                    .storage
                    .recover_command_correction_staging_scoped(self.context, request),
                Some("record_finish") => self
                    .storage
                    .recover_record_finish_staging_scoped(self.context, request),
                Some("deviation_record") => self
                    .storage
                    .recover_deviation_staging_scoped(self.context, request),
                Some("attempt_close") => self
                    .storage
                    .recover_attempt_close_staging_scoped(self.context, request),
                Some("correction") => self
                    .storage
                    .recover_correction_staging_scoped(self.context, request),
                _ => Err(WorkError::new(
                    ExitCode::WorkflowState,
                    "execution_recovery_invalid_transaction",
                    "The recovery transaction is unsupported.",
                    json!({}),
                )),
            }
        })
    }

    pub fn run_command_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: CommandProjectRequest<'_>,
        approved_sha256: &str,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        let runner =
            crate::process::isolation::IsolatedCommandRunner::new(self.context.writer().clone());
        self.run_prepared_command(
            input.execution_dir,
            approved_sha256,
            || {
                let (preview, evidence) = recheck_command_from_project(sources, self, input)?;
                runner.bind(&preview)?;
                Ok((preview, evidence))
            },
            &runner,
        )
    }
}

impl AttemptStartRepository for RuntimeExecutionStorage<'_> {
    fn publish_attempt_start(
        &self,
        publication: &AttemptStartPublication<'_>,
    ) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            self.check_index(publication.index_before)?;
            {
                self.storage
                    .publish_attempt_start_staging_scoped(self.context, publication)
            }
        })
    }
    fn attempt_names(&self, execution_dir: &str, task_id: &str) -> Result<Vec<String>, WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage.attempt_names(execution_dir, task_id)
    }
    fn read_attempt(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_attempt(relative_path)
    }
}

impl DeviationRecordRepository for RuntimeExecutionStorage<'_> {
    fn publish_deviation_record(
        &self,
        publication: &DeviationRecordPublication<'_>,
    ) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            let index_path = format!("{}/index.json", publication.execution_dir);
            let index_before = publication
                .sources
                .get(&index_path)
                .ok_or_else(transaction_conflict)?;
            self.check_index(index_before)?;
            {
                let plan = self
                    .storage
                    .deviation_staging_plan(self.context, publication)?;
                self.storage.run_execution_staging_scoped(
                    self.context,
                    &plan,
                    false,
                    &LocalFiles,
                    |_| Ok(()),
                )
            }
        })
    }
}

impl CorrectionRepository for RuntimeExecutionStorage<'_> {
    fn publish_correction(&self, publication: &CorrectionPublication<'_>) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            self.check_index(publication.index_before)?;
            {
                let plan = self
                    .storage
                    .correction_staging_plan(self.context, publication)?;
                self.storage.run_execution_staging_scoped(
                    self.context,
                    &plan,
                    false,
                    &LocalFiles,
                    |_| Ok(()),
                )
            }
        })
    }
    fn correction_names(
        &self,
        execution_dir: &str,
        task_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<String>, WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage
            .correction_names(execution_dir, task_id, attempt_id)
    }
}

impl CommandCorrectionRepository for RuntimeExecutionStorage<'_> {
    fn publish_command_correction(
        &self,
        publication: &CommandCorrectionPublication<'_>,
    ) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            self.check_index(publication.index_before)?;
            {
                let plan = self
                    .storage
                    .command_correction_staging_plan(self.context, publication)?;
                self.storage.run_execution_staging_scoped(
                    self.context,
                    &plan,
                    false,
                    &LocalFiles,
                    |_| Ok(()),
                )
            }
        })
    }
}

impl RecordBeginRepository for RuntimeExecutionStorage<'_> {
    fn effect_boundary(
        &self,
        task: &Value,
        record: &str,
    ) -> work_model::execution::VerifiedEffectBoundary {
        crate::process::isolation::record_boundary(task, record)
    }
    fn publish_record_begin(
        &self,
        publication: &RecordBeginPublication<'_>,
    ) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            self.check_index(publication.index_before)?;
            {
                let plan = self
                    .storage
                    .record_begin_staging_plan(self.context, publication)?;
                self.storage.run_execution_staging_scoped(
                    self.context,
                    &plan,
                    false,
                    &LocalFiles,
                    |_| Ok(()),
                )
            }
        })
    }
}

impl RecordFinishRepository for RuntimeExecutionStorage<'_> {
    fn publish_record_finish(
        &self,
        publication: &RecordFinishPublication<'_>,
    ) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            self.check_index(publication.index_before)?;
            {
                let plan = self
                    .storage
                    .record_finish_staging_plan(self.context, publication)?;
                self.storage.run_execution_staging_scoped(
                    self.context,
                    &plan,
                    false,
                    &LocalFiles,
                    |_| Ok(()),
                )
            }
        })
    }
}

impl AttemptCloseRepository for RuntimeExecutionStorage<'_> {
    fn publish_attempt_close(
        &self,
        publication: &AttemptClosePublication<'_>,
    ) -> Result<(), WorkError> {
        self.write(publication.execution_dir, || {
            self.check_index(publication.index_before)?;
            let completed: serde_json::Value = serde_json::from_slice(publication.attempt_after)
                .map_err(|_| transaction_conflict())?;
            if completed["status"] == "completed"
                && self.context.task_context().contract["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| {
                        t["id"] == publication.task_id
                            && t["files"].as_array().is_some_and(|f| !f.is_empty())
                    })
            {
                super::file_transaction_storage::require_published(
                    self.context,
                    publication.attempt_id,
                )?;
            }
            {
                let plan = self
                    .storage
                    .attempt_close_staging_plan(self.context, publication)?;
                self.storage.run_execution_staging_scoped(
                    self.context,
                    &plan,
                    false,
                    &LocalFiles,
                    |_| Ok(()),
                )
            }
        })
    }
}

impl ExecutionIndexRepository for RuntimeExecutionStorage<'_> {
    fn read_index(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_index(relative_path)
    }
    fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError> {
        self.storage.inspect_path(relative_path)
    }
    fn check_execution_ready(&self, task_path: &str, execution_dir: &str) -> Result<(), WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        if task_path != self.context.target().task_path {
            return Err(transaction_conflict());
        }
        self.storage.check_command_context(self.context, true)?;
        self.storage.check_execution_ready(task_path, execution_dir)
    }
    fn check_execution_task_layout(
        &self,
        execution_dir: &str,
        task_id: &str,
    ) -> Result<(), WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage
            .check_execution_task_layout(execution_dir, task_id)
    }
}

impl ExecutionWorktreeRepository for RuntimeExecutionStorage<'_> {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_file(relative_path)
    }
    fn git_status(&self) -> Result<Vec<Value>, WorkError> {
        Ok(
            work_operations::execution::worktree::runtime_worktree_records(
                &self.storage.git_status()?,
            ),
        )
    }
}

impl CommandPrepareRepository for RuntimeExecutionStorage<'_> {
    fn execution_settings(&self, settings: &Value, receipt: &str) -> Result<Value, WorkError> {
        let policy = crate::process::isolation::policy(self.context.writer(), receipt)?;
        let mut settings = settings.clone();
        settings["work_isolation"] = json!(policy);
        Ok(settings)
    }
    fn effect_boundary(
        &self,
        _task: &Value,
        _record: &str,
        actual: &Value,
    ) -> work_model::execution::VerifiedEffectBoundary {
        crate::process::isolation::invocation_boundary(self.context.writer(), actual)
    }
    fn check_ready(&self, execution_dir: &str, require_idle: bool) -> Result<(), WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage
            .check_command_context(self.context, require_idle)
    }
    fn runtime_os(&self) -> &'static str {
        self.storage.runtime_os()
    }
    fn read_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_source(relative_path)
    }
    fn working_directory(&self, relative_path: &str) -> Result<String, WorkError> {
        self.storage.working_directory(relative_path)
    }
    fn resolve_invocation(&self, argv: &[String], cwd: &str) -> Result<Value, WorkError> {
        self.storage.resolve_invocation(argv, cwd)
    }
    fn receipt_exists(&self, prefix: &str) -> Result<bool, WorkError> {
        self.storage.receipt_exists(prefix)
    }
}

impl RecoveryPrepareRepository for RuntimeExecutionStorage<'_> {
    fn staging_transactions(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
    ) -> Result<Vec<work_feature::execution::StagingRecoverySnapshot>, WorkError> {
        if self.context.writer().requirement_id != context.writer().requirement_id
            || self.context.target().execution_dir != context.target().execution_dir
            || self.context.target().task_id != context.target().task_id
        {
            return Err(runtime_scan_error(
                "runtime_manifest_context_mismatch",
                context.target().execution_dir,
            ));
        }
        self.storage.staging_transactions(context)
    }

    fn check_recovery_idle(&self, execution_dir: &str) -> Result<(), WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage.check_recovery_context(self.context)
    }
    fn temporary_names(&self, execution_dir: &str) -> Result<Vec<String>, WorkError> {
        if execution_dir != self.context.target().execution_dir {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_writer_target_mismatch",
                "The target differs from the verified context.",
                json!({}),
            ));
        }
        self.storage.temporary_names(execution_dir)
    }
    fn read_recovery_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_recovery_source(relative_path)
    }
    fn recovery_source_exists(&self, relative_path: &str) -> Result<bool, WorkError> {
        self.storage.recovery_source_exists(relative_path)
    }
}

/// CLI session defers trusted-context loading until validated feature code reaches a probe or writer.
/// Pure input rejection and primitive read errors keep their established order.
pub struct RuntimeExecutionSession<'a> {
    pub storage: &'a LocalExecutionStorage,
    pub load_context:
        &'a dyn Fn() -> Result<work_feature::execution::ExecutionWriterContext, WorkError>,
}

impl RuntimeExecutionSession<'_> {
    fn with_context<T>(
        &self,
        action: impl FnOnce(&RuntimeExecutionStorage<'_>) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        let context = (self.load_context)()?;
        let adapter = RuntimeExecutionStorage {
            storage: self.storage,
            context: &context,
        };
        action(&adapter)
    }
    pub fn recover_attempt_start_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: AttemptStartRecoveryRequest<'_>,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        validate_attempt_start_request(input.request).map_err(recovery_rule)?;
        self.with_context(|adapter| adapter.recover_attempt_start_from_project(sources, input))
    }
    pub fn recover_execution_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        target: ExecutionProjectTarget<'_>,
        request: &Value,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        validate_recovery_request(request, false).map_err(recovery_rule)?;
        self.with_context(|adapter| {
            adapter.recover_execution_from_project(sources, target, request)
        })
    }
    pub fn run_command_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: CommandProjectRequest<'_>,
        approved_sha256: &str,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: ArtifactPathRepository + SourceSnapshotReader,
        T: TaskCollectionRepository,
    {
        work_operations::execution::command_run::validate_command_run_request(input.request)
            .map_err(recovery_rule)?;
        self.with_context(|adapter| {
            adapter.run_command_from_project(sources, input, approved_sha256)
        })
    }
}

impl AttemptStartRepository for RuntimeExecutionSession<'_> {
    fn publish_attempt_start(
        &self,
        publication: &AttemptStartPublication<'_>,
    ) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_attempt_start(publication))
    }
    fn attempt_names(&self, execution_dir: &str, task_id: &str) -> Result<Vec<String>, WorkError> {
        self.with_context(|adapter| adapter.attempt_names(execution_dir, task_id))
    }
    fn read_attempt(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_attempt(relative_path)
    }
}

impl DeviationRecordRepository for RuntimeExecutionSession<'_> {
    fn publish_deviation_record(
        &self,
        publication: &DeviationRecordPublication<'_>,
    ) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_deviation_record(publication))
    }
}

impl CorrectionRepository for RuntimeExecutionSession<'_> {
    fn publish_correction(&self, publication: &CorrectionPublication<'_>) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_correction(publication))
    }
    fn correction_names(
        &self,
        execution_dir: &str,
        task_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<String>, WorkError> {
        self.with_context(|adapter| adapter.correction_names(execution_dir, task_id, attempt_id))
    }
}

impl CommandCorrectionRepository for RuntimeExecutionSession<'_> {
    fn publish_command_correction(
        &self,
        publication: &CommandCorrectionPublication<'_>,
    ) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_command_correction(publication))
    }
}

impl RecordBeginRepository for RuntimeExecutionSession<'_> {
    fn effect_boundary(
        &self,
        task: &Value,
        record: &str,
    ) -> work_model::execution::VerifiedEffectBoundary {
        crate::process::isolation::record_boundary(task, record)
    }
    fn publish_record_begin(
        &self,
        publication: &RecordBeginPublication<'_>,
    ) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_record_begin(publication))
    }
}

impl RecordFinishRepository for RuntimeExecutionSession<'_> {
    fn publish_record_finish(
        &self,
        publication: &RecordFinishPublication<'_>,
    ) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_record_finish(publication))
    }
}

impl AttemptCloseRepository for RuntimeExecutionSession<'_> {
    fn publish_attempt_close(
        &self,
        publication: &AttemptClosePublication<'_>,
    ) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.publish_attempt_close(publication))
    }
}

impl ExecutionIndexRepository for RuntimeExecutionSession<'_> {
    fn read_index(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_index(relative_path)
    }
    fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError> {
        self.storage.inspect_path(relative_path)
    }
    fn check_execution_ready(&self, task_path: &str, execution_dir: &str) -> Result<(), WorkError> {
        self.storage
            .check_execution_ready(task_path, execution_dir)?;
        self.with_context(|adapter| adapter.check_execution_ready(task_path, execution_dir))
    }
    fn check_execution_task_layout(
        &self,
        execution_dir: &str,
        task_id: &str,
    ) -> Result<(), WorkError> {
        self.storage
            .check_execution_task_layout(execution_dir, task_id)?;
        self.with_context(|adapter| adapter.check_execution_task_layout(execution_dir, task_id))
    }
}

impl ExecutionWorktreeRepository for RuntimeExecutionSession<'_> {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_file(relative_path)
    }
    fn git_status(&self) -> Result<Vec<Value>, WorkError> {
        Ok(
            work_operations::execution::worktree::runtime_worktree_records(
                &self.storage.git_status()?,
            ),
        )
    }
}

impl CommandPrepareRepository for RuntimeExecutionSession<'_> {
    fn execution_settings(&self, settings: &Value, receipt: &str) -> Result<Value, WorkError> {
        self.with_context(|adapter| adapter.execution_settings(settings, receipt))
    }
    fn effect_boundary(
        &self,
        task: &Value,
        record: &str,
        actual: &Value,
    ) -> work_model::execution::VerifiedEffectBoundary {
        self.with_context(|adapter| {
            Ok(CommandPrepareRepository::effect_boundary(
                adapter, task, record, actual,
            ))
        })
        .unwrap_or(work_model::execution::VerifiedEffectBoundary::Unknown)
    }
    fn check_ready(&self, execution_dir: &str, require_idle: bool) -> Result<(), WorkError> {
        self.storage.check_ready(execution_dir, false)?;
        self.with_context(|adapter| adapter.check_ready(execution_dir, require_idle))
    }
    fn runtime_os(&self) -> &'static str {
        self.storage.runtime_os()
    }
    fn read_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_source(relative_path)
    }
    fn working_directory(&self, relative_path: &str) -> Result<String, WorkError> {
        self.storage.working_directory(relative_path)
    }
    fn resolve_invocation(&self, argv: &[String], cwd: &str) -> Result<Value, WorkError> {
        self.storage.resolve_invocation(argv, cwd)
    }
    fn receipt_exists(&self, prefix: &str) -> Result<bool, WorkError> {
        self.storage.receipt_exists(prefix)
    }
}

impl RecoveryPrepareRepository for RuntimeExecutionSession<'_> {
    fn staging_transactions(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
    ) -> Result<Vec<work_feature::execution::StagingRecoverySnapshot>, WorkError> {
        self.with_context(|adapter| adapter.staging_transactions(context))
    }

    fn check_recovery_idle(&self, execution_dir: &str) -> Result<(), WorkError> {
        self.with_context(|adapter| adapter.check_recovery_idle(execution_dir))
    }
    fn temporary_names(&self, execution_dir: &str) -> Result<Vec<String>, WorkError> {
        self.with_context(|adapter| adapter.temporary_names(execution_dir))
    }
    fn read_recovery_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        self.storage.read_recovery_source(relative_path)
    }
    fn recovery_source_exists(&self, relative_path: &str) -> Result<bool, WorkError> {
        self.storage.recovery_source_exists(relative_path)
    }
}

impl work_feature::execution::RuntimeExecutionReadiness for LocalExecutionStorage {
    fn check_command_context(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
        require_idle: bool,
    ) -> Result<(), WorkError> {
        use work_feature::ports::RuntimeWriterLock;
        self.validate_runtime_context(context)?;
        crate::writer_lock::require_no_legacy_locks(
            context.writer(),
            Some(context.target().execution_dir),
        )?;
        if require_idle {
            LocalWriterLock.require_runtime_idle(
                context.writer(),
                work_model::runtime::LockClass::Execution,
            )?;
        }
        {
            self.require_runtime_execution_ready(context, None)?;
        }
        self.check_ready(context.target().execution_dir, false)
    }

    fn check_recovery_context(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
    ) -> Result<(), WorkError> {
        use work_feature::ports::RuntimeWriterLock;
        self.validate_runtime_context(context)?;
        crate::writer_lock::require_no_legacy_locks(
            context.writer(),
            Some(context.target().execution_dir),
        )?;
        require_no_spec_update(&self.project_root, context.target().execution_dir, None)?;
        LocalWriterLock
            .require_runtime_idle(context.writer(), work_model::runtime::LockClass::Execution)
    }
}

impl CommandPrepareRepository for LocalExecutionStorage {
    fn check_ready(&self, execution_dir: &str, require_idle: bool) -> Result<(), WorkError> {
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        if require_idle {
            require_execution_writer_context(false)?;
            #[cfg(test)]
            {
                let lock = storage_path(
                    &self.project_root,
                    &format!("{execution_dir}/.work-state-writer.lock"),
                )?;
                LocalWriterLock.require_idle(&lock)?;
            }
        }
        let directory = storage_path(&self.project_root, execution_dir)?;
        for entry in fs::read_dir(&directory).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "command_run_execution_dir",
                "The execution directory could not be read.",
                json!({}),
            )
        })? {
            let entry = entry.map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "command_run_execution_dir",
                    "The execution directory could not be read.",
                    json!({}),
                )
            })?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".work-") && name.ends_with(".tmp") {
                return Err(WorkError::new(
                    ExitCode::WorkflowState,
                    "command_run_pending_transaction",
                    "Resolve pending transactions before executing a CMD.",
                    json!({}),
                ));
            }
        }
        Ok(())
    }

    fn runtime_os(&self) -> &'static str {
        match std::env::consts::OS {
            "macos" => "macos",
            "windows" => "windows",
            "linux" => "linux",
            _ => "unsupported",
        }
    }

    fn read_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, relative_path)?;
        LocalFiles.read_raw(&path)
    }

    fn working_directory(&self, relative_path: &str) -> Result<String, WorkError> {
        let path = if relative_path == "." {
            self.project_root.canonicalize().map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "path_resolution_failed",
                    "The project root could not be resolved.",
                    json!({}),
                )
            })?
        } else {
            let (_, path) = resolve_project_path(&self.project_root, relative_path)?;
            path
        };
        if !path.is_dir() {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_cwd",
                "The specified working directory does not exist.",
                json!({}),
            ));
        }
        Ok(path.to_string_lossy().into_owned())
    }

    fn resolve_invocation(&self, argv: &[String], cwd: &str) -> Result<Value, WorkError> {
        self.resolve_command_invocation(argv, std::path::Path::new(cwd))
    }

    fn receipt_exists(&self, prefix: &str) -> Result<bool, WorkError> {
        let (attempt_root, instance) = prefix
            .rsplit_once("/receipts/")
            .ok_or_else(transaction_conflict)?;
        let (task_root, attempt) = attempt_root
            .rsplit_once('/')
            .ok_or_else(transaction_conflict)?;
        let (execution, task) = task_root
            .rsplit_once('/')
            .ok_or_else(transaction_conflict)?;
        let record = work_operations::derivation::publication::command_receipt_instance(instance)
            .map_err(|_| transaction_conflict())?;
        let paths = work_operations::derivation::publication::command_receipt_paths(
            execution, task, attempt, &record,
        )
        .map_err(|_| transaction_conflict())?;
        if paths.directory != prefix {
            return Err(transaction_conflict());
        }
        self.receipt_paths_observed(&paths)
    }
}

impl ExecutionIndexRepository for LocalExecutionStorage {
    fn read_index(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, relative_path)?;
        if !path.is_file() {
            return Err(WorkError::new(
                ExitCode::IoFailure,
                "execute_preflight_index_missing",
                "The canonical execution index does not exist.",
                json!({"path":path.to_string_lossy()}),
            ));
        }
        LocalFiles.read_raw(&path)
    }

    fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, relative_path)?;
        let mut nearest = path.as_path();
        let mut unresolved = Vec::new();
        while !nearest.exists() {
            if let Some(name) = nearest.file_name() {
                unresolved.push(name.to_os_string());
            }
            nearest = nearest.parent().ok_or_else(|| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "path_resolution_failed",
                    "The path could not be resolved.",
                    json!({"path":path.to_string_lossy()}),
                )
            })?;
        }
        let mut resolved = nearest.canonicalize().map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "path_resolution_failed",
                "The path could not be resolved.",
                json!({"path":path.to_string_lossy()}),
            )
        })?;
        for segment in unresolved.iter().rev() {
            resolved.push(segment);
        }
        Ok(FileState {
            identity: portable_path_identity(&resolved.to_string_lossy()),
            exists: resolved.exists(),
        })
    }

    fn check_execution_ready(&self, task_path: &str, execution_dir: &str) -> Result<(), WorkError> {
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        let task = storage_path(&self.project_root, task_path)?;
        if !task.is_file() {
            return Err(WorkError::new(
                ExitCode::IoFailure,
                "execute_preflight_task_missing",
                "The formal TASK document does not exist.",
                json!({"path":task.to_string_lossy()}),
            ));
        }
        let execution = storage_path(&self.project_root, execution_dir)?;
        if !execution.is_dir() {
            return Err(WorkError::new(
                ExitCode::IoFailure,
                "execute_preflight_execution_directory_missing",
                "The execution directory does not exist.",
                json!({"path":execution.to_string_lossy()}),
            ));
        }
        let mut files = Vec::new();
        for entry in
            fs::read_dir(&execution).map_err(|_| transaction_io("file_read_failed", &execution))?
        {
            let entry = entry.map_err(|_| transaction_io("file_read_failed", &execution))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|kind| kind.is_file())
                && is_attempt_start_temporary(&name)
            {
                files.push(name);
            }
        }
        files.sort();
        if !files.is_empty() {
            return Err(WorkError::new(
                ExitCode::LockConflict,
                "execute_preflight_attempt_start_transaction_present",
                "An incomplete Attempt-start transaction requires recovery.",
                json!({"files":files}),
            ));
        }
        Ok(())
    }

    fn check_execution_task_layout(
        &self,
        execution_dir: &str,
        task_id: &str,
    ) -> Result<(), WorkError> {
        let relative = format!("{execution_dir}/{task_id}");
        let directory = storage_path(&self.project_root, &relative)?;
        if !directory.is_dir() {
            return Ok(());
        }
        let mut files = Vec::new();
        for entry in
            fs::read_dir(&directory).map_err(|_| transaction_io("file_read_failed", &directory))?
        {
            let entry = entry.map_err(|_| transaction_io("file_read_failed", &directory))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("ATTEMPT-") && name.ends_with(".json") {
                files.push(name);
            }
        }
        files.sort();
        if !files.is_empty() {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_legacy_layout_unsupported",
                "Flat Attempt and Correction files are unsupported; use <TASK-ID>/<ATTEMPT-ID>/attempt.json and its corrections/ directory. No files were changed.",
                json!({"path":relative,"files":files}),
            ));
        }
        Ok(())
    }
}

fn is_attempt_start_temporary(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".work-attempt-start-TASK-") else {
        return false;
    };
    let Some((task, rest)) = rest.split_once("-ATTEMPT-") else {
        return false;
    };
    task.len() == 3
        && task.bytes().all(|byte| byte.is_ascii_digit())
        && rest
            .as_bytes()
            .get(..3)
            .is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
        && matches!(rest.get(3..), Some("-lock.tmp" | "-started.tmp"))
}

impl ExecutionWorktreeRepository for LocalExecutionStorage {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        let path = storage_path(&self.project_root, relative_path)?;
        LocalFiles.read_raw(&path)
    }

    fn git_status(&self) -> Result<Vec<Value>, WorkError> {
        let root = self.project_root.canonicalize().map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "execute_worktree_git_root_invalid",
                "The Git worktree root could not be resolved.",
                json!({}),
            )
        })?;
        let raw_root =
            LocalGit.read_only(&root, &["rev-parse".into(), "--show-toplevel".into()])?;
        let top = std::str::from_utf8(&raw_root)
            .ok()
            .and_then(|text| fs::canonicalize(text.trim()).ok())
            .ok_or_else(|| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "execute_worktree_git_root_invalid",
                    "The Git worktree root could not be resolved.",
                    json!({}),
                )
            })?;
        if top != root {
            return Err(WorkError::new(
                ExitCode::Contract,
                "execute_worktree_project_root_mismatch",
                "The project root must be the Git worktree root.",
                json!({"git_root":top.to_string_lossy(),"project_root":root.to_string_lossy()}),
            ));
        }
        let raw = LocalGit.read_only(
            &root,
            &[
                "-c".into(),
                "core.quotepath=false".into(),
                "-c".into(),
                "status.relativePaths=false".into(),
                "status".into(),
                "--porcelain=v1".into(),
                "-z".into(),
                "--untracked-files=all".into(),
                "--ignore-submodules=none".into(),
            ],
        )?;
        Ok(parse_porcelain_v1_z(&raw)?
            .into_iter()
            .map(|row| {
                let mut value = json!({"index_status":row.index_status.to_string(),
                "worktree_status":row.worktree_status.to_string(),"path":row.path});
                if let Some(original) = row.original_path {
                    value["original_path"] = json!(original);
                }
                value
            })
            .collect())
    }
}

impl AttemptStartRepository for LocalExecutionStorage {
    fn attempt_names(&self, execution_dir: &str, task_id: &str) -> Result<Vec<String>, WorkError> {
        let directory = storage_path(&self.project_root, &format!("{execution_dir}/{task_id}"))?;
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("attempt_start_history_read_failed", &directory))?
        {
            let entry = entry
                .map_err(|_| transaction_io("attempt_start_history_read_failed", &directory))?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn read_attempt(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        let path = storage_path(&self.project_root, relative_path)?;
        LocalFiles.read_raw(&path)
    }

    fn publish_attempt_start(
        &self,
        publication: &AttemptStartPublication<'_>,
    ) -> Result<(), WorkError> {
        self.publish_attempt_start_scoped(publication, false)
    }
}

impl DeviationRecordRepository for LocalExecutionStorage {
    fn publish_deviation_record(
        &self,
        publication: &DeviationRecordPublication<'_>,
    ) -> Result<(), WorkError> {
        self.publish_deviation_record_scoped(publication, false)
    }
}

impl CorrectionRepository for LocalExecutionStorage {
    fn correction_names(
        &self,
        execution_dir: &str,
        task_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<String>, WorkError> {
        let directory = storage_path(
            &self.project_root,
            &format!("{execution_dir}/{task_id}/{attempt_id}/corrections"),
        )?;
        if !directory.exists() {
            return Ok(vec![]);
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("correction_create_directory_read_failed", &directory))?
        {
            let entry = entry.map_err(|_| {
                transaction_io("correction_create_directory_read_failed", &directory)
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("{attempt_id}-CORRECTION-"))
                && name.ends_with(".json")
                && entry.path().is_file()
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn publish_correction(&self, publication: &CorrectionPublication<'_>) -> Result<(), WorkError> {
        self.publish_correction_scoped(publication, false)
    }
}

impl RecoveryPrepareRepository for LocalExecutionStorage {
    fn staging_transactions(
        &self,
        context: &work_feature::execution::ExecutionWriterContext,
    ) -> Result<Vec<work_feature::execution::StagingRecoverySnapshot>, WorkError> {
        Ok(self
            .runtime_execution_inventory(context)?
            .into_iter()
            .map(|entry| work_feature::execution::StagingRecoverySnapshot {
                transaction_dir: entry.transaction_dir,
                manifest: entry.manifest,
                files: entry.files,
            })
            .collect())
    }

    fn check_recovery_idle(&self, execution_dir: &str) -> Result<(), WorkError> {
        require_execution_writer_context(false)?;
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        #[cfg(test)]
        {
            let writer = storage_path(
                &self.project_root,
                &format!("{execution_dir}/.work-state-writer.lock"),
            )?;
            LocalWriterLock.require_idle(&writer)?;
        }
        Ok(())
    }

    fn temporary_names(&self, execution_dir: &str) -> Result<Vec<String>, WorkError> {
        let directory = storage_path(&self.project_root, execution_dir)?;
        let mut names = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| transaction_io("recovery_prepare_inventory_failed", &directory))?
        {
            let entry = entry
                .map_err(|_| transaction_io("recovery_prepare_inventory_failed", &directory))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".work-") && name.ends_with(".tmp") {
                let kind = entry
                    .file_type()
                    .map_err(|_| transaction_io("recovery_prepare_file_type", &entry.path()))?;
                if !kind.is_file() || kind.is_symlink() {
                    return Err(WorkError::new(
                        ExitCode::ArtifactIntegrity,
                        "recovery_prepare_file_type",
                        "Transaction evidence must be regular files.",
                        json!({"file":name}),
                    ));
                }
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn read_recovery_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&storage_path(&self.project_root, relative_path)?)
    }

    fn recovery_source_exists(&self, relative_path: &str) -> Result<bool, WorkError> {
        Ok(storage_path(&self.project_root, relative_path)?.is_file())
    }
}

impl CommandCorrectionRepository for LocalExecutionStorage {
    fn publish_command_correction(
        &self,
        publication: &CommandCorrectionPublication<'_>,
    ) -> Result<(), WorkError> {
        self.publish_command_correction_scoped(publication, false)
    }
}

impl RecordBeginRepository for LocalExecutionStorage {
    fn publish_record_begin(
        &self,
        publication: &RecordBeginPublication<'_>,
    ) -> Result<(), WorkError> {
        self.publish_record_begin_scoped(publication, false)
    }
}

impl RecordFinishRepository for LocalExecutionStorage {
    fn publish_record_finish(
        &self,
        publication: &RecordFinishPublication<'_>,
    ) -> Result<(), WorkError> {
        self.publish_record_finish_scoped(publication, false)
    }
}

impl AttemptCloseRepository for LocalExecutionStorage {
    fn publish_attempt_close(
        &self,
        publication: &AttemptClosePublication<'_>,
    ) -> Result<(), WorkError> {
        self.publish_attempt_close_scoped(publication, false)
    }
}

// Legacy body tests retain their isolated byte-level regression cases. Production callers
// must use the verified-context session once the complete family is enabled.
fn require_legacy_recovery_input() -> Result<(), WorkError> {
    Err(WorkError::new(
        ExitCode::ArtifactIntegrity,
        "execution_recovery_review_required",
        "Recovery requires the generated request and its reviewed staging evidence.",
        json!({}),
    ))
}

fn require_execution_writer_context(runtime_held: bool) -> Result<(), WorkError> {
    if !runtime_held && !cfg!(test) {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_writer_context_required",
            "Execution writers and idle probes require a verified TASK context.",
            json!({}),
        ));
    }
    Ok(())
}

fn transaction_conflict() -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        "attempt_start_index_changed",
        "The execution index changed during Attempt start.",
        json!({}),
    )
}

fn command_interrupted(receipt: &str) -> WorkError {
    WorkError::new(
        ExitCode::IoFailure,
        "command_run_interrupted",
        "Preserve command evidence and inspect effects; do not rerun this record.",
        json!({"receipt_prefix":receipt}),
    )
}

fn is_runnable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn recovery_rule(issue: work_operations::execution::ExecutionIssue) -> WorkError {
    let code = if issue.reason_code.starts_with("execution_authorization_")
        || matches!(
            issue.reason_code,
            "execution_recovery_record_result_missing"
                | "execution_recovery_closed_attempt_missing"
        ) {
        ExitCode::WorkflowState
    } else {
        ExitCode::ArtifactIntegrity
    };
    WorkError::new(code, issue.reason_code, issue.message, issue.details)
}

fn prepare_recovery_file(path: &std::path::Path, expected: &[u8]) -> Result<(), WorkError> {
    if path.exists() {
        if LocalFiles.read_raw(path)? != expected {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "execution_recovery_prepared_bytes_mismatch",
                "The prepared transaction bytes do not match the canonical target.",
                json!({"path":path.to_string_lossy()}),
            ));
        }
        return Ok(());
    }
    LocalFiles.create_new(path, expected).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "execution_recovery_prepare_failed",
            "The canonical recovery target could not be prepared.",
            json!({"path":path.to_string_lossy()}),
        )
    })?;
    if LocalFiles.read_raw(path)? != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            json!({"path":path.to_string_lossy(),"recovery_required":true,
                "transaction_stage":"recovery_target_prepared"}),
        ));
    }
    Ok(())
}

fn install_recovery_file(
    temporary: &std::path::Path,
    target: &std::path::Path,
    expected: &[u8],
    source: &[u8],
    stage: &str,
) -> Result<(), WorkError> {
    if LocalFiles.read_raw(temporary)? != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            json!({"path":temporary.to_string_lossy()}),
        ));
    }
    if LocalFiles.read_raw(target)? != source {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_source_changed",
            "A recovery source changed before replacement.",
            json!({"path":target.to_string_lossy()}),
        ));
    }
    LocalFiles.replace(temporary, target).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "execution_recovery_replace_failed",
            "The verified recovery target could not be installed.",
            json!({"path":temporary.to_string_lossy(),"recovery_required":true,
            "transaction_stage":stage}),
        )
    })?;
    if LocalFiles.read_raw(target)? != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_stored_bytes_mismatch",
            "The installed recovery bytes do not match the verified target.",
            json!({"path":target.to_string_lossy(),"recovery_required":true,
                "transaction_stage":stage}),
        ));
    }
    Ok(())
}

type RuntimeExecutionState = std::collections::BTreeMap<String, Option<Vec<u8>>>;

fn runtime_execution_plan_states(
    plan: &RuntimeExecutionPlan,
) -> Result<Vec<RuntimeExecutionState>, WorkError> {
    if plan.steps.is_empty() {
        return Err(runtime_scan_error(
            "execution_staging_plan_invalid",
            &plan.manifest.execution_dir,
        ));
    }
    let mut state = plan
        .manifest
        .targets
        .iter()
        .map(|target| {
            (
                target.path.clone(),
                target.before.as_ref().map(|bytes| bytes.bytes.clone()),
            )
        })
        .collect::<RuntimeExecutionState>();
    let mut states = vec![state.clone()];
    for step in &plan.steps {
        if state.get(&step.target) != Some(&step.before)
            || step
                .prepared_file
                .as_ref()
                .is_some_and(|file| plan.payloads.get(file) != Some(&step.after))
            || step.before.is_some() && step.prepared_file.is_none()
        {
            return Err(runtime_scan_error(
                "execution_staging_plan_invalid",
                &step.target,
            ));
        }
        state.insert(step.target.clone(), Some(step.after.clone()));
        states.push(state.clone());
    }
    let final_state = plan
        .manifest
        .targets
        .iter()
        .map(|target| {
            (
                target.path.clone(),
                target.after.as_ref().map(|bytes| bytes.bytes.clone()),
            )
        })
        .collect::<RuntimeExecutionState>();
    if state != final_state {
        return Err(runtime_scan_error(
            "execution_staging_plan_invalid",
            &plan.manifest.execution_dir,
        ));
    }
    Ok(states)
}

fn read_runtime_execution_targets(
    store: &impl ArtifactStore,
    root: &std::path::Path,
    manifest: &work_model::runtime::RuntimeManifest,
) -> Result<RuntimeExecutionState, WorkError> {
    manifest
        .targets
        .iter()
        .map(|target| {
            let path = crate::files::resolve_runtime_path(root, &target.path)?;
            let raw = if path.exists() {
                Some(store.read_raw(&path)?)
            } else {
                None
            };
            Ok((target.path.clone(), raw))
        })
        .collect()
}

fn runtime_scan_error(reason: &str, path: &str) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        reason,
        "Runtime evidence requires a complete verified transaction inventory.",
        json!({"path":path}),
    )
}

fn is_retained_journal_operation(operation: &str) -> bool {
    matches!(
        operation,
        "specification-update"
            | "specification-migration"
            | "specification-migration-item"
            | "specification-migration-reconcile"
            | "instruction-migration"
            | "source-refresh"
    )
}

fn collect_runtime_inventory_files(
    root: &std::path::Path,
    transaction: &str,
    parent: &str,
    expected: &std::collections::BTreeMap<String, &work_model::runtime::RuntimeFile>,
    manifest: &work_model::runtime::RuntimeManifest,
    files: &mut std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<(), WorkError> {
    let relative = if parent.is_empty() {
        transaction.to_owned()
    } else {
        format!("{transaction}/{parent}")
    };
    let directory = crate::files::resolve_runtime_path(root, &relative)?;
    for entry in fs::read_dir(&directory)
        .map_err(|_| runtime_scan_error("runtime_inventory_read_failed", &relative))?
    {
        let entry =
            entry.map_err(|_| runtime_scan_error("runtime_inventory_read_failed", &relative))?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| runtime_scan_error("runtime_inventory_foreign", &relative))?;
        let file = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        let relative = format!("{transaction}/{file}");
        let path = crate::files::resolve_runtime_path(root, &relative)?;
        if path.is_dir() {
            if !expected
                .keys()
                .any(|key| key.starts_with(&format!("{file}/")))
            {
                return Err(runtime_scan_error("runtime_inventory_foreign", &relative));
            }
            collect_runtime_inventory_files(root, transaction, &file, expected, manifest, files)?;
            continue;
        }
        if !path.is_file() {
            return Err(runtime_scan_error("runtime_inventory_foreign", &relative));
        }
        let raw = LocalFiles.read_raw(&path)?;
        if file == "transaction.json" {
            if serde_json::to_vec(manifest)
                .map_err(|_| runtime_scan_error("runtime_manifest_invalid", transaction))?
                != raw
            {
                return Err(runtime_scan_error(
                    "runtime_manifest_context_mismatch",
                    &relative,
                ));
            }
        } else if file == "transaction.json.tmp" {
            if matches!(
                manifest.operation.as_str(),
                "attempt-start"
                    | "record-begin"
                    | "command-correction"
                    | "record-finish"
                    | "deviation-record"
                    | "attempt-close"
                    | "correction"
            ) {
                work_operations::execution::recovery::execution_control_candidates(manifest, &raw)
                    .map_err(|_| {
                        runtime_scan_error("runtime_control_temporary_invalid", &relative)
                    })?;
            } else if is_retained_journal_operation(&manifest.operation) {
                work_operations::derivation::transaction::journal_control_candidates(
                    manifest, &raw,
                )
                .map_err(|_| runtime_scan_error("runtime_control_temporary_invalid", &relative))?;
            } else {
                let next: work_model::runtime::RuntimeManifest = serde_json::from_slice(&raw)
                    .map_err(|_| {
                        runtime_scan_error("runtime_control_temporary_invalid", &relative)
                    })?;
                crate::transaction_storage::runtime_manifest_directory(root, &next)?;
                let mut identity = manifest.clone();
                identity.phase = next.phase;
                identity.published_count = next.published_count;
                let rank = |phase| match phase {
                    work_model::runtime::RuntimePhase::Prepared => 0,
                    work_model::runtime::RuntimePhase::Publishing => 1,
                    work_model::runtime::RuntimePhase::PublishedVerified => 2,
                    work_model::runtime::RuntimePhase::Cleaning => 3,
                    work_model::runtime::RuntimePhase::Restoring => 4,
                    work_model::runtime::RuntimePhase::Restored => 5,
                };
                if identity != next
                    || next.published_count < manifest.published_count
                    || rank(next.phase) < rank(manifest.phase)
                    || serde_json::to_vec(&next).map_err(|_| {
                        runtime_scan_error("runtime_control_temporary_invalid", &relative)
                    })? != raw
                {
                    return Err(runtime_scan_error(
                        "runtime_control_temporary_invalid",
                        &relative,
                    ));
                }
            }
        } else {
            let evidence = expected
                .get(&file)
                .ok_or_else(|| runtime_scan_error("runtime_inventory_foreign", &relative))?;
            if raw.len() as u64 != evidence.size_bytes
                || !work_operations::derivation::fingerprint::verify_raw(&raw, &evidence.sha256)
            {
                let frozen = if is_retained_journal_operation(&manifest.operation) {
                    work_operations::derivation::transaction::restore_journal_staging(manifest)
                        .map(|prepared| prepared.payloads)
                        .map_err(|_| {
                            runtime_scan_error("runtime_inventory_hash_mismatch", &relative)
                        })?
                } else {
                    work_operations::execution::recovery::execution_staging_payloads(manifest)
                        .map_err(|_| {
                            runtime_scan_error("runtime_inventory_hash_mismatch", &relative)
                        })?
                };
                if !frozen
                    .get(&file)
                    .is_some_and(|expected| expected.starts_with(&raw))
                {
                    return Err(runtime_scan_error(
                        "runtime_inventory_hash_mismatch",
                        &relative,
                    ));
                }
            }
        }
        files.insert(file, raw);
    }
    Ok(())
}

fn transaction_io(reason: &str, path: &std::path::Path) -> WorkError {
    WorkError::new(
        ExitCode::IoFailure,
        reason,
        "Attempt-start storage could not be updated.",
        json!({"path":path.to_string_lossy()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_start_staging_recovers_each_boundary_from_original_authorization() {
        let mut boundaries = vec![
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ];
        for step in 0..3 {
            boundaries.extend([
                RuntimeExecutionStage::StepWritten(step),
                RuntimeExecutionStage::StepVerified(step),
                RuntimeExecutionStage::ProgressWritten(step),
            ]);
        }
        for boundary in boundaries {
            let (storage, writer, _) = receipt_candidate_case();
            let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
            let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
                skill_root: crate::fixture_support::historical_task_skill_root(
                    &repo.join("../skills/work"),
                )
                .unwrap(),
            };
            let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
            let paths = crate::artifact_paths::LocalArtifactPaths {
                project_root: storage.project_root.clone(),
            };
            let tasks = crate::task::storage::LocalTaskStorage {
                project_root: storage.project_root.clone(),
            };
            let sources = CommandProjectSources {
                instructions: &instructions,
                skills: &skills,
                paths: &paths,
                task_repository: &tasks,
                skill_roots: &[],
            };
            let adapter = RuntimeExecutionStorage {
                storage: &storage,
                context: &writer,
            };
            let target = writer.target();
            let choice = json!({"command_positions":[1],"validation_positions":[1],"modifiable_files":[],
                "external_operation_positions":[],"allowed_deviations":[],"authorization_evidence":"Approved","carried_records":[]});
            let reviewed = work_feature::execution::prepare_attempt_start_from_project(
                &sources,
                &adapter,
                target,
                &[],
                &choice,
            )
            .unwrap();
            let request = &reviewed["request"];
            let preflight =
                prepare_execute_preflight_from_project(&sources, &adapter, target, &[]).unwrap();
            let index_raw = fs::read(
                storage
                    .project_root
                    .join(format!("{}/index.json", target.execution_dir)),
            )
            .unwrap();
            let index = parse_json_contract(&index_raw).unwrap();
            let attempt = build_attempt_candidate(
                &preflight,
                &index,
                request,
                "ATTEMPT-001",
                "2026-10-07T10:00+08:00",
                None,
            )
            .unwrap();
            let attempt_raw =
                work_operations::execution::attempt::render_attempt(&attempt).unwrap();
            let mut locked = index.clone();
            locked["lock"] = build_execution_lock(
                target.task_id,
                "ATTEMPT-001",
                attempt["execute_instructions_sha256"].as_str().unwrap(),
            );
            let locked_raw =
                work_operations::execution::index::render_execution_index(&locked).unwrap();
            let started_raw = work_operations::execution::index::render_execution_index(
                &start_index(&locked, target.task_id, "ATTEMPT-001").unwrap(),
            )
            .unwrap();
            let plan = storage
                .attempt_start_staging_plan(
                    &writer,
                    &AttemptStartPublication {
                        execution_dir: target.execution_dir,
                        task_id: target.task_id,
                        attempt_id: "ATTEMPT-001",
                        index_before: &index_raw,
                        locked_index: &locked_raw,
                        attempt: &attempt_raw,
                        started_index: &started_raw,
                        expected_snapshot: request["worktree_snapshot_sha256"].as_str().unwrap(),
                    },
                )
                .unwrap();
            let failure = storage
                .with_runtime_execution_writer(&writer, |_| {
                    storage.run_execution_staging_scoped(
                        &writer,
                        &plan,
                        false,
                        &LocalFiles,
                        |stage| {
                            if stage == boundary {
                                Err(runtime_scan_error(
                                    "injected_attempt_start_boundary",
                                    target.execution_dir,
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(failure.reason_code, "injected_attempt_start_boundary");
            let mut altered = request.clone();
            altered["authorization"]["authorization_evidence"] = json!("Different approval");
            let rejected = storage
                .with_runtime_execution_writer(&writer, |_| {
                    storage.recover_attempt_start_staging_scoped(
                        &writer,
                        &sources,
                        AttemptStartRecoveryRequest {
                            target,
                            request: &altered,
                            confirmed_inputs: &[],
                            started_at: "ignored",
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(
                rejected.reason_code,
                "attempt_start_recovery_attempt_mismatch"
            );
            let recovered = storage
                .with_runtime_execution_writer(&writer, |_| {
                    storage.recover_attempt_start_staging_scoped(
                        &writer,
                        &sources,
                        AttemptStartRecoveryRequest {
                            target,
                            request,
                            confirmed_inputs: &[],
                            started_at: "2026-10-08T10:00+08:00",
                        },
                    )
                })
                .unwrap();
            assert_eq!(recovered["status"], "recovered");
            assert_eq!(
                fs::read(
                    storage
                        .project_root
                        .join(format!("{}/index.json", target.execution_dir))
                )
                .unwrap(),
                started_raw
            );
            assert_eq!(
                fs::read(storage.project_root.join(format!(
                    "{}/TASK-001/ATTEMPT-001/attempt.json",
                    target.execution_dir
                )))
                .unwrap(),
                attempt_raw
            );
            assert!(
                storage
                    .runtime_execution_inventory(&writer)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    fn started_staging_case() -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
    ) {
        started_staging_case_with_actions(&[])
    }

    fn started_staging_case_with_actions(
        actions: &[Value],
    ) -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
    ) {
        started_staging_case_with_downstream(actions, false)
    }

    fn started_staging_case_with_downstream(
        actions: &[Value],
        downstream: bool,
    ) -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
    ) {
        let (storage, writer, _) = receipt_candidate_case_with_downstream(downstream);
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(
                &repo.join("../skills/work"),
            )
            .unwrap(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: storage.project_root.clone(),
        };
        let tasks = crate::task::storage::LocalTaskStorage {
            project_root: storage.project_root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &instructions,
            skills: &skills,
            paths: &paths,
            task_repository: &tasks,
            skill_roots: &[],
        };
        let adapter = RuntimeExecutionStorage {
            storage: &storage,
            context: &writer,
        };
        let target = writer.target();
        let choice = json!({"command_positions":[1],"validation_positions":[1],"modifiable_files":[],
            "external_operation_positions":[],"allowed_deviations":actions,"authorization_evidence":"Approved","carried_records":[]});
        let prepared = work_feature::execution::prepare_attempt_start_from_project(
            &sources,
            &adapter,
            target,
            &[],
            &choice,
        )
        .unwrap();
        work_feature::execution::start_attempt_from_project(
            &sources,
            &adapter,
            target,
            &[],
            &prepared["request"],
            "2026-10-07T10:00+08:00",
        )
        .unwrap();
        (storage, writer)
    }

    fn reviewed_staging_request(
        storage: &LocalExecutionStorage,
        context: &work_feature::execution::ExecutionWriterContext,
        transaction: &str,
    ) -> Value {
        let pending = storage.runtime_execution_inventory(context).unwrap();
        assert_eq!(pending.len(), 1);
        let item = &pending[0];
        let binding = work_operations::execution::recovery::execution_staging_binding(
            &item.manifest,
            context.writer().canonical_project_root.to_str().unwrap(),
            &context.writer().requirement_id,
            context.target().execution_dir,
            context.target().task_id,
            &item.files,
        )
        .unwrap();
        json!({"schema":"work-execution-recovery-request","transaction":transaction,
            "attempt_id":item.manifest.business_identity["attempt_id"],"transaction_dir":binding.transaction_dir,
            "transaction_files":binding.transaction_files,"transaction_evidence_sha256":work_operations::execution::recovery::execution_staging_evidence_sha256(&binding).unwrap()})
    }

    #[test]
    fn record_begin_staging_resumes_reserved_index_and_rejects_stale_approval() {
        for boundary in [
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::StepWritten(0),
            RuntimeExecutionStage::StepVerified(0),
            RuntimeExecutionStage::ProgressWritten(0),
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ] {
            let (storage, context) = started_staging_case();
            let execution = context.target().execution_dir;
            let index_path = format!("{execution}/index.json");
            let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
            let index_raw = fs::read(storage.project_root.join(&index_path)).unwrap();
            let attempt_raw = fs::read(storage.project_root.join(&attempt_path)).unwrap();
            let index = parse_json_contract(&index_raw).unwrap();
            let attempt = parse_json_contract(&attempt_raw).unwrap();
            let task = &context.task_context().contract["tasks"][0];
            let (after, record, _) = work_operations::execution::record_begin_candidate(
                task, &attempt, &index, "TASK-001", "CMD-001", None,
            )
            .unwrap();
            let after_raw =
                work_operations::execution::index::render_execution_index(&after).unwrap();
            let publication = RecordBeginPublication {
                execution_dir: execution,
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                record_id: &record,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
                index_after: &after_raw,
            };
            let plan = storage
                .record_begin_staging_plan(&context, &publication)
                .unwrap();
            let issue = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_execution_staging_scoped(
                        &context,
                        &plan,
                        false,
                        &LocalFiles,
                        |stage| {
                            if stage == boundary {
                                Err(runtime_scan_error(
                                    "injected_record_begin_boundary",
                                    execution,
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(issue.reason_code, "injected_record_begin_boundary");
            let request = reviewed_staging_request(&storage, &context, "record_begin");
            let mut stale = request.clone();
            stale["transaction_evidence_sha256"] = json!("a".repeat(64));
            assert_eq!(
                storage
                    .with_runtime_execution_writer(&context, |_| storage
                        .recover_record_begin_staging_scoped(&context, &stale))
                    .unwrap_err()
                    .reason_code,
                "execution_recovery_evidence_changed"
            );
            let result = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.recover_record_begin_staging_scoped(&context, &request)
                })
                .unwrap();
            assert_eq!(result["record_id"], "CMD-001");
            assert_eq!(
                fs::read(storage.project_root.join(&index_path)).unwrap(),
                after_raw
            );
            assert_eq!(
                fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                attempt_raw
            );
            assert!(
                storage
                    .runtime_execution_inventory(&context)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn command_correction_staging_resumes_reserved_index_and_rejects_stale_approval() {
        for boundary in [
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::StepWritten(0),
            RuntimeExecutionStage::StepVerified(0),
            RuntimeExecutionStage::ProgressWritten(0),
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ] {
            let action = json!({"anchor_kind":"command","anchor_position":1,
                "action":{"kind":"replace_command","replacement":{"mode":"argv","argv":["python3","--version"]}}});
            let (storage, context) = started_staging_case_with_actions(&[action]);
            let execution = context.target().execution_dir;
            let index_path = format!("{execution}/index.json");
            let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
            let index_raw = fs::read(storage.project_root.join(&index_path)).unwrap();
            let attempt_raw = fs::read(storage.project_root.join(&attempt_path)).unwrap();
            let index = parse_json_contract(&index_raw).unwrap();
            let attempt = parse_json_contract(&attempt_raw).unwrap();
            let task = &context.task_context().contract["tasks"][0];
            let (reserved, _, _) = work_operations::execution::record_begin_candidate(
                task, &attempt, &index, "TASK-001", "CMD-001", None,
            )
            .unwrap();
            let index_raw =
                work_operations::execution::index::render_execution_index(&reserved).unwrap();
            fs::write(storage.project_root.join(&index_path), &index_raw).unwrap();
            let (after,record) = work_operations::execution::command_correction::build_command_correction_candidate(
                task,&attempt,&reserved,"TASK-001",&json!({"schema":"work-command-correction-request",
                    "actual_command":{"mode":"argv","argv":["python3","--version"]},"reason":"Use the equivalent approved executable."})).unwrap();
            let after_raw =
                work_operations::execution::index::render_execution_index(&after).unwrap();
            let publication = CommandCorrectionPublication {
                execution_dir: execution,
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                record_id: &record,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
                index_after: &after_raw,
            };
            let plan = storage
                .command_correction_staging_plan(&context, &publication)
                .unwrap();
            let issue = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_execution_staging_scoped(
                        &context,
                        &plan,
                        false,
                        &LocalFiles,
                        |stage| {
                            if stage == boundary {
                                Err(runtime_scan_error(
                                    "injected_command_correction_boundary",
                                    execution,
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(issue.reason_code, "injected_command_correction_boundary");
            let request = reviewed_staging_request(&storage, &context, "command_correction");
            let mut stale = request.clone();
            stale["transaction_evidence_sha256"] = json!("a".repeat(64));
            assert_eq!(
                storage
                    .with_runtime_execution_writer(&context, |_| storage
                        .recover_command_correction_staging_scoped(&context, &stale))
                    .unwrap_err()
                    .reason_code,
                "execution_recovery_evidence_changed"
            );
            let result = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.recover_command_correction_staging_scoped(&context, &request)
                })
                .unwrap();
            assert_eq!(result["record_id"], "CMD-001");
            assert_eq!(
                fs::read(storage.project_root.join(&index_path)).unwrap(),
                after_raw
            );
            assert_eq!(
                fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                attempt_raw
            );
            assert!(
                storage
                    .runtime_execution_inventory(&context)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn record_finish_staging_recovers_result_once_and_requires_failure_authorization() {
        let mut boundaries = vec![
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ];
        for step in 0..2 {
            boundaries.extend([
                RuntimeExecutionStage::StepWritten(step),
                RuntimeExecutionStage::StepVerified(step),
                RuntimeExecutionStage::ProgressWritten(step),
            ]);
        }
        for exit_code in [0, 7] {
            for boundary in &boundaries {
                let (storage, context) = started_staging_case();
                let execution = context.target().execution_dir;
                let index_path = format!("{execution}/index.json");
                let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
                let index_raw = fs::read(storage.project_root.join(&index_path)).unwrap();
                let attempt_raw = fs::read(storage.project_root.join(&attempt_path)).unwrap();
                let index = parse_json_contract(&index_raw).unwrap();
                let attempt = parse_json_contract(&attempt_raw).unwrap();
                let task = &context.task_context().contract["tasks"][0];
                let (reserved, _, _) = work_operations::execution::record_begin_candidate(
                    task, &attempt, &index, "TASK-001", "CMD-001", None,
                )
                .unwrap();
                let index_raw =
                    work_operations::execution::index::render_execution_index(&reserved).unwrap();
                fs::write(storage.project_root.join(&index_path), &index_raw).unwrap();
                let mut finish = json!({"schema":"work-record-finish-request","record":{"exit_code":exit_code,"result":"Previously captured command result"}});
                if exit_code != 0 {
                    finish["authorization_evidence"] = json!("Approved failure result");
                }
                let candidates =
                    work_operations::execution::record_finish::build_record_finish_candidates(
                        task, &attempt, &reserved, "TASK-001", &finish,
                    )
                    .unwrap();
                let after_attempt =
                    work_operations::execution::attempt::render_attempt(&candidates.attempt)
                        .unwrap();
                let after_index =
                    work_operations::execution::index::render_execution_index(&candidates.index)
                        .unwrap();
                let publication = RecordFinishPublication {
                    execution_dir: execution,
                    task_id: "TASK-001",
                    attempt_id: "ATTEMPT-001",
                    record_id: "CMD-001",
                    request: &finish,
                    index_before: &index_raw,
                    index_after: &after_index,
                    attempt_before: &attempt_raw,
                    attempt_after: &after_attempt,
                };
                let plan = storage
                    .record_finish_staging_plan(&context, &publication)
                    .unwrap();
                let failure = storage
                    .with_runtime_execution_writer(&context, |_| {
                        storage.run_execution_staging_scoped(
                            &context,
                            &plan,
                            false,
                            &LocalFiles,
                            |stage| {
                                if stage == *boundary {
                                    Err(runtime_scan_error(
                                        "injected_record_finish_boundary",
                                        execution,
                                    ))
                                } else {
                                    Ok(())
                                }
                            },
                        )
                    })
                    .unwrap_err();
                assert_eq!(failure.reason_code, "injected_record_finish_boundary");
                let mut request = reviewed_staging_request(&storage, &context, "record_finish");
                if exit_code != 0 {
                    assert_eq!(
                        storage
                            .with_runtime_execution_writer(&context, |_| storage
                                .recover_record_finish_staging_scoped(&context, &request))
                            .unwrap_err()
                            .reason_code,
                        "execution_authorization_result_required"
                    );
                    request["authorization_evidence"] = json!("Reviewed failure recovery");
                }
                let result = storage
                    .with_runtime_execution_writer(&context, |_| {
                        storage.recover_record_finish_staging_scoped(&context, &request)
                    })
                    .unwrap();
                assert_eq!(result["record_id"], "CMD-001");
                assert_eq!(
                    fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                    after_attempt
                );
                assert_eq!(
                    fs::read(storage.project_root.join(&index_path)).unwrap(),
                    after_index
                );
                assert_eq!(
                    parse_json_contract(&after_attempt).unwrap()["records"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
                assert!(
                    storage
                        .runtime_execution_inventory(&context)
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn deviation_staging_binds_preview_sources_and_recovers_append_once() {
        for boundary in [
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::StepWritten(0),
            RuntimeExecutionStage::StepVerified(0),
            RuntimeExecutionStage::ProgressWritten(0),
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ] {
            let (storage, context) = started_staging_case();
            let execution = context.target().execution_dir;
            let index_path = format!("{execution}/index.json");
            let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
            let index_raw = fs::read(storage.project_root.join(&index_path)).unwrap();
            let attempt_raw = fs::read(storage.project_root.join(&attempt_path)).unwrap();
            let index = parse_json_contract(&index_raw).unwrap();
            let attempt = parse_json_contract(&attempt_raw).unwrap();
            let task = &context.task_context().contract["tasks"][0];
            let (reserved, _, _) = work_operations::execution::record_begin_candidate(
                task, &attempt, &index, "TASK-001", "CMD-001", None,
            )
            .unwrap();
            let index_raw =
                work_operations::execution::index::render_execution_index(&reserved).unwrap();
            fs::write(storage.project_root.join(&index_path), &index_raw).unwrap();
            let mut sources = context.task_context().sources.clone();
            sources.insert(index_path.clone(), index_raw);
            sources.insert(attempt_path.clone(), attempt_raw.clone());
            sources.insert("src.txt".into(), b"source\n".to_vec());
            let hashes = sources
                .iter()
                .map(|(path, raw)| {
                    (
                        path.clone(),
                        work_operations::derivation::fingerprint::raw(raw),
                    )
                })
                .collect();
            let proposal = json!({"schema":"work-execution-deviation-proposal","task_id":"TASK-001","attempt_id":"ATTEMPT-001",
                "anchor_record_id":"CMD-001","task_basis":["CMD-001","STEP-001"],"gap":"Executable unavailable.",
                "action":{"kind":"replace_command","record_id":"CMD-001","replacement":{"mode":"argv","argv":["python3","--version"]}},
                "impact":{"summary":"Equivalent approved executable.","requirement_changed":false,"scope_changed":false,"acceptance_criteria_changed":false,
                    "deliverables_changed":false,"safety_boundary_changed":false,"external_side_effect_boundary_changed":false},"modifiable_files":[],"side_effects":["Runs equivalent existing command."]});
            let preview = work_operations::execution::deviation::build_deviation_preview(
                &proposal, "command", &hashes,
            )
            .unwrap();
            let (candidate, _) = work_operations::execution::deviation::append_approved_deviation(
                &attempt,
                &preview,
                preview["preview_sha256"].as_str().unwrap(),
                "Approved supplemental replacement",
                execution,
            )
            .unwrap();
            let after = work_operations::execution::attempt::render_attempt(&candidate).unwrap();
            let publication = DeviationRecordPublication {
                execution_dir: execution,
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                record_id: "CMD-001",
                attempt_before: &attempt_raw,
                attempt_after: &after,
                sources: &sources,
            };
            let plan = storage
                .deviation_staging_plan(&context, &publication)
                .unwrap();
            let failure = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_execution_staging_scoped(
                        &context,
                        &plan,
                        false,
                        &LocalFiles,
                        |stage| {
                            if stage == boundary {
                                Err(runtime_scan_error("injected_deviation_boundary", execution))
                            } else {
                                Ok(())
                            }
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(failure.reason_code, "injected_deviation_boundary");
            let request = reviewed_staging_request(&storage, &context, "deviation_record");
            fs::write(
                storage.project_root.join("src.txt"),
                b"changed by isolated test actor\n",
            )
            .unwrap();
            assert_eq!(
                storage
                    .with_runtime_execution_writer(&context, |_| storage
                        .recover_deviation_staging_scoped(&context, &request))
                    .unwrap_err()
                    .reason_code,
                "execution_staging_source_changed"
            );
            assert_eq!(
                fs::read(storage.project_root.join("src.txt")).unwrap(),
                b"changed by isolated test actor\n"
            );
            fs::write(storage.project_root.join("src.txt"), b"source\n").unwrap();
            let result = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.recover_deviation_staging_scoped(&context, &request)
                })
                .unwrap();
            assert_eq!(result["lock_status"], "record_reserved");
            assert_eq!(
                fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                after
            );
            assert_eq!(
                parse_json_contract(&after).unwrap()["execution_deviations"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                fs::read(storage.project_root.join(&index_path)).unwrap(),
                sources[&index_path]
            );
            assert!(
                storage
                    .runtime_execution_inventory(&context)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn attempt_close_staging_recovers_completed_stopped_and_blocked_transitions() {
        let mut boundaries = vec![
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ];
        for step in 0..2 {
            boundaries.extend([
                RuntimeExecutionStage::StepWritten(step),
                RuntimeExecutionStage::StepVerified(step),
                RuntimeExecutionStage::ProgressWritten(step),
            ]);
        }
        for status in ["completed", "stopped", "blocked"] {
            for boundary in &boundaries {
                let (storage, context) = started_staging_case();
                let execution = context.target().execution_dir;
                let index_path = format!("{execution}/index.json");
                let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
                let mut index =
                    parse_json_contract(&fs::read(storage.project_root.join(&index_path)).unwrap())
                        .unwrap();
                let mut attempt = parse_json_contract(
                    &fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                )
                .unwrap();
                let task = &context.task_context().contract["tasks"][0];
                if status == "completed" {
                    for (record, result) in [
                        (
                            "CMD-001",
                            json!({"exit_code":0,"result":"Captured success"}),
                        ),
                        (
                            "VAL-001",
                            json!({"outcome":"passed","evidence":"Validation passed"}),
                        ),
                    ] {
                        let (reserved, _, _) = work_operations::execution::record_begin_candidate(
                            task, &attempt, &index, "TASK-001", record, None,
                        )
                        .unwrap();
                        let finish = json!({"schema":"work-record-finish-request","record":result});
                        let next=work_operations::execution::record_finish::build_record_finish_candidates(task,&attempt,&reserved,"TASK-001",&finish).unwrap();
                        index = next.index;
                        attempt = next.attempt;
                    }
                }
                let before_index =
                    work_operations::execution::index::render_execution_index(&index).unwrap();
                let before_attempt =
                    work_operations::execution::attempt::render_attempt(&attempt).unwrap();
                fs::write(storage.project_root.join(&index_path), &before_index).unwrap();
                fs::write(storage.project_root.join(&attempt_path), &before_attempt).unwrap();
                let mut close = json!({"schema":"work-attempt-close-request","status":status});
                if status != "completed" {
                    close["final_type"] = json!("other");
                    close["reason"] = json!("Approved close");
                    close["authorization_evidence"] = json!("Approved close evidence");
                }
                let (closed, next_index) =
                    work_operations::execution::attempt_close::build_close_candidates(
                        task,
                        &index,
                        &attempt,
                        &close,
                        "2026-10-07T10:20+08:00",
                    )
                    .unwrap();
                let after_attempt =
                    work_operations::execution::attempt::render_attempt(&closed).unwrap();
                let after_index =
                    work_operations::execution::index::render_execution_index(&next_index).unwrap();
                let publication = AttemptClosePublication {
                    execution_dir: execution,
                    task_id: "TASK-001",
                    attempt_id: "ATTEMPT-001",
                    index_before: &before_index,
                    index_after: &after_index,
                    attempt_before: &before_attempt,
                    attempt_after: &after_attempt,
                };
                let plan = storage
                    .attempt_close_staging_plan(&context, &publication)
                    .unwrap();
                let failure = storage
                    .with_runtime_execution_writer(&context, |_| {
                        storage.run_execution_staging_scoped(
                            &context,
                            &plan,
                            false,
                            &LocalFiles,
                            |stage| {
                                if stage == *boundary {
                                    Err(runtime_scan_error(
                                        "injected_attempt_close_boundary",
                                        execution,
                                    ))
                                } else {
                                    Ok(())
                                }
                            },
                        )
                    })
                    .unwrap_err();
                assert_eq!(failure.reason_code, "injected_attempt_close_boundary");
                let request = reviewed_staging_request(&storage, &context, "attempt_close");
                let result = storage
                    .with_runtime_execution_writer(&context, |_| {
                        storage.recover_attempt_close_staging_scoped(&context, &request)
                    })
                    .unwrap();
                assert_eq!(result["attempt_status"], status);
                assert_eq!(result["lock_status"], "released");
                assert_eq!(
                    fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                    after_attempt
                );
                assert_eq!(
                    fs::read(storage.project_root.join(&index_path)).unwrap(),
                    after_index
                );
                assert!(
                    storage
                        .runtime_execution_inventory(&context)
                        .unwrap()
                        .is_empty()
                );
                if status == "completed" {
                    assert_eq!(closed["acceptance_results"][0]["status"], "completed");
                }
            }
        }
    }

    #[test]
    fn completed_close_recovery_refuses_project_file_drift_and_preserves_record_evidence() {
        use work_feature::execution::file_transaction::FileTransactionRepository;
        let root = std::env::temp_dir().join(format!(
            "work-close-project-files-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let skill = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let context = crate::fixture_support::file_transaction_fixture(&root, &skill).unwrap();
        let files = super::super::file_transaction_storage::LocalFileTransactions::for_project(
            root.clone(),
            skill,
        );
        let staged = ["b-moved.txt", "c-existing.txt", "d-new.txt"]
            .into_iter()
            .map(|p| {
                (
                    p.to_owned(),
                    format!("outputs/work/transactions/example/file-test/staging/{p}"),
                )
            })
            .collect();
        let preview = work_feature::execution::file_transaction::prepare(
            &files,
            &context,
            "ATTEMPT-001",
            &staged,
        )
        .unwrap();
        files
            .apply(&context, &preview, "Approve complete native publication")
            .unwrap();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let execution = context.target().execution_dir;
        let index_path = format!("{execution}/index.json");
        let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
        let index = parse_json_contract(&fs::read(root.join(&index_path)).unwrap()).unwrap();
        let attempt = parse_json_contract(&fs::read(root.join(&attempt_path)).unwrap()).unwrap();
        let task = &context.task_context().contract["tasks"][0];
        let (reserved, _, _) = work_operations::execution::record_begin_candidate(
            task, &attempt, &index, "TASK-001", "VAL-001", None,
        )
        .unwrap();
        let finish = json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Verified complete project-file outcome"}});
        let accepted = work_operations::execution::record_finish::build_record_finish_candidates(
            task, &attempt, &reserved, "TASK-001", &finish,
        )
        .unwrap();
        let before_index = crate::fixture_support::render_execution_index(&accepted.index).unwrap();
        let before_attempt =
            work_operations::execution::attempt::render_attempt(&accepted.attempt).unwrap();
        fs::write(root.join(&index_path), &before_index).unwrap();
        fs::write(root.join(&attempt_path), &before_attempt).unwrap();
        let (closed, index) = work_operations::execution::attempt_close::build_close_candidates(
            task,
            &accepted.index,
            &accepted.attempt,
            &json!({"schema":"work-attempt-close-request","status":"completed"}),
            &crate::clock_workspace::local_timestamp(),
        )
        .unwrap();
        let after_attempt = work_operations::execution::attempt::render_attempt(&closed).unwrap();
        let after_index = crate::fixture_support::render_execution_index(&index).unwrap();
        let publication = AttemptClosePublication {
            execution_dir: execution,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            index_before: &before_index,
            index_after: &after_index,
            attempt_before: &before_attempt,
            attempt_after: &after_attempt,
        };
        let plan = storage
            .attempt_close_staging_plan(&context, &publication)
            .unwrap();
        storage
            .with_runtime_execution_writer(&context, |_| {
                storage.run_execution_staging_scoped(&context, &plan, false, &LocalFiles, |stage| {
                    if stage == RuntimeExecutionStage::StepWritten(0) {
                        Err(runtime_scan_error("injected_close", execution))
                    } else {
                        Ok(())
                    }
                })
            })
            .unwrap_err();
        let request = reviewed_staging_request(&storage, &context, "attempt_close");
        let retained_index = fs::read(root.join(&index_path)).unwrap();
        let retained_attempt = fs::read(root.join(&attempt_path)).unwrap();
        let target = root.join("c-existing.txt");
        let published = fs::read(&target).unwrap();
        fs::write(&target, b"external actor changed output").unwrap();
        let failure = storage
            .with_runtime_execution_writer(&context, |_| {
                storage.recover_attempt_close_staging_scoped(&context, &request)
            })
            .unwrap_err();
        assert_eq!(failure.reason_code, "file_transaction_target_drift");
        assert_eq!(fs::read(root.join(&index_path)).unwrap(), retained_index);
        assert_eq!(
            fs::read(root.join(&attempt_path)).unwrap(),
            retained_attempt
        );
        assert_eq!(fs::read(&target).unwrap(), b"external actor changed output");
        // Only this synthetic actor restores its own bytes, then the original record
        // transaction may complete forward after its retained publication is reverified.
        fs::write(&target, published).unwrap();
        storage
            .with_runtime_execution_writer(&context, |_| {
                storage.recover_attempt_close_staging_scoped(&context, &request)
            })
            .unwrap();
        assert_eq!(fs::read(root.join(index_path)).unwrap(), after_index);
        assert_eq!(fs::read(root.join(attempt_path)).unwrap(), after_attempt);
    }

    #[test]
    fn readonly_inventory_context_preserves_handoff_without_execution_file_preflight() {
        let (storage, writer, _) = receipt_case(false, true);
        fs::remove_file(storage.project_root.join("src.txt")).unwrap();
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(
                &repo.join("../skills/work"),
            )
            .unwrap(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: storage.project_root.clone(),
        };
        let tasks = crate::task::storage::LocalTaskStorage {
            project_root: storage.project_root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &instructions,
            skills: &skills,
            paths: &paths,
            task_repository: &tasks,
            skill_roots: &[],
        };
        work_feature::execution::load_execution_writer_context(&sources, writer.target()).unwrap();
        let failure =
            prepare_execute_preflight_from_project(&sources, &storage, writer.target(), &[])
                .unwrap_err();
        assert_eq!(
            failure.reason_code,
            "execute_preflight_modify_target_missing"
        );
        let reader = work_feature::execution::load_execution_inventory_context(
            &sources,
            work_feature::execution::ExecutionInventoryTarget {
                task_path: writer.target().task_path,
                execution_dir: writer.target().execution_dir,
            },
        )
        .unwrap();
        storage
            .require_runtime_execution_inventory_ready(&reader)
            .unwrap();
        let (_, directory) = staging_inventory_case(
            &storage,
            &writer,
            work_operations::derivation::publication::RuntimeOperation::RecordBegin,
            'a',
        );
        let manifest = fs::read(directory.join("transaction.json")).unwrap();
        assert_eq!(
            storage
                .require_runtime_execution_inventory_ready(&reader)
                .unwrap_err()
                .reason_code,
            "runtime_execution_transaction_present"
        );
        assert_eq!(
            fs::read(directory.join("transaction.json")).unwrap(),
            manifest
        );
        assert!(!storage.project_root.join("src.txt").exists());
        assert!(
            !storage
                .project_root
                .join("outputs/work/runtime/locks/example/execution.lock")
                .exists()
        );
    }

    fn completed_staging_case_with_downstream(
        downstream: bool,
    ) -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
    ) {
        let (storage, context) = started_staging_case_with_downstream(&[], downstream);
        let execution = context.target().execution_dir;
        let index_path = format!("{execution}/index.json");
        let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
        let mut index =
            parse_json_contract(&fs::read(storage.project_root.join(&index_path)).unwrap())
                .unwrap();
        let mut attempt =
            parse_json_contract(&fs::read(storage.project_root.join(&attempt_path)).unwrap())
                .unwrap();
        let task = &context.task_context().contract["tasks"][0];
        for (record, result) in [
            (
                "CMD-001",
                json!({"exit_code":0,"result":"Captured success"}),
            ),
            (
                "VAL-001",
                json!({"outcome":"passed","evidence":"Validation passed"}),
            ),
        ] {
            let (reserved, _, _) = work_operations::execution::record_begin_candidate(
                task, &attempt, &index, "TASK-001", record, None,
            )
            .unwrap();
            let next = work_operations::execution::record_finish::build_record_finish_candidates(
                task,
                &attempt,
                &reserved,
                "TASK-001",
                &json!({"schema":"work-record-finish-request","record":result}),
            )
            .unwrap();
            index = next.index;
            attempt = next.attempt;
        }
        let (closed, mut index) =
            work_operations::execution::attempt_close::build_close_candidates(
                task,
                &index,
                &attempt,
                &json!({"schema":"work-attempt-close-request","status":"completed"}),
                "2026-10-07T10:20+08:00",
            )
            .unwrap();
        if downstream {
            index["tasks"][1]["status"] = json!("completed");
            index["tasks"][1]["latest_attempt"] = json!("ATTEMPT-001");
            index["overall_status"] = json!("completed");
        }
        fs::write(
            storage.project_root.join(&index_path),
            work_operations::execution::index::render_execution_index(&index).unwrap(),
        )
        .unwrap();
        fs::write(
            storage.project_root.join(&attempt_path),
            work_operations::execution::attempt::render_attempt(&closed).unwrap(),
        )
        .unwrap();
        (storage, context)
    }

    #[test]
    fn correction_staging_recovers_immutable_artifact_and_exact_invalidation() {
        let mut boundaries = vec![
            RuntimeExecutionStage::Prepared,
            RuntimeExecutionStage::PublishedVerified,
            RuntimeExecutionStage::BeforeCleanup,
        ];
        for step in 0..3 {
            boundaries.extend([
                RuntimeExecutionStage::StepWritten(step),
                RuntimeExecutionStage::StepVerified(step),
                RuntimeExecutionStage::ProgressWritten(step),
            ]);
        }
        for invalidates in [false, true] {
            for boundary in &boundaries {
                let (storage, context) = completed_staging_case_with_downstream(true);
                assert_eq!(
                    context.task_context().contract["tasks"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
                assert_eq!(
                    context.task_context().collection["tasks"]
                        .as_array()
                        .unwrap()
                        .len(),
                    2
                );
                let execution = context.target().execution_dir;
                let index_path = format!("{execution}/index.json");
                let attempt_path = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
                let index_raw = fs::read(storage.project_root.join(&index_path)).unwrap();
                let attempt_raw = fs::read(storage.project_root.join(&attempt_path)).unwrap();
                let index = parse_json_contract(&index_raw).unwrap();
                let attempt = parse_json_contract(&attempt_raw).unwrap();
                let correction_id = "ATTEMPT-001-CORRECTION-001";
                let request = json!({"schema":"work-correction-create-request","target_attempt_id":"ATTEMPT-001","field":"records[0].result",
                    "correct_value":"Corrected evidence","reason":"Approved immutable evidence correction","invalidates_completion":invalidates});
                let candidate = build_correction_candidates(
                    &context.task_context().collection,
                    &index,
                    &attempt,
                    "TASK-001",
                    &request,
                    correction_id,
                    "2026-10-07T10:30+08:00",
                )
                .unwrap();
                assert_eq!(
                    candidate.affected_task_ids,
                    if invalidates {
                        vec!["TASK-001".to_owned(), "TASK-002".to_owned()]
                    } else {
                        Vec::new()
                    }
                );
                let artifact = render_correction(&candidate.artifact).unwrap();
                let locked = work_operations::execution::index::render_execution_index(
                    &candidate.locked_index,
                )
                .unwrap();
                let final_index = work_operations::execution::index::render_execution_index(
                    &candidate.final_index,
                )
                .unwrap();
                let publication = CorrectionPublication {
                    execution_dir: execution,
                    task_id: "TASK-001",
                    attempt_id: "ATTEMPT-001",
                    correction_id,
                    index_before: &index_raw,
                    attempt_before: &attempt_raw,
                    artifact: &artifact,
                    locked_index: &locked,
                    final_index: &final_index,
                };
                let plan = storage
                    .correction_staging_plan(&context, &publication)
                    .unwrap();
                let failure = storage
                    .with_runtime_execution_writer(&context, |_| {
                        storage.run_execution_staging_scoped(
                            &context,
                            &plan,
                            false,
                            &LocalFiles,
                            |stage| {
                                if stage == *boundary {
                                    Err(runtime_scan_error(
                                        "injected_correction_boundary",
                                        execution,
                                    ))
                                } else {
                                    Ok(())
                                }
                            },
                        )
                    })
                    .unwrap_err();
                assert_eq!(failure.reason_code, "injected_correction_boundary");
                let recovery = reviewed_staging_request(&storage, &context, "correction");
                let result = storage
                    .with_runtime_execution_writer(&context, |_| {
                        storage.recover_correction_staging_scoped(&context, &recovery)
                    })
                    .unwrap();
                assert_eq!(
                    result["affected_task_ids"],
                    json!(candidate.affected_task_ids)
                );
                assert_eq!(result["lock_status"], "released");
                assert_eq!(
                    fs::read(storage.project_root.join(&index_path)).unwrap(),
                    final_index
                );
                assert_eq!(
                    fs::read(storage.project_root.join(&attempt_path)).unwrap(),
                    attempt_raw
                );
                assert_eq!(
                    fs::read(storage.project_root.join(format!(
                        "{execution}/TASK-001/ATTEMPT-001/corrections/{correction_id}.json"
                    )))
                    .unwrap(),
                    artifact
                );
                assert!(
                    storage
                        .runtime_execution_inventory(&context)
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }

    // Byte-publication engine cases are independent from the seven domain adapters prepared next.
    fn runtime_plan_case() -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
        RuntimeExecutionPlan,
    ) {
        use work_model::runtime::*;
        use work_operations::derivation::fingerprint;
        let (storage, context, _) = receipt_candidate_case();
        let execution = context.target().execution_dir;
        let index = format!("{execution}/index.json");
        let artifact = format!("{execution}/TASK-001/ATTEMPT-001/attempt.json");
        let before = fs::read(storage.project_root.join(&index)).unwrap();
        let mut value = parse_json_contract(&before).unwrap();
        value["tasks"][0]["status"] = json!("in_progress");
        value["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        let after = render_execution_index(&value).unwrap();
        let new = b"immutable byte-engine artifact\n".to_vec();
        let payloads = std::collections::BTreeMap::from([
            ("attempt.json.tmp".into(), new.clone()),
            ("index.json.tmp".into(), after.clone()),
        ]);
        let bytes = |raw: &[u8]| RuntimeBytes {
            sha256: fingerprint::raw(raw),
            bytes: raw.to_vec(),
        };
        let manifest=work_operations::execution::recovery::build_execution_staging_manifest(
            work_operations::execution::recovery::ExecutionStagingInput {
                canonical_root:context.writer().canonical_project_root.to_str().unwrap(),requirement:&context.writer().requirement_id,
                execution_dir:execution,operation:work_operations::derivation::publication::RuntimeOperation::RecordFinish,
                approval_sha256:&fingerprint::raw(&after),business_identity:json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"CMD-001"}),
                targets:vec![RuntimeTarget {path:artifact.clone(),before:None,after:Some(bytes(&new))},
                    RuntimeTarget {path:index.clone(),before:Some(bytes(&before)),after:Some(bytes(&after))}],payloads:&payloads,
            }).unwrap();
        let plan = RuntimeExecutionPlan {
            manifest,
            payloads,
            steps: vec![
                RuntimeExecutionStep {
                    target: artifact,
                    before: None,
                    after: new,
                    prepared_file: Some("attempt.json.tmp".into()),
                },
                RuntimeExecutionStep {
                    target: index,
                    before: Some(before),
                    after,
                    prepared_file: Some("index.json.tmp".into()),
                },
            ],
        };
        (storage, context, plan)
    }

    #[test]
    fn staging_engine_each_publication_boundary_recovers_exact_bytes_and_cleanup_without_republishing()
     {
        use RuntimeExecutionStage::*;
        let boundaries = [
            Prepared,
            StepWritten(0),
            StepVerified(0),
            ProgressWritten(0),
            StepWritten(1),
            StepVerified(1),
            ProgressWritten(1),
            PublishedVerified,
            BeforeCleanup,
            Cleaned,
        ];
        for boundary in boundaries {
            let (storage, context, plan) = runtime_plan_case();
            let issue = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_execution_staging_scoped(
                        &context,
                        &plan,
                        false,
                        &LocalFiles,
                        |stage| {
                            if stage == boundary {
                                Err(runtime_scan_error(
                                    "injected_staging_boundary",
                                    plan.manifest.operation.as_str(),
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(issue.reason_code, "injected_staging_boundary");
            storage
                .recover_execution_staging_candidate(&context, &plan)
                .unwrap();
            let directory = crate::transaction_storage::runtime_manifest_directory(
                &context.writer().canonical_project_root,
                &plan.manifest,
            )
            .unwrap();
            assert!(!directory.exists());
            for target in &plan.manifest.targets {
                assert_eq!(
                    fs::read(storage.project_root.join(&target.path)).unwrap(),
                    target.after.as_ref().unwrap().bytes
                );
            }
            storage
                .recover_execution_staging_candidate(&context, &plan)
                .unwrap();
            use work_feature::ports::RuntimeWriterLock;
            LocalWriterLock
                .require_runtime_idle(context.writer(), work_model::runtime::LockClass::Execution)
                .unwrap();
        }
    }

    struct FailStagingArtifactWrite {
        failed: Cell<bool>,
        partial: bool,
    }
    impl ArtifactStore for FailStagingArtifactWrite {
        fn read_raw(&self, path: &std::path::Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &std::path::Path, raw: &[u8]) -> Result<(), WorkError> {
            if path.file_name().is_some_and(|name| name == "attempt.json")
                && !self.failed.replace(true)
            {
                if self.partial {
                    LocalFiles.create_new(path, &raw[..7])?;
                }
                return Err(runtime_scan_error(
                    "injected_staging_artifact_write",
                    &path.to_string_lossy(),
                ));
            }
            LocalFiles.create_new(path, raw)
        }
        fn replace(
            &self,
            temporary: &std::path::Path,
            target: &std::path::Path,
        ) -> Result<(), WorkError> {
            LocalFiles.replace(temporary, target)
        }
        fn remove(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)
        }
        fn create_directories(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
    }

    struct FailStagingControlWrite {
        tail: bool,
    }
    impl ArtifactStore for FailStagingControlWrite {
        fn read_raw(&self, path: &std::path::Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &std::path::Path, raw: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, raw)
        }
        fn replace(
            &self,
            temporary: &std::path::Path,
            target: &std::path::Path,
        ) -> Result<(), WorkError> {
            if temporary
                .file_name()
                .is_some_and(|name| name == "transaction.json.tmp")
            {
                let raw = LocalFiles.read_raw(temporary)?;
                let count = if self.tail { raw.len() - 3 } else { 7 };
                fs::write(temporary, &raw[..count]).unwrap();
                return Err(runtime_scan_error(
                    "injected_staging_control_partial",
                    &temporary.to_string_lossy(),
                ));
            }
            LocalFiles.replace(temporary, target)
        }
        fn remove(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)
        }
        fn create_directories(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
    }

    #[test]
    fn staging_control_partial_recovers_with_missing_payload_and_rejects_premature_phase_before_writes()
     {
        for (tail, premature) in [(false, false), (true, false), (false, true)] {
            let (storage, context, plan) = runtime_plan_case();
            let failure = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_execution_staging_scoped(
                        &context,
                        &plan,
                        false,
                        &FailStagingControlWrite { tail },
                        |_| Ok(()),
                    )
                })
                .unwrap_err();
            assert_eq!(failure.reason_code, "spec_update_replace_failed");
            assert_eq!(failure.details["recovery_required"], true);
            let directory = crate::transaction_storage::runtime_manifest_directory(
                &context.writer().canonical_project_root,
                &plan.manifest,
            )
            .unwrap();
            let control = directory.join("transaction.json.tmp");
            let missing = directory.join("index.json.tmp");
            fs::remove_file(&missing).unwrap();
            if premature {
                let mut next = plan.manifest.clone();
                next.phase = work_model::runtime::RuntimePhase::PublishedVerified;
                next.published_count = next.targets.len();
                let raw = serde_json::to_vec(&next).unwrap();
                fs::write(&control, &raw).unwrap();
                assert_eq!(
                    storage
                        .recover_execution_staging_candidate(&context, &plan)
                        .unwrap_err()
                        .reason_code,
                    "runtime_control_temporary_invalid"
                );
                assert_eq!(fs::read(&control).unwrap(), raw);
                assert!(!missing.exists());
                assert_eq!(
                    fs::read(storage.project_root.join(&plan.steps[1].target)).unwrap(),
                    plan.steps[1].before.as_ref().unwrap().clone()
                );
            } else {
                let pending = storage.runtime_execution_inventory(&context).unwrap();
                let binding = work_operations::execution::recovery::execution_staging_binding(
                    &pending[0].manifest,
                    context.writer().canonical_project_root.to_str().unwrap(),
                    &context.writer().requirement_id,
                    context.target().execution_dir,
                    context.target().task_id,
                    &pending[0].files,
                )
                .unwrap();
                assert!(
                    binding
                        .partial_files
                        .contains(&"transaction.json.tmp".into())
                );
                assert!(binding.missing_files.contains(&"index.json.tmp".into()));
                storage
                    .recover_execution_staging_candidate(&context, &plan)
                    .unwrap();
                assert!(!directory.exists());
                for target in &plan.manifest.targets {
                    assert_eq!(
                        fs::read(storage.project_root.join(&target.path)).unwrap(),
                        target.after.as_ref().unwrap().bytes
                    );
                }
            }
        }
    }

    struct FailStagingPayloadWrite;
    impl ArtifactStore for FailStagingPayloadWrite {
        fn read_raw(&self, path: &std::path::Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &std::path::Path, raw: &[u8]) -> Result<(), WorkError> {
            if path
                .file_name()
                .is_some_and(|name| name == "attempt.json.tmp")
            {
                LocalFiles.create_new(path, &raw[..7])?;
                return Err(runtime_scan_error(
                    "injected_staging_prepare",
                    &path.to_string_lossy(),
                ));
            }
            LocalFiles.create_new(path, raw)
        }
        fn replace(
            &self,
            temporary: &std::path::Path,
            target: &std::path::Path,
        ) -> Result<(), WorkError> {
            LocalFiles.replace(temporary, target)
        }
        fn remove(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)
        }
        fn create_directories(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
    }

    #[test]
    fn staging_engine_partial_preparation_recovers_only_frozen_prefix_and_preserves_unknown_formal_state()
     {
        for conflict in [false, true] {
            let (storage, context, plan) = runtime_plan_case();
            let before = fs::read(storage.project_root.join(&plan.steps[1].target)).unwrap();
            assert_eq!(
                storage
                    .with_runtime_execution_writer(&context, |_| storage
                        .run_execution_staging_scoped(
                            &context,
                            &plan,
                            false,
                            &FailStagingPayloadWrite,
                            |_| Ok(())
                        ))
                    .unwrap_err()
                    .reason_code,
                "injected_staging_prepare"
            );
            let inventory = storage.runtime_execution_inventory(&context).unwrap();
            assert_eq!(inventory[0].missing_files, vec!["index.json.tmp"]);
            let binding = work_operations::execution::recovery::execution_staging_binding(
                &inventory[0].manifest,
                context.writer().canonical_project_root.to_str().unwrap(),
                &context.writer().requirement_id,
                context.target().execution_dir,
                "TASK-001",
                &inventory[0].files,
            )
            .unwrap();
            assert_eq!(binding.partial_files, vec!["attempt.json.tmp"]);
            let directory = crate::transaction_storage::runtime_manifest_directory(
                &context.writer().canonical_project_root,
                &plan.manifest,
            )
            .unwrap();
            if conflict {
                fs::write(
                    storage.project_root.join(&plan.steps[1].target),
                    b"unknown formal bytes",
                )
                .unwrap();
                assert_eq!(
                    storage
                        .recover_execution_staging_candidate(&context, &plan)
                        .unwrap_err()
                        .reason_code,
                    "execution_staging_formal_conflict"
                );
                assert_eq!(
                    fs::read(directory.join("attempt.json.tmp")).unwrap(),
                    plan.payloads["attempt.json.tmp"][..7]
                );
                assert!(!directory.join("index.json.tmp").exists());
                assert_eq!(
                    fs::read(storage.project_root.join(&plan.steps[1].target)).unwrap(),
                    b"unknown formal bytes"
                );
            } else {
                assert_eq!(
                    fs::read(storage.project_root.join(&plan.steps[1].target)).unwrap(),
                    before
                );
                storage
                    .recover_execution_staging_candidate(&context, &plan)
                    .unwrap();
                assert!(!directory.exists());
                for target in &plan.manifest.targets {
                    assert_eq!(
                        fs::read(storage.project_root.join(&target.path)).unwrap(),
                        target.after.as_ref().unwrap().bytes
                    );
                }
            }
        }
    }

    struct FailStagingCleanup {
        removed: Cell<usize>,
    }
    impl ArtifactStore for FailStagingCleanup {
        fn read_raw(&self, path: &std::path::Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &std::path::Path, raw: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, raw)
        }
        fn replace(
            &self,
            temporary: &std::path::Path,
            target: &std::path::Path,
        ) -> Result<(), WorkError> {
            LocalFiles.replace(temporary, target)
        }
        fn create_directories(&self, path: &std::path::Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
        fn remove(&self, path: &std::path::Path) -> Result<(), WorkError> {
            if self.removed.get() == 1 {
                return Err(runtime_scan_error(
                    "injected_staging_cleanup",
                    &path.to_string_lossy(),
                ));
            }
            LocalFiles.remove(path)?;
            self.removed.set(self.removed.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn staging_engine_partial_new_artifact_and_cleanup_failure_keep_complete_recovery_identity() {
        for partial in [false, true] {
            let (storage, context, plan) = runtime_plan_case();
            let store = FailStagingArtifactWrite {
                failed: Cell::new(false),
                partial,
            };
            assert_eq!(
                storage
                    .with_runtime_execution_writer(&context, |_| storage
                        .run_execution_staging_scoped(&context, &plan, false, &store, |_| Ok(())))
                    .unwrap_err()
                    .reason_code,
                "injected_staging_artifact_write"
            );
            storage
                .recover_execution_staging_candidate(&context, &plan)
                .unwrap();
            assert_eq!(
                fs::read(storage.project_root.join(&plan.steps[0].target)).unwrap(),
                plan.steps[0].after
            );
        }
        let (storage, context, plan) = runtime_plan_case();
        let store = FailStagingCleanup {
            removed: Cell::new(0),
        };
        assert_eq!(
            storage
                .with_runtime_execution_writer(&context, |_| storage.run_execution_staging_scoped(
                    &context,
                    &plan,
                    false,
                    &store,
                    |_| Ok(())
                ))
                .unwrap_err()
                .reason_code,
            "injected_staging_cleanup"
        );
        let directory = crate::transaction_storage::runtime_manifest_directory(
            &context.writer().canonical_project_root,
            &plan.manifest,
        )
        .unwrap();
        assert_eq!(
            crate::transaction_storage::read_runtime_manifest(
                &LocalFiles,
                &context.writer().canonical_project_root,
                &plan.manifest
            )
            .unwrap()
            .phase,
            work_model::runtime::RuntimePhase::Cleaning
        );
        assert!(directory.join("transaction.json").exists());
        let before = plan
            .manifest
            .targets
            .iter()
            .map(|target| {
                (
                    target.path.clone(),
                    fs::read(storage.project_root.join(&target.path)).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        storage
            .recover_execution_staging_candidate(&context, &plan)
            .unwrap();
        for (path, raw) in before {
            assert_eq!(fs::read(storage.project_root.join(path)).unwrap(), raw);
        }
        assert!(!directory.exists());
    }

    #[test]
    fn staging_engine_refuses_unknown_evidence_and_changed_source_before_writes() {
        let (storage, context, plan) = runtime_plan_case();
        let index = storage.project_root.join(&plan.steps[1].target);
        let original = fs::read(&index).unwrap();
        let directory = crate::transaction_storage::runtime_manifest_directory(
            &context.writer().canonical_project_root,
            &plan.manifest,
        )
        .unwrap();
        let mut wrong = plan.clone();
        wrong.steps[0].after.push(0);
        assert_eq!(
            storage
                .publish_execution_staging_candidate(&context, &wrong)
                .unwrap_err()
                .reason_code,
            "execution_staging_plan_invalid"
        );
        assert!(!directory.exists());
        assert_eq!(fs::read(&index).unwrap(), original);
        let task = context.target().task_path.to_owned();
        let task_path = storage.project_root.join(&task);
        let task_raw = fs::read(&task_path).unwrap();
        fs::write(&task_path, b"changed source").unwrap();
        assert_eq!(
            storage
                .publish_execution_staging_candidate(&context, &plan)
                .unwrap_err()
                .reason_code,
            "execution_staging_source_changed"
        );
        assert!(!directory.exists());
        fs::write(task_path, task_raw).unwrap();
        storage
            .with_runtime_execution_writer(&context, |_| {
                storage.run_execution_staging_scoped(&context, &plan, false, &LocalFiles, |stage| {
                    if stage == RuntimeExecutionStage::Prepared {
                        Err(runtime_scan_error("injected_staging_boundary", "prepared"))
                    } else {
                        Ok(())
                    }
                })
            })
            .unwrap_err();
        fs::write(directory.join("foreign.tmp"), b"foreign").unwrap();
        assert_eq!(
            storage
                .recover_execution_staging_candidate(&context, &plan)
                .unwrap_err()
                .reason_code,
            "runtime_inventory_foreign"
        );
        assert_eq!(fs::read(directory.join("foreign.tmp")).unwrap(), b"foreign");
        assert_eq!(fs::read(index).unwrap(), original);
    }

    #[test]
    fn retained_inventory_binds_all_six_journals_and_preserves_partial_evidence() {
        use work_operations::derivation::publication;
        for position in 0..6 {
            let (storage, context, _) = receipt_candidate_case();
            let writer = context.writer();
            let root = &writer.canonical_project_root;
            let execution = context.target().execution_dir;
            let (prepared, dir) = current_journal_inventory_case(&storage, &context, position);
            let journal_path = prepared.manifest.business_identity["journal_path"]
                .as_str()
                .unwrap();
            let manifest_raw = fs::read(dir.join("transaction.json")).unwrap();
            let first = prepared.payloads.keys().next().unwrap();
            let first_raw = &prepared.payloads[first];
            fs::write(dir.join(first), &first_raw[..first_raw.len() / 2]).unwrap();
            let missing = prepared
                .payloads
                .keys()
                .find(|path| *path != first)
                .unwrap();
            fs::remove_file(dir.join(missing)).unwrap();
            let mut next = prepared.manifest.clone();
            next.phase = work_model::runtime::RuntimePhase::Publishing;
            let next_raw = serde_json::to_vec(&next).unwrap();
            fs::write(dir.join("transaction.json.tmp"), &next_raw[..17]).unwrap();
            let inventory = storage
                .retained_requirement_inventory(writer, execution)
                .unwrap();
            assert_eq!(inventory.len(), 1);
            assert_eq!(inventory[0].missing_files, vec![missing.clone()]);
            assert_eq!(inventory[0].files[first], first_raw[..first_raw.len() / 2]);
            assert_eq!(
                fs::read(dir.join("transaction.json")).unwrap(),
                manifest_raw
            );
            assert!(!root.join(journal_path).exists());
            fs::write(dir.join(first), b"foreign bytes").unwrap();
            assert_eq!(
                storage
                    .retained_requirement_inventory(writer, execution)
                    .unwrap_err()
                    .reason_code,
                "runtime_inventory_hash_mismatch"
            );
            fs::write(dir.join(first), first_raw).unwrap();
            fs::write(dir.join("transaction.json.tmp"), b"foreign control").unwrap();
            assert_eq!(
                storage
                    .retained_requirement_inventory(writer, execution)
                    .unwrap_err()
                    .reason_code,
                "runtime_control_temporary_invalid"
            );
            assert_eq!(
                fs::read(dir.join("transaction.json")).unwrap(),
                manifest_raw
            );
        }
        let (storage, context, _) = receipt_candidate_case();
        staging_inventory_case(
            &storage,
            &context,
            publication::RuntimeOperation::SpecificationUpdate,
            'a',
        );
        assert_eq!(
            storage
                .retained_requirement_inventory(context.writer(), context.target().execution_dir)
                .unwrap_err()
                .reason_code,
            "runtime_journal_evidence_invalid"
        );
    }

    fn current_journal_inventory_case(
        storage: &LocalExecutionStorage,
        context: &work_feature::execution::ExecutionWriterContext,
        position: usize,
    ) -> (
        work_operations::derivation::transaction::PreparedJournalStaging,
        PathBuf,
    ) {
        use work_operations::derivation::{
            publication::{self, JournalKind},
            transaction::*,
        };
        let writer = context.writer();
        let root = &writer.canonical_project_root;
        let execution = context.target().execution_dir;
        let approved = "ab".repeat(32);
        let request = match position {
            0 => {
                json!({"schema":"work-spec-update-request","task_index":{"requirement_id":writer.requirement_id.as_str()}})
            }
            1 => json!({"migration":{},"preview_fingerprint":approved}),
            2 => {
                json!({"request_sha256":approved,"analysis_fingerprint":"c".repeat(64),"item_id":"ITEM-001"})
            }
            3 => json!({"request_sha256":approved,"phase":"reconciliation"}),
            4 => {
                json!({"kind":"instruction_migration","requirement_id":writer.requirement_id.as_str(),"preview_fingerprint":approved})
            }
            _ => {
                json!({"kind":"source_refresh","requirement_id":writer.requirement_id.as_str(),"preview_fingerprint":approved})
            }
        };
        let kind = match position {
            1 | 2 => TransactionKind::Migration,
            3 => TransactionKind::Reconciliation,
            4 => TransactionKind::InstructionMigration,
            5 => TransactionKind::SourceRefresh,
            _ => TransactionKind::Update,
        };
        let journal = TransactionDeriver::derive(TransactionInput {
            kind,
            order: PublicationOrder::Flat,
            request,
            artifacts: json!({"execution":execution}),
            affected_task_ids: vec![],
            history: std::collections::BTreeMap::new(),
            source: std::collections::BTreeMap::new(),
            candidate: std::collections::BTreeMap::from([(
                "new-artifact.json".into(),
                b"approved new bytes".to_vec(),
            )]),
        })
        .unwrap()
        .journal;
        let kind = match position {
            0 => JournalKind::SpecificationUpdate(journal["transaction_id"].as_str().unwrap()),
            1 => JournalKind::SpecificationMigration(&approved),
            2 => JournalKind::SpecificationMigrationItem {
                approved: &approved,
                position: 0,
            },
            3 => JournalKind::SpecificationMigrationReconcile(&approved),
            4 => JournalKind::InstructionMigration(&approved),
            _ => JournalKind::SourceRefresh(&approved),
        };
        let journal_path = publication::retained_journal_path(execution, kind).unwrap();
        let prepared = build_journal_staging(JournalStagingInput {
            canonical_root: root.to_str().unwrap(),
            requirement: &writer.requirement_id,
            execution_dir: execution,
            journal_path: &journal_path,
            kind,
            journal: &journal,
        })
        .unwrap();
        let dir = crate::transaction_storage::prepare_runtime_transaction(
            &LocalFiles,
            root,
            &prepared.manifest,
            &prepared.payloads,
        )
        .unwrap();
        assert_eq!(storage.project_root.canonicalize().unwrap(), *root);
        (prepared, dir)
    }

    fn staging_inventory_case(
        storage: &LocalExecutionStorage,
        context: &work_feature::execution::ExecutionWriterContext,
        operation: work_operations::derivation::publication::RuntimeOperation,
        approval_digit: char,
    ) -> (work_model::runtime::RuntimeManifest, PathBuf) {
        use work_model::runtime::*;
        use work_operations::derivation::{fingerprint, identity, publication};
        let root = &context.writer().canonical_project_root;
        let execution = context.target().execution_dir;
        let target = format!("{execution}/index.json");
        let raw = fs::read(root.join(&target)).unwrap();
        let approval = approval_digit.to_string().repeat(64);
        let business =
            json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"VAL-001"});
        let identity = identity::runtime_transaction_identity(
            root.to_str().unwrap(),
            &context.writer().requirement_id,
            operation.as_str(),
            &approval,
            &business,
            std::slice::from_ref(&target),
        )
        .unwrap();
        let inventory = publication::runtime_inventory(operation, 1)
            .into_iter()
            .filter(|file| file != "transaction.json")
            .map(|path| RuntimeFile {
                path,
                sha256: fingerprint::raw(&raw),
                size_bytes: raw.len() as u64,
            })
            .collect();
        let evidence = RuntimeBytes {
            bytes: raw.clone(),
            sha256: fingerprint::raw(&raw),
        };
        let manifest = RuntimeManifest {
            schema: "work-runtime-transaction".into(),
            canonical_root: root.to_str().unwrap().into(),
            requirement_id: context.writer().requirement_id.as_str().into(),
            execution_dir: execution.into(),
            operation: operation.as_str().into(),
            transaction_identity: identity,
            approval_sha256: approval,
            business_identity: business,
            targets: vec![RuntimeTarget {
                path: target,
                before: Some(evidence.clone()),
                after: Some(evidence),
            }],
            inventory,
            published_count: 0,
            phase: RuntimePhase::Prepared,
        };
        let payloads = manifest
            .inventory
            .iter()
            .map(|file| (file.path.clone(), raw.clone()))
            .collect();
        let manifest = if matches!(
            operation,
            publication::RuntimeOperation::AttemptStart
                | publication::RuntimeOperation::RecordBegin
                | publication::RuntimeOperation::CommandCorrection
                | publication::RuntimeOperation::RecordFinish
                | publication::RuntimeOperation::DeviationRecord
                | publication::RuntimeOperation::AttemptClose
                | publication::RuntimeOperation::Correction
        ) {
            work_operations::execution::recovery::build_execution_staging_manifest(
                work_operations::execution::recovery::ExecutionStagingInput {
                    canonical_root: root.to_str().unwrap(),
                    requirement: &context.writer().requirement_id,
                    execution_dir: execution,
                    operation,
                    approval_sha256: &manifest.approval_sha256,
                    business_identity: manifest.business_identity.clone(),
                    targets: manifest.targets.clone(),
                    payloads: &payloads,
                },
            )
            .unwrap()
        } else {
            manifest
        };
        let directory = crate::transaction_storage::prepare_runtime_transaction(
            &LocalFiles,
            root,
            &manifest,
            &payloads,
        )
        .unwrap();
        assert_eq!(storage.project_root.canonicalize().unwrap(), *root);
        assert!(directory.starts_with(root));
        (manifest, directory)
    }

    #[test]
    fn staging_reader_prepares_typed_recovery_from_real_verified_context_without_writes() {
        use work_model::runtime::*;
        use work_operations::derivation::fingerprint;
        use work_operations::execution::recovery::*;
        let (storage, writer, _) = receipt_candidate_case();
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(
                &repo.join("../skills/work"),
            )
            .unwrap(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: storage.project_root.clone(),
        };
        let tasks = crate::task::storage::LocalTaskStorage {
            project_root: storage.project_root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &instructions,
            skills: &skills,
            paths: &paths,
            task_repository: &tasks,
            skill_roots: &[],
        };
        let target = writer.target();
        let load_context =
            || work_feature::execution::load_execution_writer_context(&sources, target);
        let adapter = RuntimeExecutionSession {
            storage: &storage,
            load_context: &load_context,
        };
        let choice = json!({"command_positions":[1],"validation_positions":[1],"modifiable_files":[],
            "external_operation_positions":[],"allowed_deviations":[],"authorization_evidence":"Approved","carried_records":[]});
        let prepared = work_feature::execution::prepare_attempt_start_from_project(
            &sources,
            &adapter,
            target,
            &[],
            &choice,
        )
        .unwrap();
        work_feature::execution::start_attempt_from_project(
            &sources,
            &adapter,
            target,
            &[],
            &prepared["request"],
            "2026-10-06T10:00+08:00",
        )
        .unwrap();
        let index_path = format!("{}/index.json", target.execution_dir);
        let attempt_path = format!("{}/TASK-001/ATTEMPT-001/attempt.json", target.execution_dir);
        let index_raw = fs::read(storage.project_root.join(&index_path)).unwrap();
        let attempt_raw = fs::read(storage.project_root.join(&attempt_path)).unwrap();
        let index = parse_json_contract(&index_raw).unwrap();
        let attempt = parse_json_contract(&attempt_raw).unwrap();
        let task = &writer.task_context().contract["tasks"][0];
        let (next, record, _) = work_operations::execution::record_begin_candidate(
            task, &attempt, &index, "TASK-001", "CMD-001", None,
        )
        .unwrap();
        let after = render_execution_index(&next).unwrap();
        let bytes = |raw: &[u8]| RuntimeBytes {
            sha256: fingerprint::raw(raw),
            bytes: raw.to_vec(),
        };
        let payloads = std::collections::BTreeMap::from([("index.json.tmp".into(), after.clone())]);
        let manifest=build_execution_staging_manifest(ExecutionStagingInput {
            canonical_root:writer.writer().canonical_project_root.to_str().unwrap(),requirement:&writer.writer().requirement_id,
            execution_dir:target.execution_dir,operation:work_operations::derivation::publication::RuntimeOperation::RecordBegin,
            approval_sha256:&fingerprint::raw(&after),business_identity:json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":record}),
            targets:vec![RuntimeTarget {path:index_path.clone(),before:Some(bytes(&index_raw)),after:Some(bytes(&after))},
                RuntimeTarget {path:attempt_path.clone(),before:Some(bytes(&attempt_raw)),after:Some(bytes(&attempt_raw))}],payloads:&payloads,
        }).unwrap();
        let directory = crate::transaction_storage::prepare_runtime_transaction(
            &LocalFiles,
            &writer.writer().canonical_project_root,
            &manifest,
            &payloads,
        )
        .unwrap();
        let manifest_raw = fs::read(directory.join("transaction.json")).unwrap();
        let request = json!({"schema":"work-execution-recovery-prepare-request","transaction":"record_begin","attempt_id":"ATTEMPT-001"});
        let result = work_feature::execution::prepare_staging_recovery_from_project(
            &sources, &adapter, target, &request,
        )
        .unwrap();
        let _: work_model::execution::response::PreparedExecutionRecoveryPrepare =
            serde_json::from_value(result.clone()).unwrap();
        assert_eq!(
            result["request"]["transaction_files"],
            json!(["index.json.tmp", "transaction.json"])
        );
        let relative = directory
            .strip_prefix(&writer.writer().canonical_project_root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        assert_eq!(result["request"]["transaction_dir"], relative);
        assert_eq!(
            result["evidence"][format!("{relative}/transaction.json")]["raw_sha256"],
            fingerprint::raw(&manifest_raw)
        );
        assert_eq!(
            fs::read(directory.join("transaction.json")).unwrap(),
            manifest_raw
        );
        assert_eq!(fs::read(directory.join("index.json.tmp")).unwrap(), after);
        assert_eq!(
            fs::read(storage.project_root.join(&index_path)).unwrap(),
            index_raw
        );
        assert_eq!(
            fs::read(storage.project_root.join(&attempt_path)).unwrap(),
            attempt_raw
        );
        let mut wrong = request;
        wrong["attempt_id"] = json!("ATTEMPT-002");
        assert_eq!(
            work_feature::execution::prepare_staging_recovery_from_project(
                &sources, &adapter, target, &wrong
            )
            .unwrap_err()
            .reason_code,
            "execution_recovery_transaction_changed"
        );
        assert!(
            !storage
                .project_root
                .join(format!("{}/.work-state-writer.lock", target.execution_dir))
                .exists()
        );
        use work_feature::ports::RuntimeWriterLock;
        LocalWriterLock
            .require_runtime_idle(writer.writer(), LockClass::Execution)
            .unwrap();
    }

    #[test]
    fn staging_inventory_scans_all_execute_and_spec_operations_without_writing_or_other_requirement_interference()
     {
        use work_operations::derivation::publication::RuntimeOperation::*;
        let (storage, context, _) = receipt_candidate_case();
        let other = storage
            .project_root
            .join("outputs/work/runtime/staging/another/unknown/bad");
        fs::create_dir_all(&other).unwrap();
        fs::write(
            other.join("transaction.json"),
            b"partial foreign requirement",
        )
        .unwrap();
        let operations = [
            AttemptStart,
            RecordBegin,
            CommandCorrection,
            RecordFinish,
            DeviationRecord,
            AttemptClose,
            Correction,
            SpecificationUpdate,
            SpecificationMigration,
            SpecificationMigrationItem,
            SpecificationMigrationReconcile,
            InstructionMigration,
            SourceRefresh,
        ];
        let mut originals = std::collections::BTreeMap::new();
        for operation in operations {
            let dir = if is_retained_journal_operation(operation.as_str()) {
                let position = match operation {
                    SpecificationUpdate => 0,
                    SpecificationMigration => 1,
                    SpecificationMigrationItem => 2,
                    SpecificationMigrationReconcile => 3,
                    InstructionMigration => 4,
                    SourceRefresh => 5,
                    _ => unreachable!(),
                };
                current_journal_inventory_case(&storage, &context, position).1
            } else {
                staging_inventory_case(&storage, &context, operation, 'a').1
            };
            originals.insert(
                dir.join("transaction.json"),
                fs::read(dir.join("transaction.json")).unwrap(),
            );
        }
        let inventory = storage
            .runtime_execution_inventory_scope(
                &context.writer().canonical_project_root,
                &context.writer().requirement_id,
                context.target().execution_dir,
            )
            .unwrap();
        assert_eq!(inventory.len(), 13);
        for transaction in inventory {
            assert!(transaction.missing_files.is_empty());
            assert_eq!(
                transaction.files.len(),
                transaction.manifest.inventory.len() + 1
            );
        }
        for (path, raw) in originals {
            assert_eq!(fs::read(path).unwrap(), raw);
        }
        assert_eq!(
            fs::read(other.join("transaction.json")).unwrap(),
            b"partial foreign requirement"
        );
    }

    #[test]
    fn staging_inventory_keeps_missing_payload_explicit_and_rejects_foreign_drift_missing_manifest_and_legacy()
     {
        use work_operations::derivation::publication::RuntimeOperation::RecordBegin;
        let (storage, context, _) = receipt_candidate_case();
        let (_, dir) = staging_inventory_case(&storage, &context, RecordBegin, 'b');
        let raw = fs::read(dir.join("index.json.tmp")).unwrap();
        fs::remove_file(dir.join("index.json.tmp")).unwrap();
        assert_eq!(
            storage.runtime_execution_inventory(&context).unwrap()[0].missing_files,
            vec!["index.json.tmp"]
        );
        fs::write(dir.join("index.json.tmp"), b"changed").unwrap();
        assert_eq!(
            storage
                .runtime_execution_inventory(&context)
                .unwrap_err()
                .reason_code,
            "runtime_inventory_hash_mismatch"
        );
        fs::write(dir.join("index.json.tmp"), &raw).unwrap();
        fs::write(dir.join("foreign.tmp"), b"foreign").unwrap();
        assert_eq!(
            storage
                .runtime_execution_inventory(&context)
                .unwrap_err()
                .reason_code,
            "runtime_inventory_foreign"
        );
        fs::remove_file(dir.join("foreign.tmp")).unwrap();
        let manifest = fs::read(dir.join("transaction.json")).unwrap();
        fs::remove_file(dir.join("transaction.json")).unwrap();
        assert_eq!(
            storage
                .runtime_execution_inventory(&context)
                .unwrap_err()
                .reason_code,
            "runtime_manifest_missing"
        );
        fs::write(dir.join("transaction.json"), &manifest).unwrap();
        let legacy = storage.project_root.join(format!(
            "{}/.work-spec-update-unknown.tmp",
            context.target().execution_dir
        ));
        fs::write(&legacy, b"partial legacy").unwrap();
        assert_eq!(
            storage
                .runtime_execution_inventory(&context)
                .unwrap_err()
                .reason_code,
            "legacy_execution_transaction_present"
        );
        assert_eq!(fs::read(legacy).unwrap(), b"partial legacy");
        assert_eq!(fs::read(dir.join("transaction.json")).unwrap(), manifest);
    }

    #[test]
    fn staging_readiness_refuses_mixed_transactions_and_control_temporary_drift() {
        use work_operations::derivation::publication::RuntimeOperation::RecordBegin;
        let (storage, context, _) = receipt_candidate_case();
        let (manifest, dir) = staging_inventory_case(&storage, &context, RecordBegin, 'e');
        let relative = dir
            .strip_prefix(&context.writer().canonical_project_root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        assert_eq!(
            storage
                .require_runtime_execution_ready(&context, None)
                .unwrap_err()
                .reason_code,
            "runtime_execution_transaction_present"
        );
        storage
            .require_runtime_execution_ready(&context, Some(&relative))
            .unwrap();
        let mut next = manifest.clone();
        next.phase = work_model::runtime::RuntimePhase::Publishing;
        fs::write(
            dir.join("transaction.json.tmp"),
            serde_json::to_vec(&next).unwrap(),
        )
        .unwrap();
        assert!(
            storage.runtime_execution_inventory(&context).unwrap()[0]
                .files
                .contains_key("transaction.json.tmp")
        );
        next.approval_sha256 = "f".repeat(64);
        fs::write(
            dir.join("transaction.json.tmp"),
            serde_json::to_vec(&next).unwrap(),
        )
        .unwrap();
        assert!(storage.runtime_execution_inventory(&context).is_err());
        fs::remove_file(dir.join("transaction.json.tmp")).unwrap();
        staging_inventory_case(&storage, &context, RecordBegin, 'f');
        assert_eq!(
            storage.runtime_execution_inventory(&context).unwrap().len(),
            2
        );
        assert_eq!(
            storage
                .require_runtime_execution_ready(&context, Some(&relative))
                .unwrap_err()
                .reason_code,
            "runtime_execution_transaction_present"
        );
        assert_eq!(
            storage
                .require_runtime_execution_ready(
                    &context,
                    Some("outputs/work/runtime/staging/other/record-begin/unknown")
                )
                .unwrap_err()
                .reason_code,
            "runtime_execution_transaction_present"
        );
    }

    #[test]
    fn staging_inventory_refuses_unknown_operation_and_foreign_manifest_identity() {
        use work_operations::derivation::publication::RuntimeOperation::RecordBegin;
        for foreign in [false, true] {
            let (storage, context, _) = receipt_candidate_case();
            if foreign {
                let (mut manifest, dir) =
                    staging_inventory_case(&storage, &context, RecordBegin, 'c');
                manifest.requirement_id = "another".into();
                fs::write(
                    dir.join("transaction.json"),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
                assert!(storage.runtime_execution_inventory(&context).is_err());
            } else {
                let dir = storage.project_root.join(format!(
                    "outputs/work/runtime/staging/{}/unknown",
                    context.writer().requirement_id.as_str()
                ));
                fs::create_dir_all(&dir).unwrap();
                assert_eq!(
                    storage
                        .runtime_execution_inventory(&context)
                        .unwrap_err()
                        .reason_code,
                    "runtime_operation_invalid"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn staging_inventory_refuses_symlink_and_hardlink_aliases_without_mutating_payload() {
        use work_operations::derivation::publication::RuntimeOperation::RecordBegin;
        for hard in [false, true] {
            let (storage, context, _) = receipt_candidate_case();
            let (_, dir) = staging_inventory_case(&storage, &context, RecordBegin, 'd');
            let original = dir.join("index.json.tmp");
            let raw = fs::read(&original).unwrap();
            let retained = storage.project_root.join("retained-evidence");
            fs::rename(&original, &retained).unwrap();
            if hard {
                fs::hard_link(&retained, &original).unwrap();
            } else {
                std::os::unix::fs::symlink(&retained, &original).unwrap();
            }
            assert!(storage.runtime_execution_inventory(&context).is_err());
            assert_eq!(fs::read(retained).unwrap(), raw);
        }
    }

    fn receipt_candidate_case() -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
        Value,
    ) {
        receipt_candidate_case_with_downstream(false)
    }

    fn receipt_candidate_case_with_downstream(
        downstream: bool,
    ) -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
        Value,
    ) {
        receipt_case(downstream, false)
    }

    fn receipt_case(
        downstream: bool,
        file_changing: bool,
    ) -> (
        LocalExecutionStorage,
        work_feature::execution::ExecutionWriterContext,
        Value,
    ) {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/shared/task-diagnostics-project");
        let root = test_root();
        let task_path = "outputs/work/tasks/example/index.json";
        let execution = "custom executions/example";
        for relative in [task_path, "outputs/work/tasks/example/tasks/TASK-001.json"] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let mut index: Value =
            serde_json::from_slice(&fs::read(root.join(task_path)).unwrap()).unwrap();
        index["artifacts"]["execution"] = json!(execution);
        if !file_changing {
            // Legacy execution-record tests use fileless commands. Managed file publication
            // and the rejection of unisolated file-changing commands have separate tests.
            let item_path = root.join("outputs/work/tasks/example/tasks/TASK-001.json");
            let mut item: Value = serde_json::from_slice(&fs::read(&item_path).unwrap()).unwrap();
            item.as_object_mut().unwrap().remove("files");
            item["goal"] = json!("Record and validate an authorized fileless command.");
            item["steps"].as_array_mut().unwrap().remove(0);
            item["steps"][0]["id"] = json!("STEP-001");
            let raw = crate::fixture_support::render_task_item(&item).unwrap();
            index["tasks"][0]["canonical_sha256"] = json!(crate::fixture_support::raw_sha256(&raw));
            fs::write(item_path, raw).unwrap();
        }
        if downstream {
            let item_path = "outputs/work/tasks/example/tasks/TASK-001.json";
            let original = fs::read(root.join(item_path)).unwrap();
            let text = std::str::from_utf8(&original)
                .unwrap()
                .replace("TASK-001", "TASK-002");
            let mut item: Value = serde_json::from_str(&text).unwrap();
            item["dependencies"] = json!(["TASK-001"]);
            let raw = work_operations::task::ordering::render_task(
                &item,
                work_operations::task::ordering::TaskDocumentKind::Item,
            )
            .unwrap();
            fs::write(
                root.join("outputs/work/tasks/example/tasks/TASK-002.json"),
                &raw,
            )
            .unwrap();
            index["tasks"].as_array_mut().unwrap().push(json!({
                "id":"TASK-002", "path":"tasks/TASK-002.json", "canonical_sha256":"0".repeat(64)
            }));
            let items = std::collections::BTreeMap::from([
                ("TASK-001".to_owned(), original),
                ("TASK-002".to_owned(), raw),
            ]);
            let (raw, _) = crate::fixture_support::rebuild_fixture_bindings(
                &index,
                &items,
                None,
                &std::collections::BTreeSet::from([
                    crate::fixture_support::ArtifactNode::TaskItemBytes("TASK-002".into()),
                ]),
            )
            .unwrap();
            index = parse_json_contract(&raw).unwrap();
        }
        fs::write(
            root.join(task_path),
            work_operations::task::ordering::render_task(
                &index,
                work_operations::task::ordering::TaskDocumentKind::Index,
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(
                &repo.join("../skills/work"),
            )
            .unwrap(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: root.clone(),
        };
        let tasks = crate::task::storage::LocalTaskStorage {
            project_root: root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &instructions,
            skills: &skills,
            paths: &paths,
            task_repository: &tasks,
            skill_roots: &[],
        };
        let full = load_collection(&instructions, &skills, &paths, &tasks, &[], task_path).unwrap();
        let index = build_initial_execution_index(&full["collection_contract"], &full).unwrap();
        let raw = render_execution_index(&index).unwrap();
        let relative = format!("{execution}/index.json");
        fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
        fs::write(root.join(&relative), &raw).unwrap();
        let context = work_feature::execution::load_execution_writer_context(
            &sources,
            ExecutionProjectTarget {
                task_path,
                execution_dir: execution,
                task_id: "TASK-001",
            },
        )
        .unwrap();
        let preview = work_operations::execution::command_run::build_command_preview_with_receipts(work_operations::execution::command_run::CommandReceiptPreviewInput {
            request: &json!({"schema":"work-command-run-request","timeout_seconds":60}), execution_dir: execution,
            task_id: "TASK-001", attempt_id: "ATTEMPT-001", record_id: "CMD-001",
            working_directory: root.to_str().unwrap(), execution: &json!({"os":"macos","working_directory":"."}),
            invocation: &json!({"kind":"direct","executable":"/usr/bin/printf","executable_sha256":"a".repeat(64),"argv":["printf","ok"]}),
            sources: &json!({relative:work_operations::derivation::fingerprint::raw(&raw)}),
        }).unwrap();
        (
            LocalExecutionStorage { project_root: root },
            context,
            preview,
        )
    }

    #[test]
    fn receipt_directory_partial_or_finished_only_evidence_never_starts_runner() {
        for filename in [
            None,
            Some("started.json"),
            Some("finished.json"),
            Some("foreign.partial"),
        ] {
            let (storage, context, preview) = receipt_candidate_case();
            let paths = work_operations::execution::command_run::validate_command_receipt_preview(
                &preview,
                context.target().execution_dir,
                preview["approved_sha256"].as_str().unwrap(),
            )
            .unwrap();
            let directory = storage.project_root.join(&paths.directory);
            fs::create_dir_all(&directory).unwrap();
            if let Some(filename) = filename {
                fs::write(directory.join(filename), b"{partial unknown bytes").unwrap();
            }
            let runner = FakeCommandRunner {
                calls: Cell::new(0),
                exit_code: 0,
                timed_out: false,
            };
            let error = storage
                .run_prepared_command_with_receipts(
                    &context,
                    preview["approved_sha256"].as_str().unwrap(),
                    || Ok((preview.clone(), "Approved".into())),
                    &runner,
                )
                .unwrap_err();
            assert_eq!(error.reason_code, "command_run_already_started");
            assert_eq!(runner.calls.get(), 0);
            if let Some(filename) = filename {
                assert_eq!(
                    fs::read(directory.join(filename)).unwrap(),
                    b"{partial unknown bytes"
                );
            }
            assert!(
                !storage
                    .project_root
                    .join("outputs/work/runtime/locks/example/execution.lock")
                    .exists()
            );
        }
    }

    #[test]
    fn receipt_directory_every_interruption_retains_evidence_and_never_reexecutes() {
        for stage in [
            CommandReceiptStage::StartedWritten,
            CommandReceiptStage::StartedVerified,
            CommandReceiptStage::CommandReturned,
            CommandReceiptStage::FinishedWritten,
            CommandReceiptStage::FinishedVerified,
        ] {
            let (storage, context, preview) = receipt_candidate_case();
            let approval = preview["approved_sha256"].as_str().unwrap();
            let runner = FakeCommandRunner {
                calls: Cell::new(0),
                exit_code: 0,
                timed_out: false,
            };
            let error = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_receipts_scoped(
                        &context,
                        approval,
                        || Ok((preview.clone(), "Approved".into())),
                        &runner,
                        |current| {
                            if current == stage {
                                Err(WorkError::new(
                                    ExitCode::IoFailure,
                                    "injected_command_boundary",
                                    "Injected command boundary.",
                                    json!({}),
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(error.reason_code, "injected_command_boundary");
            let calls = runner.calls.get();
            assert_eq!(
                calls,
                usize::from(matches!(
                    stage,
                    CommandReceiptStage::CommandReturned
                        | CommandReceiptStage::FinishedWritten
                        | CommandReceiptStage::FinishedVerified
                ))
            );
            let error = storage
                .run_prepared_command_with_receipts(
                    &context,
                    approval,
                    || Ok((preview.clone(), "Approved".into())),
                    &runner,
                )
                .unwrap_err();
            assert_eq!(error.reason_code, "command_run_already_started");
            assert_eq!(runner.calls.get(), calls);
            assert!(
                !storage
                    .project_root
                    .join("outputs/work/runtime/locks/example/execution.lock")
                    .exists()
            );
        }
    }

    #[test]
    fn receipt_directory_readback_and_finished_write_failures_never_reexecute() {
        for corrupt_started in [true, false] {
            let (storage, context, preview) = receipt_candidate_case();
            let approval = preview["approved_sha256"].as_str().unwrap();
            let paths = work_operations::execution::command_run::validate_command_receipt_preview(
                &preview,
                context.target().execution_dir,
                approval,
            )
            .unwrap();
            let runner = FakeCommandRunner {
                calls: Cell::new(0),
                exit_code: 0,
                timed_out: false,
            };
            let error = storage
                .with_runtime_execution_writer(&context, |_| {
                    storage.run_receipts_scoped(
                        &context,
                        approval,
                        || Ok((preview.clone(), "Approved".into())),
                        &runner,
                        |stage| {
                            if corrupt_started && stage == CommandReceiptStage::StartedWritten {
                                fs::write(storage.project_root.join(&paths.started), b"{partial")
                                    .unwrap();
                            }
                            if !corrupt_started && stage == CommandReceiptStage::CommandReturned {
                                fs::create_dir(storage.project_root.join(&paths.finished)).unwrap();
                            }
                            Ok(())
                        },
                    )
                })
                .unwrap_err();
            assert_eq!(error.reason_code, "command_run_interrupted");
            let calls = usize::from(!corrupt_started);
            assert_eq!(runner.calls.get(), calls);
            assert!(
                storage
                    .run_prepared_command_with_receipts(
                        &context,
                        approval,
                        || Ok((preview.clone(), "Approved".into())),
                        &runner
                    )
                    .is_err()
            );
            assert_eq!(runner.calls.get(), calls);
            if corrupt_started {
                assert_eq!(
                    fs::read(storage.project_root.join(paths.started)).unwrap(),
                    b"{partial"
                );
            }
        }
    }

    #[test]
    fn receipt_directory_rejects_file_alias_and_legacy_finished_evidence_before_runner() {
        for legacy in [false, true] {
            let (storage, context, preview) = receipt_candidate_case();
            let approval = preview["approved_sha256"].as_str().unwrap();
            let paths = work_operations::execution::command_run::validate_command_receipt_preview(
                &preview,
                context.target().execution_dir,
                approval,
            )
            .unwrap();
            let original = storage.project_root.join("foreign-evidence");
            fs::write(&original, b"retained evidence").unwrap();
            if legacy {
                let target = storage.project_root.join(&paths.legacy_finished);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(target, b"{partial legacy finished").unwrap();
            } else {
                fs::create_dir_all(storage.project_root.join(&paths.directory)).unwrap();
                fs::hard_link(&original, storage.project_root.join(&paths.started)).unwrap();
            }
            let runner = FakeCommandRunner {
                calls: Cell::new(0),
                exit_code: 0,
                timed_out: false,
            };
            let error = storage
                .run_prepared_command_with_receipts(
                    &context,
                    approval,
                    || Ok((preview.clone(), "Approved".into())),
                    &runner,
                )
                .unwrap_err();
            assert_eq!(
                error.reason_code,
                if legacy {
                    "legacy_command_receipt_present"
                } else {
                    "runtime_path_alias"
                }
            );
            assert_eq!(runner.calls.get(), 0);
            assert_eq!(fs::read(original).unwrap(), b"retained evidence");
        }
    }

    #[cfg(unix)]
    #[test]
    fn receipt_directory_rejects_cross_attempt_directory_link_without_reading_receipts() {
        let (storage, context, preview) = receipt_candidate_case();
        let approval = preview["approved_sha256"].as_str().unwrap();
        let paths = work_operations::execution::command_run::validate_command_receipt_preview(
            &preview,
            context.target().execution_dir,
            approval,
        )
        .unwrap();
        let other = storage.project_root.join(format!(
            "{}/TASK-001/ATTEMPT-002/receipts/CMD-001",
            context.target().execution_dir
        ));
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("finished.json"), b"other Attempt evidence").unwrap();
        let directory = storage.project_root.join(&paths.directory);
        fs::create_dir_all(directory.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&other, &directory).unwrap();
        let runner = FakeCommandRunner {
            calls: Cell::new(0),
            exit_code: 0,
            timed_out: false,
        };
        let error = storage
            .run_prepared_command_with_receipts(
                &context,
                approval,
                || Ok((preview.clone(), "Approved".into())),
                &runner,
            )
            .unwrap_err();
        assert_eq!(error.reason_code, "runtime_path_alias");
        assert_eq!(runner.calls.get(), 0);
        assert_eq!(
            fs::read(other.join("finished.json")).unwrap(),
            b"other Attempt evidence"
        );
    }

    #[test]
    fn receipt_directory_success_and_nonzero_exit_keep_immutable_receipts() {
        for code in [0, 7] {
            let (storage, context, preview) = receipt_candidate_case();
            let approval = preview["approved_sha256"].as_str().unwrap();
            let runner = FakeCommandRunner {
                calls: Cell::new(0),
                exit_code: code,
                timed_out: false,
            };
            let result = storage.run_prepared_command_with_receipts(
                &context,
                approval,
                || Ok((preview.clone(), "Approved".into())),
                &runner,
            );
            if code == 0 {
                assert_eq!(result.unwrap()["receipt_dir"], preview["receipt_dir"]);
            } else {
                assert_eq!(result.unwrap_err().reason_code, "command_run_failed");
            }
            let paths = work_operations::execution::command_run::validate_command_receipt_preview(
                &preview,
                context.target().execution_dir,
                approval,
            )
            .unwrap();
            let started = fs::read(storage.project_root.join(&paths.started)).unwrap();
            let finished = fs::read(storage.project_root.join(&paths.finished)).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&finished).unwrap()["exit_code"],
                code
            );
            assert!(
                storage
                    .run_prepared_command_with_receipts(
                        &context,
                        approval,
                        || Ok((preview.clone(), "Approved".into())),
                        &runner
                    )
                    .is_err()
            );
            assert_eq!(runner.calls.get(), 1);
            assert_eq!(
                fs::read(storage.project_root.join(paths.started)).unwrap(),
                started
            );
            assert_eq!(
                fs::read(storage.project_root.join(paths.finished)).unwrap(),
                finished
            );
        }
    }
    use std::cell::Cell;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(target_os = "macos")]
    use crate::process::LocalCommandRunner;
    use serde_json::json;
    use work_feature::execution::{preflight_index, prepare_initial_index};
    use work_operations::derivation::fingerprint::structured as canonical_json_sha256;
    use work_operations::execution::attempt::render_attempt;
    use work_operations::execution::attempt_close::build_close_candidates;
    use work_operations::execution::build_execution_lock;
    use work_operations::execution::command_run::{CommandPreviewInput, build_command_preview};
    use work_operations::execution::index::{
        build_initial_execution_index, render_execution_index,
    };

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn test_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-execution-transaction-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("execution")).unwrap();
        LocalGit
            .mutate(&root, &["init".into(), "-q".into()])
            .unwrap();
        root
    }

    struct FakeCommandRunner {
        calls: Cell<usize>,
        exit_code: i32,
        timed_out: bool,
    }

    struct SourceChangingWorktree<'a> {
        storage: &'a LocalExecutionStorage,
        source: Option<PathBuf>,
    }
    impl ExecutionIndexRepository for SourceChangingWorktree<'_> {
        fn read_index(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            self.storage.read_index(path)
        }
        fn inspect_path(&self, path: &str) -> Result<FileState, WorkError> {
            self.storage.inspect_path(path)
        }
        fn check_execution_task_layout(
            &self,
            execution: &str,
            task: &str,
        ) -> Result<(), WorkError> {
            self.storage.check_execution_task_layout(execution, task)
        }
        fn check_execution_ready(&self, task: &str, execution: &str) -> Result<(), WorkError> {
            self.storage.check_execution_ready(task, execution)
        }
    }
    impl ExecutionWorktreeRepository for SourceChangingWorktree<'_> {
        fn read_file(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            self.storage.read_file(path)
        }
        fn git_status(&self) -> Result<Vec<Value>, WorkError> {
            if let Some(path) = &self.source {
                fs::write(path, b"changed during Git inspection").unwrap();
            }
            Ok(vec![])
        }
    }

    #[test]
    fn task_only_project_sources_accept_both_provenances_and_recheck_after_worktree_reads() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        for fixture_name in [
            "shared/task-diagnostics-project",
            "cases/specification/update/revision-migration/project",
        ] {
            let fixture = repo
                .join("crates/work-infrastructure/fixtures")
                .join(fixture_name);
            let root = test_root();
            let task_path = "outputs/work/tasks/example/index.json";
            let index: Value =
                serde_json::from_slice(&fs::read(fixture.join(task_path)).unwrap()).unwrap();
            let mut paths = vec![task_path.to_owned()];
            for row in index["tasks"].as_array().unwrap() {
                paths.push(format!(
                    "outputs/work/tasks/example/{}",
                    row["path"].as_str().unwrap()
                ));
            }
            for path in paths {
                let target = root.join(&path);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::copy(fixture.join(&path), target).unwrap();
            }
            if fixture_name == "shared/task-diagnostics-project" {
                crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
            }
            fs::write(root.join("src.txt"), b"source\n").unwrap();
            let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
                skill_root: crate::fixture_support::historical_task_skill_root(
                    &repo.join("../skills/work"),
                )
                .unwrap(),
            };
            let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
            let paths = crate::artifact_paths::LocalArtifactPaths {
                project_root: root.clone(),
            };
            let tasks = crate::task::storage::LocalTaskStorage {
                project_root: root.clone(),
            };
            let sources = CommandProjectSources {
                instructions: &instructions,
                skills: &skills,
                paths: &paths,
                task_repository: &tasks,
                skill_roots: &[],
            };
            let context = load_task_execution_context(
                &instructions,
                &skills,
                &paths,
                &tasks,
                &[],
                task_path,
                "TASK-001",
            )
            .unwrap();
            assert_eq!(
                context.contract["source"]["kind"],
                if fixture_name == "shared/task-diagnostics-project" {
                    "snapshot"
                } else {
                    "migration"
                }
            );
            let execution_dir = context.contract["artifacts"]["execution"].as_str().unwrap();
            let full =
                load_collection(&instructions, &skills, &paths, &tasks, &[], task_path).unwrap();
            let execution =
                build_initial_execution_index(&full["collection_contract"], &full).unwrap();
            let execution_path = root.join(format!("{execution_dir}/index.json"));
            fs::create_dir_all(execution_path.parent().unwrap()).unwrap();
            fs::write(&execution_path, render_execution_index(&execution).unwrap()).unwrap();
            let before = fs::read(&execution_path).unwrap();
            let storage = LocalExecutionStorage {
                project_root: root.clone(),
            };
            let target = ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            };
            {
                use work_feature::execution::RuntimeExecutionReadiness;
                let writer =
                    work_feature::execution::load_execution_writer_context(&sources, target)
                        .unwrap();
                storage.check_command_context(&writer, true).unwrap();
                storage.check_recovery_context(&writer).unwrap();
                let failure = storage
                    .with_runtime_execution_writer::<()>(&writer, |_| {
                        assert!(storage.check_command_context(&writer, true).is_err());
                        assert!(storage.check_recovery_context(&writer).is_err());
                        storage.check_command_context(&writer, false).unwrap();
                        Err(WorkError::new(
                            ExitCode::IoFailure,
                            "injected_execution_failure",
                            "Injected failure.",
                            json!({}),
                        ))
                    })
                    .unwrap_err();
                assert_eq!(failure.reason_code, "injected_execution_failure");
                storage.check_recovery_context(&writer).unwrap();
                let other = LocalExecutionStorage {
                    project_root: test_root(),
                };
                assert!(
                    other
                        .with_runtime_execution_writer(&writer, |_| Ok(()))
                        .is_err()
                );
                assert!(!other.project_root.join("outputs/work/runtime").exists());
            }
            let preview =
                prepare_execute_preflight_from_project(&sources, &storage, target, &[]).unwrap();
            assert_eq!(preview["task_status"], "pending");
            let clean = SourceChangingWorktree {
                storage: &storage,
                source: None,
            };
            work_feature::execution::inspect_worktree_from_project(&sources, &clean, target, &[])
                .unwrap();
            if fixture_name == "shared/task-diagnostics-project" {
                let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
                let changed = SourceChangingWorktree {
                    storage: &storage,
                    source: Some(source),
                };
                let error = work_feature::execution::inspect_worktree_from_project(
                    &sources,
                    &changed,
                    target,
                    &[],
                )
                .unwrap_err();
                assert_eq!(error.exit_code, ExitCode::ArtifactIntegrity);
            }
            assert_eq!(fs::read(execution_path).unwrap(), before);
            assert!(!root.join("outputs/work/plans").exists());
            assert!(
                !root
                    .join(format!("{execution_dir}/TASK-001/ATTEMPT-001"))
                    .exists()
            );
        }
    }

    #[test]
    fn runtime_adapter_starts_verified_attempt_without_legacy_lock_and_releases_errors() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/shared/task-diagnostics-project");
        let root = test_root();
        let task_path = "outputs/work/tasks/example/index.json";
        let execution_dir = "custom execution/example";
        for relative in [task_path, "outputs/work/tasks/example/tasks/TASK-001.json"] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let mut task_index: Value =
            serde_json::from_slice(&fs::read(root.join(task_path)).unwrap()).unwrap();
        task_index["artifacts"]["execution"] = json!(execution_dir);
        fs::write(
            root.join(task_path),
            work_operations::task::ordering::render_task(
                &task_index,
                work_operations::task::ordering::TaskDocumentKind::Index,
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(
                &repo.join("../skills/work"),
            )
            .unwrap(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: root.clone(),
        };
        let tasks = crate::task::storage::LocalTaskStorage {
            project_root: root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &instructions,
            skills: &skills,
            paths: &paths,
            task_repository: &tasks,
            skill_roots: &[],
        };
        let full = load_collection(&instructions, &skills, &paths, &tasks, &[], task_path).unwrap();
        let index = build_initial_execution_index(&full["collection_contract"], &full).unwrap();

        let index_path = root.join(format!("{execution_dir}/index.json"));
        fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        fs::write(&index_path, render_execution_index(&index).unwrap()).unwrap();
        let target = ExecutionProjectTarget {
            task_path,
            execution_dir,
            task_id: "TASK-001",
        };
        let writer =
            work_feature::execution::load_execution_writer_context(&sources, target).unwrap();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let loads = Cell::new(0);
        let load_context = || {
            loads.set(loads.get() + 1);
            work_feature::execution::load_execution_writer_context(&sources, target)
        };
        let adapter = RuntimeExecutionSession {
            storage: &storage,
            load_context: &load_context,
        };
        assert!(
            adapter
                .check_execution_ready("missing.json", execution_dir)
                .is_err()
        );
        assert_eq!(loads.get(), 0);
        assert!(
            adapter
                .recover_execution_from_project(&sources, target, &json!({"schema":"wrong"}))
                .is_err()
        );
        assert_eq!(loads.get(), 0);
        let before = fs::read(&index_path).unwrap();
        let invalid = RecordBeginPublication {
            execution_dir,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "VAL-001",
            index_before: b"invalid",
            attempt_before: b"invalid",
            index_after: b"invalid",
        };
        assert!(adapter.publish_record_begin(&invalid).is_err());
        LocalWriterLock
            .require_runtime_idle(writer.writer(), work_model::runtime::LockClass::Execution)
            .unwrap();
        assert_eq!(fs::read(&index_path).unwrap(), before);
        let choice = json!({"command_positions":[],"validation_positions":[],"modifiable_files":[],
            "external_operation_positions":[],"allowed_deviations":[],"authorization_evidence":"Approved","carried_records":[]});
        let confirmed = vec![];
        let prepared = work_feature::execution::prepare_attempt_start_from_project(
            &sources, &adapter, target, &confirmed, &choice,
        )
        .unwrap();
        let held = LocalWriterLock
            .acquire_runtime(writer.writer(), work_model::runtime::LockClass::Execution)
            .unwrap();
        assert!(
            work_feature::execution::start_attempt_from_project(
                &sources,
                &adapter,
                target,
                &confirmed,
                &prepared["request"],
                "2026-10-06T10:00+08:00"
            )
            .is_err()
        );
        held.release().unwrap();
        let started = work_feature::execution::start_attempt_from_project(
            &sources,
            &adapter,
            target,
            &confirmed,
            &prepared["request"],
            "2026-10-06T10:00+08:00",
        )
        .unwrap();
        assert_eq!(started["attempt_id"], "ATTEMPT-001");
        assert!(
            !root
                .join(format!("{execution_dir}/.work-state-writer.lock"))
                .exists()
        );
        LocalWriterLock
            .require_runtime_idle(writer.writer(), work_model::runtime::LockClass::Execution)
            .unwrap();
        assert!(
            root.join(format!("{execution_dir}/TASK-001/ATTEMPT-001/attempt.json"))
                .is_file()
        );
        let legacy = root.join(format!("{execution_dir}/.work-state-writer.lock"));
        fs::write(&legacy, b"legacy owner").unwrap();
        assert_eq!(
            storage
                .with_runtime_execution_writer(&writer, |_| Ok(()))
                .unwrap_err()
                .reason_code,
            "legacy_writer_lock_present"
        );
        assert_eq!(
            adapter
                .check_ready(execution_dir, true)
                .unwrap_err()
                .reason_code,
            "legacy_writer_lock_present"
        );
        assert_eq!(
            adapter
                .check_recovery_idle(execution_dir)
                .unwrap_err()
                .reason_code,
            "legacy_writer_lock_present"
        );
        assert_eq!(fs::read(legacy).unwrap(), b"legacy owner");
    }

    #[test]
    fn deviation_record_preserves_source_and_prepared_attempt() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let attempt_path = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(attempt_path.parent().unwrap()).unwrap();
        fs::write(&attempt_path, b"before").unwrap();
        fs::write(root.join("task.json"), b"task").unwrap();
        let sources = std::collections::HashMap::from([("task.json".to_owned(), b"task".to_vec())]);
        let publication = DeviationRecordPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            attempt_before: b"before",
            attempt_after: b"after",
            sources: &sources,
        };
        storage.publish_deviation_record(&publication).unwrap();
        assert_eq!(fs::read(&attempt_path).unwrap(), b"after");
        assert_eq!(
            storage
                .publish_deviation_record(&publication)
                .unwrap_err()
                .reason_code,
            "deviation_record_source_changed"
        );
        fs::write(
            root.join("execution/.work-deviation-record-other.tmp"),
            b"pending",
        )
        .unwrap();
        assert_eq!(
            storage
                .publish_deviation_record(&publication)
                .unwrap_err()
                .reason_code,
            "deviation_record_transaction_present"
        );
    }

    #[test]
    fn correction_publication_preserves_sources_and_installs_exclusively() {
        let setup = || {
            let root = test_root();
            let attempt = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
            fs::create_dir_all(attempt.parent().unwrap()).unwrap();
            fs::write(root.join("execution/index.json"), b"before").unwrap();
            fs::write(&attempt, b"attempt").unwrap();
            (root, attempt)
        };
        let publication = CorrectionPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            correction_id: "ATTEMPT-001-CORRECTION-001",
            index_before: b"before",
            attempt_before: b"attempt",
            artifact: b"correction",
            locked_index: b"locked",
            final_index: b"final",
        };
        let (root, attempt) = setup();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        storage.publish_correction(&publication).unwrap();
        let correction =
            root.join("execution/TASK-001/ATTEMPT-001/corrections/ATTEMPT-001-CORRECTION-001.json");
        assert_eq!(fs::read(&correction).unwrap(), b"correction");
        assert_eq!(
            fs::read(root.join("execution/index.json")).unwrap(),
            b"final"
        );
        assert_eq!(fs::read(&attempt).unwrap(), b"attempt");
        for suffix in ["artifact", "lock", "index"] {
            assert!(
                !root
                    .join(format!(
                        "execution/.work-correction-TASK-001-ATTEMPT-001-CORRECTION-001-{suffix}.tmp"
                    ))
                    .exists()
            );
        }

        let (root, _) = setup();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let pending = root.join("execution/.work-correction-other.tmp");
        fs::write(&pending, b"existing").unwrap();
        assert_eq!(
            storage
                .publish_correction(&publication)
                .unwrap_err()
                .reason_code,
            "correction_create_transaction_present"
        );
        assert_eq!(fs::read(&pending).unwrap(), b"existing");
        assert_eq!(
            fs::read(root.join("execution/index.json")).unwrap(),
            b"before"
        );

        let (root, _) = setup();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        fs::write(root.join("execution/index.json"), b"changed").unwrap();
        assert_eq!(
            storage
                .publish_correction(&publication)
                .unwrap_err()
                .reason_code,
            "correction_create_source_changed"
        );
        assert_eq!(
            fs::read(root.join("execution/index.json")).unwrap(),
            b"changed"
        );
        assert!(
            !root
                .join("execution/.work-correction-TASK-001-ATTEMPT-001-CORRECTION-001-artifact.tmp")
                .exists()
        );

        let (root, _) = setup();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let correction =
            root.join("execution/TASK-001/ATTEMPT-001/corrections/ATTEMPT-001-CORRECTION-001.json");
        fs::create_dir_all(correction.parent().unwrap()).unwrap();
        fs::write(&correction, b"existing").unwrap();
        assert_eq!(
            storage
                .publish_correction(&publication)
                .unwrap_err()
                .reason_code,
            "correction_create_artifact_install_failed"
        );
        assert_eq!(fs::read(&correction).unwrap(), b"existing");
        assert_eq!(
            fs::read(root.join("execution/index.json")).unwrap(),
            b"locked"
        );
        assert_eq!(
            fs::read(root.join(
                "execution/.work-correction-TASK-001-ATTEMPT-001-CORRECTION-001-artifact.tmp"
            ))
            .unwrap(),
            b"correction"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (root, attempt) = setup();
            let storage = LocalExecutionStorage {
                project_root: root.clone(),
            };
            let parent = attempt.parent().unwrap();
            let original_permissions = fs::metadata(parent).unwrap().permissions();
            fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).unwrap();
            let failure = storage.publish_correction(&publication).unwrap_err();
            fs::set_permissions(parent, original_permissions).unwrap();
            assert_eq!(failure.reason_code, "correction_create_directory_failed");
            assert_eq!(failure.details["recovery_required"], true);
            assert_eq!(
                fs::read(root.join("execution/index.json")).unwrap(),
                b"locked"
            );
            assert_eq!(fs::read(&attempt).unwrap(), b"attempt");
            assert_eq!(
                fs::read(root.join(
                    "execution/.work-correction-TASK-001-ATTEMPT-001-CORRECTION-001-artifact.tmp"
                ))
                .unwrap(),
                b"correction"
            );
        }
    }

    impl CommandRunner for FakeCommandRunner {
        fn run(&self, request: &CommandRequest) -> work_feature::ports::CommandOutcome {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(request.argv, ["/usr/bin/printf", "ok"]);
            work_feature::ports::CommandOutcome {
                status: if self.timed_out {
                    CommandStatus::TimedOut
                } else {
                    CommandStatus::Exited
                },
                exit_code: (!self.timed_out).then_some(self.exit_code),
                stdout_tail: "ok".into(),
                stdout_truncated: false,
                stderr_tail: String::new(),
                stderr_truncated: false,
            }
        }
    }

    #[test]
    fn failed_result_receipt_blocks_command_reexecution() {
        struct BlockFinishedReceipt {
            path: PathBuf,
            calls: Cell<usize>,
        }
        impl CommandRunner for BlockFinishedReceipt {
            fn run(&self, _request: &CommandRequest) -> work_feature::ports::CommandOutcome {
                self.calls.set(self.calls.get() + 1);
                fs::create_dir(&self.path).unwrap();
                work_feature::ports::CommandOutcome {
                    status: CommandStatus::Exited,
                    exit_code: Some(0),
                    stdout_tail: String::new(),
                    stdout_truncated: false,
                    stderr_tail: String::new(),
                    stderr_truncated: false,
                }
            }
        }

        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let attempt_dir = root.join("execution/TASK-001/ATTEMPT-001");
        fs::create_dir_all(&attempt_dir).unwrap();
        let preview = build_command_preview(CommandPreviewInput {
            request: &json!({"schema":"work-command-run-request","timeout_seconds":60}),
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            working_directory: &root.to_string_lossy(),
            execution: &json!({"os":"macos","working_directory":"."}),
            invocation: &json!({"kind":"direct","executable":"/usr/bin/printf",
                "executable_sha256":"a".repeat(64),"argv":["printf","ok"]}),
            receipt_prefix: "execution/TASK-001/ATTEMPT-001/.work-command-CMD-001",
            sources: &json!({}),
        })
        .unwrap();
        let approved = preview["approved_sha256"].as_str().unwrap();
        let started = attempt_dir.join(".work-command-CMD-001.started.json");
        let runner = BlockFinishedReceipt {
            path: attempt_dir.join(".work-command-CMD-001.finished.json"),
            calls: Cell::new(0),
        };
        let failure = storage
            .run_prepared_command(
                "execution",
                approved,
                || Ok((preview.clone(), "Approved".into())),
                &runner,
            )
            .unwrap_err();
        assert_eq!(failure.reason_code, "command_run_interrupted");
        assert!(started.is_file());
        assert_eq!(runner.calls.get(), 1);
        assert_eq!(
            storage
                .run_prepared_command(
                    "execution",
                    approved,
                    || Ok((preview.clone(), "Approved".into())),
                    &runner,
                )
                .unwrap_err()
                .reason_code,
            "command_run_already_started"
        );
        assert_eq!(runner.calls.get(), 1);
    }

    #[test]
    fn command_correction_publishes_index_and_rejects_stale_or_pending_source() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let attempt_dir = root.join("execution/TASK-001/ATTEMPT-001");
        fs::create_dir_all(&attempt_dir).unwrap();
        fs::write(root.join("execution/index.json"), b"before").unwrap();
        fs::write(attempt_dir.join("attempt.json"), b"attempt").unwrap();
        let publication = CommandCorrectionPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            attempt_before: b"attempt",
            index_before: b"before",
            index_after: b"after",
        };
        storage.publish_command_correction(&publication).unwrap();
        assert_eq!(
            fs::read(root.join("execution/index.json")).unwrap(),
            b"after"
        );
        assert_eq!(
            storage
                .publish_command_correction(&publication)
                .unwrap_err()
                .reason_code,
            "command_correction_source_changed"
        );
        fs::write(
            root.join("execution/.work-command-correction-other.tmp"),
            b"pending",
        )
        .unwrap();
        assert_eq!(
            storage
                .publish_command_correction(&publication)
                .unwrap_err()
                .reason_code,
            "command_correction_transaction_present"
        );
        assert_eq!(
            fs::read(root.join("execution/index.json")).unwrap(),
            b"after"
        );
    }

    #[test]
    fn command_correction_recovery_installs_only_prepared_canonical_index() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"e".repeat(64));
        index["lock"]["record_id"] = json!("CMD-001");
        let index_raw = render_execution_index(&index).unwrap();
        let mut prepared = index.clone();
        prepared["lock"]["command_correction"] = json!({
            "original_command":{"mode":"argv","argv":["tool"]},
            "actual_command":{"mode":"argv","argv":["/bin/tool"]},
            "reason":"Use full path","authorization_evidence":"Approved"});
        let prepared_raw = render_execution_index(&prepared).unwrap();
        let task = json!({"commands":[{"id":"CMD-001","mode":"argv","argv":["tool"]}]});
        let index_path = root.join("execution/index.json");
        let attempt_path = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(attempt_path.parent().unwrap()).unwrap();
        LocalFiles.create_new(&index_path, &index_raw).unwrap();
        LocalFiles.create_new(&attempt_path, b"attempt\n").unwrap();
        let temporary =
            root.join("execution/.work-command-correction-TASK-001-ATTEMPT-001-CMD-001.tmp");
        LocalFiles.create_new(&temporary, &prepared_raw).unwrap();
        let result = storage
            .recover_command_correction(CommandCorrectionRecoveryInput {
                execution_dir: "execution",
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                task: &task,
                index_before: &index_raw,
                attempt_before: b"attempt\n",
            })
            .unwrap();
        assert_eq!(result["record_id"], "CMD-001");
        assert_eq!(LocalFiles.read_raw(&index_path).unwrap(), prepared_raw);
        assert!(!temporary.exists());
    }

    #[test]
    fn command_prepare_repository_checks_sources_directory_and_receipts() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        storage.check_ready("execution", true).unwrap();
        assert_eq!(
            storage.working_directory(".").unwrap(),
            root.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(
            storage
                .working_directory("missing")
                .unwrap_err()
                .reason_code,
            "command_run_cwd"
        );
        fs::write(root.join("execution/index.json"), b"source").unwrap();
        assert_eq!(
            storage.read_source("execution/index.json").unwrap(),
            b"source"
        );
        {
            let directory = "execution/TASK-001/ATTEMPT-001/receipts/CMD-001";
            assert!(!storage.receipt_exists(directory).unwrap());
            fs::create_dir_all(root.join(directory)).unwrap();
            fs::write(root.join(format!("{directory}/started.json")), b"{partial").unwrap();
            assert!(storage.receipt_exists(directory).unwrap());
            assert!(
                storage
                    .receipt_exists("execution/.work-command-CMD-001")
                    .is_err()
            );
        }
        let writer = LocalWriterLock
            .acquire(&root.join("execution/.work-state-writer.lock"))
            .unwrap();
        storage.check_ready("execution", false).unwrap();
        assert!(storage.check_ready("execution", true).is_err());
        drop(writer);
        fs::write(root.join("execution/.work-transaction.tmp"), b"pending").unwrap();
        assert_eq!(
            storage
                .check_ready("execution", true)
                .unwrap_err()
                .reason_code,
            "command_run_pending_transaction"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn command_run_real_process_writes_receipts_and_never_reexecutes() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        fs::create_dir_all(root.join("execution/TASK-001/ATTEMPT-001")).unwrap();
        let invocation = storage
            .resolve_command_invocation(
                &[
                    "/usr/bin/printf".into(),
                    "%s|".into(),
                    "two words".into(),
                    "$HOME".into(),
                    "$(touch unwanted)".into(),
                    "a;b".into(),
                    "x|y".into(),
                    "a\"b".into(),
                ],
                &root,
            )
            .unwrap();
        let request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let preview = build_command_preview(CommandPreviewInput {
            request: &request,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            working_directory: &root.to_string_lossy(),
            execution: &json!({"os":"macos","working_directory":"."}),
            invocation: &invocation,
            receipt_prefix: "execution/TASK-001/ATTEMPT-001/.work-command-CMD-001",
            sources: &json!({}),
        })
        .unwrap();
        assert!(!root.join("execution/.work-state-writer.lock").exists());
        assert!(
            !root
                .join("execution/TASK-001/ATTEMPT-001/.work-command-CMD-001.started.json")
                .exists()
        );
        let approved = preview["approved_sha256"].as_str().unwrap();
        let result = storage
            .run_prepared_command(
                "execution",
                approved,
                || Ok((preview.clone(), "Approved".into())),
                &LocalCommandRunner,
            )
            .unwrap();
        assert_eq!(
            result["stdout_tail"],
            "two words|$HOME|$(touch unwanted)|a;b|x|y|a\"b|"
        );
        assert!(!root.join("unwanted").exists());
        assert_eq!(result["record_finish_required"], true);
        assert_eq!(
            storage
                .run_prepared_command(
                    "execution",
                    approved,
                    || Ok((preview.clone(), "Approved".into())),
                    &LocalCommandRunner
                )
                .unwrap_err()
                .reason_code,
            "command_run_already_started"
        );
    }

    #[test]
    fn command_receipts_prevent_retry_and_preserve_failed_result() {
        for code in [0, 7, -1] {
            let root = test_root();
            let storage = LocalExecutionStorage {
                project_root: root.clone(),
            };
            fs::create_dir_all(root.join("execution/TASK-001/ATTEMPT-001")).unwrap();
            let request = json!({"schema":"work-command-run-request","timeout_seconds":60});
            let execution = json!({"os":"macos","working_directory":"."});
            let invocation = json!({"kind":"direct","executable":"/usr/bin/printf",
                "executable_sha256":"a".repeat(64),"argv":["printf","ok"]});
            let sources = json!({"task/index.json":"b".repeat(64)});
            let preview = build_command_preview(CommandPreviewInput {
                request: &request,
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                record_id: "CMD-001",
                working_directory: &root.to_string_lossy(),
                execution: &execution,
                invocation: &invocation,
                receipt_prefix: "execution/TASK-001/ATTEMPT-001/.work-command-CMD-001",
                sources: &sources,
            })
            .unwrap();
            let approved = preview["approved_sha256"].as_str().unwrap();
            let runner = FakeCommandRunner {
                calls: Cell::new(0),
                exit_code: code,
                timed_out: code == -1,
            };
            let prefix = root.join("execution/TASK-001/ATTEMPT-001/.work-command-CMD-001");
            let started = PathBuf::from(format!("{}.started.json", prefix.display()));
            let finished = PathBuf::from(format!("{}.finished.json", prefix.display()));
            assert_eq!(
                storage
                    .run_prepared_command(
                        "execution",
                        &"0".repeat(64),
                        || Ok((preview.clone(), "Wrong approval".into())),
                        &runner
                    )
                    .unwrap_err()
                    .reason_code,
                "command_run_approval_changed"
            );
            let mut stale = preview.clone();
            stale["sources"]["task/index.json"] = json!("c".repeat(64));
            assert_eq!(
                storage
                    .run_prepared_command(
                        "execution",
                        approved,
                        || Ok((stale, "Stale approval".into())),
                        &runner
                    )
                    .unwrap_err()
                    .reason_code,
                "command_run_approval_changed"
            );
            assert_eq!(runner.calls.get(), 0);
            assert!(!started.exists() && !finished.exists());
            for receipt in [&started, &finished] {
                fs::write(receipt, b"preserved").unwrap();
                assert_eq!(
                    storage
                        .run_prepared_command(
                            "execution",
                            approved,
                            || Ok((preview.clone(), "Approved command".into())),
                            &runner
                        )
                        .unwrap_err()
                        .reason_code,
                    "command_run_already_started"
                );
                assert_eq!(runner.calls.get(), 0);
                assert_eq!(fs::read(receipt).unwrap(), b"preserved");
                fs::remove_file(receipt).unwrap();
            }
            let result = storage.run_prepared_command(
                "execution",
                approved,
                || Ok((preview.clone(), "Approved command".into())),
                &runner,
            );
            if code == 0 {
                assert_eq!(result.unwrap()["record_finish_required"], true);
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.reason_code, "command_run_failed");
                if code == -1 {
                    assert_eq!(failure.details["status"], "timed_out");
                    assert!(failure.details["exit_code"].is_null());
                    assert!(failure.details.get("record_finish_request").is_none());
                } else {
                    assert_eq!(failure.details["exit_code"], 7);
                }
            }
            assert_eq!(runner.calls.get(), 1);
            assert!(started.is_file() && finished.is_file());
            let started_value =
                parse_json_contract(&LocalFiles.read_raw(&started).unwrap()).unwrap();
            let finished_value =
                parse_json_contract(&LocalFiles.read_raw(&finished).unwrap()).unwrap();
            assert_eq!(started_value["authorization_evidence"], "Approved command");
            assert_eq!(
                finished_value["exit_code"],
                if code == -1 { Value::Null } else { json!(code) }
            );
            assert_eq!(
                storage
                    .run_prepared_command(
                        "execution",
                        approved,
                        || Ok((preview.clone(), "Approved command".into())),
                        &runner
                    )
                    .unwrap_err()
                    .reason_code,
                "command_run_already_started"
            );
            assert_eq!(runner.calls.get(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn direct_command_resolution_preserves_symlink_name_and_rejects_batch() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let executable = root.join("real tool");
        fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("venv tool");
        symlink(&executable, &link).unwrap();
        let invocation = storage
            .resolve_command_invocation(&["./venv tool".into(), "a b".into()], &root)
            .unwrap();
        assert_eq!(invocation["kind"], "direct");
        assert_eq!(invocation["executable"], link.to_string_lossy().as_ref());
        assert_eq!(invocation["argv"], json!(["./venv tool", "a b"]));
        assert_eq!(
            invocation["executable_sha256"],
            work_operations::derivation::fingerprint::raw(b"#!/bin/sh\nexit 0\n")
        );
        let batch = root.join("run.cmd");
        fs::write(&batch, b"echo hello\n").unwrap();
        assert_eq!(
            storage
                .resolve_command_invocation(&["./run.cmd".into()], &root)
                .unwrap_err()
                .reason_code,
            "command_run_argv_only"
        );
    }

    #[test]
    fn recovery_authorization_rejection_uses_workflow_exit_code() {
        let issue = work_operations::execution::ExecutionIssue {
            reason_code: "execution_authorization_result_required",
            message: "Fresh evidence required.",
            details: json!({}),
        };
        let failure = recovery_rule(issue);
        assert_eq!(failure.exit_code, ExitCode::WorkflowState);
    }

    #[test]
    fn attempt_start_publication_orders_lock_attempt_and_started_index() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let index = root.join("execution/index.json");
        LocalFiles.create_new(&index, b"before\n").unwrap();
        let publication = AttemptStartPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            index_before: b"before\n",
            locked_index: b"locked\n",
            attempt: b"attempt\n",
            started_index: b"started\n",
            expected_snapshot: "ddda6f7ea41371550f146888805ee13aad40bcf55280881fb378b18e333b755d",
        };
        storage.publish_attempt_start(&publication).unwrap();
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"started\n");
        assert_eq!(
            storage.attempt_names("execution", "TASK-001").unwrap(),
            ["ATTEMPT-001"]
        );
        assert_eq!(
            LocalFiles
                .read_raw(&root.join("execution/TASK-001/ATTEMPT-001/attempt.json"))
                .unwrap(),
            b"attempt\n"
        );
        assert!(
            !root
                .join("execution/.work-attempt-start-TASK-001-ATTEMPT-001-started.tmp")
                .exists()
        );
        assert_eq!(
            storage
                .publish_attempt_start(&publication)
                .unwrap_err()
                .reason_code,
            "attempt_start_transaction_present"
        );
        let retry = AttemptStartPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-002",
            index_before: b"started\n",
            locked_index: b"retry locked\n",
            attempt: b"retry attempt\n",
            started_index: b"retry started\n",
            expected_snapshot: publication.expected_snapshot,
        };
        storage.publish_attempt_start(&retry).unwrap();
        assert_eq!(
            storage.attempt_names("execution", "TASK-001").unwrap(),
            ["ATTEMPT-001", "ATTEMPT-002"]
        );
        assert_eq!(
            LocalFiles
                .read_raw(&root.join("execution/TASK-001/ATTEMPT-001/attempt.json"))
                .unwrap(),
            b"attempt\n"
        );
        assert_eq!(
            LocalFiles
                .read_raw(&root.join("execution/TASK-001/ATTEMPT-002/attempt.json"))
                .unwrap(),
            b"retry attempt\n"
        );
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"retry started\n");
    }

    #[cfg(unix)]
    #[test]
    fn attempt_start_failure_reports_recovery_stage_after_lock_install() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let index = root.join("execution/index.json");
        LocalFiles.create_new(&index, b"before\n").unwrap();
        let task_directory = root.join("execution/TASK-001");
        fs::create_dir(&task_directory).unwrap();
        let original_permissions = fs::metadata(&task_directory).unwrap().permissions();
        fs::set_permissions(&task_directory, fs::Permissions::from_mode(0o500)).unwrap();
        let publication = AttemptStartPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            index_before: b"before\n",
            locked_index: b"locked\n",
            attempt: b"attempt\n",
            started_index: b"started\n",
            expected_snapshot: "ddda6f7ea41371550f146888805ee13aad40bcf55280881fb378b18e333b755d",
        };
        let failure = storage.publish_attempt_start(&publication).unwrap_err();
        fs::set_permissions(&task_directory, original_permissions).unwrap();
        assert_eq!(
            failure.reason_code,
            "attempt_start_attempt_directory_failed"
        );
        assert_eq!(failure.details["attempt_id"], "ATTEMPT-001");
        assert_eq!(failure.details["recovery_required"], true);
        assert_eq!(failure.details["transaction_stage"], "lock_installed");
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"locked\n");
    }

    #[test]
    fn attempt_start_rejects_bad_layout_before_publication() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let index = root.join("execution/index.json");
        LocalFiles.create_new(&index, b"before\n").unwrap();
        let publication = AttemptStartPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            index_before: b"before\n",
            locked_index: b"locked\n",
            attempt: b"attempt\n",
            started_index: b"started\n",
            expected_snapshot: "ddda6f7ea41371550f146888805ee13aad40bcf55280881fb378b18e333b755d",
        };
        fs::create_dir(root.join("execution/TASK-001")).unwrap();
        fs::write(root.join("execution/TASK-001/ATTEMPT-001"), b"blocked").unwrap();
        let failure = storage.publish_attempt_start(&publication).unwrap_err();
        assert_eq!(failure.reason_code, "path_resolution_failed");
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"before\n");
        assert!(
            !root
                .join("execution/.work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp")
                .exists()
        );
    }

    #[test]
    fn attempt_start_rechecks_git_snapshot_under_writer_lock() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let index = root.join("execution/index.json");
        LocalFiles.create_new(&index, b"before\n").unwrap();
        LocalFiles
            .create_new(&root.join("changed.txt"), b"new\n")
            .unwrap();
        let publication = AttemptStartPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            index_before: b"before\n",
            locked_index: b"locked\n",
            attempt: b"attempt\n",
            started_index: b"started\n",
            expected_snapshot: "ddda6f7ea41371550f146888805ee13aad40bcf55280881fb378b18e333b755d",
        };
        let mismatch = storage.publish_attempt_start(&publication).unwrap_err();
        assert_eq!(
            mismatch.reason_code,
            "attempt_start_worktree_snapshot_changed"
        );
        assert_eq!(mismatch.details["expected"], publication.expected_snapshot);
        assert!(mismatch.details["actual"].as_str().is_some());
        assert_ne!(mismatch.details["actual"], mismatch.details["expected"]);
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"before\n");
        assert!(
            !root
                .join("execution/.work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp")
                .exists()
        );
    }

    #[test]
    fn execution_readiness_rejects_incomplete_start_and_writer_contention() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let task = root.join("task/index.json");
        fs::create_dir_all(task.parent().unwrap()).unwrap();
        LocalFiles.create_new(&task, b"task\n").unwrap();
        storage
            .check_execution_ready("task/index.json", "execution")
            .unwrap();
        let legacy_directory = root.join("execution/TASK-001");
        fs::create_dir_all(&legacy_directory).unwrap();
        let legacy = legacy_directory.join("ATTEMPT-001.json");
        LocalFiles.create_new(&legacy, b"legacy").unwrap();
        let error = storage
            .check_execution_task_layout("execution", "TASK-001")
            .unwrap_err();
        assert_eq!(error.exit_code, ExitCode::ArtifactIntegrity);
        assert_eq!(error.reason_code, "execution_legacy_layout_unsupported");
        assert_eq!(error.details["files"], json!(["ATTEMPT-001.json"]));
        assert_eq!(LocalFiles.read_raw(&legacy).unwrap(), b"legacy");
        let old_correction = legacy_directory.join("ATTEMPT-001-CORRECTION-001.json");
        LocalFiles
            .create_new(&old_correction, b"old correction")
            .unwrap();
        let error = storage
            .check_execution_task_layout("execution", "TASK-001")
            .unwrap_err();
        assert_eq!(error.reason_code, "execution_legacy_layout_unsupported");
        assert_eq!(
            error.details["files"],
            json!(["ATTEMPT-001-CORRECTION-001.json", "ATTEMPT-001.json"])
        );
        assert_eq!(
            LocalFiles.read_raw(&old_correction).unwrap(),
            b"old correction"
        );
        let pending = root.join("execution/.work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp");
        LocalFiles.create_new(&pending, b"prepared\n").unwrap();
        assert_eq!(
            storage
                .check_execution_ready("task/index.json", "execution")
                .unwrap_err()
                .reason_code,
            "execute_preflight_attempt_start_transaction_present"
        );
        LocalFiles.remove(&pending).unwrap();
        let lock = root.join("execution/.work-state-writer.lock");
        let _writer = LocalWriterLock.acquire(&lock).unwrap();
        let index = root.join("execution/index.json");
        LocalFiles.create_new(&index, b"before\n").unwrap();
        let publication = AttemptStartPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            index_before: b"before\n",
            locked_index: b"locked\n",
            attempt: b"attempt\n",
            started_index: b"started\n",
            expected_snapshot: "ddda6f7ea41371550f146888805ee13aad40bcf55280881fb378b18e333b755d",
        };
        assert_eq!(
            storage
                .publish_attempt_start(&publication)
                .unwrap_err()
                .reason_code,
            "work_state_writer_busy"
        );
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"before\n");
    }

    #[test]
    fn project_repository_reads_canonical_index() {
        let root = std::env::temp_dir().join(format!(
            "work-execution-storage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let path = root.join("execution/index.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let c = "c".repeat(64);
        let d = "d".repeat(64);
        let e = "e".repeat(64);
        let f = "f".repeat(64);
        let z = "0".repeat(64);
        let validation = json!({"collection_contract":{"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "artifacts":{"execution":"execution"},"tasks":[{"id":"TASK-001","dependencies":[],"inputs":[]}]},
            "task_ids":["TASK-001"],"task_instructions_sha256":{"TASK-001":a},
            "task_skill_ids":{"TASK-001":null},"task_item_sha256":{"TASK-001":b},
            "instructions_sha256":c,"hierarchy_selection_sha256":d,"skill_selection_sha256":e,
            "task_collection_sha256":f,"task_index_sha256":z});
        let (_, raw) = prepare_initial_index(&validation).unwrap();
        LocalFiles.create_new(&path, &raw).unwrap();
        let ready = preflight_index(
            &storage,
            &validation,
            "execution",
            "TASK-001",
            &[],
            None,
            &["pending", "pending_retry"],
        )
        .unwrap();
        assert_eq!(ready["index_validation"]["task_count"], 1);
        let composed = storage.inspect_path("Straße/CAFÉ.txt").unwrap();
        let decomposed = storage.inspect_path("STRASSE/cafe\u{301}.txt").unwrap();
        assert_eq!(composed.identity, decomposed.identity);
        assert!(!composed.exists);
        assert!(!decomposed.exists);
        assert_eq!(
            storage
                .read_index("missing/index.json")
                .unwrap_err()
                .reason_code,
            "execute_preflight_index_missing"
        );
    }

    #[test]
    fn record_begin_publishes_reserved_index_and_refuses_stale_source() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let index = root.join("execution/index.json");
        let attempt = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(attempt.parent().unwrap()).unwrap();
        LocalFiles.create_new(&index, b"before\n").unwrap();
        LocalFiles.create_new(&attempt, b"attempt\n").unwrap();
        let publication = RecordBeginPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001#1",
            index_before: b"before\n",
            attempt_before: b"attempt\n",
            index_after: b"reserved\n",
        };
        storage.publish_record_begin(&publication).unwrap();
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"reserved\n");
        assert!(
            !root
                .join("execution/.work-record-begin-TASK-001-ATTEMPT-001-CMD-001-retry-1.tmp")
                .exists()
        );
        assert_eq!(
            storage
                .publish_record_begin(&publication)
                .unwrap_err()
                .reason_code,
            "record_begin_index_changed"
        );
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"reserved\n");
    }

    #[test]
    fn record_begin_rejects_another_preserved_transaction() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let index = root.join("execution/index.json");
        let attempt = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(attempt.parent().unwrap()).unwrap();
        LocalFiles.create_new(&index, b"before\n").unwrap();
        LocalFiles.create_new(&attempt, b"attempt\n").unwrap();
        let pending = root.join("execution/.work-record-begin-TASK-001-ATTEMPT-001-VAL-001.tmp");
        LocalFiles.create_new(&pending, b"preserved\n").unwrap();
        let publication = RecordBeginPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            index_before: b"before\n",
            attempt_before: b"attempt\n",
            index_after: b"reserved\n",
        };
        let failure = storage.publish_record_begin(&publication).unwrap_err();
        assert_eq!(failure.reason_code, "record_begin_transaction_present");
        assert_eq!(
            failure.details["files"],
            json!([pending.file_name().unwrap().to_str().unwrap()])
        );
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"before\n");
    }

    #[test]
    fn record_begin_recovery_installs_exact_retry_index() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"c".repeat(64),
            "hierarchy_selection_sha256":"d".repeat(64),
            "skill_selection_sha256":"e".repeat(64),
            "task_item_sha256":{"TASK-001":"f".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"0".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"1".repeat(64));
        let original = render_execution_index(&index).unwrap();
        let attempt = json!({"records":[{"id":"VAL-001"}],
            "authorization":{"authorization_evidence":"Original"}});
        let task =
            json!({"id":"TASK-001","commands":[],"operations":[],"validations":[{"id":"VAL-001"}]});
        let mut prepared = index.clone();
        prepared["lock"]["record_id"] = json!("VAL-001#1");
        prepared["lock"]["retry_authorization_evidence"] = json!("Approved retry");
        let prepared_raw = render_execution_index(&prepared).unwrap();
        let index_path = root.join("execution/index.json");
        let attempt_path = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        fs::create_dir_all(attempt_path.parent().unwrap()).unwrap();
        LocalFiles.create_new(&index_path, &original).unwrap();
        LocalFiles.create_new(&attempt_path, b"attempt\n").unwrap();
        let temporary =
            root.join("execution/.work-record-begin-TASK-001-ATTEMPT-001-VAL-001-retry-1.tmp");
        let mut tampered = prepared.clone();
        tampered["title"] = json!("Changed");
        LocalFiles
            .create_new(&temporary, &render_execution_index(&tampered).unwrap())
            .unwrap();
        assert_eq!(
            storage
                .recover_record_begin(RecordBeginRecoveryInput {
                    execution_dir: "execution",
                    task_id: "TASK-001",
                    attempt_id: "ATTEMPT-001",
                    task: &task,
                    attempt: &attempt,
                    index_before: &original,
                    attempt_before: b"attempt\n",
                })
                .unwrap_err()
                .reason_code,
            "execution_recovery_record_begin_target_mismatch"
        );
        assert_eq!(LocalFiles.read_raw(&index_path).unwrap(), original);
        assert!(temporary.exists());
        fs::write(&temporary, &prepared_raw).unwrap();
        let recovered = storage
            .recover_record_begin(RecordBeginRecoveryInput {
                execution_dir: "execution",
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                task: &task,
                attempt: &attempt,
                index_before: &original,
                attempt_before: b"attempt\n",
            })
            .unwrap();
        assert_eq!(recovered["record_id"], "VAL-001#1");
        assert_eq!(LocalFiles.read_raw(&index_path).unwrap(), prepared_raw);
        assert!(!temporary.exists());
    }

    #[test]
    fn attempt_close_publishes_attempt_before_index_and_refuses_stale_source() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let attempt = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        let index = root.join("execution/index.json");
        fs::create_dir_all(attempt.parent().unwrap()).unwrap();
        LocalFiles.create_new(&attempt, b"open attempt\n").unwrap();
        LocalFiles.create_new(&index, b"open index\n").unwrap();
        let publication = AttemptClosePublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            attempt_before: b"open attempt\n",
            attempt_after: b"closed attempt\n",
            index_before: b"open index\n",
            index_after: b"closed index\n",
        };
        storage.publish_attempt_close(&publication).unwrap();
        assert_eq!(LocalFiles.read_raw(&attempt).unwrap(), b"closed attempt\n");
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"closed index\n");
        assert_eq!(
            storage
                .publish_attempt_close(&publication)
                .unwrap_err()
                .reason_code,
            "attempt_close_source_changed"
        );
    }

    #[test]
    fn record_finish_publishes_both_artifacts_and_rejects_pending_transaction() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let attempt = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        let index = root.join("execution/index.json");
        fs::create_dir_all(attempt.parent().unwrap()).unwrap();
        LocalFiles
            .create_new(&attempt, b"before attempt\n")
            .unwrap();
        LocalFiles.create_new(&index, b"before index\n").unwrap();
        let publication = RecordFinishPublication {
            execution_dir: "execution",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "VAL-001#1",
            request: &json!({"schema":"work-record-finish-request","record":{"status":"completed","outcome":"passed","evidence":"Passed"}}),
            attempt_before: b"before attempt\n",
            attempt_after: b"after attempt\n",
            index_before: b"before index\n",
            index_after: b"after index\n",
        };
        let pending = root
            .join("execution/.work-record-finish-TASK-001-ATTEMPT-001-VAL-001-retry-1-attempt.tmp");
        LocalFiles.create_new(&pending, b"prepared\n").unwrap();
        assert_eq!(
            storage
                .publish_record_finish(&publication)
                .unwrap_err()
                .reason_code,
            "record_finish_transaction_present"
        );
        assert_eq!(LocalFiles.read_raw(&attempt).unwrap(), b"before attempt\n");
        LocalFiles.remove(&pending).unwrap();
        storage.publish_record_finish(&publication).unwrap();
        assert_eq!(LocalFiles.read_raw(&attempt).unwrap(), b"after attempt\n");
        assert_eq!(LocalFiles.read_raw(&index).unwrap(), b"after index\n");
        assert!(!pending.exists());
        assert_eq!(
            storage
                .publish_record_finish(&publication)
                .unwrap_err()
                .reason_code,
            "record_finish_source_changed"
        );
    }

    #[test]
    fn record_finish_recovery_installs_verified_prepared_attempt_and_index() {
        let root = test_root();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001","TASK-001-ACCEPTANCE-001"]}],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":work_operations::execution::acceptance::pending(["ACCEPTANCE-001".to_owned(),"TASK-001-ACCEPTANCE-001".to_owned()]),"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "acceptance_criteria":[{"id":"ACCEPTANCE-001"}],"tasks":[{"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001"}]}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"skill_selection_sha256":"0".repeat(64),
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"e".repeat(64));
        index["lock"]["record_id"] = json!("VAL-001");
        let task = json!({"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001"}],"commands":[],"operations":[],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001","TASK-001-ACCEPTANCE-001"]}]});
        let request = json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Passed actual VAL"}});
        let candidate = work_operations::execution::record_finish::build_record_finish_candidates(
            &task, &attempt, &index, "TASK-001", &request,
        )
        .unwrap();
        let prepared = candidate.attempt;
        assert!(
            prepared["acceptance_results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["status"] == "completed")
        );
        let attempt_raw = render_attempt(&attempt).unwrap();
        let index_raw = render_execution_index(&index).unwrap();
        let prepared_raw = render_attempt(&prepared).unwrap();
        let attempt_path = root.join("execution/TASK-001/ATTEMPT-001/attempt.json");
        let index_path = root.join("execution/index.json");
        fs::create_dir_all(attempt_path.parent().unwrap()).unwrap();
        LocalFiles.create_new(&attempt_path, &attempt_raw).unwrap();
        LocalFiles.create_new(&index_path, &index_raw).unwrap();
        let temporary =
            root.join("execution/.work-record-finish-TASK-001-ATTEMPT-001-VAL-001-attempt.tmp");
        LocalFiles.create_new(&temporary, &prepared_raw).unwrap();
        let recovered = storage
            .recover_record_finish(RecordFinishRecoveryInput {
                execution_dir: "execution",
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                task: &task,
                attempt: &attempt,
                index: &index,
                index_before: &index_raw,
                attempt_before: &attempt_raw,
                authorization_evidence: None,
            })
            .unwrap();
        assert_eq!(recovered["record_id"], "VAL-001");
        assert_eq!(LocalFiles.read_raw(&attempt_path).unwrap(), prepared_raw);
        assert!(!temporary.exists());
        let installed_index = LocalFiles.read_raw(&index_path).unwrap();
        let parsed_index = parse_json_contract(&installed_index).unwrap();
        assert!(parsed_index["lock"].get("record_id").is_none());
        assert_eq!(parsed_index, candidate.index);
        assert_eq!(parsed_index["acceptance_results"][0]["status"], "completed");
        assert_eq!(
            parsed_index["tasks"][0]["acceptance_results"],
            prepared["acceptance_results"]
        );
        for row in prepared["acceptance_results"].as_array().unwrap() {
            assert_eq!(row["evidence"][0]["record_id"], "VAL-001");
            assert_eq!(
                row["evidence"][0]["task_item_sha256"],
                attempt["task_item_sha256"]
            );
        }

        let interrupted_index = root.join("execution/record-finish-interrupted-index.tmp");
        LocalFiles
            .create_new(&interrupted_index, &index_raw)
            .unwrap();
        LocalFiles.replace(&interrupted_index, &index_path).unwrap();
        let mut missing_progress = prepared.clone();
        missing_progress["acceptance_results"] =
            work_operations::execution::acceptance::reset(&missing_progress["acceptance_results"])
                .unwrap();
        let missing_progress_raw = render_attempt(&missing_progress).unwrap();
        fs::write(&attempt_path, &missing_progress_raw).unwrap();
        assert_eq!(
            storage
                .recover_record_finish(RecordFinishRecoveryInput {
                    execution_dir: "execution",
                    task_id: "TASK-001",
                    attempt_id: "ATTEMPT-001",
                    task: &task,
                    attempt: &missing_progress,
                    index: &index,
                    index_before: &index_raw,
                    attempt_before: &missing_progress_raw,
                    authorization_evidence: None
                })
                .unwrap_err()
                .reason_code,
            "execution_recovery_acceptance_result_mismatch"
        );
        assert_eq!(fs::read(&index_path).unwrap(), index_raw);
        assert_eq!(fs::read(&attempt_path).unwrap(), missing_progress_raw);
        fs::write(&attempt_path, &prepared_raw).unwrap();
        let resumed_finish = storage
            .recover_record_finish(RecordFinishRecoveryInput {
                execution_dir: "execution",
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                task: &task,
                attempt: &prepared,
                index: &index,
                index_before: &index_raw,
                attempt_before: &prepared_raw,
                authorization_evidence: None,
            })
            .unwrap();
        assert_eq!(resumed_finish["record_id"], "VAL-001");
        assert_eq!(LocalFiles.read_raw(&index_path).unwrap(), installed_index);

        let close_request = json!({"schema":"work-attempt-close-request",
            "status":"completed"});
        let (closed, closed_index) = build_close_candidates(
            &task,
            &parsed_index,
            &prepared,
            &close_request,
            "2026-09-01T10:10+08:00",
        )
        .unwrap();
        let closed_raw = render_attempt(&closed).unwrap();
        let close_temp =
            root.join("execution/.work-attempt-close-TASK-001-ATTEMPT-001-attempt.tmp");
        LocalFiles.create_new(&close_temp, &closed_raw).unwrap();
        let recovered_close = storage
            .recover_attempt_close(AttemptCloseRecoveryInput {
                execution_dir: "execution",
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                task: &task,
                attempt: &prepared,
                index: &parsed_index,
                index_before: &installed_index,
                attempt_before: &prepared_raw,
            })
            .unwrap();
        assert_eq!(recovered_close["attempt_status"], "completed");
        assert_eq!(LocalFiles.read_raw(&attempt_path).unwrap(), closed_raw);
        let recovered_attempt =
            parse_json_contract(&LocalFiles.read_raw(&attempt_path).unwrap()).unwrap();
        assert_eq!(recovered_attempt["schema"], "work-attempt");
        assert_eq!(recovered_attempt["status"], "completed");
        assert_eq!(
            recovered_attempt["acceptance_results"],
            prepared["acceptance_results"]
        );
        assert_eq!(recovered_attempt["records"], prepared["records"]);
        for field in [
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
        ] {
            assert_eq!(recovered_attempt[field], attempt[field]);
        }
        assert!(recovered_attempt.get("task_sha256").is_none());
        assert_eq!(
            LocalFiles.read_raw(&index_path).unwrap(),
            render_execution_index(&closed_index).unwrap()
        );
        assert!(!close_temp.exists());

        let interrupted_index = root.join("execution/interrupted-index.tmp");
        LocalFiles
            .create_new(&interrupted_index, &installed_index)
            .unwrap();
        LocalFiles.replace(&interrupted_index, &index_path).unwrap();
        let resumed_close = storage
            .recover_attempt_close(AttemptCloseRecoveryInput {
                execution_dir: "execution",
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                task: &task,
                attempt: &closed,
                index: &parsed_index,
                index_before: &installed_index,
                attempt_before: &closed_raw,
            })
            .unwrap();
        assert_eq!(resumed_close["attempt_status"], "completed");
        assert_eq!(
            LocalFiles.read_raw(&index_path).unwrap(),
            render_execution_index(&closed_index).unwrap()
        );
    }
}
