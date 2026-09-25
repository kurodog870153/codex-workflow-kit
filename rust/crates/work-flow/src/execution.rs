//! Execution command flows.

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::execution::document;
use work_feature::execution::{
    AttemptCloseRepository, AttemptStartRepository, CommandCorrectionRepository,
    CommandPrepareRepository, CorrectionRepository, DeviationRecordRepository,
    ExecutionIndexRepository, ExecutionWorktreeRepository, RecordBeginRepository,
    RecordFinishRepository, RecoveryPrepareRepository,
};
use work_feature::instruction::InstructionSourceRepository;
use work_feature::plan::PlanPathRepository;
use work_feature::skill::SkillSnapshotRepository;
use work_feature::task::TaskCollectionRepository;

pub use work_feature::execution::{
    CommandProjectRequest, CommandProjectSources, ExecutionProjectTarget,
};
use work_feature::execution::{
    DeviationProjectRequest, begin_record_from_project, close_attempt_from_project,
    create_correction_from_project, finish_record_from_project, inspect_worktree_from_project,
    prepare_attempt_start_from_project, prepare_command_from_project,
    prepare_execute_preflight_from_project, prepare_recovery_from_project,
    prepare_semantic_deviation_from_project, record_command_correction_from_project,
    record_deviation_from_project, start_attempt_from_project,
};

fn required(value: Option<&str>) -> Result<&str, WorkError> {
    value.ok_or_else(|| {
        WorkError::new(
            ExitCode::InternalError,
            "unreachable_command",
            "The parsed command could not be dispatched.",
            json!({}),
        )
    })
}

pub struct ExecutionCommandInput<'a> {
    pub command: &'a str,
    pub target: ExecutionProjectTarget<'a>,
    pub confirmed: &'a [String],
    pub record_id: Option<&'a str>,
    pub approved_sha256: Option<&'a str>,
    pub authorization_evidence: Option<&'a str>,
}

pub struct ExecutionTechnical<Start, Recover, Command> {
    pub recover_start: Start,
    pub recover: Recover,
    pub command_run: Command,
}

pub fn run<H, S, P, T, E>(
    input: ExecutionCommandInput<'_>,
    sources: &CommandProjectSources<'_, H, S, P, T>,
    repository: &E,
    mut request: impl FnMut() -> Result<Value, WorkError>,
    mut timestamp: impl FnMut() -> String,
    technical: ExecutionTechnical<
        impl FnOnce(Value, String) -> Result<Value, WorkError>,
        impl FnOnce(Value) -> Result<Value, WorkError>,
        impl FnOnce(Value, String) -> Result<Value, WorkError>,
    >,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
    T: TaskCollectionRepository,
    E: ExecutionIndexRepository
        + ExecutionWorktreeRepository
        + AttemptStartRepository
        + RecoveryPrepareRepository
        + CorrectionRepository
        + RecordBeginRepository
        + RecordFinishRepository
        + AttemptCloseRepository
        + CommandCorrectionRepository
        + CommandPrepareRepository
        + DeviationRecordRepository,
{
    let ExecutionCommandInput {
        command,
        target,
        confirmed,
        record_id,
        approved_sha256,
        authorization_evidence,
    } = input;
    let ExecutionTechnical {
        recover_start,
        recover,
        command_run,
    } = technical;
    match command {
        "preflight" => {
            prepare_execute_preflight_from_project(sources, repository, target, confirmed)
        }
        "worktree" => inspect_worktree_from_project(sources, repository, target, confirmed),
        "attempt-start-prepare" => {
            prepare_attempt_start_from_project(sources, repository, target, confirmed, &request()?)
        }
        "attempt-start" => start_attempt_from_project(
            sources,
            repository,
            target,
            confirmed,
            &request()?,
            &timestamp(),
        ),
        "recover-attempt-start" => recover_start(request()?, timestamp()),
        "recovery-prepare" => {
            prepare_recovery_from_project(sources, repository, target, &request()?)
        }
        "command-prepare" => {
            let value = request()?;
            prepare_command_from_project(
                sources,
                repository,
                CommandProjectRequest {
                    task_path: target.task_path,
                    execution_dir: target.execution_dir,
                    task_id: target.task_id,
                    request: &value,
                },
            )
        }
        "deviation-prepare-semantic" => {
            prepare_semantic_deviation_from_project(sources, repository, target, &request()?)
        }
        "record-begin" => begin_record_from_project(
            sources,
            repository,
            target,
            work_feature::execution::validate_record_id(required(record_id)?)?,
            authorization_evidence,
        ),
        "record-finish" => finish_record_from_project(sources, repository, target, &request()?),
        "attempt-close" => {
            close_attempt_from_project(sources, repository, target, &request()?, &timestamp())
        }
        "correction-create" => {
            create_correction_from_project(sources, repository, target, &request()?, &timestamp())
        }
        "command-correction" => {
            record_command_correction_from_project(sources, repository, target, &request()?)
        }
        "recover" => recover(request()?),
        "command-run" => command_run(request()?, required(approved_sha256)?.to_owned()),
        "deviation-record" => {
            let proposal = request()?;
            record_deviation_from_project(
                sources,
                repository,
                DeviationProjectRequest {
                    task_path: target.task_path,
                    execution_dir: target.execution_dir,
                    task_id: target.task_id,
                    proposal: &proposal,
                },
                required(approved_sha256)?,
                required(authorization_evidence)?,
            )
        }
        _ => Err(WorkError::new(
            ExitCode::InternalError,
            "unreachable_command",
            "The parsed command could not be dispatched.",
            json!({}),
        )),
    }
}

pub fn attempt_render(value: Value) -> Result<Value, WorkError> {
    document::render_attempt_document(value)
}

pub fn attempt_validate(
    value: &Value,
    raw: Option<&[u8]>,
    path: Option<&str>,
) -> Result<Value, WorkError> {
    document::validate_attempt_document(value, raw, path)
}

pub fn correction_render(value: Value) -> Result<Value, WorkError> {
    document::render_correction_document(value)
}

pub fn correction_validate(value: &Value, path: Option<&str>) -> Result<Value, WorkError> {
    document::validate_correction_document(value, path)
}

pub fn ordered_attempt(value: &Value) -> document::OrderedAttempt<'_> {
    document::OrderedAttempt(value)
}

pub fn ordered_correction(value: &Value) -> document::OrderedCorrection<'_> {
    document::OrderedCorrection(value)
}
