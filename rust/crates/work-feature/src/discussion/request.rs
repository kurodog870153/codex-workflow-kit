//! Staged request orchestration using committed Session and publication ports.

use super::assembly::{DiscussionPublicationRepository, PublicationRequest};
use super::repository::DiscussionRepository;
use crate::error::WorkError;
use serde_json::{Value, json};
use work_model::common::Nullable;
use work_model::discussion::DiscussionChange;
use work_model::discussion::request::{DiscussionCommand, DiscussionRequest};
use work_operations::derivation::fingerprint;

pub fn dispatch(
    repository: &impl DiscussionRepository,
    publication: &impl DiscussionPublicationRepository,
    request: &DiscussionRequest,
) -> Result<Value, WorkError> {
    request
        .requirement_id
        .parse::<work_model::identifiers::RequirementId>()
        .map_err(|_| super::rejected("invalid_requirement_id"))?;
    let requirement = &request.requirement_id;
    let encode =
        |value| serde_json::to_value(value).map_err(|_| super::rejected("invalid_contract_value"));
    let data = match &request.command {
        DiscussionCommand::Init { session } => {
            if session.requirement_id != *requirement {
                return Err(super::rejected("discussion_request_scope_mismatch"));
            }
            encode(super::initialize(repository, session, false)?)?
        }
        DiscussionCommand::Read {
            decision_id: Some(id),
        } => serde_json::to_value(super::decision_detail(repository, requirement, id, None)?)
            .expect("decision serializes"),
        DiscussionCommand::Read { decision_id: None } | DiscussionCommand::Status {} => {
            serde_json::to_value(super::read(repository, requirement)?).expect("view serializes")
        }
        DiscussionCommand::History { revision } => {
            let session = repository.read_history(requirement, *revision)?;
            work_operations::discussion::verify_integrity(&session)
                .map_err(|e| super::rejected(e.0))?;
            serde_json::to_value(session).expect("session serializes")
        }
        DiscussionCommand::Update { operation }
        | DiscussionCommand::Question { operation }
        | DiscussionCommand::Answer { operation } => {
            let correct = match &request.command {
                DiscussionCommand::Question { .. } => {
                    matches!(operation.change, DiscussionChange::PrepareQuestion { .. })
                }
                DiscussionCommand::Answer { .. } => {
                    matches!(operation.change, DiscussionChange::AnswerQuestion { .. })
                }
                _ => !matches!(
                    operation.change,
                    DiscussionChange::PrepareQuestion { .. }
                        | DiscussionChange::AnswerQuestion { .. }
                ),
            };
            if !correct {
                return Err(super::rejected("discussion_command_action_mismatch"));
            }
            encode(super::submit(repository, requirement, operation, false)?)?
        }
        DiscussionCommand::Recover { session, operation } => match (session, operation) {
            (Nullable::Value(session), Nullable::Null)
                if session.requirement_id == *requirement =>
            {
                encode(super::initialize(repository, session, true)?)?
            }
            (Nullable::Null, Nullable::Value(operation)) => {
                encode(super::submit(repository, requirement, operation, true)?)?
            }
            _ => return Err(super::rejected("ambiguous_discussion_recovery")),
        },
        DiscussionCommand::Preview { metadata } => publication.preview(requirement, metadata)?,
        DiscussionCommand::Apply {
            expected_revision,
            session_sha256,
            metadata,
            approved_sha256,
            publication_evidence,
        }
        | DiscussionCommand::RecoverPublication {
            expected_revision,
            session_sha256,
            metadata,
            approved_sha256,
            publication_evidence,
        } => publication.publish(PublicationRequest {
            requirement_id: requirement,
            expected_revision: *expected_revision,
            session_sha256,
            metadata,
            approved_sha256,
            publication_evidence,
            recovery: matches!(
                request.command,
                DiscussionCommand::RecoverPublication { .. }
            ),
        })?,
    };
    serde_json::to_value(work_model::discussion::response::DiscussionResult {
        schema: work_model::discussion::response::DiscussionResultSchema::Result,
        command: request.command.name().into(),
        requirement_id: requirement.clone(),
        data,
    })
    .map_err(|_| super::rejected("invalid_contract_value"))
}

pub fn context(
    repository: &impl DiscussionRepository,
    request: &DiscussionRequest,
) -> Result<Value, WorkError> {
    let session = repository
        .read_current(&request.requirement_id)?
        .ok_or_else(super::missing)?;
    work_operations::discussion::verify_integrity(&session).map_err(|e| super::rejected(e.0))?;
    let request_sha256 = fingerprint::discussion_operation(
        &serde_json::to_value(request).expect("request serializes"),
    );
    let mut context = json!({"requirement_id":session.requirement_id,"revision":session.revision,
        "session_sha256":session.commit.content_sha256,"request_sha256":request_sha256,
        "session_path":format!("outputs/work/discussions/{}/session.json",session.requirement_id)});
    context["context_sha256"] = json!(fingerprint::discussion_operation(&context));
    Ok(context)
}

pub fn verify_context(
    repository: &impl DiscussionRepository,
    request: &DiscussionRequest,
    saved: &Value,
) -> Result<(), WorkError> {
    if context(repository, request)? != *saved {
        return Err(super::rejected("stale_discussion_context"));
    }
    Ok(())
}

pub fn resume(
    repository: &impl DiscussionRepository,
    requirement: &str,
) -> Result<Value, WorkError> {
    let session = repository
        .read_current(requirement)?
        .ok_or_else(super::missing)?;
    work_operations::discussion::workflow::resume(&session).map_err(|e| super::rejected(e.0))
}

pub fn command_definition(name: &str) -> Option<Value> {
    work_operations::discussion::workflow::command_definition(name)
}
