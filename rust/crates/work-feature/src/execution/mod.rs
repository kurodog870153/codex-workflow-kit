//! Execution index creation and read-only preflight over a repository port.

pub mod command_publication;
pub mod document;
pub mod recovery;

use std::collections::{HashMap, HashSet};

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::instruction::{self, InstructionSourceRepository};
use crate::ports::SourceSnapshotReader;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::{self, TaskCollectionRepository};
use serde_json::{Value, json};
use work_operations::canonical::{JsonContractIssue, parse_json_contract};
use work_operations::derivation::fingerprint;
use work_operations::derivation::fingerprint::skill_selection as skill_selection_sha256;
use work_operations::execution::ExecutionIssue;
use work_operations::execution::attempt::{render_attempt, validate_attempt_bytes};
use work_operations::execution::attempt_close::build_close_candidates;
use work_operations::execution::attempt_prepare::{build_prepare_request, validate_prepare_choice};
use work_operations::execution::attempt_start::{
    build_attempt_candidate, validate_attempt_namespace,
};
use work_operations::execution::authorization::{authorization_evidence, effective_task};
use work_operations::execution::command_correction::{
    build_command_correction_candidate, validate_command_correction_request,
};
use work_operations::execution::command_run::{
    select_reserved_argv_command, validate_command_run_request,
};
use work_operations::execution::correction::{build_correction_candidates, render_correction};
use work_operations::execution::deviation::{
    append_approved_deviation, build_deviation_preview, formalize_semantic_action,
    validate_deviation_proposal, validate_semantic_deviation_request,
};
use work_operations::execution::index::{
    build_initial_execution_index, render_execution_index, validate_execution_index,
};
use work_operations::execution::preflight::{
    FileState, check_confirmed_inputs, check_file_lifecycle, check_input_sources,
    check_task_eligibility,
};
use work_operations::execution::record_finish::build_record_finish_candidates;
use work_operations::execution::recovery::validate_recovery_direction;
use work_operations::execution::requests::validate_correction_create_request;
use work_operations::execution::requests::{
    validate_attempt_close_request, validate_attempt_start_request, validate_record_finish_request,
};
use work_operations::execution::worktree::inspect_records;
use work_operations::execution::{
    build_execution_lock, deviation_reconciliation_target, formal_record_kind,
    record_begin_candidate, start_index, validate_authorization_scope, validate_completed_coverage,
    validate_deviation_action, validate_execution_identity, validate_preflight_identity,
};
pub fn validate_record_id(raw: &str) -> Result<&str, WorkError> {
    work_operations::derivation::identity::next_record_id(raw, &json!({})).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    Ok(raw)
}

pub trait ExecutionIndexRepository {
    fn read_index(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
    fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError>;
    fn check_execution_ready(&self, task_path: &str, execution_dir: &str) -> Result<(), WorkError>;
    fn check_execution_task_layout(
        &self,
        execution_dir: &str,
        task_id: &str,
    ) -> Result<(), WorkError>;
}

pub trait AttemptStartRepository {
    fn attempt_names(&self, execution_dir: &str, task_id: &str) -> Result<Vec<String>, WorkError>;
    fn read_attempt(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
    fn publish_attempt_start(
        &self,
        publication: &AttemptStartPublication<'_>,
    ) -> Result<(), WorkError>;
}

pub trait ExecutionWorktreeRepository {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
    fn git_status(&self) -> Result<Vec<Value>, WorkError>;
}

pub trait CommandPrepareRepository {
    fn check_ready(&self, execution_dir: &str, require_idle: bool) -> Result<(), WorkError>;
    fn runtime_os(&self) -> &'static str;
    fn read_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
    fn working_directory(&self, relative_path: &str) -> Result<String, WorkError>;
    fn resolve_invocation(&self, argv: &[String], cwd: &str) -> Result<Value, WorkError>;
    fn receipt_exists(&self, prefix: &str) -> Result<bool, WorkError>;
}

pub struct CommandPrepareContext<'a> {
    pub lifecycle: RecordBeginContext<'a>,
    pub task_path: &'a str,
    pub source_files: &'a HashMap<String, Vec<u8>>,
    pub execute_selection: &'a Value,
}

pub fn prepare_command_from_context(
    repository: &impl CommandPrepareRepository,
    context: CommandPrepareContext<'_>,
    request: &Value,
) -> Result<Value, WorkError> {
    prepare_command_context(repository, context, request, true)
}

fn prepare_command_context(
    repository: &impl CommandPrepareRepository,
    context: CommandPrepareContext<'_>,
    request: &Value,
    require_idle: bool,
) -> Result<Value, WorkError> {
    validate_command_run_request(request).map_err(rule)?;
    let CommandPrepareContext {
        lifecycle,
        task_path,
        source_files,
        execute_selection,
    } = context;
    let RecordBeginContext {
        collection,
        validation,
        task,
        attempt,
        attempt_raw,
        index,
        index_raw,
        execution_dir,
        task_id,
    } = lifecycle;
    repository.check_ready(execution_dir, require_idle)?;
    if collection["artifacts"]["task"] != task_path
        || collection["artifacts"]["execution"] != execution_dir
    {
        return Err(error(
            ExitCode::WorkflowState,
            "command_run_paths",
            "Explicit paths must match the formal TASK.",
            json!({}),
        ));
    }
    validate_execution_index(index, index_raw).map_err(rule)?;
    validate_attempt_bytes(attempt, attempt_raw).map_err(rule)?;
    validate_execution_identity(collection, validation, index, attempt, task_id).map_err(rule)?;
    if task["id"] != task_id
        || !collection["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|row| row["id"] == task_id))
    {
        return Err(error(
            ExitCode::Contract,
            "command_run_identity",
            "Unknown TASK ID.",
            json!({}),
        ));
    }
    if execute_selection["selected_paths"] != task["instruction_selection"]["selected_paths"]
        || execute_selection["resolved_paths"] != task["instruction_selection"]["resolved_paths"]
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_run_execute_instruction_hierarchy_mismatch",
            "The current Execute hierarchy does not match the target TASK.",
            json!({}),
        ));
    }
    if execute_selection["instructions_sha256"] != attempt["execute_instructions_sha256"] {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_run_execute_instructions_changed",
            "The Execute instruction fingerprint changed after Attempt start.",
            json!({"expected":attempt["execute_instructions_sha256"],
                "actual":execute_selection["instructions_sha256"]}),
        ));
    }
    let selected = select_reserved_argv_command(task, attempt, index, task_id).map_err(rule)?;
    let effective = effective_task(task, attempt).map_err(rule)?;
    let formal = effective["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|command| command["id"] == selected.base_record_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "command_correction_command_not_found",
                "The reserved command is not defined by the target TASK.",
                json!({}),
            )
        })?;
    let settings = formal
        .get("execution")
        .unwrap_or(&collection["execution_defaults"]);
    if settings["os"] != repository.runtime_os() {
        return Err(error(
            ExitCode::WorkflowState,
            "command_run_os",
            "The CMD execution OS differs from the current runtime.",
            json!({}),
        ));
    }
    let cwd_relative = settings["working_directory"].as_str().unwrap_or("");
    let cwd = repository.working_directory(cwd_relative)?;
    let argv: Vec<String> = selected.command["argv"]
        .as_array()
        .expect("validated argv")
        .iter()
        .map(|value| value.as_str().expect("validated argument").to_owned())
        .collect();
    let invocation = repository.resolve_invocation(&argv, &cwd)?;
    let receipt = {
        work_operations::derivation::publication::command_receipt_paths(
            execution_dir,
            task_id,
            &selected.attempt_id,
            &selected.record_id,
        )
        .map_err(|_| {
            error(
                ExitCode::ArtifactIntegrity,
                "command_run_receipt_invalid",
                "The command receipt identity is invalid.",
                json!({}),
            )
        })?
        .directory
    };
    if repository.receipt_exists(&receipt)? {
        return Err(error(
            ExitCode::WorkflowState,
            "command_run_already_started",
            "This record already has execution evidence; never run it again.",
            json!({"receipt":receipt}),
        ));
    }
    let mut observed = source_files.clone();
    for path in task::source::evidence_paths(collection)? {
        if !observed.contains_key(&path) {
            observed.insert(path.clone(), repository.read_source(&path)?);
        }
    }
    observed.insert(format!("{execution_dir}/index.json"), index_raw.to_vec());
    observed.insert(
        format!(
            "{execution_dir}/{task_id}/{}/attempt.json",
            selected.attempt_id
        ),
        attempt_raw.to_vec(),
    );
    let mut source_hashes = serde_json::Map::new();
    for (path, bytes) in &observed {
        if repository.read_source(path)? != *bytes {
            return Err(error(
                ExitCode::WorkflowState,
                "command_run_source_changed",
                "A command source changed during preparation.",
                json!({}),
            ));
        }
        source_hashes.insert(path.clone(), json!(fingerprint::raw(bytes)));
    }

    work_operations::execution::command_run::build_command_preview_with_receipts(
        work_operations::execution::command_run::CommandReceiptPreviewInput {
            request,
            execution_dir,
            task_id,
            attempt_id: &selected.attempt_id,
            record_id: &selected.record_id,
            working_directory: &cwd,
            execution: settings,
            invocation: &invocation,
            sources: &Value::Object(source_hashes),
        },
    )
    .map_err(rule)
}

pub fn prepare_command_run_from_context(
    repository: &impl CommandPrepareRepository,
    context: CommandPrepareContext<'_>,
    request: &Value,
) -> Result<(Value, String), WorkError> {
    let attempt = context.lifecycle.attempt;
    let lock = &context.lifecycle.index["lock"];
    let preview = prepare_command_context(repository, context, request, false)?;
    let base = preview["record_id"]
        .as_str()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("");
    let evidence = authorization_evidence(attempt, lock, base).map_err(rule)?;
    Ok((preview, evidence))
}

pub struct CommandProjectSources<'a, H, S, P, T> {
    pub instructions: &'a H,
    pub skills: &'a S,
    pub paths: &'a P,
    pub task_repository: &'a T,
    pub skill_roots: &'a [SkillRoot],
}

#[derive(Clone, Copy)]
pub struct CommandProjectRequest<'a> {
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub request: &'a Value,
}

#[derive(Clone, Copy)]
pub struct DeviationProjectRequest<'a> {
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub proposal: &'a Value,
}

#[derive(Clone, Copy)]
pub struct ExecutionProjectTarget<'a> {
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
}

/// Retains the verified TASK bytes and requirement identity for publication/readiness adapters.
pub struct ExecutionWriterContext {
    writer: crate::ports::RequirementWriterContext,
    task_path: String,
    execution_dir: String,
    task_id: String,
    task_context: task::ExecutionTaskContext,
}

impl ExecutionWriterContext {
    fn from_verified_task(
        canonical_root: std::path::PathBuf,
        target: ExecutionProjectTarget<'_>,
        context: task::ExecutionTaskContext,
    ) -> Result<Self, WorkError> {
        let requirement = context.index["requirement_id"].as_str().ok_or_else(|| {
            error(
                ExitCode::ArtifactIntegrity,
                "execution_writer_requirement_missing",
                "Verified TASK context requires a requirement identity.",
                json!({}),
            )
        })?;
        if context.index["artifacts"]["task"] != target.task_path
            || context.index["artifacts"]["execution"] != target.execution_dir
        {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "execute_preflight_artifact_path_mismatch",
                "The explicit TASK and execution paths do not match the formal artifacts.",
                json!({}),
            ));
        }
        if !canonical_root.is_absolute()
            || context.contract["requirement_id"] != requirement
            || context.collection["requirement_id"] != requirement
            || context.validation["requirement_id"] != requirement
            || context.contract["artifacts"] != context.index["artifacts"]
            || context.collection["artifacts"] != context.index["artifacts"]
            || !context.contract["tasks"]
                .as_array()
                .is_some_and(|tasks| tasks.iter().any(|task| task["id"] == target.task_id))
            || !work_operations::derivation::identity::runtime_relative_path(target.task_path)
            || !work_operations::derivation::identity::runtime_relative_path(target.execution_dir)
        {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "execution_writer_context_mismatch",
                "Publication paths and TASK must match the verified requirement context.",
                json!({}),
            ));
        }
        let requirement_id = requirement.parse().map_err(|_| {
            error(
                ExitCode::Contract,
                "invalid_requirement_id",
                "Verified TASK requirement ID is invalid.",
                json!({}),
            )
        })?;
        Ok(Self {
            writer: crate::ports::RequirementWriterContext {
                canonical_project_root: canonical_root,
                requirement_id,
            },
            task_path: target.task_path.into(),
            execution_dir: target.execution_dir.into(),
            task_id: target.task_id.into(),
            task_context: context,
        })
    }

    pub fn writer(&self) -> &crate::ports::RequirementWriterContext {
        &self.writer
    }
    pub fn target(&self) -> ExecutionProjectTarget<'_> {
        ExecutionProjectTarget {
            task_path: &self.task_path,
            execution_dir: &self.execution_dir,
            task_id: &self.task_id,
        }
    }
    pub fn task_context(&self) -> &task::ExecutionTaskContext {
        &self.task_context
    }

    pub fn check_execution_index(&self, index: &Value, raw: &[u8]) -> Result<(), WorkError> {
        validate_execution_index(index, raw).map_err(rule)?;
        validate_preflight_identity(
            index,
            &self.task_context.collection,
            &self.task_context.validation,
        )
        .map_err(rule)
    }
}

pub fn load_execution_writer_context<H, S, P, T>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    target: ExecutionProjectTarget<'_>,
) -> Result<ExecutionWriterContext, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
{
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    // Obtain the physical root from the trusted path port and the validated full TASK path.
    // No requirement identity or root is guessed from the execution directory's basename.
    let mut canonical_root = sources.paths.resolve(target.task_path)?;
    if !work_operations::derivation::identity::runtime_relative_path(target.task_path) {
        return Err(error(
            ExitCode::Contract,
            "execution_writer_task_path",
            "TASK path must be canonical and project-relative.",
            json!({}),
        ));
    }
    for _ in target.task_path.split('/') {
        if !canonical_root.pop() {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "execution_writer_root",
                "Resolved TASK path does not bind a project root.",
                json!({}),
            ));
        }
    }
    ExecutionWriterContext::from_verified_task(canonical_root, target, context)
}

/// Verified collection identity for read-only inventory queries, without execution file preflight.
pub struct ExecutionInventoryContext {
    writer: crate::ports::RequirementWriterContext,
    execution_dir: String,
}

pub struct ExecutionInventoryTarget<'a> {
    pub task_path: &'a str,
    pub execution_dir: &'a str,
}

impl ExecutionInventoryContext {
    pub fn writer(&self) -> &crate::ports::RequirementWriterContext {
        &self.writer
    }
    pub fn execution_dir(&self) -> &str {
        &self.execution_dir
    }
}

pub fn load_execution_inventory_context<H, S, P, T>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    target: ExecutionInventoryTarget<'_>,
) -> Result<ExecutionInventoryContext, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
{
    let validated = task::load_collection(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
    )?;
    let raw = sources.task_repository.read_task_file(target.task_path)?;
    if fingerprint::raw(&raw) != validated["task_index_sha256"] {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "task_execution_source_changed",
            "The verified TASK index changed during inventory loading.",
            json!({}),
        ));
    }
    let index = parse_json_contract(&raw).map_err(|_| {
        error(
            ExitCode::ArtifactIntegrity,
            "execution_writer_context_mismatch",
            "The verified TASK index is invalid.",
            json!({}),
        )
    })?;
    if index["requirement_id"] != validated["requirement_id"]
        || index["artifacts"]["task"] != target.task_path
        || index["artifacts"]["execution"] != target.execution_dir
        || !work_operations::derivation::identity::runtime_relative_path(target.task_path)
        || !work_operations::derivation::identity::runtime_relative_path(target.execution_dir)
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execution_writer_context_mismatch",
            "Inventory identity must match the verified TASK declarations.",
            json!({}),
        ));
    }
    let requirement_id = index["requirement_id"]
        .as_str()
        .unwrap_or("")
        .parse()
        .map_err(|_| {
            error(
                ExitCode::Contract,
                "invalid_requirement_id",
                "The verified requirement identity is invalid.",
                json!({}),
            )
        })?;
    let mut canonical_project_root = sources.paths.resolve(target.task_path)?;
    for _ in target.task_path.split('/') {
        if !canonical_project_root.pop() {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "execution_writer_root",
                "The verified TASK path does not bind a project root.",
                json!({}),
            ));
        }
    }
    if !canonical_project_root.is_absolute() {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execution_writer_root",
            "The verified project root must be absolute.",
            json!({}),
        ));
    }
    Ok(ExecutionInventoryContext {
        writer: crate::ports::RequirementWriterContext {
            canonical_project_root,
            requirement_id,
        },
        execution_dir: target.execution_dir.into(),
    })
}

