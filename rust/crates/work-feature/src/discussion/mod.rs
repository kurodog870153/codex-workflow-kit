//! Discussion save, recovery, and repository orchestration.

pub mod assembly;
pub mod repository;
pub mod request;

use serde::Serialize;
use serde_json::json;
use work_model::discussion::{DiscussionDecision, DiscussionSession, SavedQuestion};
use work_operations::discussion::{self, DiscussionChange};
pub use work_operations::discussion::{DiscussionOperation, DiscussionView};

use crate::error::{ExitCode, WorkError};
use repository::DiscussionRepository;

fn missing() -> WorkError {
    WorkError::new(
        ExitCode::WorkflowState,
        "discussion_not_initialized",
        "Read or initialize an authorized discussion before continuing.",
        json!({"saved":false}),
    )
}

fn rejected(reason: &str) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        reason,
        "The discussion is not verified; do not advance to another question.",
        json!({"saved":false}),
    )
}

pub fn read(
    repository: &impl DiscussionRepository,
    requirement: &str,
) -> Result<DiscussionView, WorkError> {
    let session = repository.read_current(requirement)?.ok_or_else(missing)?;
    discussion::view(&session).map_err(|e| rejected(e.0))
}

#[derive(Debug, Serialize)]
pub struct SavedDiscussion {
    pub saved: bool,
    pub operation_revision: u64,
    pub view: DiscussionView,
    pub added_decision_id: Option<String>,
    pub added_reason: Option<String>,
    pub known_total_change: i64,
    pub remaining_change: i64,
}

pub fn initialize(
    repository: &impl DiscussionRepository,
    session: &DiscussionSession,
    recover: bool,
) -> Result<SavedDiscussion, WorkError> {
    discussion::verify_integrity(session).map_err(|e| rejected(e.0))?;
    let committed = repository.initialize(session, recover)?;
    let view = read(repository, &session.requirement_id)?;
    Ok(SavedDiscussion {
        saved: true,
        operation_revision: committed.revision,
        view,
        added_decision_id: None,
        added_reason: None,
        known_total_change: 0,
        remaining_change: 0,
    })
}

pub fn submit(
    repository: &impl DiscussionRepository,
    requirement: &str,
    operation: &DiscussionOperation,
    recover: bool,
) -> Result<SavedDiscussion, WorkError> {
    // Every continuation begins with the repository, including retry and recovery.
    let previous = repository.read_current(requirement)?.ok_or_else(missing)?;
    let before = discussion::progress(&previous);
    let annotate = |mut error: WorkError, revision: u64| {
        if !error.details.is_object() {
            error.details = json!({"cause":error.details});
        }
        error.details["saved"] = json!(false);
        error.details["last_verified_revision"] = json!(revision);
        error.details["operation_id"] = json!(operation.operation_id);
        error.details["check_or_recover_required"] = json!(true);
        error
    };
    let committed = repository
        .submit(requirement, operation, recover)
        .map_err(|error| annotate(error, previous.revision))?;
    discussion::verify_integrity(&committed).map_err(|e| rejected(e.0))?;
    if committed.commit.operation_id != operation.operation_id
        || committed.commit.operation_sha256 != discussion::operation_sha256(operation)
    {
        return Err(rejected("discussion_operation_readback_mismatch"));
    }
    let view =
        read(repository, requirement).map_err(|error| annotate(error, committed.revision))?;
    if let DiscussionChange::PrepareQuestion { .. } = &operation.change {
        if view.continuation.question != committed.continuation.question {
            return Err(rejected("discussion_question_changed_after_save"));
        }
    }
    let added = match &operation.change {
        DiscussionChange::AddDecision { decision } => Some(decision),
        _ => None,
    };
    let after = discussion::progress(&committed);
    let replayed = committed.revision <= previous.revision;
    Ok(SavedDiscussion {
        saved: true,
        operation_revision: committed.revision,
        view,
        added_decision_id: added.map(|d| d.id.clone()),
        added_reason: added.map(|d| d.added_reason.clone()),
        known_total_change: if replayed {
            0
        } else {
            after.known_total as i64 - before.known_total as i64
        },
        remaining_change: if replayed {
            0
        } else {
            after.remaining as i64 - before.remaining as i64
        },
    })
}

pub fn prepare_question(
    repository: &impl DiscussionRepository,
    requirement: &str,
    operation_id: &str,
    question: SavedQuestion,
) -> Result<SavedDiscussion, WorkError> {
    let session = repository.read_current(requirement)?.ok_or_else(missing)?;
    let operation = DiscussionOperation {
        operation_id: operation_id.into(),
        expected_revision: session.revision,
        previous_sha256: session.commit.content_sha256,
        change: DiscussionChange::PrepareQuestion { question },
    };
    submit(repository, requirement, &operation, false)
}

pub fn decision_detail(
    repository: &impl DiscussionRepository,
    requirement: &str,
    id: &str,
    revision: Option<u64>,
) -> Result<DiscussionDecision, WorkError> {
    let session = match revision {
        Some(revision) => repository.read_history(requirement, revision)?,
        None => repository.read_current(requirement)?.ok_or_else(missing)?,
    };
    discussion::verify_integrity(&session).map_err(|e| rejected(e.0))?;
    session
        .decisions
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| rejected("unknown_decision"))
}
