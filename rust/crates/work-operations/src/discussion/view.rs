//! Decision progress and minimal committed-state views.

use std::collections::BTreeSet;

use serde::Serialize;
use work_model::common::Nullable;
use work_model::discussion::*;

use super::Result;
use super::validation::verify_integrity;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DecisionProgress {
    pub confirmed: usize,
    pub pending: usize,
    pub deferred: usize,
    pub blocked: usize,
    pub needs_review: usize,
    pub withdrawn: usize,
    pub known_total: usize,
    pub remaining: usize,
}

pub fn progress(session: &DiscussionSession) -> DecisionProgress {
    let mut p = DecisionProgress::default();
    for decision in &session.decisions {
        match decision.status {
            DecisionStatus::Confirmed => p.confirmed += 1,
            DecisionStatus::Pending => p.pending += 1,
            DecisionStatus::Deferred => p.deferred += 1,
            DecisionStatus::Blocked => p.blocked += 1,
            DecisionStatus::NeedsReview => p.needs_review += 1,
            DecisionStatus::Withdrawn => p.withdrawn += 1,
        }
    }
    p.remaining = p.pending + p.deferred + p.blocked + p.needs_review;
    p.known_total = p.confirmed + p.remaining;
    p
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscussionView {
    pub requirement_id: String,
    pub goal: String,
    pub scope: Vec<String>,
    pub constraints: Vec<String>,
    pub discussion_level: String,
    pub task_count: usize,
    pub current_task_title: Option<String>,
    pub current_task_position: Option<usize>,
    pub revision: u64,
    pub content_sha256: String,
    pub progress: DecisionProgress,
    pub continuation: Continuation,
    pub pending_decision_ids: Vec<String>,
    pub decisions: Vec<DecisionSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionSummary {
    pub id: String,
    pub question: String,
    pub status: DecisionStatus,
    pub dependencies: Vec<String>,
    pub task_ids: Vec<String>,
    pub options: Vec<DecisionOption>,
}

pub fn view(session: &DiscussionSession) -> Result<DiscussionView> {
    verify_integrity(session)?;
    let mut needed: BTreeSet<String> = session
        .decisions
        .iter()
        .filter(|d| {
            !matches!(
                d.status,
                DecisionStatus::Confirmed | DecisionStatus::Withdrawn
            )
        })
        .map(|d| d.id.clone())
        .collect();
    if let Nullable::Value(q) = &session.continuation.question {
        needed.insert(q.decision_id.clone());
    }
    loop {
        let before = needed.len();
        for d in &session.decisions {
            if needed.contains(&d.id) {
                needed.extend(d.dependencies.iter().cloned());
            }
        }
        if needed.len() == before {
            break;
        }
    }
    Ok(DiscussionView {
        requirement_id: session.requirement_id.clone(),
        goal: session.context.goal.clone(),
        scope: session.context.scope.clone(),
        constraints: session.context.constraints.clone(),
        discussion_level: if session.continuation.current_task_id == Nullable::Null { "requirement" } else { "task" }.into(),
        task_count: session.tasks.len(),
        current_task_title: session.tasks.iter().find(|t| session.continuation.current_task_id == Nullable::Value(t.id.clone())).map(|t|t.title.clone()),
        current_task_position: session.tasks.iter().position(|t| session.continuation.current_task_id == Nullable::Value(t.id.clone())).map(|n|n+1),
        revision: session.revision,
        content_sha256: session.commit.content_sha256.clone(),
        progress: progress(session),
        continuation: session.continuation.clone(),
        pending_decision_ids: session
            .decisions
            .iter()
            .filter(|d| {
                !matches!(
                    d.status,
                    DecisionStatus::Confirmed | DecisionStatus::Withdrawn
                )
            })
            .map(|d| d.id.clone())
            .collect(),
        decisions: session.decisions.iter().filter(|d| needed.contains(&d.id))
            .map(|d| DecisionSummary { id: d.id.clone(), question: d.question.clone(), status: d.status,
                dependencies: d.dependencies.clone(), task_ids: d.task_ids.clone(),
                options: if matches!(&session.continuation.question, Nullable::Value(q) if q.decision_id == d.id) {
                    d.options.clone()
                } else { vec![] } }).collect(),
    })
}