pub trait RuntimeExecutionReadiness {
    fn check_command_context(
        &self,
        context: &ExecutionWriterContext,
        require_idle: bool,
    ) -> Result<(), WorkError>;
    fn check_recovery_context(&self, context: &ExecutionWriterContext) -> Result<(), WorkError>;
}

pub struct ScopedExecutionPublication<'a, P> {
    pub context: &'a ExecutionWriterContext,
    pub publication: &'a P,
}

pub struct CorrectionPublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub correction_id: &'a str,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
    pub artifact: &'a [u8],
    pub locked_index: &'a [u8],
    pub final_index: &'a [u8],
}

pub trait CorrectionRepository {
    fn correction_names(
        &self,
        execution_dir: &str,
        task_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<String>, WorkError>;
    fn publish_correction(&self, publication: &CorrectionPublication<'_>) -> Result<(), WorkError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingRecoverySnapshot {
    pub transaction_dir: String,
    pub manifest: work_model::runtime::RuntimeManifest,
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
}

pub trait RecoveryPrepareRepository {
    fn staging_transactions(
        &self,
        _context: &ExecutionWriterContext,
    ) -> Result<Vec<StagingRecoverySnapshot>, WorkError> {
        Err(error(
            ExitCode::ArtifactIntegrity,
            "execution_staging_repository_required",
            "Recovery requires the complete verified staging reader.",
            json!({}),
        ))
    }
    fn check_recovery_idle(&self, execution_dir: &str) -> Result<(), WorkError>;
    fn temporary_names(&self, execution_dir: &str) -> Result<Vec<String>, WorkError>;
    fn recovery_source_exists(&self, relative_path: &str) -> Result<bool, WorkError>;
    fn read_recovery_source(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
}

pub fn prepare_recovery_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: RecoveryPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    prepare_staging_recovery_from_project(sources, repository, target, request)
}

pub fn prepare_legacy_recovery_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: RecoveryPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    work_operations::execution::requests::validate_legacy_recovery_request(request, true)
        .map_err(rule)?;
    repository.check_recovery_idle(target.execution_dir)?;
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    if context.contract["artifacts"]["task"] != target.task_path
        || context.contract["artifacts"]["execution"] != target.execution_dir
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "recovery_prepare_artifact_paths",
            "Explicit paths do not match the formal TASK.",
            json!({}),
        ));
    }
    let mut observed = context.sources;
    for path in task::source::evidence_paths(&context.contract)? {
        observed.insert(path.clone(), repository.read_recovery_source(&path)?);
    }
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    observed.insert(index_path.clone(), index_raw);
    let attempt_id = request["attempt_id"].as_str().unwrap_or("");
    let attempt_path = format!(
        "{}/{}/{attempt_id}/attempt.json",
        target.execution_dir, target.task_id
    );
    let attempt_raw = repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    observed.insert(attempt_path.clone(), attempt_raw);
    if attempt["attempt_id"] != attempt_id {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "recovery_prepare_attempt_id",
            "The Attempt identity differs from the requested path.",
            json!({}),
        ));
    }
    validate_execution_identity(
        &context.collection,
        &context.validation,
        &index,
        &attempt,
        target.task_id,
    )
    .map_err(rule)?;
    let files = repository.temporary_names(target.execution_dir)?;
    let transaction = request["transaction"].as_str().unwrap_or("");
    let direction = validate_recovery_direction(
        transaction,
        target.task_id,
        attempt_id,
        &files,
        &index,
        &attempt,
    )
    .map_err(rule)?;
    if transaction == "correction" {
        let correction_id = direction["correction_id"].as_str().unwrap_or("");
        let path = format!(
            "{}/{}/{attempt_id}/corrections/{correction_id}.json",
            target.execution_dir, target.task_id
        );
        if repository.recovery_source_exists(&path)? {
            observed.insert(path.clone(), repository.read_recovery_source(&path)?);
        }
    }
    for file in &files {
        let path = format!("{}/{}", target.execution_dir, file);
        let raw = repository.read_recovery_source(&path)?;
        let document = parse_execution_document(&raw, &path)?;
        if file.ends_with("-attempt.tmp") {
            validate_attempt_bytes(&document, &raw).map_err(rule)?;
        } else if transaction == "correction" && file.ends_with("-artifact.tmp") {
            if render_correction(&document).map_err(rule)? != raw {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "recovery_prepare_noncanonical_correction",
                    "The preserved Correction is not canonical.",
                    json!({}),
                ));
            }
        } else {
            validate_execution_index(&document, &raw).map_err(rule)?;
        }
        observed.insert(path, raw);
    }
    let mut evidence = serde_json::Map::new();
    for (path, raw) in &observed {
        if repository.read_recovery_source(path)? != *raw {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "recovery_prepare_source_changed",
                "Recovery evidence changed during preparation.",
                json!({"path":path}),
            ));
        }
        evidence.insert(
            path.clone(),
            serde_json::to_value(work_model::execution::recovery::ExecutionRecoveryEvidence {
                raw_sha256: fingerprint::raw(raw),
                size_bytes: raw.len() as u64,
            })
            .expect("recovery evidence serializes"),
        );
    }
    if files != repository.temporary_names(target.execution_dir)? {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "recovery_prepare_source_changed",
            "Recovery evidence changed during preparation.",
            json!({}),
        ));
    }
    repository.check_recovery_idle(target.execution_dir)?;
    let recovery_request = json!({"schema":"work-execution-recovery-request",
        "transaction":transaction,"attempt_id":attempt_id,"transaction_files":files});
    work_operations::execution::requests::validate_legacy_recovery_request(
        &recovery_request,
        false,
    )
    .map_err(rule)?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::LegacyExecutionRecoveryPrepare,
    >(
        json!({"schema":"work-execution-recovery-prepare","status":"prepared",
        "request":recovery_request,"task_id":target.task_id,
        "attempt_path":attempt_path,"index_path":index_path,"lock":index["lock"],
        "attempt_status":attempt["status"],"evidence":evidence,
        "recovery_validation":"requires_authorized_recover","recovery_authorized":false}),
    ))
}

/// Candidate uses a trusted context and complete transaction snapshots, with no root-file fallback.
pub fn prepare_staging_recovery_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: RecoveryPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    work_operations::execution::requests::validate_staging_recovery_request(request, true)
        .map_err(rule)?;
    repository.check_recovery_idle(target.execution_dir)?;
    let context = load_execution_writer_context(sources, target)?;
    let transactions = repository.staging_transactions(&context)?;
    if transactions.len() != 1 {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "recovery_prepare_mixed_transactions",
            "Recovery requires exactly one complete transaction; foreign evidence must not be omitted.",
            json!({}),
        ));
    }
    let snapshot = &transactions[0];
    let manifest = &snapshot.manifest;
    let binding = work_operations::execution::recovery::execution_staging_binding(
        manifest,
        context
            .writer()
            .canonical_project_root
            .to_str()
            .ok_or_else(|| {
                error(
                    ExitCode::ArtifactIntegrity,
                    "execution_writer_root",
                    "The canonical root must have a portable representation.",
                    json!({}),
                )
            })?,
        &context.writer().requirement_id,
        target.execution_dir,
        target.task_id,
        &snapshot.files,
    )
    .map_err(rule)?;
    if binding.transaction_dir != snapshot.transaction_dir
        || manifest.operation
            != request["transaction"]
                .as_str()
                .unwrap_or("")
                .replace('_', "-")
        || manifest.business_identity["attempt_id"] != request["attempt_id"]
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_transaction_changed",
            "The requested operation and Attempt differ from preserved evidence.",
            json!({}),
        ));
    }
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    context.check_execution_index(&index, &index_raw)?;
    let attempt_id = request["attempt_id"].as_str().unwrap_or("");
    let attempt_path = format!(
        "{}/{}/{attempt_id}/attempt.json",
        target.execution_dir, target.task_id
    );
    let attempt_raw = repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    validate_execution_identity(
        &context.task_context().collection,
        &context.task_context().validation,
        &index,
        &attempt,
        target.task_id,
    )
    .map_err(rule)?;
    if attempt["attempt_id"] != attempt_id {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "recovery_prepare_attempt_id",
            "The current Attempt differs from the requested identity.",
            json!({}),
        ));
    }
    // Domain recovery validates transitions before writing; preparation also verifies every frozen artifact shape.
    for artifact in &manifest.targets {
        for bytes in artifact.before.iter().chain(artifact.after.iter()) {
            let document = parse_execution_document(&bytes.bytes, &artifact.path)?;
            if artifact.path == index_path {
                context.check_execution_index(&document, &bytes.bytes)?;
            } else if artifact.path == attempt_path {
                validate_attempt_bytes(&document, &bytes.bytes).map_err(rule)?;
                if document["attempt_id"] != attempt_id || document["task_id"] != target.task_id {
                    return Err(error(
                        ExitCode::ArtifactIntegrity,
                        "execution_recovery_transaction_changed",
                        "Frozen Attempt evidence belongs to another identity.",
                        json!({}),
                    ));
                }
            } else if artifact.path.starts_with(&format!(
                "{}/{}/{attempt_id}/corrections/",
                target.execution_dir, target.task_id
            )) {
                if render_correction(&document).map_err(rule)? != bytes.bytes {
                    return Err(error(
                        ExitCode::ArtifactIntegrity,
                        "recovery_prepare_noncanonical_correction",
                        "Preserved Correction evidence is not canonical.",
                        json!({}),
                    ));
                }
            } else {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "execution_recovery_transaction_changed",
                    "Unexpected formal targets must not be guessed during recovery.",
                    json!({}),
                ));
            }
        }
    }
    let mut observed = context.task_context().sources.clone();
    for path in task::source::evidence_paths(&context.task_context().contract)? {
        observed.insert(path.clone(), repository.read_recovery_source(&path)?);
    }
    observed.insert(index_path.clone(), index_raw);
    observed.insert(attempt_path.clone(), attempt_raw);
    for (name, raw) in &snapshot.files {
        observed.insert(format!("{}/{name}", binding.transaction_dir), raw.clone());
    }
    for artifact in &manifest.targets {
        if repository.recovery_source_exists(&artifact.path)? {
            observed.insert(
                artifact.path.clone(),
                repository.read_recovery_source(&artifact.path)?,
            );
        } else if artifact.before.is_some() {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "recovery_prepare_source_changed",
                "A preserved formal source disappeared.",
                json!({"path":artifact.path}),
            ));
        }
    }
    let mut evidence = serde_json::Map::new();
    for (path, raw) in &observed {
        if repository.read_recovery_source(path)? != *raw {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "recovery_prepare_source_changed",
                "Recovery evidence changed during preparation.",
                json!({"path":path}),
            ));
        }
        evidence.insert(
            path.clone(),
            json!({"raw_sha256":fingerprint::raw(raw),"size_bytes":raw.len() as u64}),
        );
    }
    let fresh = repository.staging_transactions(&context)?;
    if fresh != transactions {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "recovery_prepare_source_changed",
            "The complete transaction inventory changed during preparation.",
            json!({}),
        ));
    }
    repository.check_recovery_idle(target.execution_dir)?;
    let recovery_request = json!({"schema":"work-execution-recovery-request","transaction":request["transaction"],
        "attempt_id":attempt_id,"transaction_dir":binding.transaction_dir,"transaction_files":binding.transaction_files,
        "transaction_evidence_sha256":work_operations::execution::recovery::execution_staging_evidence_sha256(&binding).map_err(rule)?});
    recovery::require_staging_recovery_binding(&recovery_request, manifest, &binding)?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::PreparedExecutionRecoveryPrepare,
    >(
        json!({"schema":"work-execution-recovery-prepare","status":"prepared","request":recovery_request,
            "task_id":target.task_id,"attempt_path":attempt_path,"index_path":index_path,"lock":index["lock"],
            "attempt_status":attempt["status"],"evidence":evidence,
            "recovery_validation":"requires_authorized_recover","recovery_authorized":false}),
    ))
}

pub fn create_correction_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
    created_at: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + AttemptStartRepository + CorrectionRepository,
{
    validate_correction_create_request(request).map_err(rule)?;
    let validation = task::load_collection(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
    )?;
    let collection = &validation["collection_contract"];
    if collection["artifacts"]["task"] != target.task_path
        || collection["artifacts"]["execution"] != target.execution_dir
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "correction_create_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
            json!({}),
        ));
    }
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    if index.get("lock").is_some() {
        return Err(error(
            ExitCode::LockConflict,
            "correction_create_lock_present",
            "A Correction requires an unlocked execution index.",
            json!({"lock":index["lock"]}),
        ));
    }
    let attempt_id = request["target_attempt_id"].as_str().unwrap_or("");
    let attempt_path = format!(
        "{}/{}/{attempt_id}/attempt.json",
        target.execution_dir, target.task_id
    );
    let attempt_raw = repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    validate_execution_identity(collection, &validation, &index, &attempt, target.task_id)
        .map_err(rule)?;
    let names = repository.correction_names(target.execution_dir, target.task_id, attempt_id)?;
    let correction_id = work_operations::derivation::identity::next_correction_id(
        attempt_id, &names,
    )
    .map_err(|issue| match issue {
        work_operations::derivation::identity::CorrectionIdentityIssue::InvalidExistingName(
            name,
        ) => error(
            ExitCode::ArtifactIntegrity,
            "correction_create_invalid_existing_name",
            "An existing Correction-like filename is not canonical.",
            json!({"name":name}),
        ),
        work_operations::derivation::identity::CorrectionIdentityIssue::Exhausted => error(
            ExitCode::WorkflowState,
            "correction_create_id_exhausted",
            "The Correction ID range is exhausted for this Attempt.",
            json!({}),
        ),
    })?;
    let candidates = build_correction_candidates(
        collection,
        &index,
        &attempt,
        target.task_id,
        request,
        &correction_id,
        created_at,
    )
    .map_err(rule)?;
    let artifact = render_correction(&candidates.artifact).map_err(rule)?;
    let locked_index = render_execution_index(&candidates.locked_index).expect("JSON index");
    let final_index = render_execution_index(&candidates.final_index).expect("JSON index");
    repository.publish_correction(&CorrectionPublication {
        execution_dir: target.execution_dir,
        task_id: target.task_id,
        attempt_id,
        correction_id: &correction_id,
        index_before: &index_raw,
        attempt_before: &attempt_raw,
        artifact: &artifact,
        locked_index: &locked_index,
        final_index: &final_index,
    })?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::CorrectionCreateResponse,
    >(
        json!({"schema":"work-correction-create","task_id":target.task_id,
        "attempt_id":attempt_id,"correction_id":correction_id,
        "correction_path":format!("{}/{}/{attempt_id}/corrections/{correction_id}.json",
            target.execution_dir, target.task_id),
        "index_path":index_path,"affected_task_ids":candidates.affected_task_ids,
        "lock_status":"released"}),
    ))
}

pub fn prepare_execute_preflight_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    confirmed_inputs: &[String],
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository,
{
    repository.check_execution_ready(target.task_path, target.execution_dir)?;
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    preflight_from_loaded_context(sources, repository, target, confirmed_inputs, &context)
}

fn preflight_from_loaded_context<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    confirmed_inputs: &[String],
    context: &task::ExecutionTaskContext,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository,
{
    task::recheck_task_execution_context(sources.paths, sources.task_repository, context)?;
    let mut validation = context.validation.clone();
    validation["collection_contract"] = context.collection.clone();
    let result = prepare_execute_preflight(
        repository,
        sources.instructions,
        ExecutePreflightRequest {
            validation: &validation,

            task_path: target.task_path,
            execution_dir: target.execution_dir,
            task_id: target.task_id,
            confirmed_inputs,
        },
    )?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, context)?;
    Ok(result)
}

pub fn inspect_worktree_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    confirmed_inputs: &[String],
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + ExecutionWorktreeRepository,
{
    repository.check_execution_ready(target.task_path, target.execution_dir)?;
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    let preflight =
        preflight_from_loaded_context(sources, repository, target, confirmed_inputs, &context)?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    let result = inspect_worktree(
        repository,
        &preflight,
        &context.validation,
        &context.contract,
    )?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    Ok(result)
}

