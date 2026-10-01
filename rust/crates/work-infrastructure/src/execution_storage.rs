//! Project-relative execution index repository.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
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
    RecoveryPrepareRepository, inspect_worktree, prepare_execute_preflight_from_project,
    recheck_command_from_project,
};
use work_feature::instruction::InstructionSourceRepository;
use work_feature::plan::PlanPathRepository;
use work_feature::ports::{ArtifactStore, CommandRunner, Git, WriterLock};
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
    pub fn recover_attempt_start_from_project<H, S, P, T>(
        &self,
        sources: &CommandProjectSources<'_, H, S, P, T>,
        input: AttemptStartRecoveryRequest<'_>,
    ) -> Result<Value, WorkError>
    where
        H: InstructionSourceRepository,
        S: SkillSnapshotRepository,
        P: PlanPathRepository,
        T: TaskCollectionRepository,
    {
        validate_attempt_start_request(input.request).map_err(recovery_rule)?;
        let target = input.target;
        require_no_spec_update(&self.project_root, target.execution_dir, None)?;
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", target.execution_dir),
        )?;
        let _writer = LocalWriterLock.acquire(&writer_path)?;
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
            json!({"schema":"work-attempt-start-recovery/v1","task_id":target.task_id,
            "attempt_id":attempt_id,"attempt_path":attempt_relative,
            "index_path":format!("{}/index.json", target.execution_dir),
            "status":"recovered","lock_status":"held"}),
        ))
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
        P: PlanPathRepository,
        T: TaskCollectionRepository,
    {
        validate_recovery_request(request, false).map_err(recovery_rule)?;
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
            &context.contract,
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
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        let _writer = LocalWriterLock.acquire(&writer_path)?;
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
        let request = json!({"schema":"work-correction-create-request/v1",
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
            json!({"schema":"work-execution-recovery/v1","transaction":"correction",
            "task_id":input.task_id,"attempt_id":input.attempt_id,
            "correction_id":input.correction_id,"correction_path":correction_relative,
            "index_path":format!("{}/index.json", input.execution_dir),
            "affected_task_ids":candidates.affected_task_ids,
            "lock_status":"released","status":"recovered"}),
        ))
    }

    pub fn recover_deviation_record(
        &self,
        input: DeviationRecordRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        let _writer = LocalWriterLock.acquire(&writer_path)?;
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
            json!({"schema":"work-execution-recovery/v1","status":"recovered",
            "transaction":"deviation_record","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":record_id,
            "lock_status":"record_reserved"}),
        ))
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
        P: PlanPathRepository,
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
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let writer_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        let _writer = LocalWriterLock.acquire(&writer_path)?;
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
            json!({"schema":"work-execution-recovery/v1","status":"recovered",
            "transaction":"command_correction","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":record_id,
            "lock_status":"record_reserved"}),
        ))
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
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution_dir}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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

    pub fn recover_attempt_close(
        &self,
        input: AttemptCloseRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        let execution = input.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
            json!({"schema":"work-execution-recovery/v1","status":"recovered",
            "transaction":"attempt_close","task_id":input.task_id,
            "attempt_id":input.attempt_id,"attempt_status":target.status,
            "attempt_path":format!("{execution}/{}/{}/attempt.json", input.task_id, input.attempt_id),
            "index_path":format!("{execution}/index.json"),"lock_status":"released"}),
        ))
    }

    pub fn recover_record_finish(
        &self,
        input: RecordFinishRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        let execution = input.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
            json!({"schema":"work-execution-recovery/v1","status":"recovered",
            "transaction":"record_finish","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":target.record_id,
            "attempt_path":format!("{execution}/{}/{}/attempt.json", input.task_id, input.attempt_id),
            "index_path":format!("{execution}/index.json"),"lock_status":"attempt_held"}),
        ))
    }

    pub fn recover_record_begin(
        &self,
        input: RecordBeginRecoveryInput<'_>,
    ) -> Result<Value, WorkError> {
        let execution = storage_path(&self.project_root, input.execution_dir)?;
        require_no_spec_update(&self.project_root, input.execution_dir, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{}/.work-state-writer.lock", input.execution_dir),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
            json!({"schema":"work-execution-recovery/v1","status":"recovered",
            "transaction":"record_begin","task_id":input.task_id,
            "attempt_id":input.attempt_id,"record_id":record_id,
            "lock_status":"record_reserved"}),
        ))
    }
}

