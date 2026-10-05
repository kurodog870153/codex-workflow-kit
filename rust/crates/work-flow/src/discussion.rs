//! Internal discussion flows; public dispatch is enabled at the replacement boundary.

pub use work_feature::discussion::repository::DiscussionRepository;
use work_feature::discussion::{DiscussionOperation, DiscussionView, SavedDiscussion};
use work_feature::error::WorkError;
use work_model::discussion::{DiscussionSession, SavedQuestion};

pub fn dispatch(
    repository: &impl DiscussionRepository,
    publication: &impl DiscussionPublicationRepository,
    request: &work_model::discussion::request::DiscussionRequest,
) -> Result<serde_json::Value, WorkError> {
    work_feature::discussion::request::dispatch(repository, publication, request)
}

pub fn context(
    repository: &impl DiscussionRepository,
    request: &work_model::discussion::request::DiscussionRequest,
) -> Result<serde_json::Value, WorkError> {
    work_feature::discussion::request::context(repository, request)
}

pub fn verify_context(
    repository: &impl DiscussionRepository,
    request: &work_model::discussion::request::DiscussionRequest,
    context: &serde_json::Value,
) -> Result<(), WorkError> {
    work_feature::discussion::request::verify_context(repository, request, context)
}

pub fn resume(
    repository: &impl DiscussionRepository,
    requirement: &str,
) -> Result<serde_json::Value, WorkError> {
    work_feature::discussion::request::resume(repository, requirement)
}

pub fn command_definition(name: &str) -> Option<serde_json::Value> {
    work_feature::discussion::request::command_definition(name)
}

pub use work_feature::discussion::assembly::{DiscussionPublicationRepository, PublicationRequest};

pub fn preview(
    repository: &impl DiscussionPublicationRepository,
    requirement: &str,
    metadata: &serde_json::Value,
) -> Result<serde_json::Value, WorkError> {
    repository.preview(requirement, metadata)
}

pub fn publish(
    repository: &impl DiscussionPublicationRepository,
    request: PublicationRequest<'_>,
) -> Result<serde_json::Value, WorkError> {
    repository.publish(request)
}

pub fn read(
    repository: &impl DiscussionRepository,
    requirement: &str,
) -> Result<DiscussionView, WorkError> {
    work_feature::discussion::read(repository, requirement)
}

pub fn initialize(
    repository: &impl DiscussionRepository,
    session: &DiscussionSession,
    recover: bool,
) -> Result<SavedDiscussion, WorkError> {
    work_feature::discussion::initialize(repository, session, recover)
}

pub fn submit(
    repository: &impl DiscussionRepository,
    requirement: &str,
    operation: &DiscussionOperation,
    recover: bool,
) -> Result<SavedDiscussion, WorkError> {
    work_feature::discussion::submit(repository, requirement, operation, recover)
}

pub fn prepare_question(
    repository: &impl DiscussionRepository,
    requirement: &str,
    operation_id: &str,
    question: SavedQuestion,
) -> Result<SavedDiscussion, WorkError> {
    work_feature::discussion::prepare_question(repository, requirement, operation_id, question)
}