pub fn prepare_attempt_start_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    confirmed_inputs: &[String],
    choice: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + ExecutionWorktreeRepository + AttemptStartRepository,
{
    validate_prepare_choice(choice).map_err(rule)?;
    repository.check_execution_ready(target.task_path, target.execution_dir)?;
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    let preflight =
        preflight_from_loaded_context(sources, repository, target, confirmed_inputs, &context)?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    let worktree = inspect_worktree(
        repository,
        &preflight,
        &context.validation,
        &context.contract,
    )?;
    let task = context.contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["id"] == target.task_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "attempt_start_task_not_found",
                "The target TASK is missing.",
                json!({}),
            )
        })?;
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == target.task_id);
    let source_path = row
        .filter(|row| row["status"] == "pending_retry")
        .and_then(|row| row["latest_attempt"].as_str())
        .map(|latest| {
            format!(
                "{}/{}/{latest}/attempt.json",
                target.execution_dir, target.task_id
            )
        });
    let source_raw = source_path
        .as_ref()
        .map(|path| repository.read_attempt(path))
        .transpose()?;
    let source_attempt = source_raw
        .as_ref()
        .zip(source_path.as_ref())
        .map(|(raw, path)| {
            let value = parse_execution_document(raw, path)?;
            validate_attempt_bytes(&value, raw).map_err(rule)?;
            if source_path.as_ref().is_some_and(|path| {
                !path.ends_with(&format!(
                    "/{}/attempt.json",
                    value["attempt_id"].as_str().unwrap_or("")
                ))
            }) {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "attempt_start_source_attempt_mismatch",
                    "The retry source Attempt ID differs from its formal path.",
                    json!({}),
                ));
            }
            Ok::<_, WorkError>(value)
        })
        .transpose()?;
    let prepared = build_prepare_request(
        choice,
        task,
        &context.contract["execution_defaults"],
        &worktree,
        &index,
        source_attempt.as_ref(),
    )
    .map_err(rule)?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    repository.check_execution_ready(target.task_path, target.execution_dir)?;
    let current_preflight =
        preflight_from_loaded_context(sources, repository, target, confirmed_inputs, &context)?;
    let current = inspect_worktree(
        repository,
        &current_preflight,
        &context.validation,
        &context.contract,
    )?;
    let source_changed = if let Some((path, raw)) = source_path.as_ref().zip(source_raw.as_ref()) {
        repository.read_attempt(path)? != *raw
    } else {
        false
    };
    if current != worktree || repository.read_index(&index_path)? != index_raw || source_changed {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "attempt_start_prepare_source_changed",
            "Attempt sources changed during preparation.",
            json!({}),
        ));
    }
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    Ok(prepared)
}

pub fn start_attempt_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    confirmed_inputs: &[String],
    request: &Value,
    started_at: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + ExecutionWorktreeRepository + AttemptStartRepository,
{
    validate_attempt_start_request(request).map_err(rule)?;
    let mut preflight =
        prepare_execute_preflight_from_project(sources, repository, target, confirmed_inputs)?;
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    let worktree = inspect_worktree(
        repository,
        &preflight,
        &context.validation,
        &context.contract,
    )?;
    let formal = context.contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["id"] == target.task_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "attempt_start_task_not_found",
                "The target TASK is not present in the formal collection.",
                json!({}),
            )
        })?;
    validate_authorization_scope(
        &request["authorization"],
        formal,
        &context.contract["execution_defaults"],
    )
    .map_err(rule)?;
    preflight["snapshot_sha256"] = worktree["snapshot_sha256"].clone();
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    task::recheck_task_execution_context(sources.paths, sources.task_repository, &context)?;
    start_attempt_from_preflight(
        repository, &preflight, &index, &index_raw, request, started_at,
    )
}

pub fn begin_record_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    base_record_id: &str,
    authorization_evidence: Option<&str>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + AttemptStartRepository + RecordBeginRepository,
{
    with_lifecycle_project_context(sources, repository, target, |context, selection| {
        require_current_execute_instructions(
            context.task,
            context.attempt,
            selection,
            "record_begin",
        )?;
        begin_record_from_context(repository, context, base_record_id, authorization_evidence)
    })
}

pub fn finish_record_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + AttemptStartRepository + RecordFinishRepository,
{
    with_lifecycle_project_context(sources, repository, target, |context, selection| {
        require_current_execute_instructions(
            context.task,
            context.attempt,
            selection,
            "record_finish",
        )?;
        finish_record_from_context(repository, context, request)
    })
}

pub fn record_command_correction_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + AttemptStartRepository + CommandCorrectionRepository,
{
    validate_command_correction_request(request).map_err(rule)?;
    with_lifecycle_project_context(sources, repository, target, |context, selection| {
        record_command_correction_from_context(
            repository,
            context,
            target.task_path,
            request,
            selection,
        )
    })
}

pub fn close_attempt_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    request: &Value,
    ended_at: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + AttemptStartRepository + AttemptCloseRepository,
{
    with_lifecycle_project_context(sources, repository, target, |context, selection| {
        close_attempt_from_context(repository, context, request, ended_at, selection)
    })
}

fn require_current_execute_instructions(
    task: &Value,
    attempt: &Value,
    selection: &Value,
    operation: &str,
) -> Result<(), WorkError> {
    if selection["selected_paths"] != task["instruction_selection"]["selected_paths"]
        || selection["resolved_paths"] != task["instruction_selection"]["resolved_paths"]
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            format!("{operation}_execute_instruction_hierarchy_mismatch"),
            "The current Execute hierarchy does not match the target TASK.",
            json!({}),
        ));
    }
    if selection["instructions_sha256"] != attempt["execute_instructions_sha256"] {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            format!("{operation}_execute_instructions_changed"),
            "The Execute instruction fingerprint changed after Attempt start.",
            json!({"expected":attempt["execute_instructions_sha256"],
                "actual":selection["instructions_sha256"]}),
        ));
    }
    Ok(())
}

fn execute_references(attempt: &Value) -> Vec<String> {
    let mut references = vec!["execute.general.execution-records".to_owned()];
    if attempt.get("continued_from").is_some() {
        references.push("execute.general.execution-recovery".to_owned());
    }
    references
}

fn require_current_correction_instructions(
    task: &Value,
    attempt: &Value,
    selection: &Value,
) -> Result<(), WorkError> {
    if selection["selected_paths"] != task["instruction_selection"]["selected_paths"]
        || selection["resolved_paths"] != task["instruction_selection"]["resolved_paths"]
        || selection["instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_correction_execute_instructions_changed",
            "The current Execute instructions differ from the active Attempt.",
            json!({}),
        ));
    }
    Ok(())
}

fn with_lifecycle_project_context<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    action: impl FnOnce(RecordBeginContext<'_>, &Value) -> Result<Value, WorkError>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository + AttemptStartRepository,
{
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    if context.contract["artifacts"]["task"] != target.task_path
        || context.contract["artifacts"]["execution"] != target.execution_dir
    {
        return Err(error(
            ExitCode::WorkflowState,
            "execution_paths",
            "Explicit paths must match the formal TASK.",
            json!({}),
        ));
    }
    repository.check_execution_task_layout(target.execution_dir, target.task_id)?;
    let task = context.contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["id"] == target.task_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "execution_task_identity",
                "Unknown TASK ID.",
                json!({}),
            )
        })?;
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    let attempt_id = index["lock"]["attempt_id"].as_str().ok_or_else(|| {
        error(
            ExitCode::WorkflowState,
            "execution_lock_required",
            "An active execution lock is required.",
            json!({}),
        )
    })?;
    let attempt_path = format!(
        "{}/{}/{attempt_id}/attempt.json",
        target.execution_dir, target.task_id
    );
    let attempt_raw = repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    let selected_paths = task["instruction_selection"]["selected_paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let references = execute_references(&attempt);
    let selection = json!(instruction::select(
        sources.instructions,
        "execute",
        &selected_paths,
        &references
    )?);
    action(
        RecordBeginContext {
            collection: &context.collection,
            validation: &context.validation,
            task,
            attempt: &attempt,
            attempt_raw: &attempt_raw,
            index: &index,
            index_raw: &index_raw,
            execution_dir: target.execution_dir,
            task_id: target.task_id,
        },
        &selection,
    )
}

pub fn prepare_deviation_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    input: DeviationProjectRequest<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    with_deviation_project_context(sources, repository, input, |context| {
        prepare_deviation_from_context(repository, context, input.proposal)
    })
}

pub fn prepare_semantic_deviation_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    target: ExecutionProjectTarget<'_>,
    semantic: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    let required = [
        "gap",
        "action",
        "modifiable_files",
        "impact",
        "side_effects",
    ];
    if semantic.as_object().is_none_or(|object| {
        object.len() != required.len() || required.iter().any(|field| !object.contains_key(*field))
    }) {
        return Err(error(
            ExitCode::Contract,
            "invalid_contract_value",
            "The semantic deviation request has missing or unknown fields.",
            json!({}),
        ));
    }
    validate_semantic_deviation_request(semantic).map_err(rule)?;
    repository.check_ready(target.execution_dir, true)?;
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        target.task_path,
        target.task_id,
    )?;
    if context.contract["artifacts"]["task"] != target.task_path
        || context.contract["artifacts"]["execution"] != target.execution_dir
    {
        return Err(error(
            ExitCode::WorkflowState,
            "deviation_paths",
            "Explicit paths must match the formal TASK.",
            json!({}),
        ));
    }
    let index_path = format!("{}/index.json", target.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    let lock = &index["lock"];
    let anchor = lock["record_id"].as_str().ok_or_else(|| {
        error(
            ExitCode::WorkflowState,
            "deviation_active_record",
            "A matching reserved execution record is required.",
            json!({}),
        )
    })?;
    let attempt_id = lock["attempt_id"].as_str().unwrap_or("");
    if lock["kind"] != "execution" || lock["task_id"] != target.task_id {
        return Err(error(
            ExitCode::WorkflowState,
            "deviation_active_record",
            "A matching reserved execution record is required.",
            json!({}),
        ));
    }
    let attempt_path = format!(
        "{}/{}/{attempt_id}/attempt.json",
        target.execution_dir, target.task_id
    );
    let attempt_raw = repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    let row = validate_execution_identity(
        &context.contract,
        &context.validation,
        &index,
        &attempt,
        target.task_id,
    )
    .map_err(rule)?;
    if attempt["status"] != "in_progress"
        || row["status"] != "in_progress"
        || row["latest_attempt"] != attempt_id
    {
        return Err(error(
            ExitCode::WorkflowState,
            "deviation_active_record",
            "The reserved record must belong to the current in-progress Attempt.",
            json!({}),
        ));
    }
    let task = context.contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["id"] == target.task_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "deviation_task_identity",
                "Unknown TASK ID.",
                json!({}),
            )
        })?;
    let base = anchor.split('#').next().unwrap_or(anchor);
    let mut basis = vec![base.to_owned()];
    for step in task["steps"].as_array().into_iter().flatten() {
        if step["id"] != base
            && step["references"]
                .as_array()
                .is_some_and(|references| references.iter().any(|reference| reference == base))
        {
            if let Some(id) = step["id"].as_str() {
                basis.push(id.to_owned());
            }
        }
    }
    let action =
        formalize_semantic_action(&semantic["action"], task, &attempt, anchor).map_err(rule)?;
    let proposal = work_model::execution::response::verified::<
        work_model::execution::deviation::DeviationProposal,
    >(json!({"schema":"work-execution-deviation-proposal",
        "task_id":target.task_id,"attempt_id":attempt_id,"anchor_record_id":anchor,
        "task_basis":basis,"gap":semantic["gap"],"action":action,
        "modifiable_files":semantic["modifiable_files"],"impact":semantic["impact"],
        "side_effects":semantic["side_effects"]}));
    prepare_deviation_from_project(
        sources,
        repository,
        DeviationProjectRequest {
            task_path: target.task_path,
            execution_dir: target.execution_dir,
            task_id: target.task_id,
            proposal: &proposal,
        },
    )
}

pub fn record_deviation_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    input: DeviationProjectRequest<'_>,
    approved_sha256: &str,
    evidence: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository
        + ExecutionIndexRepository
        + AttemptStartRepository
        + DeviationRecordRepository,
{
    with_deviation_project_context(sources, repository, input, |context| {
        record_deviation_from_context(
            repository,
            context,
            input.proposal,
            approved_sha256,
            evidence,
        )
    })
}

fn with_deviation_project_context<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    input: DeviationProjectRequest<'_>,
    action: impl FnOnce(DeviationContext<'_>) -> Result<Value, WorkError>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    let context = task::load_task_execution_context(
        sources.instructions,
        sources.skills,
        sources.paths,
        sources.task_repository,
        sources.skill_roots,
        input.task_path,
        input.task_id,
    )?;
    let task = context.contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["id"] == input.task_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "deviation_task_identity",
                "Unknown TASK ID.",
                json!({}),
            )
        })?;
    let index_path = format!("{}/index.json", input.execution_dir);
    let index_raw = repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    let attempt_id = input.proposal["attempt_id"].as_str().unwrap_or("");
    let attempt_path = format!(
        "{}/{}/{attempt_id}/attempt.json",
        input.execution_dir, input.task_id
    );
    let attempt_raw = repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    let selected_paths = task["instruction_selection"]["selected_paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let references = execute_references(&attempt);
    let execute_selection = json!(instruction::select(
        sources.instructions,
        "execute",
        &selected_paths,
        &references,
    )?);
    let mut source_files = context.sources.clone();
    for path in task::source::evidence_paths(&context.contract)? {
        source_files.insert(path.clone(), repository.read_source(&path)?);
    }
    source_files.insert(index_path, index_raw.clone());
    source_files.insert(attempt_path, attempt_raw.clone());
    let lifecycle = RecordBeginContext {
        collection: &context.collection,
        validation: &context.validation,
        task,
        attempt: &attempt,
        attempt_raw: &attempt_raw,
        index: &index,
        index_raw: &index_raw,
        execution_dir: input.execution_dir,
        task_id: input.task_id,
    };
    action(DeviationContext {
        lifecycle,
        task_path: input.task_path,
        source_files: &source_files,
        execute_selection: &execute_selection,
    })
}

pub fn prepare_command_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    execution_repository: &E,
    input: CommandProjectRequest<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    prepare_command_project(sources, execution_repository, input, true).map(|(preview, _)| preview)
}

pub fn recheck_command_from_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    execution_repository: &E,
    input: CommandProjectRequest<'_>,
) -> Result<(Value, String), WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    prepare_command_project(sources, execution_repository, input, false)
}

fn prepare_command_project<H, S, P, T, E>(
    sources: &CommandProjectSources<'_, H, S, P, T>,
    execution_repository: &E,
    input: CommandProjectRequest<'_>,
    require_idle: bool,
) -> Result<(Value, String), WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCollectionRepository,
    E: CommandPrepareRepository + ExecutionIndexRepository + AttemptStartRepository,
{
    let CommandProjectSources {
        instructions,
        skills,
        paths,
        task_repository,
        skill_roots,
    } = sources;
    let CommandProjectRequest {
        task_path,
        execution_dir,
        task_id,
        request,
    } = input;
    validate_command_run_request(request).map_err(rule)?;
    execution_repository.check_ready(execution_dir, require_idle)?;
    let context = task::load_task_execution_context(
        *instructions,
        *skills,
        *paths,
        *task_repository,
        skill_roots,
        task_path,
        task_id,
    )?;
    if context.contract["artifacts"]["task"] != task_path
        || context.contract["artifacts"]["execution"] != execution_dir
    {
        return Err(error(
            ExitCode::WorkflowState,
            "command_run_paths",
            "Explicit paths must match the formal TASK.",
            json!({}),
        ));
    }
    let task = context.contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "command_run_identity",
                "Unknown TASK ID.",
                json!({}),
            )
        })?;
    let index_path = format!("{execution_dir}/index.json");
    let index_raw = execution_repository.read_index(&index_path)?;
    let index = parse_execution_document(&index_raw, &index_path)?;
    validate_execution_index(&index, &index_raw).map_err(rule)?;
    let attempt_id = index["lock"]["attempt_id"].as_str().ok_or_else(|| {
        error(
            ExitCode::WorkflowState,
            "command_run_lock",
            "The active execution lock must reserve a CMD record.",
            json!({}),
        )
    })?;
    let attempt_path = format!("{execution_dir}/{task_id}/{attempt_id}/attempt.json");
    let attempt_raw = execution_repository.read_attempt(&attempt_path)?;
    let attempt = parse_execution_document(&attempt_raw, &attempt_path)?;
    validate_attempt_bytes(&attempt, &attempt_raw).map_err(rule)?;
    let selected_paths: Vec<String> = task["instruction_selection"]["selected_paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let references = execute_references(&attempt);
    let execute_selection = json!(instruction::select(
        *instructions,
        "execute",
        &selected_paths,
        &references
    )?);
    let lifecycle = RecordBeginContext {
        collection: &context.collection,
        validation: &context.validation,
        task,
        attempt: &attempt,
        attempt_raw: &attempt_raw,
        index: &index,
        index_raw: &index_raw,
        execution_dir,
        task_id,
    };
    let command_context = CommandPrepareContext {
        lifecycle,
        task_path,
        source_files: &context.sources,
        execute_selection: &execute_selection,
    };
    if require_idle {
        let preview = prepare_command_from_context(execution_repository, command_context, request)?;
        let base = preview["record_id"]
            .as_str()
            .unwrap_or("")
            .split('#')
            .next()
            .unwrap_or("");
        let evidence = authorization_evidence(&attempt, &index["lock"], base).map_err(rule)?;
        Ok((preview, evidence))
    } else {
        prepare_command_run_from_context(execution_repository, command_context, request)
    }
}