impl CommandPrepareRepository for LocalExecutionStorage {
    fn check_ready(&self, execution_dir: &str, require_idle: bool) -> Result<(), WorkError> {
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        if require_idle {
            let lock = storage_path(
                &self.project_root,
                &format!("{execution_dir}/.work-state-writer.lock"),
            )?;
            LocalWriterLock.require_idle(&lock)?;
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
        let started = storage_path(&self.project_root, &format!("{prefix}.started.json"))?;
        let finished = storage_path(&self.project_root, &format!("{prefix}.finished.json"))?;
        Ok(started.exists() || finished.exists())
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
        let execution = publication.execution_dir;
        let task = publication.task_id;
        let attempt = publication.attempt_id;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
}

impl DeviationRecordRepository for LocalExecutionStorage {
    fn publish_deviation_record(
        &self,
        publication: &DeviationRecordPublication<'_>,
    ) -> Result<(), WorkError> {
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let writer_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&writer_path)?;
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
}

impl RecoveryPrepareRepository for LocalExecutionStorage {
    fn check_recovery_idle(&self, execution_dir: &str) -> Result<(), WorkError> {
        require_no_spec_update(&self.project_root, execution_dir, None)?;
        let writer = storage_path(
            &self.project_root,
            &format!("{execution_dir}/.work-state-writer.lock"),
        )?;
        LocalWriterLock.require_idle(&writer)
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
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
}

impl RecordBeginRepository for LocalExecutionStorage {
    fn publish_record_begin(
        &self,
        publication: &RecordBeginPublication<'_>,
    ) -> Result<(), WorkError> {
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
}

impl RecordFinishRepository for LocalExecutionStorage {
    fn publish_record_finish(
        &self,
        publication: &RecordFinishPublication<'_>,
    ) -> Result<(), WorkError> {
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
}

impl AttemptCloseRepository for LocalExecutionStorage {
    fn publish_attempt_close(
        &self,
        publication: &AttemptClosePublication<'_>,
    ) -> Result<(), WorkError> {
        let execution = publication.execution_dir;
        require_no_spec_update(&self.project_root, execution, None)?;
        let lock_path = storage_path(
            &self.project_root,
            &format!("{execution}/.work-state-writer.lock"),
        )?;
        let _writer = LocalWriterLock.acquire(&lock_path)?;
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
    use std::cell::Cell;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(target_os = "macos")]
    use crate::process::LocalCommandRunner;
    use serde_json::json;
    use work_feature::execution::{preflight_index, prepare_initial_index};
    use work_operations::canonical::canonical_json_sha256;
    use work_operations::execution::attempt::render_attempt;
    use work_operations::execution::attempt_close::build_close_candidates;
    use work_operations::execution::build_execution_lock;
    use work_operations::execution::command_run::{CommandPreviewInput, build_command_preview};
    use work_operations::execution::finish_attempt_candidate;
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
            request: &json!({"schema":"work-command-run-request/v1","timeout_seconds":60}),
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
        let prefix = "execution/.work-command-CMD-001";
        assert!(!storage.receipt_exists(prefix).unwrap());
        fs::write(
            root.join("execution/.work-command-CMD-001.started.json"),
            b"{}",
        )
        .unwrap();
        assert!(storage.receipt_exists(prefix).unwrap());
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
        let request = json!({"schema":"work-command-run-request/v1","timeout_seconds":60});
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
            let request = json!({"schema":"work-command-run-request/v1","timeout_seconds":60});
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
            work_operations::canonical::sha256_hex(b"#!/bin/sh\nexit 0\n")
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
            expected_snapshot: "0aeba5399596c359343ed836e0cf15a88e0f3643a5495992a39c07aae7cdfe4d",
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
            expected_snapshot: "0aeba5399596c359343ed836e0cf15a88e0f3643a5495992a39c07aae7cdfe4d",
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
            expected_snapshot: "0aeba5399596c359343ed836e0cf15a88e0f3643a5495992a39c07aae7cdfe4d",
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
            expected_snapshot: "0aeba5399596c359343ed836e0cf15a88e0f3643a5495992a39c07aae7cdfe4d",
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
            expected_snapshot: "0aeba5399596c359343ed836e0cf15a88e0f3643a5495992a39c07aae7cdfe4d",
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
        let task = json!({"commands":[],"operations":[],"validations":[{"id":"VAL-001"}]});
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
        let authorization = json!({"schema":"work-attempt-authorization/v1",
            "task_id":"TASK-001","commands":[],"validations":[{"id":"VAL-001"}],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"schema":"work-attempt/v1","attempt_id":"ATTEMPT-001",
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
            "tasks":[{"id":"TASK-001"}]});
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
        let task = json!({"commands":[],"operations":[],"validations":[{"id":"VAL-001"}]});
        let request = json!({"record":{"outcome":"passed","evidence":"Passed"}});
        let prepared =
            finish_attempt_candidate(&attempt, &request, "VAL-001", "validation", None).unwrap();
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

        let interrupted_index = root.join("execution/record-finish-interrupted-index.tmp");
        LocalFiles
            .create_new(&interrupted_index, &index_raw)
            .unwrap();
        LocalFiles.replace(&interrupted_index, &index_path).unwrap();
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

        let close_request = json!({"schema":"work-attempt-close-request/v1",
            "status":"completed"});
        let (closed, closed_index) = build_close_candidates(
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
        assert_eq!(recovered_attempt["schema"], "work-attempt/v1");
        assert_eq!(recovered_attempt["status"], "completed");
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
