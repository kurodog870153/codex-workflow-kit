//! Serializable local discussion commands.

use super::{
    DiscussionAction, DiscussionDecision, DiscussionTask, RequirementContext, SavedQuestion,
};
use crate::common::Nullable;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscussionChange {
    AddDecision {
        decision: DiscussionDecision,
    },
    UpdateDecision {
        decision: DiscussionDecision,
    },
    UpdatePlanning {
        task: Box<DiscussionTask>,
    },
    PrepareQuestion {
        question: SavedQuestion,
    },
    AnswerQuestion {
        decision_id: String,
        question_version: u64,
        display_number: String,
        rationale: String,
        evidence: String,
    },
    UpdateContext {
        context: Box<RequirementContext>,
    },
    UpdateContinuation {
        current_task_id: Nullable<String>,
    },
}

impl DiscussionChange {
    pub fn action(&self) -> DiscussionAction {
        match self {
            Self::AddDecision { .. } => DiscussionAction::AddDecision,
            Self::UpdateDecision { .. } => DiscussionAction::UpdateDecision,
            Self::UpdatePlanning { .. } => DiscussionAction::UpdatePlanning,
            Self::PrepareQuestion { .. } => DiscussionAction::PrepareQuestion,
            Self::AnswerQuestion { .. } => DiscussionAction::AnswerQuestion,
            Self::UpdateContext { .. } => DiscussionAction::UpdateContext,
            Self::UpdateContinuation { .. } => DiscussionAction::UpdateContinuation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionOperation {
    pub operation_id: String,
    pub expected_revision: u64,
    pub previous_sha256: String,
    pub change: DiscussionChange,
}