pub struct RecordBeginPublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
    pub index_after: &'a [u8],
}

pub struct CommandCorrectionPublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub attempt_before: &'a [u8],
    pub index_before: &'a [u8],
    pub index_after: &'a [u8],
}

pub trait CommandCorrectionRepository {
    fn publish_command_correction(
        &self,
        publication: &CommandCorrectionPublication<'_>,
    ) -> Result<(), WorkError>;
}

pub fn record_command_correction_from_context(
    repository: &impl CommandCorrectionRepository,
    context: RecordBeginContext<'_>,
    task_path: &str,
    request: &Value,
    current_execute_selection: &Value,
) -> Result<Value, WorkError> {
    let RecordBeginContext {
        collection,
        validation,
        task,
        attempt,
        attempt_raw,
        index,
        index_raw,
        execution_dir,
        task_id,
    } = context;
    if collection["artifacts"]["task"] != task_path
        || collection["artifacts"]["execution"] != execution_dir
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_correction_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match formal artifacts.",
            json!({}),
        ));
    }
    validate_execution_index(index, index_raw).map_err(rule)?;
    validate_attempt_bytes(attempt, attempt_raw).map_err(rule)?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::WorkflowState,
                "command_correction_task_not_found",
                "The requested TASK is not present in the formal TASK.",
                json!({"task_id":task_id}),
            )
        })?;
    if row["status"] != "in_progress" || row["latest_attempt"].is_null() {
        return Err(error(
            ExitCode::WorkflowState,
            "command_correction_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            json!({"status":row["status"]}),
        ));
    }
    if attempt["status"] != "in_progress" || attempt["attempt_id"] != row["latest_attempt"] {
        return Err(error(
            ExitCode::WorkflowState,
            "command_correction_attempt_not_in_progress",
            "The latest Attempt is not in progress.",
            json!({}),
        ));
    }
    validate_execution_identity(collection, validation, index, attempt, task_id).map_err(rule)?;
    if task["id"] != task_id {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_correction_task_identity_mismatch",
            "The selected TASK does not match the formal collection.",
            json!({}),
        ));
    }
    require_current_correction_instructions(task, attempt, current_execute_selection)?;
    let (candidate, record_id) =
        build_command_correction_candidate(task, attempt, index, task_id, request).map_err(rule)?;
    let rendered = render_execution_index(&candidate).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_execution_index(&candidate, &rendered).map_err(rule)?;
    let attempt_id = attempt["attempt_id"].as_str().unwrap_or("");
    repository.publish_command_correction(&CommandCorrectionPublication {
        execution_dir,
        task_id,
        attempt_id,
        record_id: &record_id,
        attempt_before: attempt_raw,
        index_before: index_raw,
        index_after: &rendered,
    })?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::CommandCorrectionResponse,
    >(
        json!({"schema":"work-command-correction","task_id":task_id,
        "attempt_id":attempt_id,"record_id":record_id,
        "index_path":format!("{execution_dir}/index.json"),
        "correction_status":"recorded","lock_status":"record_reserved"}),
    ))
}

pub struct AttemptClosePublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub attempt_before: &'a [u8],
    pub attempt_after: &'a [u8],
    pub index_before: &'a [u8],
    pub index_after: &'a [u8],
}

pub struct RecordFinishPublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub request: &'a Value,
    pub attempt_before: &'a [u8],
    pub attempt_after: &'a [u8],
    pub index_before: &'a [u8],
    pub index_after: &'a [u8],
}

pub trait RecordFinishRepository {
    fn publish_record_finish(
        &self,
        publication: &RecordFinishPublication<'_>,
    ) -> Result<(), WorkError>;
}

pub trait AttemptCloseRepository {
    fn publish_attempt_close(
        &self,
        publication: &AttemptClosePublication<'_>,
    ) -> Result<(), WorkError>;
}

pub trait RecordBeginRepository {
    fn publish_record_begin(
        &self,
        publication: &RecordBeginPublication<'_>,
    ) -> Result<(), WorkError>;
}

#[derive(Clone, Copy)]
pub struct RecordBeginContext<'a> {
    pub collection: &'a Value,
    pub validation: &'a Value,
    pub task: &'a Value,
    pub attempt: &'a Value,
    pub attempt_raw: &'a [u8],
    pub index: &'a Value,
    pub index_raw: &'a [u8],
    pub execution_dir: &'a str,
    pub task_id: &'a str,
}

#[derive(Clone, Copy)]
pub struct DeviationContext<'a> {
    pub lifecycle: RecordBeginContext<'a>,
    pub task_path: &'a str,
    pub source_files: &'a HashMap<String, Vec<u8>>,
    pub execute_selection: &'a Value,
}

pub struct DeviationRecordPublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub attempt_before: &'a [u8],
    pub attempt_after: &'a [u8],
    pub sources: &'a HashMap<String, Vec<u8>>,
}

pub trait DeviationRecordRepository {
    fn publish_deviation_record(
        &self,
        publication: &DeviationRecordPublication<'_>,
    ) -> Result<(), WorkError>;
}

pub fn prepare_deviation_from_context(
    repository: &impl CommandPrepareRepository,
    context: DeviationContext<'_>,
    proposal: &Value,
) -> Result<Value, WorkError> {
    prepare_deviation_context(repository, context, proposal, true)
}

fn prepare_deviation_context(
    repository: &impl CommandPrepareRepository,
    context: DeviationContext<'_>,
    proposal: &Value,
    require_idle: bool,
) -> Result<Value, WorkError> {
    let DeviationContext {
        lifecycle,
        task_path,
        source_files,
        execute_selection,
    } = context;
    let RecordBeginContext {
        collection,
        validation,
        task,
        attempt,
        attempt_raw,
        index,
        index_raw,
        execution_dir,
        task_id,
    } = lifecycle;
    repository.check_ready(execution_dir, require_idle)?;
    validate_deviation_proposal(proposal).map_err(rule)?;
    if proposal["task_id"] != task_id {
        return Err(error(
            ExitCode::WorkflowState,
            "deviation_task_identity",
            "The proposal TASK ID must match the selected TASK.",
            json!({}),
        ));
    }
    if collection["artifacts"]["task"] != task_path
        || collection["artifacts"]["execution"] != execution_dir
    {
        return Err(error(
            ExitCode::WorkflowState,
            "deviation_paths",
            "Explicit paths must match the formal TASK.",
            json!({}),
        ));
    }
    validate_execution_index(index, index_raw).map_err(rule)?;
    validate_attempt_bytes(attempt, attempt_raw).map_err(rule)?;
    let row = validate_execution_identity(collection, validation, index, attempt, task_id)
        .map_err(rule)?;
    let lock = &index["lock"];
    let attempt_id = proposal["attempt_id"].as_str().unwrap_or("");
    let record_id = proposal["anchor_record_id"].as_str().unwrap_or("");
    if attempt["status"] != "in_progress"
        || row["status"] != "in_progress"
        || row["latest_attempt"] != attempt_id
        || lock["kind"] != "execution"
        || lock["task_id"] != task_id
        || lock["attempt_id"] != attempt_id
        || lock["record_id"] != record_id
        || lock["execute_instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(error(
            ExitCode::WorkflowState,
            "deviation_active_record",
            "The proposal must match the active Attempt and reserved record.",
            json!({}),
        ));
    }
    if execute_selection["selected_paths"] != task["instruction_selection"]["selected_paths"]
        || execute_selection["resolved_paths"] != task["instruction_selection"]["resolved_paths"]
        || execute_selection["instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "deviation_execute_instructions_changed",
            "The current Execute instructions differ from the active Attempt.",
            json!({}),
        ));
    }
    let record_kind =
        formal_record_kind(task, record_id.split('#').next().unwrap_or(record_id)).map_err(rule)?;
    validate_deviation_action(proposal, task, record_kind).map_err(rule)?;
    let mut sources = std::collections::BTreeMap::new();
    for (path, raw) in source_files {
        if repository.read_source(path)? != *raw {
            return Err(error(
                ExitCode::WorkflowState,
                "deviation_source_changed",
                "A deviation source changed during preparation.",
                json!({"path":path}),
            ));
        }
        sources.insert(path.clone(), fingerprint::raw(raw));
    }
    build_deviation_preview(proposal, record_kind, &sources).map_err(rule)
}

pub fn record_deviation_from_context<R: CommandPrepareRepository + DeviationRecordRepository>(
    repository: &R,
    context: DeviationContext<'_>,
    proposal: &Value,
    approved_sha256: &str,
    evidence: &str,
) -> Result<Value, WorkError> {
    let preview = prepare_deviation_context(repository, context, proposal, false)?;
    let lifecycle = context.lifecycle;
    let (candidate, response) = append_approved_deviation(
        lifecycle.attempt,
        &preview,
        approved_sha256,
        evidence,
        lifecycle.execution_dir,
    )
    .map_err(rule)?;
    let rendered = render_attempt(&candidate).map_err(rule)?;
    validate_attempt_bytes(&candidate, &rendered).map_err(rule)?;
    repository.publish_deviation_record(&DeviationRecordPublication {
        execution_dir: lifecycle.execution_dir,
        task_id: lifecycle.task_id,
        attempt_id: lifecycle.attempt["attempt_id"].as_str().unwrap_or(""),
        record_id: lifecycle.index["lock"]["record_id"].as_str().unwrap_or(""),
        attempt_before: lifecycle.attempt_raw,
        attempt_after: &rendered,
        sources: context.source_files,
    })?;
    Ok(response)
}

pub fn begin_record_from_context(
    repository: &impl RecordBeginRepository,
    context: RecordBeginContext<'_>,
    base_record_id: &str,
    authorization_evidence: Option<&str>,
) -> Result<Value, WorkError> {
    let RecordBeginContext {
        collection,
        validation,
        task,
        attempt,
        attempt_raw,
        index,
        index_raw,
        execution_dir,
        task_id,
    } = context;
    validate_execution_index(index, index_raw).map_err(rule)?;
    validate_attempt_bytes(attempt, attempt_raw).map_err(rule)?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::WorkflowState,
                "record_begin_task_not_found",
                "The requested TASK is not present in the formal TASK.",
                json!({"task_id":task_id}),
            )
        })?;
    if row["status"] != "in_progress" || row["latest_attempt"].is_null() {
        return Err(error(
            ExitCode::WorkflowState,
            "record_begin_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            json!({"status":row["status"]}),
        ));
    }
    if attempt["status"] != "in_progress" {
        return Err(error(
            ExitCode::WorkflowState,
            "record_begin_attempt_not_in_progress",
            "The latest Attempt is not in progress.",
            json!({}),
        ));
    }
    let attempt_id = row["latest_attempt"].as_str().unwrap_or("");
    validate_execution_identity(collection, validation, index, attempt, task_id).map_err(rule)?;
    if task["id"] != task_id || attempt["attempt_id"] != attempt_id {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "record_begin_identity_mismatch",
            "The formal TASK, Attempt, and index identities do not match.",
            json!({}),
        ));
    }
    let (candidate, record_id, record_kind) = record_begin_candidate(
        task,
        attempt,
        index,
        task_id,
        base_record_id,
        authorization_evidence,
    )
    .map_err(rule)?;
    let index_after = render_execution_index(&candidate).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_execution_index(&candidate, &index_after).map_err(rule)?;
    repository.publish_record_begin(&RecordBeginPublication {
        execution_dir,
        task_id,
        attempt_id,
        record_id: &record_id,
        index_before: index_raw,
        attempt_before: attempt_raw,
        index_after: &index_after,
    })?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::RecordBeginResponse,
    >(json!({"schema":"work-record-begin","task_id":task_id,
        "attempt_id":attempt_id,"base_record_id":base_record_id,"record_id":record_id,
        "record_kind":record_kind,"index_path":format!("{execution_dir}/index.json"),
        "lock_status":"record_reserved"})))
}

pub fn finish_record_from_context(
    repository: &impl RecordFinishRepository,
    context: RecordBeginContext<'_>,
    request: &Value,
) -> Result<Value, WorkError> {
    validate_record_finish_request(request).map_err(rule)?;
    let RecordBeginContext {
        collection,
        validation,
        task,
        attempt,
        attempt_raw,
        index,
        index_raw,
        execution_dir,
        task_id,
    } = context;
    validate_execution_index(index, index_raw).map_err(rule)?;
    validate_attempt_bytes(attempt, attempt_raw).map_err(rule)?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::WorkflowState,
                "record_finish_task_not_found",
                "The requested TASK is not present in the formal TASK.",
                json!({"task_id":task_id}),
            )
        })?;
    if row["status"] != "in_progress" || row["latest_attempt"].is_null() {
        return Err(error(
            ExitCode::WorkflowState,
            "record_finish_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            json!({"status":row["status"]}),
        ));
    }
    if attempt["status"] != "in_progress" {
        return Err(error(
            ExitCode::WorkflowState,
            "record_finish_attempt_not_in_progress",
            "The latest Attempt is not in progress.",
            json!({}),
        ));
    }
    validate_execution_identity(collection, validation, index, attempt, task_id).map_err(rule)?;
    let attempt_id = row["latest_attempt"].as_str().unwrap_or("");
    if task["id"] != task_id || attempt["attempt_id"] != attempt_id {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "record_finish_identity_mismatch",
            "The formal TASK, Attempt, and index identities do not match.",
            json!({}),
        ));
    }
    let candidates =
        build_record_finish_candidates(task, attempt, index, task_id, request).map_err(rule)?;
    work_model::execution::request::verified::<work_model::execution::request::RecordFinishRequest>(
        request,
    );
    let attempt_after = render_attempt(&candidates.attempt).map_err(rule)?;
    let index_after = render_execution_index(&candidates.index).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_attempt_bytes(&candidates.attempt, &attempt_after).map_err(rule)?;
    validate_execution_index(&candidates.index, &index_after).map_err(rule)?;
    repository.publish_record_finish(&RecordFinishPublication {
        execution_dir,
        task_id,
        attempt_id,
        record_id: &candidates.record_id,
        request,
        attempt_before: attempt_raw,
        attempt_after: &attempt_after,
        index_before: index_raw,
        index_after: &index_after,
    })?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::RecordFinishResponse,
    >(
        json!({"schema":"work-record-finish","task_id":task_id,
        "attempt_id":attempt_id,"record_id":candidates.record_id,
        "record_kind":candidates.record_kind,
        "attempt_path":format!("{execution_dir}/{task_id}/{attempt_id}/attempt.json"),
        "index_path":format!("{execution_dir}/index.json"),
        "record_status":"recorded","lock_status":"attempt_held"})
    ))
}

fn has_blocking_deviation_for_record(attempt: &Value, record_id: &Value) -> bool {
    attempt["execution_deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|deviation| {
            deviation["decision"]["outcome"] == "approved"
                && deviation["reconciliation_status"] == "pending"
                && deviation["proposal"]["anchor_record_id"] == *record_id
                && deviation_reconciliation_target(&deviation["proposal"]) == "task_and_execution"
        })
}

fn validate_close_instruction_state(
    task: &Value,
    attempt: &Value,
    request: &Value,
    current_execute_selection: &Value,
) -> Result<(), WorkError> {
    if current_execute_selection["selected_paths"]
        != task["instruction_selection"]["selected_paths"]
        || current_execute_selection["resolved_paths"]
            != task["instruction_selection"]["resolved_paths"]
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "attempt_close_execute_instruction_hierarchy_mismatch",
            "The current Execute hierarchy does not match the target TASK.",
            json!({}),
        ));
    }
    let instructions_changed =
        current_execute_selection["instructions_sha256"] != attempt["execute_instructions_sha256"];
    let closing_for_change =
        request["status"] == "stopped" && request["final_type"] == "instructions_changed";
    if instructions_changed != closing_for_change {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "attempt_close_execute_instructions_state_mismatch",
            "The requested close reason does not match the Execute instruction fingerprint state.",
            json!({"expected":attempt["execute_instructions_sha256"],
                "actual":current_execute_selection["instructions_sha256"],
                "closing_for_instruction_change":closing_for_change}),
        ));
    }
    Ok(())
}

