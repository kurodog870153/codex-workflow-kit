//! Session structure, authorization, integrity, and generation readiness.

use std::collections::{BTreeMap, BTreeSet};

use work_model::common::Nullable;
use work_model::discussion::*;

use super::{DiscussionIssue, Result, progress};
use crate::derivation::fingerprint;
use crate::protocol::valid_sha256;

fn number(id: &str, prefix: &str) -> Result<u64> {
    let digits = id
        .strip_prefix(prefix)
        .ok_or(DiscussionIssue("invalid_discussion_id"))?;
    let n = digits
        .parse::<u64>()
        .map_err(|_| DiscussionIssue("invalid_discussion_id"))?;
    if n == 0 || digits != format!("{n:03}") {
        return Err(DiscussionIssue("invalid_discussion_id"));
    }
    Ok(n)
}

pub(super) fn nonempty(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        Err(DiscussionIssue("missing_discussion_evidence"))
    } else {
        Ok(())
    }
}

fn unique<'a>(ids: impl Iterator<Item = &'a String>) -> Result<BTreeSet<&'a String>> {
    let mut set = BTreeSet::new();
    for id in ids {
        if !set.insert(id) {
            return Err(DiscussionIssue("duplicate_discussion_reference"));
        }
    }
    Ok(set)
}

