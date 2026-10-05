//! Pure discussion rules and views of verified committed state.

pub mod assembly;
mod update;
mod validation;
mod view;
pub mod workflow;

pub use update::{
    DiscussionChange, DiscussionOperation, allowed_transition, apply, operation_sha256,
};
pub use validation::validate_trace;
pub use validation::{ready_to_generate, validate, validate_authorization, verify_integrity};
pub use view::{DecisionProgress, DecisionSummary, DiscussionView, progress, view};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscussionIssue(pub &'static str);
type Result<T> = std::result::Result<T, DiscussionIssue>;