pub fn close_attempt_from_context(
    repository: &impl AttemptCloseRepository,
    context: RecordBeginContext<'_>,
    request: &Value,
    ended_at: &str,
    current_execute_selection: &Value,
) -> Result<Value, WorkError> {
    validate_attempt_close_request(request).map_err(rule)?;
    let RecordBeginContext {
        collection,
        validation,
        task,
        attempt,
        attempt_raw,
        index,
        index_raw,
        execution_dir,
        task_id,
    } = context;
    validate_execution_index(index, index_raw).map_err(rule)?;
    validate_attempt_bytes(attempt, attempt_raw).map_err(rule)?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::WorkflowState,
                "attempt_close_task_not_found",
                "The requested TASK is not present in the formal TASK.",
                json!({"task_id":task_id}),
            )
        })?;
    if row["status"] != "in_progress" || row["latest_attempt"].is_null() {
        return Err(error(
            ExitCode::WorkflowState,
            "attempt_close_task_not_in_progress",
            "The target TASK must have an active Attempt.",
            json!({"status":row["status"]}),
        ));
    }
    if attempt["status"] != "in_progress" {
        return Err(error(
            ExitCode::WorkflowState,
            "attempt_close_attempt_not_in_progress",
            "The latest Attempt is already closed.",
            json!({}),
        ));
    }
    validate_execution_identity(collection, validation, index, attempt, task_id).map_err(rule)?;
    let attempt_id = row["latest_attempt"].as_str().unwrap_or("");
    if task["id"] != task_id || attempt["attempt_id"] != attempt_id {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "attempt_close_identity_mismatch",
            "The formal TASK, Attempt, and index identities do not match.",
            json!({}),
        ));
    }
    let lock = &index["lock"];
    if lock["kind"] != "execution"
        || lock["task_id"] != task_id
        || lock["attempt_id"] != attempt_id
        || lock["execute_instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(error(
            ExitCode::LockConflict,
            "attempt_close_lock_mismatch",
            "The execution lock does not match the active Attempt.",
            json!({"actual":lock}),
        ));
    }
    if lock.get("record_id").is_some() || lock.get("command_correction").is_some() {
        let reserved = &lock["record_id"];
        let blocking_recorded = has_blocking_deviation_for_record(attempt, reserved);
        let blocking_close =
            request["status"] == "stopped" && request["final_type"] == "specification_defect";
        if lock.get("command_correction").is_some() || !blocking_recorded || !blocking_close {
            return Err(error(
                ExitCode::LockConflict,
                "attempt_close_record_reserved",
                "A reserved record may close only for its recorded blocking specification deviation.",
                json!({"record_id":reserved}),
            ));
        }
    }
    validate_close_instruction_state(task, attempt, request, current_execute_selection)?;
    if request["status"] == "completed" {
        validate_completed_coverage(task, attempt).map_err(rule)?;
    }
    let (closed, updated_index) =
        build_close_candidates(task, index, attempt, request, ended_at).map_err(rule)?;
    let attempt_after = render_attempt(&closed).map_err(rule)?;
    let index_after = render_execution_index(&updated_index).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_attempt_bytes(&closed, &attempt_after).map_err(rule)?;
    validate_execution_index(&updated_index, &index_after).map_err(rule)?;
    repository.publish_attempt_close(&AttemptClosePublication {
        execution_dir,
        task_id,
        attempt_id,
        attempt_before: attempt_raw,
        attempt_after: &attempt_after,
        index_before: index_raw,
        index_after: &index_after,
    })?;
    let pending: Vec<Value> = closed["execution_deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|deviation| {
            deviation["decision"]["outcome"] == "approved"
                && deviation["reconciliation_status"] == "pending"
        })
        .map(|deviation| {
            let classification = deviation_reconciliation_target(&deviation["proposal"]);
            json!({"deviation_id":deviation["deviation_id"],
                "classification":classification,"blocking":classification == "task_and_execution"})
        })
        .collect();
    let task_status = updated_index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .map(|row| &row["status"])
        .unwrap_or(&Value::Null)
        .clone();
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::AttemptCloseResponse,
    >(
        json!({"schema":"work-attempt-close","task_id":task_id,
        "attempt_id":attempt_id,"attempt_path":format!("{execution_dir}/{task_id}/{attempt_id}/attempt.json"),
        "index_path":format!("{execution_dir}/index.json"),
        "attempt_status":request["status"],"task_status":task_status,
        "overall_status":updated_index["overall_status"],
        "pending_deviations":pending,"lock_status":"released"})
    ))
}

pub fn inspect_worktree(
    repository: &impl ExecutionWorktreeRepository,
    preflight: &Value,
    validation: &Value,
    collection: &Value,
) -> Result<Value, WorkError> {
    let task_id = preflight["task_id"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "execute_worktree_task_id",
            "A canonical TASK ID is required.",
            json!({}),
        )
    })?;
    let task_path = preflight["task_path"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "execute_worktree_task_path",
            "The formal TASK path is required.",
            json!({}),
        )
    })?;
    let task_parent = task_path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "execute_worktree_task_path",
                "The formal TASK path is invalid.",
                json!({}),
            )
        })?;
    let item_path = format!("{task_parent}/tasks/{task_id}.json");
    let raw_index = repository.read_file(task_path)?;
    let raw_item = repository.read_file(&item_path)?;
    let unchanged = fingerprint::raw(&raw_index) == preflight["task_index_sha256"]
        && fingerprint::raw(&raw_item) == preflight["task_item_sha256"]
        && validation["task_collection_sha256"] == preflight["task_collection_sha256"]
        && validation["task_index_sha256"] == preflight["task_index_sha256"]
        && validation["task_item_sha256"][task_id] == preflight["task_item_sha256"];
    if !unchanged {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execute_worktree_task_changed",
            "The formal TASK changed after preflight.",
            json!({}),
        ));
    }
    let tasks = collection["tasks"].as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "execute_worktree_task_collection",
            "The formal TASK collection is invalid.",
            json!({}),
        )
    })?;
    let target = tasks
        .iter()
        .find(|task| task["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::ArtifactIntegrity,
                "execute_worktree_task_changed",
                "The formal TASK changed after preflight.",
                json!({}),
            )
        })?;
    let dependencies: Vec<Value> = preflight["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| tasks.iter().find(|task| task["id"] == *id).cloned())
        .collect();
    let execution_dir = preflight["execution_dir"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "execute_worktree_execution_dir",
            "The execution directory is required.",
            json!({}),
        )
    })?;
    let records = repository.git_status()?;
    let report = inspect_records(&records, execution_dir, task_id, target, &dependencies);
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::ExecuteWorktree,
    >(json!({"schema":"work-execute-worktree",
        "requirement_id":preflight["requirement_id"],
        "task_spec_id":preflight["task_spec_id"],"task_id":task_id,
        "task_collection_sha256":preflight["task_collection_sha256"],
        "task_index_sha256":preflight["task_index_sha256"],
        "task_item_sha256":preflight["task_item_sha256"],
        "task_instructions_sha256":preflight["task_instructions_sha256"],
        "execute_instructions_sha256":preflight["execute_instructions_sha256"],
        "task_status":preflight["task_status"],"task_path":task_path,
        "index_sha256":preflight["index_sha256"],"execution_dir":execution_dir,
        "snapshot_sha256":report["snapshot_sha256"],
        "review_status":report["review_status"],
        "excluded_execution_change_count":report["excluded_execution_change_count"],
        "counts":report["counts"],"changes":report["changes"]})))
}

pub struct AttemptStartPublication<'a> {
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub index_before: &'a [u8],
    pub locked_index: &'a [u8],
    pub attempt: &'a [u8],
    pub started_index: &'a [u8],
    pub expected_snapshot: &'a str,
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn parse_execution_document(raw: &[u8], source: &str) -> Result<Value, WorkError> {
    parse_json_contract(raw).map_err(|issue| match issue {
        JsonContractIssue::InvalidUtf8(offset) => error(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":source,"byte_offset":offset}),
        ),
        JsonContractIssue::MultipleBom => error(
            ExitCode::InputFormat,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            json!({"source":source}),
        ),
        JsonContractIssue::DuplicateKey(key) => error(
            ExitCode::Contract,
            "duplicate_json_key",
            "Duplicate keys are ambiguous; no parsed document will be used.",
            json!({"keys":[key]}),
        ),
        JsonContractIssue::InvalidConstant(value) => error(
            ExitCode::InputFormat,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            json!({"value": value}),
        ),
        JsonContractIssue::InvalidJson { line, column } => error(
            ExitCode::Contract,
            "invalid_json_contract",
            "The execution document JSON is invalid.",
            json!({"line":line,"column":column}),
        ),
        JsonContractIssue::NotObject => error(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        ),
    })
}

fn rule(issue: ExecutionIssue) -> WorkError {
    let code = if issue.reason_code.starts_with("execution_authorization_")
        || issue.reason_code.starts_with("command_run_")
    {
        ExitCode::WorkflowState
    } else {
        match issue.reason_code {
            "execute_preflight_index_identity_mismatch"
            | "execute_preflight_index_task_set_mismatch"
            | "execute_preflight_task_instructions_mismatch"
            | "record_begin_index_identity_mismatch"
            | "record_begin_index_task_set_mismatch"
            | "record_begin_task_instructions_mismatch"
            | "record_begin_attempt_identity_mismatch"
            | "attempt_start_unexpected_attempt_history"
            | "attempt_start_latest_attempt_missing"
            | "attempt_start_attempt_history_conflict"
            | "attempt_start_continuation_skill_identity_mismatch"
            | "command_correction_retry_sequence_mismatch" => ExitCode::ArtifactIntegrity,
            "execute_preflight_lock_present"
            | "record_begin_lock_mismatch"
            | "record_begin_record_already_reserved"
            | "command_correction_lock_mismatch"
            | "command_correction_already_recorded" => ExitCode::LockConflict,
            "execute_preflight_task_not_found"
            | "record_begin_task_not_found"
            | "execute_preflight_task_not_eligible"
            | "execute_preflight_dependency_incomplete"
            | "execute_preflight_input_confirmation_required"
            | "attempt_start_prepare_ineligible"
            | "attempt_start_unexpected_continuation"
            | "attempt_start_latest_attempt_required"
            | "attempt_start_source_not_closed"
            | "attempt_start_invalid_original_status"
            | "attempt_start_id_mismatch"
            | "attempt_start_id_exhausted" => ExitCode::WorkflowState,
            "attempt_close_incomplete_validations" => ExitCode::WorkflowState,
            "execution_authorization_scope_expansion"
            | "execution_authorization_retry_required"
            | "execution_authorization_retry_evidence_reused"
            | "execution_authorization_unexpected_retry_evidence" => ExitCode::WorkflowState,
            _ => ExitCode::Contract,
        }
    };
    error(code, issue.reason_code, issue.message, issue.details)
}

pub fn start_attempt_from_preflight(
    repository: &impl AttemptStartRepository,
    preflight: &Value,
    index: &Value,
    index_raw: &[u8],
    request: &Value,
    started_at: &str,
) -> Result<Value, WorkError> {
    validate_attempt_start_request(request).map_err(rule)?;
    let expected_snapshot = preflight["snapshot_sha256"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "attempt_start_worktree_snapshot_required",
            "A reviewed Git worktree snapshot is required.",
            json!({}),
        )
    })?;
    if request["worktree_snapshot_sha256"] != expected_snapshot {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "attempt_start_worktree_snapshot_changed",
            "The Git worktree snapshot changed after review.",
            json!({"expected":request["worktree_snapshot_sha256"],"actual":expected_snapshot}),
        ));
    }
    let index_validation = validate_execution_index(index, index_raw).map_err(rule)?;
    if preflight["index_sha256"] != index_validation["index_sha256"] {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "attempt_start_index_changed",
            "The execution index changed after preflight.",
            json!({}),
        ));
    }
    let task_id = preflight["task_id"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "attempt_start_task_id",
            "A canonical TASK ID is required.",
            json!({}),
        )
    })?;
    let execution_dir = preflight["execution_dir"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "attempt_start_execution_dir",
            "An execution directory is required.",
            json!({}),
        )
    })?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            error(
                ExitCode::ArtifactIntegrity,
                "attempt_start_task_not_found",
                "The target TASK is not present in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    let latest = row["latest_attempt"].as_str();
    if preflight["task_status"] != row["status"] {
        return Err(error(
            ExitCode::WorkflowState,
            "attempt_start_prepare_ineligible",
            "The TASK is not eligible for a new Attempt.",
            json!({}),
        ));
    }
    let attempt_id = work_operations::derivation::identity::next_attempt_id(
        if row["status"] == "pending_retry" {
            latest
        } else {
            None
        },
    )
    .map_err(rule)?;
    let names = repository.attempt_names(execution_dir, task_id)?;
    validate_attempt_namespace(
        &names,
        row["status"].as_str().unwrap_or(""),
        latest,
        &attempt_id,
        false,
    )
    .map_err(rule)?;
    let source_attempt = if row["status"] == "pending_retry" {
        let latest = latest.ok_or_else(|| {
            error(
                ExitCode::WorkflowState,
                "attempt_start_latest_attempt_required",
                "A pending_retry TASK must identify its latest Attempt.",
                json!({}),
            )
        })?;
        let path = format!("{execution_dir}/{task_id}/{latest}/attempt.json");
        let raw = repository.read_attempt(&path)?;
        let value = parse_json_contract(&raw).map_err(|_| {
            error(
                ExitCode::Contract,
                "invalid_json_contract",
                "The source Attempt JSON is invalid.",
                json!({"source":path}),
            )
        })?;
        validate_attempt_bytes(&value, &raw).map_err(rule)?;
        Some(value)
    } else {
        None
    };
    let attempt = build_attempt_candidate(
        preflight,
        index,
        request,
        &attempt_id,
        started_at,
        source_attempt.as_ref(),
    )
    .map_err(rule)?;
    let attempt_raw = render_attempt(&attempt).map_err(rule)?;
    let mut locked = index.clone();
    locked["lock"] = build_execution_lock(
        task_id,
        &attempt_id,
        attempt["execute_instructions_sha256"]
            .as_str()
            .unwrap_or(""),
    );
    let locked_raw = render_execution_index(&locked).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_execution_index(&locked, &locked_raw).map_err(rule)?;
    let started = start_index(&locked, task_id, &attempt_id).map_err(rule)?;
    let started_raw = render_execution_index(&started).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_execution_index(&started, &started_raw).map_err(rule)?;
    repository.publish_attempt_start(&AttemptStartPublication {
        execution_dir,
        task_id,
        attempt_id: &attempt_id,
        index_before: index_raw,
        locked_index: &locked_raw,
        attempt: &attempt_raw,
        started_index: &started_raw,
        expected_snapshot,
    })?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::AttemptStartResponse,
    >(
        json!({"schema":"work-attempt-start","task_id":task_id,
        "attempt_id":attempt_id,
        "attempt_path":format!("{execution_dir}/{task_id}/{attempt_id}/attempt.json"),
        "index_path":format!("{execution_dir}/index.json"),"status":"started",
        "lock_status":"held"})
    ))
}

pub fn prepare_initial_index(validation: &Value) -> Result<(Value, Vec<u8>), WorkError> {
    let collection = &validation["collection_contract"];
    let index = build_initial_execution_index(collection, validation).map_err(rule)?;
    let raw = render_execution_index(&index).map_err(|_| {
        error(
            ExitCode::InternalError,
            "execution_index_render_failed",
            "The execution index could not be rendered.",
            json!({}),
        )
    })?;
    validate_execution_index(&index, &raw).map_err(rule)?;
    Ok((index, raw))
}