fn graph(rows: &BTreeMap<String, Vec<String>>) -> Result<()> {
    fn visit(
        id: &str,
        rows: &BTreeMap<String, Vec<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        if !active.insert(id.into()) {
            return Err(DiscussionIssue("cyclic_discussion_dependency"));
        }
        for dep in rows
            .get(id)
            .ok_or(DiscussionIssue("dangling_discussion_reference"))?
        {
            visit(dep, rows, active, done)?;
        }
        active.remove(id);
        done.insert(id.into());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for id in rows.keys() {
        visit(id, rows, &mut BTreeSet::new(), &mut done)?;
    }
    Ok(())
}

pub fn validate(session: &DiscussionSession) -> Result<()> {
    session
        .requirement_id
        .parse::<work_model::identifiers::RequirementId>()
        .map_err(|_| DiscussionIssue("invalid_requirement_id"))?;
    if session.revision == 0
        || session.context.revision == 0
        || session.next_task_number == 0
        || session.next_decision_number == 0
    {
        return Err(DiscussionIssue("invalid_discussion_revision"));
    }
    validate_authorization(session, None)?;
    nonempty(&session.context.goal)?;
    if session.context.scope.is_empty() {
        return Err(DiscussionIssue("missing_discussion_scope"));
    }
    if let Nullable::Value(source) = &session.context.confirmed_source {
        crate::task::source::validate_planning_source(
            &serde_json::to_value(source).expect("source serializes"),
            &session.requirement_id,
        )
        .map_err(|e| DiscussionIssue(e.reason_code))?;
    }
    let tasks = unique(session.tasks.iter().map(|t| &t.id))?;
    let retired = unique(session.retired_task_ids.iter())?;
    if !tasks.is_disjoint(&retired) {
        return Err(DiscussionIssue("reused_discussion_id"));
    }
    for id in tasks.union(&retired) {
        if number(id, "TASK-")? >= session.next_task_number {
            return Err(DiscussionIssue("reused_discussion_id"));
        }
    }
    let decisions = unique(session.decisions.iter().map(|d| &d.id))?;
    for d in &session.decisions {
        if number(&d.id, "D")? >= session.next_decision_number || d.question_version == 0 {
            return Err(DiscussionIssue("reused_discussion_id"));
        }
        nonempty(&d.question)?;
        nonempty(&d.added_reason)?;
        let options = unique(d.options.iter().map(|o| &o.id))?;
        for o in &d.options {
            nonempty(&o.id)?;
            nonempty(&o.label)?;
            nonempty(&o.explanation)?;
        }
        if let Nullable::Value(id) = &d.tentative_option_id {
            if !options.contains(id) {
                return Err(DiscussionIssue("invalid_decision_option"));
            }
        }
        if let Nullable::Value(r) = &d.resolution {
            if !options.contains(&r.option_id) {
                return Err(DiscussionIssue("invalid_decision_option"));
            }
            nonempty(&r.rationale)?;
            nonempty(&r.confirmation_evidence)?;
        } else if matches!(
            d.status,
            DecisionStatus::Confirmed | DecisionStatus::NeedsReview
        ) {
            return Err(DiscussionIssue("decision_confirmation_required"));
        }
        if matches!(
            d.status,
            DecisionStatus::Deferred
                | DecisionStatus::Blocked
                | DecisionStatus::NeedsReview
                | DecisionStatus::Withdrawn
        ) {
            nonempty(&d.status_reason)?;
            nonempty(&d.status_evidence)?;
        }
        for id in unique(d.task_ids.iter())? {
            if !tasks.contains(id) {
                return Err(DiscussionIssue("dangling_discussion_reference"));
            }
        }
        for id in unique(d.dependencies.iter())? {
            if !decisions.contains(id) {
                return Err(DiscussionIssue("dangling_discussion_reference"));
            }
        }
    }
    graph(
        &session
            .decisions
            .iter()
            .map(|d| (d.id.clone(), d.dependencies.clone()))
            .collect(),
    )?;
    for task in &session.tasks {
        nonempty(&task.title)?;
        nonempty(&task.goal)?;
        for id in unique(task.dependencies.iter())? {
            if !tasks.contains(id) {
                return Err(DiscussionIssue("dangling_discussion_reference"));
            }
        }
        for id in unique(task.decision_ids.iter())? {
            if !decisions.contains(id) {
                return Err(DiscussionIssue("dangling_discussion_reference"));
            }
        }
        if let Nullable::Value(review) = &task.review {
            if review.context_revision > session.context.revision {
                return Err(DiscussionIssue("invalid_planning_review"));
            }
            for (id, version) in &review.decision_versions {
                let d = session
                    .decisions
                    .iter()
                    .find(|d| &d.id == id)
                    .ok_or(DiscussionIssue("dangling_discussion_reference"))?;
                if *version > d.question_version {
                    return Err(DiscussionIssue("invalid_planning_review"));
                }
            }
        }
    }
    graph(
        &session
            .tasks
            .iter()
            .map(|t| (t.id.clone(), t.dependencies.clone()))
            .collect(),
    )?;
    if let Nullable::Value(id) = &session.continuation.current_task_id {
        if !tasks.contains(id) {
            return Err(DiscussionIssue("dangling_discussion_reference"));
        }
    }
    if let Nullable::Value(q) = &session.continuation.question {
        validate_question(session, q)?;
    }
    if !valid_sha256(&session.commit.operation_sha256)
        || !valid_sha256(&session.commit.content_sha256)
    {
        return Err(DiscussionIssue("invalid_discussion_fingerprint"));
    }
    nonempty(&session.commit.operation_id)?;
    match &session.commit.previous_sha256 {
        Nullable::Null if session.revision == 1 => (),
        Nullable::Value(hash) if session.revision > 1 && valid_sha256(hash) => (),
        _ => return Err(DiscussionIssue("invalid_discussion_previous_revision")),
    }
    Ok(())
}

pub fn validate_authorization(
    session: &DiscussionSession,
    action: Option<DiscussionAction>,
) -> Result<()> {
    let a = &session.authorization;
    if a.requirement_id != session.requirement_id
        || a.directory != format!("outputs/work/discussions/{}", session.requirement_id)
        || a.project_root.trim().is_empty()
        || a.allowed_actions.is_empty()
        || a.evidence.trim().is_empty()
        || a.revoked_reason != Nullable::Null
        || action.is_some_and(|action| !a.allowed_actions.contains(&action))
    {
        return Err(DiscussionIssue("discussion_save_not_authorized"));
    }
    Ok(())
}

pub(super) fn validate_question(session: &DiscussionSession, q: &SavedQuestion) -> Result<()> {
    let d = session
        .decisions
        .iter()
        .find(|d| d.id == q.decision_id)
        .ok_or(DiscussionIssue("unknown_decision"))?;
    if q.question_version != d.question_version
        || q.text != d.question
        || d.status == DecisionStatus::Withdrawn
        || q.display_options.is_empty()
    {
        return Err(DiscussionIssue("stale_discussion_question"));
    }
    let actual: BTreeSet<_> = q.display_options.values().collect();
    if actual.len() != q.display_options.len()
        || actual != d.options.iter().map(|o| &o.id).collect()
        || (1..=q.display_options.len()).any(|n| !q.display_options.contains_key(&n.to_string()))
    {
        return Err(DiscussionIssue("ambiguous_discussion_question"));
    }
    Ok(())
}

pub fn verify_integrity(session: &DiscussionSession) -> Result<()> {
    validate(session)?;
    if fingerprint::discussion_session(session) != session.commit.content_sha256 {
        return Err(DiscussionIssue("discussion_content_integrity"));
    }
    Ok(())
}

pub fn ready_to_generate(session: &DiscussionSession) -> Result<()> {
    verify_integrity(session)?;
    if progress(session).remaining != 0 {
        return Err(DiscussionIssue("discussion_not_confirmed"));
    }
    if session.tasks.is_empty() || session.context.confirmed_source == Nullable::Null {
        return Err(DiscussionIssue("discussion_planning_incomplete"));
    }
    for t in &session.tasks {
        let Nullable::Value(review) = &t.review else {
            return Err(DiscussionIssue("discussion_planning_incomplete"));
        };
        if review.needs_review
            || review.context_revision != session.context.revision
            || review.semantic_consistency_evidence.trim().is_empty()
            || t.scope.is_empty()
            || t.acceptance_ids.is_empty()
            || t.acceptance_criteria.is_empty()
            || t.steps.is_empty()
            || t.validations.is_empty()
            || t.instruction_selection == Nullable::Null
            || t.instructions_sha256 == Nullable::Null
        {
            return Err(DiscussionIssue("discussion_planning_incomplete"));
        }
        for d in session
            .decisions
            .iter()
            .filter(|d| t.decision_ids.contains(&d.id) || d.task_ids.contains(&t.id))
        {
            if d.status != DecisionStatus::Confirmed
                || review.decision_versions.get(&d.id) != Some(&d.question_version)
            {
                return Err(DiscussionIssue("discussion_planning_needs_review"));
            }
        }
        validate_granularity(session, t)?;
    }
    Ok(())
}

pub fn validate_granularity(session: &DiscussionSession, task: &DiscussionTask) -> Result<()> {
    let Nullable::Value(review) = &task.review else {
        return Err(DiscussionIssue("task_granularity_review_required"));
    };
    let Some(g) = &review.granularity else {
        return Err(DiscussionIssue("task_granularity_review_required"));
    };
    if g.outcome.trim().is_empty() || g.evidence.trim().is_empty() || !g.transaction_feasible {
        return Err(DiscussionIssue("task_granularity_unsafe"));
    }
    match g.split_decision {
        SplitDecision::SplitRequired => return Err(DiscussionIssue("task_split_required")),
        SplitDecision::NeedsConfirmation => {
            return Err(DiscussionIssue("task_granularity_needs_confirmation"));
        }
        SplitDecision::Indivisible if g.indivisibility_reason.trim().is_empty() => {
            return Err(DiscussionIssue("task_indivisibility_evidence_required"));
        }
        _ => {}
    }
    if review.needs_review
        || g.planning_sha256 != fingerprint::discussion_planning(session, &task.id)
    {
        return Err(DiscussionIssue("task_granularity_review_stale"));
    }
    validate_outcome_consistency(task, g)?;
    Ok(())
}

fn validate_outcome_consistency(
    task: &DiscussionTask,
    review: &work_model::discussion::GranularityReview,
) -> Result<()> {
    let Some(semantic) = &review.semantic else {
        return Err(DiscussionIssue("task_outcome_review_required"));
    };
    let outcomes = &semantic.outcomes;
    if outcomes.is_empty() || outcomes.iter().any(|o| o.needs_confirmation) {
        return Err(DiscussionIssue("task_granularity_needs_confirmation"));
    }
    if outcomes.len() > 1
        && (review.split_decision == SplitDecision::SingleOutcome
            || outcomes.iter().any(|o| o.independently_acceptable))
    {
        return Err(DiscussionIssue("task_split_required"));
    }
    let mut ids = BTreeSet::new();
    let mut acceptance = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut scope = BTreeSet::new();
    for outcome in outcomes {
        if outcome.id.trim().is_empty()
            || !ids.insert(outcome.id.as_str())
            || outcome.statement.trim().is_empty()
            || outcome.evidence.trim().is_empty()
            || outcome.acceptance_ids.is_empty()
            || outcome.scope.is_empty()
        {
            return Err(DiscussionIssue("task_outcome_review_inconsistent"));
        }
        for id in &outcome.acceptance_ids {
            if !acceptance.insert(id.as_str()) {
                return Err(DiscussionIssue("task_outcome_review_inconsistent"));
            }
        }
        files.extend(outcome.file_keys.iter().map(String::as_str));
        scope.extend(outcome.scope.iter().map(String::as_str));
    }
    if acceptance
        != task
            .acceptance_criteria
            .iter()
            .map(|a| a.id.as_str())
            .collect()
        || files != task.files.iter().map(|f| f.key.as_str()).collect()
        || scope != task.scope.iter().map(String::as_str).collect()
        || review.outcome != task.goal
    {
        return Err(DiscussionIssue("task_outcome_review_inconsistent"));
    }
    if outcomes.len() == 1 && outcomes[0].statement != task.goal {
        return Err(DiscussionIssue("task_outcome_review_inconsistent"));
    }
    if review.split_decision == SplitDecision::Indivisible
        && (semantic
            .coupled_outcome_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != ids
            || semantic.coupled_outcome_ids.len() != ids.len()
            || semantic.separation_consequence.trim().is_empty())
    {
        return Err(DiscussionIssue("task_indivisibility_evidence_required"));
    }
    Ok(())
}

/// Validate stored provenance without consulting a later Session revision.
pub fn validate_trace(
    value: &serde_json::Value,
    requirement: &str,
    task_ids: &[String],
) -> Result<()> {
    let trace: work_model::task::discussion_trace::DiscussionTrace =
        serde_json::from_value(value.clone())
            .map_err(|_| DiscussionIssue("invalid_discussion_trace"))?;
    if trace.session_path != format!("outputs/work/discussions/{requirement}/session.json")
        || trace.revision == 0
        || !valid_sha256(&trace.content_sha256)
        || trace.task_decisions.keys().collect::<BTreeSet<_>>() != task_ids.iter().collect()
    {
        return Err(DiscussionIssue("invalid_discussion_trace"));
    }
    for (id, version) in &trace.decision_versions {
        number(id, "D")?;
        if *version == 0 {
            return Err(DiscussionIssue("invalid_discussion_trace"));
        }
    }
    for decisions in trace.task_decisions.values() {
        unique(decisions.iter())?;
        if decisions
            .iter()
            .any(|id| !trace.decision_versions.contains_key(id))
        {
            return Err(DiscussionIssue("invalid_discussion_trace"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn formal_trace_binds_every_task_and_rejects_malformed_evidence() {
        let trace = json!({"session_path":"outputs/work/discussions/example/session.json","revision":3,
            "content_sha256":"a".repeat(64),"decision_versions":{"D008":2},"task_decisions":{"TASK-001":["D008"]}});
        let ids = vec!["TASK-001".into()];
        validate_trace(&trace, "example", &ids).unwrap();
        for (pointer, value) in [
            ("/revision", json!(0)),
            ("/session_path", json!("other/session.json")),
            ("/content_sha256", json!("bad")),
            ("/decision_versions/D008", json!(0)),
            ("/task_decisions/TASK-001", json!(["D009"])),
            ("/task_decisions/TASK-001", json!(["D008", "D008"])),
        ] {
            let mut invalid = trace.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                validate_trace(&invalid, "example", &ids).is_err(),
                "{pointer}"
            );
        }
        let mut extra = trace.clone();
        extra["latest_session_required"] = json!(true);
        assert!(validate_trace(&extra, "example", &ids).is_err());
        assert!(validate_trace(&trace, "example", &["TASK-002".into()]).is_err());
    }
}