pub fn preflight_index<R: ExecutionIndexRepository>(
    repository: &R,
    validation: &Value,
    execution_dir: &str,
    task_id: &str,
    confirmed_inputs: &[String],
    allowed_lock: Option<&Value>,
    eligible_statuses: &[&str],
) -> Result<Value, WorkError> {
    let collection = &validation["collection_contract"];
    let expected_dir = collection["artifacts"]["execution"]
        .as_str()
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_artifact_path",
                "The execution artifact path is invalid.",
                json!({}),
            )
        })?;
    if expected_dir != execution_dir {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match the formal artifacts.",
            json!({}),
        ));
    }
    let path = format!("{execution_dir}/index.json");
    let raw = repository.read_index(&path)?;
    let index = parse_json_contract(&raw).map_err(|issue| match issue {
        JsonContractIssue::InvalidUtf8(offset) => error(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":path,"byte_offset":offset}),
        ),
        JsonContractIssue::MultipleBom => error(
            ExitCode::InputFormat,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            json!({"source":path}),
        ),
        JsonContractIssue::DuplicateKey(key) => error(
            ExitCode::Contract,
            "duplicate_json_key",
            "Duplicate keys are ambiguous; no parsed document will be used.",
            json!({"keys":[key]}),
        ),
        JsonContractIssue::InvalidConstant(value) => error(
            ExitCode::InputFormat,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            json!({"value": value}),
        ),
        JsonContractIssue::InvalidJson { line, column } => error(
            ExitCode::Contract,
            "invalid_json_contract",
            "The execution index JSON is invalid.",
            json!({"line":line,"column":column}),
        ),
        JsonContractIssue::NotObject => error(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        ),
    })?;
    let index_validation = validate_execution_index(&index, &raw).map_err(rule)?;
    validate_preflight_identity(&index, collection, validation).map_err(rule)?;
    let task = check_task_eligibility(collection, &index, task_id, allowed_lock, eligible_statuses)
        .map_err(rule)?;
    check_confirmed_inputs(&task, confirmed_inputs).map_err(rule)?;
    let mut present_sources = HashSet::new();
    for input in task["inputs"].as_array().into_iter().flatten() {
        let source = match input["kind"].as_str() {
            Some("project_state") => input["source"].as_str(),
            Some("task_output") => {
                let reference = input["source"].as_str().unwrap_or("");
                let (source_task_id, source_file_id) =
                    reference.split_once('/').unwrap_or(("", ""));
                collection["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|row| row["id"] == source_task_id)
                    .and_then(|row| row["files"].as_array())
                    .and_then(|rows| rows.iter().find(|row| row["id"] == source_file_id))
                    .and_then(|row| {
                        if row["action"] == "move" {
                            row["destination"].as_str()
                        } else {
                            row["path"].as_str()
                        }
                    })
            }
            _ => None,
        };
        if let Some(source) = source {
            if repository.inspect_path(source)?.exists {
                present_sources.insert(source.to_owned());
            }
        }
    }
    let input_readiness = check_input_sources(collection, &task, &present_sources).map_err(rule)?;
    let mut file_state = HashMap::new();
    for file in task["files"].as_array().into_iter().flatten() {
        for field in ["path", "source", "destination"] {
            if let Some(path) = file[field].as_str() {
                if !file_state.contains_key(path) {
                    file_state.insert(path.to_owned(), repository.inspect_path(path)?);
                }
            }
        }
    }
    let file_readiness = check_file_lifecycle(&task, &file_state).map_err(rule)?;
    Ok(
        json!({"index":index,"index_validation":index_validation,"task":task,
        "task_id":task_id,"input_readiness":input_readiness,"file_readiness":file_readiness,
        "execution_index_path":path}),
    )
}

pub struct ExecutePreflightRequest<'a> {
    pub validation: &'a Value,
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub confirmed_inputs: &'a [String],
}

pub fn prepare_execute_preflight(
    index_repository: &impl ExecutionIndexRepository,
    instruction_repository: &impl InstructionSourceRepository,
    request: ExecutePreflightRequest<'_>,
) -> Result<Value, WorkError> {
    let ExecutePreflightRequest {
        validation,
        task_path,
        execution_dir,
        task_id,
        confirmed_inputs,
    } = request;
    index_repository.check_execution_ready(task_path, execution_dir)?;
    index_repository.check_execution_task_layout(execution_dir, task_id)?;
    if validation["collection_contract"]["artifacts"]["task"] != task_path {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_artifact_path_mismatch",
            "The explicit TASK and execution paths do not match the formal artifacts.",
            json!({}),
        ));
    }
    let base = preflight_index(
        index_repository,
        validation,
        execution_dir,
        task_id,
        confirmed_inputs,
        None,
        &["pending", "pending_retry"],
    )?;
    let task = &base["task"];
    let collection = &validation["collection_contract"];
    let row = base["index"]["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == task_id))
        .expect("preflight verified TASK row");
    if base["index"]["skill_selection_sha256"] != collection["skill_selection"]["selection_sha256"]
        || row["skill_id"] != task["skill_id"]
        || row["task_item_sha256"] != validation["task_item_sha256"][task_id]
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_task_binding_mismatch",
            "Execution must retain the formal TASK skill selection and item binding.",
            json!({"task_id":task_id}),
        ));
    }
    let stored =
        instruction::parse_selection(&task["instruction_selection"], "task.instruction_selection")?;
    let mut references = vec!["execute.general.execution-records".to_owned()];
    if base["index"]["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["id"] == task_id && row["status"] == "pending_retry")
    {
        references.push("execute.general.execution-recovery".to_owned());
    }
    let execute = instruction::select(
        instruction_repository,
        "execute",
        &stored.selected_paths,
        &references,
    )?;
    if execute.selected_paths != stored.selected_paths
        || execute.resolved_paths != stored.resolved_paths
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_instruction_hierarchy_mismatch",
            "The Execute hierarchy does not match the target TASK hierarchy.",
            json!({}),
        ));
    }
    let target_skill = task["skill_id"].as_str();
    let decision = if target_skill.is_some() {
        "external_skills"
    } else {
        "base_only"
    };
    let selected_skills: Vec<_> = validation["collection_contract"]["skill_selection"]["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|skill| skill["id"].as_str() == target_skill && target_skill.is_some())
        .cloned()
        .collect();
    if selected_skills.len() != usize::from(target_skill.is_some())
        || selected_skills.iter().any(|skill| {
            skill["mode_support"]["execute"] == "unsupported"
                || skill["dependency_status"] != "available"
        })
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_skill_binding_mismatch",
            "The target TASK requires its confirmed available executable skill.",
            json!({"task_id":task_id,"skill_id":task["skill_id"]}),
        ));
    }
    let skill_selection = json!({"schema":"work-skill-selection","decision":decision,
        "selection_sha256":skill_selection_sha256(decision, &selected_skills),
        "skills":selected_skills});
    let row = base["index"]["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .expect("preflight verified TASK row");
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::ExecutePreflight,
    >(json!({"schema":"work-execute-preflight",
        "requirement_id":validation["collection_contract"]["requirement_id"],
        "task_spec_id":validation["collection_contract"]["spec_id"],
        "task_id":task_id,"skill_id":task["skill_id"],"task_path":task_path,
        "execution_dir":execution_dir,"index_path":base["execution_index_path"],
        "task_status":row["status"],"dependencies":task["dependencies"],
        "confirmed_inputs":confirmed_inputs,"inputs":base["input_readiness"],
        "files":base["file_readiness"],
        "task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_item_sha256":validation["task_item_sha256"][task_id],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"],
        "task_instructions_sha256":row["instructions_sha256"],
        "execute_instructions_sha256":execute.instructions_sha256,
        "execute_instruction_selection":execute,
        "execute_skill_selection":skill_selection,
        "index_sha256":base["index_validation"]["index_sha256"],
        "eligibility":"passed"})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy::HierarchyCatalogRepository;
    use std::cell::RefCell;
    use work_operations::derivation::fingerprint::{
        raw as sha256_hex, structured as canonical_json_sha256,
    };
    use work_operations::hierarchy::{CrossModeCatalog, Hierarchy};
    use work_operations::instruction::{LoadedSource, SourceSet, SourceSummary};

    #[test]
    fn writer_context_uses_verified_requirement_and_rejects_cross_context_paths() {
        let root = std::path::PathBuf::from(if cfg!(windows) {
            "C:/project"
        } else {
            "/project"
        });
        let make = || {
            let index = json!({"requirement_id":"example","artifacts":{"task":"custom/tasks/example/index.json","execution":"custom/not-derived-from-requirement"},"tasks":[{"id":"TASK-001"}]});
            task::ExecutionTaskContext {
                contract: index.clone(),
                collection: index.clone(),
                index,
                validation: json!({"requirement_id":"example"}),
                sources: HashMap::new(),
            }
        };
        let target = ExecutionProjectTarget {
            task_path: "custom/tasks/example/index.json",
            execution_dir: "custom/not-derived-from-requirement",
            task_id: "TASK-001",
        };
        let context =
            ExecutionWriterContext::from_verified_task(root.clone(), target, make()).unwrap();
        assert_eq!(context.writer().requirement_id.as_str(), "example");
        assert_eq!(
            context.target().execution_dir,
            "custom/not-derived-from-requirement"
        );
        assert_eq!(context.task_context().index["requirement_id"], "example");
        assert!(
            ExecutionWriterContext::from_verified_task("relative".into(), target, make()).is_err()
        );
        let mut mismatched = make();
        mismatched.validation["requirement_id"] = json!("other");
        assert!(
            ExecutionWriterContext::from_verified_task(root.clone(), target, mismatched).is_err()
        );
        let wrong_path = ExecutionProjectTarget {
            execution_dir: "foreign/execution",
            ..target
        };
        assert!(
            ExecutionWriterContext::from_verified_task(root.clone(), wrong_path, make()).is_err()
        );
        let wrong_task = ExecutionProjectTarget {
            task_id: "TASK-002",
            ..target
        };
        assert!(
            ExecutionWriterContext::from_verified_task(root.clone(), wrong_task, make()).is_err()
        );
        assert!(
            context
                .check_execution_index(&json!({"requirement_id":"other"}), b"{}")
                .is_err()
        );
    }

    #[test]
    fn record_lifecycle_instruction_selection_keeps_operation_specific_errors() {
        let task = json!({"instruction_selection":{"selected_paths":["web/backend/java/jpa"],
            "resolved_paths":["general","web","web/backend","web/backend/java",
                "web/backend/java/jpa"]}});
        let selection = json!({"selected_paths":["web/backend/java/jpa"],
            "resolved_paths":["general","web","web/backend","web/backend/java",
                "web/backend/java/jpa"],"instructions_sha256":"a".repeat(64)});
        let attempt = json!({"execute_instructions_sha256":"a".repeat(64)});
        let stale = json!({"execute_instructions_sha256":"0".repeat(64)});
        let mut wrong_hierarchy = selection.clone();
        wrong_hierarchy["selected_paths"] = json!([]);
        for operation in ["record_begin", "record_finish"] {
            require_current_execute_instructions(&task, &attempt, &selection, operation).unwrap();
            let error =
                require_current_execute_instructions(&task, &attempt, &wrong_hierarchy, operation)
                    .unwrap_err();
            assert_eq!(
                error.reason_code,
                format!("{operation}_execute_instruction_hierarchy_mismatch")
            );
            let error = require_current_execute_instructions(&task, &stale, &selection, operation)
                .unwrap_err();
            assert_eq!(
                error.reason_code,
                format!("{operation}_execute_instructions_changed")
            );
            assert_eq!(error.details["expected"], "0".repeat(64));
            assert_eq!(error.details["actual"], "a".repeat(64));
        }
        assert_eq!(
            execute_references(&attempt),
            ["execute.general.execution-records"]
        );
        let continued = json!({"continued_from":"ATTEMPT-001"});
        assert_eq!(
            execute_references(&continued),
            [
                "execute.general.execution-records",
                "execute.general.execution-recovery"
            ]
        );
    }

    #[test]
    fn correction_instruction_binding_accepts_current_and_rejects_stale_selection() {
        let task = json!({"instruction_selection":{"selected_paths":["web/backend/java/jpa"],
            "resolved_paths":["general","web","web/backend","web/backend/java","web/backend/java/jpa"]}});
        let selection = json!({"selected_paths":["web/backend/java/jpa"],
            "resolved_paths":["general","web","web/backend","web/backend/java","web/backend/java/jpa"],
            "instructions_sha256":"c".repeat(64)});
        let attempt = json!({"execute_instructions_sha256":"c".repeat(64)});
        require_current_correction_instructions(&task, &attempt, &selection).unwrap();
        let stale = json!({"execute_instructions_sha256":"0".repeat(64)});
        assert_eq!(
            require_current_correction_instructions(&task, &stale, &selection)
                .unwrap_err()
                .reason_code,
            "command_correction_execute_instructions_changed"
        );
    }

    #[test]
    fn attempt_close_instruction_reason_and_blocking_record_match_current_contract() {
        let task = json!({"instruction_selection":{"selected_paths":["web/backend/java/jpa"],
            "resolved_paths":["general","web","web/backend","web/backend/java","web/backend/java/jpa"]}});
        let selection = json!({"selected_paths":["web/backend/java/jpa"],
            "resolved_paths":["general","web","web/backend","web/backend/java","web/backend/java/jpa"],
            "instructions_sha256":"c".repeat(64)});
        let attempt = json!({"execute_instructions_sha256":"c".repeat(64)});
        let normal = json!({"status":"stopped","final_type":"user_stopped"});
        let changed = json!({"status":"stopped","final_type":"instructions_changed"});
        validate_close_instruction_state(&task, &attempt, &normal, &selection).unwrap();
        let stale = json!({"execute_instructions_sha256":"0".repeat(64)});
        validate_close_instruction_state(&task, &stale, &changed, &selection).unwrap();
        let error =
            validate_close_instruction_state(&task, &attempt, &changed, &selection).unwrap_err();
        assert_eq!(
            error.reason_code,
            "attempt_close_execute_instructions_state_mismatch"
        );
        assert_eq!(error.details["closing_for_instruction_change"], true);
        let attempt = json!({"execution_deviations":[
            {"proposal":{"anchor_record_id":"CMD-001","impact":{"scope_changed":true}},
                "decision":{"outcome":"rejected"},"reconciliation_status":"not_needed"},
            {"proposal":{"anchor_record_id":"CMD-002","impact":{"scope_changed":true}},
                "decision":{"outcome":"approved"},"reconciliation_status":"pending"}]});
        assert!(has_blocking_deviation_for_record(
            &attempt,
            &json!("CMD-002")
        ));
        assert!(!has_blocking_deviation_for_record(
            &attempt,
            &json!("CMD-001")
        ));
    }

    #[test]
    fn attempt_prepare_ineligible_keeps_workflow_exit_code() {
        let mapped = rule(ExecutionIssue {
            reason_code: "attempt_start_prepare_ineligible",
            message: "The TASK is not eligible for a new Attempt.",
            details: json!({}),
        });
        assert_eq!(mapped.exit_code, ExitCode::WorkflowState);
    }

    struct FakeCommandPrepare {
        sources: HashMap<String, Vec<u8>>,
        receipt: bool,
        idle_checks: RefCell<Vec<bool>>,
    }

    #[derive(Default)]
    struct FakeCommandCorrection(RefCell<Option<Vec<u8>>>);

    impl CommandCorrectionRepository for FakeCommandCorrection {
        fn publish_command_correction(
            &self,
            publication: &CommandCorrectionPublication<'_>,
        ) -> Result<(), WorkError> {
            *self.0.borrow_mut() = Some(publication.index_after.to_vec());
            Ok(())
        }
    }

    impl CommandPrepareRepository for FakeCommandPrepare {
        fn check_ready(&self, _execution_dir: &str, require_idle: bool) -> Result<(), WorkError> {
            self.idle_checks.borrow_mut().push(require_idle);
            Ok(())
        }
        fn runtime_os(&self) -> &'static str {
            "macos"
        }
        fn read_source(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            self.sources.get(path).cloned().ok_or_else(|| {
                error(
                    ExitCode::IoFailure,
                    "file_not_found",
                    "The source is missing.",
                    json!({}),
                )
            })
        }
        fn working_directory(&self, _path: &str) -> Result<String, WorkError> {
            Ok("/project".into())
        }
        fn resolve_invocation(&self, argv: &[String], _cwd: &str) -> Result<Value, WorkError> {
            Ok(json!({"kind":"direct","executable":"/usr/bin/printf",
                "executable_sha256":"a".repeat(64),"argv":argv}))
        }
        fn receipt_exists(&self, _prefix: &str) -> Result<bool, WorkError> {
            Ok(self.receipt)
        }
    }

    #[test]
    fn command_prepare_rechecks_formal_sources_and_reserved_command() {
        let replacement = json!({"mode":"argv","argv":["printf","corrected"]});
        let action = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":replacement});
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[{"id":"CMD-001"}],"validations":[],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[action],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":[],"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let mut index = json!({"acceptance_results":[],"schema":"work-execution-index","requirement_id":"demo",
            "title":"Execution","task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),"overall_status":"in_progress",
            "lock":build_execution_lock("TASK-001","ATTEMPT-001",&"e".repeat(64)),
            "tasks":[{"id":"TASK-001","status":"in_progress","skill_id":null,
                "task_item_sha256":"c".repeat(64),"acceptance_results":[],"instructions_sha256":"d".repeat(64),
                "latest_attempt":"ATTEMPT-001"}]});
        index["lock"]["record_id"] = json!("CMD-001");
        let task = json!({"id":"TASK-001","commands":[{"id":"CMD-001","mode":"argv",
            "argv":["printf","ok"]}],"operations":[],"validations":[],
            "instruction_selection":{"selected_paths":[],"resolved_paths":[]}});
        let mut manifest=work_model::contract_data::registry_value()["items"]["work-source-snapshot"]["description"]["example"].clone();
        manifest["requirement_id"] = json!("demo");
        manifest["content"] =
            json!({"path":"source.txt","size":12,"sha256":sha256_hex(b"requirements")});
        let manifest_raw = work_operations::canonical::canonical_json(&manifest).unwrap();
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "artifacts":{"source":"outputs/work/sources/demo","task":"task/index.json",
                "execution":"execution"},"source":{"kind":"snapshot","manifest":manifest},
            "execution_defaults":{"os":"macos","working_directory":"."},
            "tasks":[task.clone()]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"task_ids":["TASK-001"],
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)}});
        let attempt_raw = render_attempt(&attempt).unwrap();
        let index_raw = render_execution_index(&index).unwrap();
        let sources = HashMap::from([
            ("task/index.json".into(), b"task".to_vec()),
            (
                "outputs/work/sources/demo/SRC-001/manifest.json".into(),
                manifest_raw.clone(),
            ),
            (
                "outputs/work/sources/demo/SRC-001/manifest.json.done".into(),
                b"complete".to_vec(),
            ),
            (
                "outputs/work/sources/demo/SRC-001/source.txt".into(),
                b"requirements".to_vec(),
            ),
            ("execution/index.json".into(), index_raw.clone()),
            (
                "execution/TASK-001/ATTEMPT-001/attempt.json".into(),
                attempt_raw.clone(),
            ),
        ]);
        let mut repository = FakeCommandPrepare {
            sources,
            receipt: false,
            idle_checks: RefCell::new(Vec::new()),
        };
        let selected = json!({"selected_paths":[],"resolved_paths":[],
            "instructions_sha256":"e".repeat(64)});
        let request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let source_files = HashMap::from([("task/index.json".into(), b"task".to_vec())]);
        let make_context = || CommandPrepareContext {
            lifecycle: RecordBeginContext {
                collection: &collection,
                validation: &validation,
                task: &task,
                attempt: &attempt,
                attempt_raw: &attempt_raw,
                index: &index,
                index_raw: &index_raw,
                execution_dir: "execution",
                task_id: "TASK-001",
            },
            task_path: "task/index.json",
            source_files: &source_files,
            execute_selection: &selected,
        };
        let (preview, evidence) =
            prepare_command_run_from_context(&repository, make_context(), &request).unwrap();
        assert_eq!(evidence, "Approved");
        assert_eq!(*repository.idle_checks.borrow(), [false]);
        prepare_command_from_context(&repository, make_context(), &request).unwrap();
        assert_eq!(*repository.idle_checks.borrow(), [false, true]);
        assert_eq!(preview["invocation"]["argv"], json!(["printf", "ok"]));
        assert_eq!(
            preview["sources"]["outputs/work/sources/demo/SRC-001/manifest.json"],
            sha256_hex(&manifest_raw)
        );
        assert_eq!(
            preview["sources"]["outputs/work/sources/demo/SRC-001/manifest.json.done"],
            sha256_hex(b"complete")
        );
        assert_eq!(
            preview["sources"]["outputs/work/sources/demo/SRC-001/source.txt"],
            sha256_hex(b"requirements")
        );
        {
            assert_eq!(
                preview["receipt_dir"],
                "execution/TASK-001/ATTEMPT-001/receipts/CMD-001"
            );
            assert!(preview.get("receipt_prefix").is_none());
        }
        let candidate =
            prepare_command_context(&repository, make_context(), &request, false).unwrap();
        assert_eq!(
            candidate["receipt_dir"],
            "execution/TASK-001/ATTEMPT-001/receipts/CMD-001"
        );
        assert!(candidate.get("receipt_prefix").is_none());
        let correction_repo = FakeCommandCorrection::default();
        let correction = record_command_correction_from_context(
            &correction_repo,
            make_context().lifecycle,
            "task/index.json",
            &json!({"schema":"work-command-correction-request",
                "actual_command":replacement,"reason":"Correct the argument"}),
            &selected,
        )
        .unwrap();
        assert_eq!(correction["correction_status"], "recorded");
        let corrected_raw = correction_repo.0.borrow();
        let corrected_raw = corrected_raw.as_ref().unwrap();
        let corrected_index = parse_json_contract(corrected_raw).unwrap();
        assert_eq!(
            corrected_index["lock"]["command_correction"]["actual_command"],
            replacement
        );
        repository.receipt = true;
        assert_eq!(
            prepare_command_from_context(&repository, make_context(), &request)
                .unwrap_err()
                .reason_code,
            "command_run_already_started"
        );
        repository.receipt = false;
        repository
            .sources
            .insert("task/index.json".into(), b"changed".to_vec());
        assert_eq!(
            prepare_command_from_context(&repository, make_context(), &request)
                .unwrap_err()
                .reason_code,
            "command_run_source_changed"
        );
        let deviation_proposal = json!({"schema":"work-execution-deviation-proposal",
            "task_id":"TASK-001","attempt_id":"ATTEMPT-001",
            "anchor_record_id":"CMD-001","task_basis":["CMD-001"],
            "gap":"The reviewed command differs.","action":action,
            "modifiable_files":[],"impact":{"summary":"Use reviewed command.",
                "requirement_changed":false,"scope_changed":false,
                "acceptance_criteria_changed":false,"deliverables_changed":false,
                "safety_boundary_changed":false,"external_side_effect_boundary_changed":false},
            "side_effects":[]});
        assert_eq!(
            prepare_deviation_from_context(
                &repository,
                DeviationContext {
                    lifecycle: make_context().lifecycle,
                    task_path: "task/index.json",
                    source_files: &source_files,
                    execute_selection: &selected,
                },
                &deviation_proposal,
            )
            .unwrap_err()
            .reason_code,
            "deviation_source_changed"
        );
    }

    struct FakeInstruction;

    impl HierarchyCatalogRepository for FakeInstruction {
        fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
            unreachable!("execute selection does not use cross-mode catalog")
        }

        fn mode_paths(&self, _mode: &str) -> Result<Vec<String>, WorkError> {
            Ok(vec!["general".into()])
        }
    }

    impl InstructionSourceRepository for FakeInstruction {
        fn load_sources(
            &self,
            mode: &str,
            hierarchy: &Hierarchy,
            references: &[String],
        ) -> Result<SourceSet, WorkError> {
            Ok(SourceSet {
                mode: mode.into(),
                hierarchy: hierarchy.clone(),
                sources: vec![LoadedSource {
                    summary: SourceSummary {
                        kind: "instruction".into(),
                        logical_name: "execute.general".into(),
                        canonical_sha256: "a".repeat(64),
                    },
                    canonical_content: b"execute\n".to_vec(),
                }],
                references: references.to_vec(),
                instructions_sha256: "e".repeat(64),
            })
        }
    }

    struct FakeIndex(Vec<u8>);
    impl ExecutionIndexRepository for FakeIndex {
        fn read_index(&self, _relative_path: &str) -> Result<Vec<u8>, WorkError> {
            Ok(self.0.clone())
        }
        fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError> {
            Ok(FileState {
                identity: relative_path.into(),
                exists: false,
            })
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
            _execution_dir: &str,
            _task_id: &str,
        ) -> Result<(), WorkError> {
            Ok(())
        }
    }

    struct FakePaths {
        raw: Vec<u8>,
        states: HashMap<String, FileState>,
    }

    struct FakeWorktree {
        index: Vec<u8>,
        item: Vec<u8>,
        records: Vec<Value>,
    }

    impl ExecutionWorktreeRepository for FakeWorktree {
        fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
            Ok(if relative_path.ends_with("/index.json") {
                self.index.clone()
            } else {
                self.item.clone()
            })
        }

        fn git_status(&self) -> Result<Vec<Value>, WorkError> {
            Ok(self.records.clone())
        }
    }

    #[test]
    fn worktree_rechecks_formal_bytes_and_classifies_git_changes() {
        let repository = FakeWorktree {
            index: b"index\n".to_vec(),
            item: b"item\n".to_vec(),
            records: vec![
                json!({"index_status":"?","worktree_status":"?",
                "path":"execution/index.json"}),
                json!({"index_status":"M","worktree_status":" ","path":"src/main.rs"}),
            ],
        };
        let preflight = json!({"requirement_id":"demo","task_spec_id":"TASK-SPEC-001",
            "task_id":"TASK-001","task_path":"task/index.json",
            "execution_dir":"execution","task_status":"pending","dependencies":[],
            "task_collection_sha256":"a".repeat(64),
            "task_index_sha256":sha256_hex(&repository.index),
            "task_item_sha256":sha256_hex(&repository.item),
            "task_instructions_sha256":"b".repeat(64),
            "execute_instructions_sha256":"c".repeat(64),"index_sha256":"d".repeat(64)});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":sha256_hex(&repository.index),
            "task_item_sha256":{"TASK-001":sha256_hex(&repository.item)}});
        let collection = json!({"tasks":[{"id":"TASK-001",
            "files":[{"path":"src/main.rs"}]}]});
        let result = inspect_worktree(&repository, &preflight, &validation, &collection).unwrap();
        assert_eq!(result["excluded_execution_change_count"], 1);
        assert_eq!(result["counts"]["target_task"], 1);
        assert_eq!(result["changes"][0]["path"], "src/main.rs");
        let changed_item = FakeWorktree {
            index: repository.index.clone(),
            item: [b"\xef\xbb\xbf".as_slice(), &repository.item].concat(),
            records: repository.records.clone(),
        };
        assert_eq!(
            inspect_worktree(&changed_item, &preflight, &validation, &collection)
                .unwrap_err()
                .reason_code,
            "execute_worktree_task_changed"
        );
        let mut changed = preflight;
        changed["task_index_sha256"] = json!("f".repeat(64));
        assert_eq!(
            inspect_worktree(&repository, &changed, &validation, &collection)
                .unwrap_err()
                .reason_code,
            "execute_worktree_task_changed"
        );
    }

    #[derive(Default)]
    struct FakeAttemptStart(RefCell<Option<Vec<u8>>>);

    struct FakeRetryStart {
        source: Vec<u8>,
        stored: RefCell<Option<Vec<u8>>>,
    }

    #[derive(Default)]
    struct FakeRecordBegin(RefCell<Option<Vec<u8>>>);

    #[derive(Default)]
    struct FakeAttemptClose(RefCell<Option<(Vec<u8>, Vec<u8>)>>);

    impl AttemptCloseRepository for FakeAttemptClose {
        fn publish_attempt_close(
            &self,
            publication: &AttemptClosePublication<'_>,
        ) -> Result<(), WorkError> {
            *self.0.borrow_mut() = Some((
                publication.attempt_after.to_vec(),
                publication.index_after.to_vec(),
            ));
            Ok(())
        }
    }

    impl RecordBeginRepository for FakeRecordBegin {
        fn publish_record_begin(
            &self,
            publication: &RecordBeginPublication<'_>,
        ) -> Result<(), WorkError> {
            assert_eq!(publication.record_id, "CMD-001");
            *self.0.borrow_mut() = Some(publication.index_after.to_vec());
            Ok(())
        }
    }

    #[test]
    fn record_begin_context_publishes_only_a_valid_reservation() {
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[{"id":"CMD-001"}],"validations":[],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":[],"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let index = json!({"acceptance_results":[],"schema":"work-execution-index","requirement_id":"demo",
            "title":"Execution","task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),"overall_status":"in_progress",
            "lock":build_execution_lock("TASK-001","ATTEMPT-001",&"e".repeat(64)),
            "tasks":[{"id":"TASK-001","status":"in_progress","skill_id":null,
                "task_item_sha256":"c".repeat(64),"acceptance_results":[],"instructions_sha256":"d".repeat(64),
                "latest_attempt":"ATTEMPT-001"}]});
        let task = json!({"id":"TASK-001","commands":[{"id":"CMD-001"}],
            "operations":[],"validations":[],"instruction_selection":{
                "selected_paths":[],"resolved_paths":[]}});
        let collection =
            json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001","tasks":[task.clone()]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"task_ids":["TASK-001"],
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)}});
        let attempt_raw = render_attempt(&attempt).unwrap();
        let index_raw = render_execution_index(&index).unwrap();
        let repository = FakeRecordBegin::default();
        let context = RecordBeginContext {
            collection: &collection,
            validation: &validation,
            task: &task,
            attempt: &attempt,
            attempt_raw: &attempt_raw,
            index: &index,
            index_raw: &index_raw,
            execution_dir: "execution",
            task_id: "TASK-001",
        };
        let result = begin_record_from_context(&repository, context, "CMD-001", None).unwrap();
        assert_eq!(result["record_id"], "CMD-001");
        let stored = repository.0.borrow();
        let stored = stored.as_ref().unwrap();
        let candidate = parse_json_contract(stored).unwrap();
        assert_eq!(candidate["lock"]["record_id"], "CMD-001");
        validate_execution_index(&candidate, stored).unwrap();
        assert_eq!(
            begin_record_from_context(
                &repository,
                RecordBeginContext {
                    collection: &collection,
                    validation: &validation,
                    task: &task,
                    attempt: &attempt,
                    attempt_raw: &attempt_raw,
                    index: &index,
                    index_raw: &index_raw,
                    execution_dir: "execution",
                    task_id: "TASK-001"
                },
                "CMD-001",
                Some("Unneeded")
            )
            .unwrap_err()
            .reason_code,
            "execution_authorization_unexpected_retry_evidence"
        );
        let mut stale = validation.clone();
        stale["task_index_sha256"] = json!("9".repeat(64));
        assert_eq!(
            begin_record_from_context(
                &repository,
                RecordBeginContext {
                    collection: &collection,
                    validation: &stale,
                    task: &task,
                    attempt: &attempt,
                    attempt_raw: &attempt_raw,
                    index: &index,
                    index_raw: &index_raw,
                    execution_dir: "execution",
                    task_id: "TASK-001",
                },
                "CMD-001",
                None
            )
            .unwrap_err()
            .reason_code,
            "record_begin_index_identity_mismatch"
        );
        let close_repository = FakeAttemptClose::default();
        let close_request = json!({"schema":"work-attempt-close-request","status":"completed"});
        let selection = json!({"selected_paths":[],"resolved_paths":[],
            "instructions_sha256":"e".repeat(64)});
        let closed = close_attempt_from_context(
            &close_repository,
            RecordBeginContext {
                collection: &collection,
                validation: &validation,
                task: &task,
                attempt: &attempt,
                attempt_raw: &attempt_raw,
                index: &index,
                index_raw: &index_raw,
                execution_dir: "execution",
                task_id: "TASK-001",
            },
            &close_request,
            "2026-09-01T10:10+08:00",
            &selection,
        )
        .unwrap();
        assert_eq!(closed["task_status"], "completed");
        assert_eq!(closed["lock_status"], "released");
        let stored = close_repository.0.borrow();
        let (attempt_after, index_after) = stored.as_ref().unwrap();
        let closed_attempt = parse_json_contract(attempt_after).unwrap();
        let closed_index = parse_json_contract(index_after).unwrap();
        validate_attempt_bytes(&closed_attempt, attempt_after).unwrap();
        validate_execution_index(&closed_index, index_after).unwrap();
        assert_eq!(closed_attempt["status"], "completed");
        assert!(closed_index.get("lock").is_none());
        let proposal = json!({"schema":"work-execution-deviation-proposal","task_id":"TASK-001","attempt_id":"ATTEMPT-001","anchor_record_id":"CMD-001","task_basis":["CMD-001"],"gap":"The acceptance boundary requires coordinated revision.","action":{"kind":"skip_record","record_id":"CMD-001","reason":"Stop for specification revision."},"modifiable_files":[],"impact":{"summary":"Acceptance changed.","requirement_changed":false,"scope_changed":false,"acceptance_criteria_changed":true,"deliverables_changed":false,"safety_boundary_changed":false,"external_side_effect_boundary_changed":false},"side_effects":[]});
        let preview =
            build_deviation_preview(&proposal, "command", &std::collections::BTreeMap::new())
                .unwrap();
        let (deviated, response) = append_approved_deviation(
            &attempt,
            &preview,
            preview["preview_sha256"].as_str().unwrap(),
            "Approved specification deviation",
            "execution",
        )
        .unwrap();
        assert_eq!(response["classification"], "task_and_execution");
        let deviated_raw = render_attempt(&deviated).unwrap();
        let mut reserved = index.clone();
        reserved["lock"]["record_id"] = json!("CMD-001");
        let reserved_raw = render_execution_index(&reserved).unwrap();
        let stopped = json!({"schema":"work-attempt-close-request","status":"stopped","final_type":"specification_defect","reason":"Return to Task for coordinated revision.","authorization_evidence":"Approved specification close"});
        let sink = FakeAttemptClose::default();
        let context = || RecordBeginContext {
            collection: &collection,
            validation: &validation,
            task: &task,
            attempt: &deviated,
            attempt_raw: &deviated_raw,
            index: &reserved,
            index_raw: &reserved_raw,
            execution_dir: "execution",
            task_id: "TASK-001",
        };
        let mut unrelated = stopped.clone();
        unrelated["final_type"] = json!("user_stopped");
        assert_eq!(
            close_attempt_from_context(
                &sink,
                context(),
                &unrelated,
                "2026-09-01T10:10+08:00",
                &selection
            )
            .unwrap_err()
            .reason_code,
            "attempt_close_record_reserved"
        );
        assert!(sink.0.borrow().is_none());
        let closed = close_attempt_from_context(
            &sink,
            context(),
            &stopped,
            "2026-09-01T10:10+08:00",
            &selection,
        )
        .unwrap();
        assert_eq!(closed["attempt_status"], "stopped");
        assert_eq!(closed["task_status"], "blocked");
        assert_eq!(closed["lock_status"], "released");
        assert_eq!(
            closed["pending_deviations"],
            json!([{"deviation_id":"DEVIATION-001","classification":"task_and_execution","blocking":true}])
        );
        let stored = sink.0.borrow();
        let (after, index_after) = stored.as_ref().unwrap();
        let after = parse_json_contract(after).unwrap();
        assert_eq!(after["final_type"], "specification_defect");
        assert_eq!(
            after["execution_deviations"],
            deviated["execution_deviations"]
        );
        let index_after = parse_json_contract(index_after).unwrap();
        assert!(index_after.get("lock").is_none());
    }

    impl AttemptStartRepository for FakeRetryStart {
        fn attempt_names(
            &self,
            _execution_dir: &str,
            _task_id: &str,
        ) -> Result<Vec<String>, WorkError> {
            Ok(vec!["ATTEMPT-001".into()])
        }

        fn read_attempt(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            assert!(path.ends_with("/TASK-001/ATTEMPT-001/attempt.json"));
            Ok(self.source.clone())
        }

        fn publish_attempt_start(
            &self,
            publication: &AttemptStartPublication<'_>,
        ) -> Result<(), WorkError> {
            assert_eq!(publication.attempt_id, "ATTEMPT-002");
            *self.stored.borrow_mut() = Some(publication.attempt.to_vec());
            Ok(())
        }
    }

    impl AttemptStartRepository for FakeAttemptStart {
        fn attempt_names(
            &self,
            _execution_dir: &str,
            _task_id: &str,
        ) -> Result<Vec<String>, WorkError> {
            Ok(Vec::new())
        }

        fn read_attempt(&self, _relative_path: &str) -> Result<Vec<u8>, WorkError> {
            unreachable!("initial Attempt has no source")
        }

        fn publish_attempt_start(
            &self,
            publication: &AttemptStartPublication<'_>,
        ) -> Result<(), WorkError> {
            assert_eq!(publication.attempt_id, "ATTEMPT-001");
            let locked = parse_json_contract(publication.locked_index).unwrap();
            let started = parse_json_contract(publication.started_index).unwrap();
            assert_eq!(locked["lock"]["attempt_id"], "ATTEMPT-001");
            assert_eq!(started["tasks"][0]["status"], "in_progress");
            assert_eq!(started["tasks"][0]["latest_attempt"], "ATTEMPT-001");
            *self.0.borrow_mut() = Some(publication.attempt.to_vec());
            Ok(())
        }
    }

    impl ExecutionIndexRepository for FakePaths {
        fn read_index(&self, _relative_path: &str) -> Result<Vec<u8>, WorkError> {
            Ok(self.raw.clone())
        }

        fn inspect_path(&self, relative_path: &str) -> Result<FileState, WorkError> {
            Ok(self
                .states
                .get(relative_path)
                .cloned()
                .unwrap_or(FileState {
                    identity: relative_path.into(),
                    exists: false,
                }))
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
            _execution_dir: &str,
            _task_id: &str,
        ) -> Result<(), WorkError> {
            Ok(())
        }
    }

    #[test]
    fn initial_index_and_preflight_share_formal_identity() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let c = "c".repeat(64);
        let d = "d".repeat(64);
        let e = "e".repeat(64);
        let f = "f".repeat(64);
        let z = "0".repeat(64);
        let validation = json!({"collection_contract":{"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "artifacts":{"execution":".work/demo/execution"},
            "tasks":[{"id":"TASK-001","dependencies":[],"inputs":[]}]},
            "task_ids":["TASK-001"],"task_instructions_sha256":{"TASK-001":a},
            "task_skill_ids":{"TASK-001":null},"task_item_sha256":{"TASK-001":b},
            "instructions_sha256":c,"hierarchy_selection_sha256":d,"skill_selection_sha256":e,
            "task_collection_sha256":f,"task_index_sha256":z});
        let (_, raw) = prepare_initial_index(&validation).unwrap();
        let ready = preflight_index(
            &FakeIndex(raw),
            &validation,
            ".work/demo/execution",
            "TASK-001",
            &[],
            None,
            &["pending", "pending_retry"],
        )
        .unwrap();
        assert_eq!(ready["index"]["overall_status"], "pending");
        assert_eq!(ready["input_readiness"], json!([]));
        assert_eq!(ready["file_readiness"], json!([]));

        let mut with_sources = validation;
        with_sources["collection_contract"]["tasks"][0]["inputs"] = json!([
            {"id":"INPUT-001","kind":"project_state","source":"src/Straße.txt"},
            {"id":"INPUT-002","kind":"external"}]);
        with_sources["collection_contract"]["tasks"][0]["files"] = json!([
            {"id":"FILE-001","action":"modify","path":"src/Straße.txt"},
            {"id":"FILE-002","action":"create","path":"dst/output.txt"}]);
        let (_, raw) = prepare_initial_index(&with_sources).unwrap();
        let states = HashMap::from([
            (
                "src/Straße.txt".into(),
                FileState {
                    identity: "src/strasse.txt".into(),
                    exists: true,
                },
            ),
            (
                "dst/output.txt".into(),
                FileState {
                    identity: "dst/output.txt".into(),
                    exists: false,
                },
            ),
        ]);
        let repository = FakePaths { raw, states };
        let confirmed = vec!["TASK-001/INPUT-002".into()];
        let ready = preflight_index(
            &repository,
            &with_sources,
            ".work/demo/execution",
            "TASK-001",
            &confirmed,
            None,
            &["pending"],
        )
        .unwrap();
        assert_eq!(
            ready["input_readiness"][0]["resolved_source"],
            "src/Straße.txt"
        );
        assert_eq!(ready["file_readiness"][1]["status"], "ready");
        let mut collision = with_sources;
        collision["collection_contract"]["tasks"][0]["files"][1]["path"] = json!("src/STRASSE.txt");
        let (_, raw) = prepare_initial_index(&collision).unwrap();
        let repository = FakePaths {
            raw,
            states: HashMap::from([
                (
                    "src/Straße.txt".into(),
                    FileState {
                        identity: "src/strasse.txt".into(),
                        exists: true,
                    },
                ),
                (
                    "src/STRASSE.txt".into(),
                    FileState {
                        identity: "src/strasse.txt".into(),
                        exists: false,
                    },
                ),
            ]),
        };
        assert_eq!(
            preflight_index(
                &repository,
                &collision,
                ".work/demo/execution",
                "TASK-001",
                &confirmed,
                None,
                &["pending"]
            )
            .unwrap_err()
            .reason_code,
            "execute_preflight_create_target_exists"
        );
    }

    #[test]
    fn complete_preflight_selects_execute_references_and_source_fingerprints() {
        let source = json!({"kind":"instruction","logical_name":"task.general",
            "canonical_sha256":"a".repeat(64)});
        let validation = json!({"collection_contract":{"requirement_id":"demo",
            "skill_selection":{"skills":[]},"spec_id":"TASK-SPEC-001","artifacts":{"task":"task/index.json","execution":"execution"},
            "tasks":[{"id":"TASK-001","skill_id":null,"dependencies":[],"inputs":[],"files":[],
                "instruction_selection":{"selected_paths":[],"resolved_paths":["general"],
                    "sources":[source],"references":[],"instructions_sha256":"d".repeat(64)}}]},
            "task_ids":["TASK-001"],"task_instructions_sha256":{"TASK-001":"d".repeat(64)},
            "task_skill_ids":{"TASK-001":null},"task_item_sha256":{"TASK-001":"c".repeat(64)},
            "instructions_sha256":"d".repeat(64),"hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64)});
        let mut validation = validation;
        validation["skill_selection_sha256"] = json!(skill_selection_sha256("base_only", &[]));
        validation["collection_contract"]["skill_selection"]["selection_sha256"] =
            validation["skill_selection_sha256"].clone();
        let (_, raw) = prepare_initial_index(&validation).unwrap();
        let repository = FakeIndex(raw);
        assert_eq!(
            prepare_execute_preflight(
                &repository,
                &FakeInstruction,
                ExecutePreflightRequest {
                    validation: &validation,

                    task_path: "other/index.json",
                    execution_dir: "execution",
                    task_id: "TASK-001",
                    confirmed_inputs: &[]
                }
            )
            .unwrap_err()
            .reason_code,
            "execute_preflight_artifact_path_mismatch"
        );
        let mut result = prepare_execute_preflight(
            &repository,
            &FakeInstruction,
            ExecutePreflightRequest {
                validation: &validation,

                task_path: "task/index.json",
                execution_dir: "execution",
                task_id: "TASK-001",
                confirmed_inputs: &[],
            },
        )
        .unwrap();
        assert_eq!(result["execute_instructions_sha256"], "e".repeat(64));
        assert_eq!(
            result["task_collection_sha256"],
            validation["task_collection_sha256"]
        );
        assert_eq!(
            result["task_item_sha256"],
            validation["task_item_sha256"]["TASK-001"]
        );
        assert!(result.get("task_sha256").is_none());
        assert_eq!(
            result["execute_instruction_selection"]["references"],
            json!(["execute.general.execution-records"])
        );
        assert_eq!(result["execute_skill_selection"]["decision"], "base_only");
        assert_eq!(result["eligibility"], "passed");
        let selected_skill = json!({"id":"repo:backend","name":"backend",
            "summary_sha256":"1".repeat(64),"bundle_sha256":"2".repeat(64),"mode_support":{"execute":"supported"},"dependency_status":"available"});
        let mut skilled_validation = validation.clone();
        skilled_validation["collection_contract"]["tasks"][0]["skill_id"] = json!("repo:backend");
        skilled_validation["task_skill_ids"]["TASK-001"] = json!("repo:backend");
        skilled_validation["collection_contract"]["skill_selection"] =
            json!({"skills":[selected_skill]});
        skilled_validation["skill_selection_sha256"] = json!(skill_selection_sha256(
            "external_skills",
            skilled_validation["collection_contract"]["skill_selection"]["skills"]
                .as_array()
                .unwrap(),
        ));
        skilled_validation["collection_contract"]["skill_selection"]["selection_sha256"] =
            skilled_validation["skill_selection_sha256"].clone();
        let (_, skilled_raw) = prepare_initial_index(&skilled_validation).unwrap();
        let skilled = prepare_execute_preflight(
            &FakeIndex(skilled_raw),
            &FakeInstruction,
            ExecutePreflightRequest {
                validation: &skilled_validation,

                task_path: "task/index.json",
                execution_dir: "execution",
                task_id: "TASK-001",
                confirmed_inputs: &[],
            },
        )
        .unwrap();
        for kind in ["missing", "unsupported", "unavailable"] {
            let mut invalid = skilled_validation.clone();
            match kind {
                "missing" => {
                    invalid["collection_contract"]["skill_selection"]["skills"] = json!([])
                }
                "unsupported" => {
                    invalid["collection_contract"]["skill_selection"]["skills"][0]["mode_support"]
                        ["execute"] = json!("unsupported")
                }
                _ => {
                    invalid["collection_contract"]["skill_selection"]["skills"][0]["dependency_status"] =
                        json!("unavailable")
                }
            }
            let (_, raw) = prepare_initial_index(&invalid).unwrap();
            assert_eq!(
                prepare_execute_preflight(
                    &FakeIndex(raw),
                    &FakeInstruction,
                    ExecutePreflightRequest {
                        validation: &invalid,
                        task_path: "task/index.json",
                        execution_dir: "execution",
                        task_id: "TASK-001",
                        confirmed_inputs: &[]
                    }
                )
                .unwrap_err()
                .reason_code,
                "execute_preflight_skill_binding_mismatch",
                "{kind}"
            );
        }
        assert_eq!(skilled["eligibility"], "passed");
        assert_eq!(skilled["skill_id"], "repo:backend");
        assert_eq!(
            skilled["execute_skill_selection"]["decision"],
            "external_skills"
        );
        assert_eq!(
            skilled["execute_skill_selection"]["skills"],
            skilled_validation["collection_contract"]["skill_selection"]["skills"]
        );
        result["snapshot_sha256"] = json!("0".repeat(64));
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[],"validations":[],"modifiable_files":[],
            "working_directories":[],"external_operations":[],"allowed_deviations":[],
            "reapproval_conditions":["scope_expansion","source_or_worktree_drift",
                "failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let request = json!({"schema":"work-attempt-start-request",
            "worktree_snapshot_sha256":"0".repeat(64),"authorization":authorization});
        let sink = FakeAttemptStart::default();
        let index = parse_json_contract(&repository.0).unwrap();
        let response = start_attempt_from_preflight(
            &sink,
            &result,
            &index,
            &repository.0,
            &request,
            "2026-09-01T10:00+08:00",
        )
        .unwrap();
        assert_eq!(response["attempt_id"], "ATTEMPT-001");
        assert_eq!(response["status"], "started");
        assert_eq!(response["lock_status"], "held");
        assert!(sink.0.borrow().is_some());
        let mut source = parse_json_contract(sink.0.borrow().as_ref().unwrap()).unwrap();
        source["status"] = json!("stopped");
        source["final_type"] = json!("other");
        source["reason"] = json!("Retry is required.");
        source["closing_authorization_evidence"] = json!("Approved close");
        source["ended_at"] = json!("2026-09-01T10:05+08:00");
        let source_raw = render_attempt(&source).unwrap();
        let mut retry_index = index.clone();
        retry_index["tasks"][0]["status"] = json!("pending_retry");
        retry_index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        retry_index["tasks"][0]["status_reason"] = json!({"kind":"attempt","ref":"ATTEMPT-001"});
        let retry_raw = render_execution_index(&retry_index).unwrap();
        validate_execution_index(&retry_index, &retry_raw).unwrap();
        let retry_selection = prepare_execute_preflight(
            &FakeIndex(retry_raw.clone()),
            &FakeInstruction,
            ExecutePreflightRequest {
                validation: &validation,

                task_path: "task/index.json",
                execution_dir: "execution",
                task_id: "TASK-001",
                confirmed_inputs: &[],
            },
        )
        .unwrap();
        assert_eq!(
            retry_selection["execute_instruction_selection"]["references"],
            json!([
                "execute.general.execution-records",
                "execute.general.execution-recovery"
            ])
        );
        let mut retry_preflight = result.clone();
        retry_preflight["task_status"] = json!("pending_retry");
        retry_preflight["index_sha256"] = json!(sha256_hex(&retry_raw));
        let retry = FakeRetryStart {
            source: source_raw,
            stored: RefCell::new(None),
        };
        let retry_request = json!({"schema":"work-attempt-start-request",
            "worktree_snapshot_sha256":"0".repeat(64),
            "authorization":request["authorization"],
            "continuation":{"source_attempt_id":"ATTEMPT-001","carried_records":[]}});
        let retry_result = start_attempt_from_preflight(
            &retry,
            &retry_preflight,
            &retry_index,
            &retry_raw,
            &retry_request,
            "2026-09-01T10:10+08:00",
        )
        .unwrap();
        assert_eq!(retry_result["attempt_id"], "ATTEMPT-002");
        assert!(retry.stored.borrow().is_some());
        let mut changed = request;
        changed["worktree_snapshot_sha256"] = json!("1".repeat(64));
        assert_eq!(
            start_attempt_from_preflight(
                &sink,
                &result,
                &index,
                &repository.0,
                &changed,
                "2026-09-01T10:00+08:00",
            )
            .unwrap_err()
            .reason_code,
            "attempt_start_worktree_snapshot_changed"
        );
    }

    #[test]
    fn record_identity_drift_maps_to_artifact_integrity_exit() {
        for reason_code in [
            "record_begin_index_identity_mismatch",
            "record_begin_index_task_set_mismatch",
            "record_begin_task_instructions_mismatch",
            "record_begin_attempt_identity_mismatch",
        ] {
            let error = rule(ExecutionIssue {
                reason_code,
                message: "Identity changed.",
                details: json!({}),
            });
            assert_eq!(error.exit_code, ExitCode::ArtifactIntegrity);
        }
    }

    #[test]
    fn missing_record_task_keeps_current_contract_workflow_exit_and_details() {
        let error = rule(ExecutionIssue {
            reason_code: "record_begin_task_not_found",
            message: "The requested TASK is not present in the execution index.",
            details: json!({"task_id":"TASK-002"}),
        });
        assert_eq!(error.exit_code, ExitCode::WorkflowState);
        assert_eq!(error.exit_code as i32, 6);
        assert_eq!(error.reason_code, "record_begin_task_not_found");
        assert_eq!(error.details, json!({"task_id":"TASK-002"}));
    }
}
