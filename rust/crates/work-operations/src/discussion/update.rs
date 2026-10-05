//! Local changes, decision transitions, and dependent review invalidation.

use std::collections::BTreeSet;

use work_model::common::Nullable;
use work_model::discussion::*;

use super::validation::{nonempty, validate_authorization, validate_question, verify_integrity};
use super::{DiscussionIssue, Result};
use crate::derivation::fingerprint;

pub use work_model::discussion::operation::{DiscussionChange, DiscussionOperation};

/// Same-state changes may edit evidence; withdrawn decisions remain immutable.
pub fn allowed_transition(from: DecisionStatus, to: DecisionStatus) -> bool {
    use DecisionStatus::*;
    match from {
        Withdrawn => false,
        Confirmed => matches!(to, Confirmed | NeedsReview | Withdrawn),
        Pending | Deferred | Blocked => to != NeedsReview,
        NeedsReview => true,
    }
}

fn affected_decisions(
    session: &DiscussionSession,
    changed: &str,
    context: bool,
) -> BTreeSet<String> {
    let mut affected = BTreeSet::from([changed.to_owned()]);
    loop {
        let len = affected.len();
        for d in &session.decisions {
            if context || d.dependencies.iter().any(|id| affected.contains(id)) {
                affected.insert(d.id.clone());
            }
        }
        if len == affected.len() {
            break;
        }
    }
    affected
}

fn affected_tasks(
    session: &DiscussionSession,
    affected: &BTreeSet<String>,
    context: bool,
) -> BTreeSet<String> {
    let mut affected_tasks: BTreeSet<String> = session
        .tasks
        .iter()
        .filter(|t| {
            context
                || t.decision_ids.iter().any(|id| affected.contains(id))
                || t.steps.iter().flat_map(|s| &s.references).any(|r| {
                    r.kind == work_model::task::candidate::CandidateReferenceKind::Decisions
                        && affected
                            .iter()
                            .any(|id| r.key == *id || r.key == id.to_ascii_lowercase())
                })
                || session
                    .decisions
                    .iter()
                    .any(|d| affected.contains(&d.id) && d.task_ids.contains(&t.id))
        })
        .map(|t| t.id.clone())
        .collect();
    loop {
        let len = affected_tasks.len();
        for t in &session.tasks {
            if t.dependencies.iter().any(|id| affected_tasks.contains(id)) {
                affected_tasks.insert(t.id.clone());
            }
        }
        if len == affected_tasks.len() {
            break;
        }
    }
    affected_tasks
}

fn invalidate(session: &mut DiscussionSession, changed: &str, context: bool) {
    invalidate_with_previous(session, changed, context, BTreeSet::new());
}

fn invalidate_with_previous(
    session: &mut DiscussionSession,
    changed: &str,
    context: bool,
    mut previous_tasks: BTreeSet<String>,
) {
    let affected = affected_decisions(session, changed, context);
    previous_tasks.extend(affected_tasks(session, &affected, context));
    for d in &mut session.decisions {
        if (context || d.id != changed)
            && affected.contains(&d.id)
            && d.status == DecisionStatus::Confirmed
        {
            d.status = DecisionStatus::NeedsReview;
            d.status_reason = format!("Upstream discussion changed: {changed}");
            d.status_evidence = format!("Session revision {}", session.revision);
        }
    }
    for t in &mut session.tasks {
        if previous_tasks.contains(&t.id) {
            if let Nullable::Value(review) = &mut t.review {
                review.needs_review = true;
            }
        }
    }
    if let Nullable::Value(q) = &session.continuation.question {
        if affected.contains(&q.decision_id) {
            session.continuation.question = Nullable::Null;
        }
    }
}

fn invalidate_planning(session: &mut DiscussionSession, changed: &str) {
    let mut affected = BTreeSet::from([changed.to_owned()]);
    loop {
        let before = affected.len();
        for task in &session.tasks {
            if task.dependencies.iter().any(|id| affected.contains(id)) {
                affected.insert(task.id.clone());
            }
        }
        if before == affected.len() {
            break;
        }
    }
    for task in &mut session.tasks {
        if affected.contains(&task.id) {
            if let Nullable::Value(review) = &mut task.review {
                review.needs_review = true;
            }
        }
    }
}

pub fn operation_sha256(operation: &DiscussionOperation) -> String {
    fingerprint::discussion_operation(
        &serde_json::to_value(operation).expect("operation serializes"),
    )
}

pub fn apply(
    session: &DiscussionSession,
    operation: &DiscussionOperation,
) -> Result<DiscussionSession> {
    verify_integrity(session)?;
    validate_authorization(session, Some(operation.change.action()))?;
    nonempty(&operation.operation_id)?;
    if operation.expected_revision != session.revision
        || operation.previous_sha256 != session.commit.content_sha256
    {
        return Err(DiscussionIssue("stale_discussion_revision"));
    }
    let mut next = session.clone();
    next.revision = session
        .revision
        .checked_add(1)
        .ok_or(DiscussionIssue("discussion_revision_overflow"))?;
    match &operation.change {
        DiscussionChange::AddDecision { decision } => {
            if decision.id != format!("D{:03}", session.next_decision_number)
                || decision.status != DecisionStatus::Pending
                || decision.resolution != Nullable::Null
            {
                return Err(DiscussionIssue("invalid_new_decision"));
            }
            next.next_decision_number = next
                .next_decision_number
                .checked_add(1)
                .ok_or(DiscussionIssue("discussion_revision_overflow"))?;
            next.decisions.push(decision.clone());
        }
        DiscussionChange::UpdateDecision { decision } => {
            let old = session
                .decisions
                .iter()
                .find(|d| d.id == decision.id)
                .ok_or(DiscussionIssue("unknown_decision"))?;
            if !allowed_transition(old.status, decision.status) {
                return Err(DiscussionIssue("invalid_decision_transition"));
            }
            let changed_question =
                old.question != decision.question || old.options != decision.options;
            let expected = old
                .question_version
                .checked_add(u64::from(changed_question))
                .ok_or(DiscussionIssue("discussion_revision_overflow"))?;
            if decision.question_version != expected {
                return Err(DiscussionIssue("stale_discussion_question"));
            }
            if changed_question && decision.status == DecisionStatus::Confirmed {
                return Err(DiscussionIssue("decision_reconfirmation_required"));
            }
            let previous_tasks = affected_tasks(
                session,
                &affected_decisions(session, &decision.id, false),
                false,
            );
            let slot = next
                .decisions
                .iter_mut()
                .find(|d| d.id == decision.id)
                .expect("known ID");
            *slot = decision.clone();
            if changed_question
                || old.resolution != decision.resolution
                || old.status != decision.status
                || old.dependencies != decision.dependencies
                || old.task_ids != decision.task_ids
            {
                invalidate_with_previous(&mut next, &decision.id, false, previous_tasks);
            }
            if decision.status == DecisionStatus::Withdrawn {
                // The immutable predecessor retains the withdrawn decision's original links.
                next.decisions
                    .iter_mut()
                    .find(|d| d.id == decision.id)
                    .expect("known ID")
                    .task_ids
                    .clear();
                for task in &mut next.tasks {
                    task.decision_ids.retain(|id| id != &decision.id);
                    for step in &mut task.steps {
                        step.references.retain(|r| {
                            !(r.kind
                                == work_model::task::candidate::CandidateReferenceKind::Decisions
                                && (r.key == decision.id
                                    || r.key == decision.id.to_ascii_lowercase()))
                        });
                    }
                    if let Nullable::Value(review) = &mut task.review {
                        review.decision_versions.remove(&decision.id);
                    }
                }
            }
        }
        DiscussionChange::UpdatePlanning { task } => {
            if let Some(slot) = next.tasks.iter_mut().find(|t| t.id == task.id) {
                let mut old_content = slot.clone();
                let mut new_content = *task.clone();
                old_content.review = Nullable::Null;
                new_content.review = Nullable::Null;
                let changed = old_content != new_content;
                *slot = *task.clone();
                if changed {
                    invalidate_planning(&mut next, &task.id);
                }
            } else {
                if task.id != format!("TASK-{:03}", session.next_task_number) {
                    return Err(DiscussionIssue("reused_discussion_id"));
                }
                next.next_task_number = next
                    .next_task_number
                    .checked_add(1)
                    .ok_or(DiscussionIssue("discussion_revision_overflow"))?;
                next.tasks.push(*task.clone());
            }
        }
        DiscussionChange::PrepareQuestion { question } => {
            validate_question(session, question)?;
            let mut saved = question.clone();
            saved.question_version = saved
                .question_version
                .checked_add(1)
                .ok_or(DiscussionIssue("discussion_revision_overflow"))?;
            let d = next
                .decisions
                .iter_mut()
                .find(|d| d.id == saved.decision_id)
                .expect("validated decision");
            d.question_version = saved.question_version;
            if d.status == DecisionStatus::Confirmed {
                d.status = DecisionStatus::NeedsReview;
                d.status_reason = "Question prepared for renewed confirmation".into();
                d.status_evidence = format!("Session revision {}", next.revision);
            }
            invalidate(&mut next, &saved.decision_id, false);
            next.continuation.question = Nullable::Value(saved);
        }
        DiscussionChange::AnswerQuestion {
            decision_id,
            question_version,
            display_number,
            rationale,
            evidence,
        } => {
            let Nullable::Value(q) = &session.continuation.question else {
                return Err(DiscussionIssue("no_saved_discussion_question"));
            };
            if &q.decision_id != decision_id || &q.question_version != question_version {
                return Err(DiscussionIssue("stale_discussion_question"));
            }
            let option = q
                .display_options
                .get(display_number)
                .ok_or(DiscussionIssue("unknown_discussion_answer"))?;
            nonempty(rationale)?;
            nonempty(evidence)?;
            let d = next
                .decisions
                .iter_mut()
                .find(|d| &d.id == decision_id)
                .expect("validated question");
            if !allowed_transition(d.status, DecisionStatus::Confirmed) {
                return Err(DiscussionIssue("invalid_decision_transition"));
            }
            d.status = DecisionStatus::Confirmed;
            d.resolution = Nullable::Value(DecisionResolution {
                option_id: option.clone(),
                rationale: rationale.clone(),
                confirmation_evidence: evidence.clone(),
            });
            invalidate(&mut next, decision_id, false);
            next.continuation.question = Nullable::Null;
        }
        DiscussionChange::UpdateContext { context } => {
            if context.revision
                != session
                    .context
                    .revision
                    .checked_add(1)
                    .ok_or(DiscussionIssue("discussion_revision_overflow"))?
            {
                return Err(DiscussionIssue("stale_discussion_context"));
            }
            next.context = *context.clone();
            invalidate(&mut next, "requirement context", true);
        }
        DiscussionChange::UpdateContinuation { current_task_id } => {
            next.continuation.current_task_id = current_task_id.clone();
        }
    }
    next.commit = DiscussionCommit {
        operation_id: operation.operation_id.clone(),
        operation_sha256: operation_sha256(operation),
        previous_sha256: Nullable::Value(session.commit.content_sha256.clone()),
        content_sha256: "0".repeat(64),
    };
    next.commit.content_sha256 = fingerprint::discussion_session(&next);
    verify_integrity(&next)?;
    Ok(next)
}

#[cfg(test)]
use super::{progress, ready_to_generate, validate, view};
#[cfg(test)]
use std::collections::BTreeMap;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decision(n: u64, status: DecisionStatus) -> DiscussionDecision {
        DiscussionDecision {
            id: format!("D{n:03}"),
            question: format!("Question {n}?"),
            question_version: 1,
            options: vec![
                DecisionOption {
                    id: "first".into(),
                    label: "First".into(),
                    explanation: "Reason".into(),
                },
                DecisionOption {
                    id: "second".into(),
                    label: "Second".into(),
                    explanation: "Alternative".into(),
                },
            ],
            status,
            tentative_option_id: Nullable::Value("second".into()),
            resolution: if matches!(
                status,
                DecisionStatus::Confirmed | DecisionStatus::NeedsReview
            ) {
                Nullable::Value(DecisionResolution {
                    option_id: "first".into(),
                    rationale: format!("Rationale {n}"),
                    confirmation_evidence: format!("User {n}"),
                })
            } else {
                Nullable::Null
            },
            status_reason: "Explicit status reason".into(),
            status_evidence: "User instruction".into(),
            added_reason: "Required choice".into(),
            task_ids: vec![],
            dependencies: vec![],
            source_references: vec![format!("source-{n}.txt")],
        }
    }

    fn seal(session: &mut DiscussionSession) {
        session.commit.content_sha256 = fingerprint::discussion_session(session);
    }

    fn session(statuses: &[DecisionStatus]) -> DiscussionSession {
        let mut session = DiscussionSession {
            schema: DiscussionSchema::Session,
            requirement_id: "example".into(),
            revision: 1,
            context: RequirementContext {
                original_references: vec!["source.txt".into()],
                goal: "Deliver requirement".into(),
                scope: vec!["Task".into()],
                constraints: vec![],
                acceptance_criteria: vec![work_model::task::source::TaskAcceptance {
                    id: "ACCEPTANCE-001".into(),
                    criterion: "Verified".into(),
                }],
                confirmed_source: Nullable::Null,
                work_type: "task".into(),
                revision: 1,
            },
            authorization: SaveAuthorization {
                requirement_id: "example".into(),
                project_root: "/project".into(),
                directory: "outputs/work/discussions/example".into(),
                allowed_actions: vec![
                    DiscussionAction::AddDecision,
                    DiscussionAction::UpdateDecision,
                    DiscussionAction::UpdatePlanning,
                    DiscussionAction::PrepareQuestion,
                    DiscussionAction::AnswerQuestion,
                    DiscussionAction::UpdateContext,
                    DiscussionAction::UpdateContinuation,
                ],
                evidence: "User approved bounded saves".into(),
                revoked_reason: Nullable::Null,
            },
            tasks: vec![],
            retired_task_ids: vec![],
            decisions: statuses
                .iter()
                .enumerate()
                .map(|(i, s)| decision(i as u64 + 1, *s))
                .collect(),
            next_task_number: 1,
            next_decision_number: statuses.len() as u64 + 1,
            continuation: Continuation {
                current_task_id: Nullable::Null,
                question: Nullable::Null,
            },
            commit: DiscussionCommit {
                operation_id: "init".into(),
                operation_sha256: "a".repeat(64),
                previous_sha256: Nullable::Null,
                content_sha256: "0".repeat(64),
            },
        };
        seal(&mut session);
        session
    }

    fn op(session: &DiscussionSession, change: DiscussionChange) -> DiscussionOperation {
        DiscussionOperation {
            operation_id: format!("operation-{}", session.revision + 1),
            expected_revision: session.revision,
            previous_sha256: session.commit.content_sha256.clone(),
            change,
        }
    }

    fn task(n: u64, decisions: Vec<String>, dependencies: Vec<String>) -> DiscussionTask {
        DiscussionTask {
            id: format!("TASK-{n:03}"),
            title: "Task".into(),
            goal: "Deliver".into(),
            scope: vec!["Scope".into()],
            skill_id: Nullable::Null,
            dependencies,
            decision_ids: decisions,
            acceptance_ids: vec![],
            acceptance_criteria: vec![],
            instruction_selection: Nullable::Null,
            instructions_sha256: Nullable::Null,
            inputs: vec![],
            files: vec![],
            risks: vec![],
            steps: vec![],
            commands: vec![],
            operations: vec![],
            validations: vec![],
            review: Nullable::Value(PlanningReview {
                context_revision: 1,
                decision_versions: BTreeMap::new(),
                semantic_consistency_evidence: "Reviewed".into(),
                needs_review: false,
            }),
        }
    }

    #[test]
    fn progress_is_seven_of_twelve_and_withdrawn_is_separate() {
        let mut statuses = vec![DecisionStatus::Confirmed; 7];
        statuses.extend([
            DecisionStatus::Pending,
            DecisionStatus::Pending,
            DecisionStatus::Deferred,
            DecisionStatus::Blocked,
            DecisionStatus::NeedsReview,
            DecisionStatus::Withdrawn,
        ]);
        let s = session(&statuses);
        verify_integrity(&s).unwrap();
        let p = progress(&s);
        assert_eq!(
            (p.confirmed, p.known_total, p.remaining, p.withdrawn),
            (7, 12, 5, 1)
        );
        let empty = session(&[]);
        assert_eq!(progress(&empty).known_total, 0);
        assert_eq!(
            ready_to_generate(&empty).unwrap_err().0,
            "discussion_planning_incomplete"
        );
    }

    #[test]
    fn answering_saved_d008_preserves_all_seven_decisions_without_resending() {
        let mut statuses = vec![DecisionStatus::Confirmed; 7];
        statuses.push(DecisionStatus::Pending);
        let s = session(&statuses);
        let q = SavedQuestion {
            decision_id: "D008".into(),
            question_version: 1,
            text: "Question 8?".into(),
            display_options: BTreeMap::from([
                ("1".into(), "first".into()),
                ("2".into(), "second".into()),
            ]),
        };
        let prepared = apply(
            &s,
            &op(&s, DiscussionChange::PrepareQuestion { question: q }),
        )
        .unwrap();
        let restored: DiscussionSession =
            serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        let answered = apply(
            &restored,
            &op(
                &restored,
                DiscussionChange::AnswerQuestion {
                    decision_id: "D008".into(),
                    question_version: 2,
                    display_number: "1".into(),
                    rationale: "Selected format".into(),
                    evidence: "User answered 1".into(),
                },
            ),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&answered.decisions[..7]).unwrap(),
            serde_json::to_vec(&s.decisions[..7]).unwrap()
        );
        assert_eq!(answered.decisions[7].status, DecisionStatus::Confirmed);
        assert_eq!(progress(&answered).confirmed, 8);
        assert_eq!(answered.continuation.question, Nullable::Null);
    }

    #[test]
    fn transition_matrix_and_confirmation_evidence_are_enforced() {
        use DecisionStatus::*;
        let states = [
            Pending,
            Confirmed,
            Deferred,
            Blocked,
            NeedsReview,
            Withdrawn,
        ];
        let expected = [
            [true, true, true, true, false, true],
            [false, true, false, false, true, true],
            [true, true, true, true, false, true],
            [true, true, true, true, false, true],
            [true, true, true, true, true, true],
            [false, false, false, false, false, false],
        ];
        for (row, from) in states.into_iter().enumerate() {
            for (column, to) in states.into_iter().enumerate() {
                let s = session(&[from]);
                let mut d = s.decisions[0].clone();
                d.status = to;
                if to == Confirmed && d.resolution == Nullable::Null {
                    d.resolution = decision(1, Confirmed).resolution;
                }
                let result = apply(
                    &s,
                    &op(&s, DiscussionChange::UpdateDecision { decision: d }),
                );
                assert_eq!(result.is_ok(), expected[row][column], "{from:?} -> {to:?}");
            }
        }
        let s = session(&[Pending]);
        let mut d = s.decisions[0].clone();
        d.status = Confirmed;
        assert_eq!(
            apply(
                &s,
                &op(&s, DiscussionChange::UpdateDecision { decision: d })
            )
            .unwrap_err()
            .0,
            "decision_confirmation_required"
        );
        let mut d = s.decisions[0].clone();
        d.status = Withdrawn;
        d.status_evidence.clear();
        assert_eq!(
            apply(
                &s,
                &op(&s, DiscussionChange::UpdateDecision { decision: d })
            )
            .unwrap_err()
            .0,
            "missing_discussion_evidence"
        );
    }

    #[test]
    fn dependency_change_invalidates_direct_indirect_and_shared_planning() {
        let mut s = session(&[DecisionStatus::Confirmed; 3]);
        s.decisions[1].dependencies = vec!["D001".into()];
        s.decisions[2].dependencies = vec!["D002".into()];
        s.tasks = vec![
            task(1, vec!["D001".into()], vec![]),
            task(2, vec!["D003".into()], vec!["TASK-001".into()]),
        ];
        s.next_task_number = 3;
        seal(&mut s);
        let mut d = s.decisions[0].clone();
        if let Nullable::Value(r) = &mut d.resolution {
            r.option_id = "second".into();
        }
        let next = apply(
            &s,
            &op(&s, DiscussionChange::UpdateDecision { decision: d }),
        )
        .unwrap();
        assert_eq!(next.decisions[0].status, DecisionStatus::Confirmed);
        assert!(
            next.decisions[1..]
                .iter()
                .all(|d| d.status == DecisionStatus::NeedsReview)
        );
        assert!(
            next.tasks
                .iter()
                .all(|t| matches!(&t.review,Nullable::Value(r) if r.needs_review))
        );
        assert_eq!(progress(&next).remaining, 2);
    }

    #[test]
    fn decision_rebinding_and_unlinking_review_old_new_and_transitive_tasks_only() {
        for (new_links, change_resolution) in [
            (vec!["TASK-002".into()], false),
            (vec![], false),
            (vec!["TASK-002".into()], true),
        ] {
            let mut s = session(&[DecisionStatus::Confirmed]);
            // Deliberately no reverse decision_ids or step references.
            s.decisions[0].task_ids = vec!["TASK-001".into()];
            s.tasks = vec![
                task(1, vec![], vec![]),
                task(2, vec![], vec![]),
                task(3, vec![], vec!["TASK-005".into()]),
                task(4, vec![], vec![]),
                task(5, vec![], vec!["TASK-001".into()]),
                task(6, vec![], vec!["TASK-002".into()]),
            ];
            s.next_task_number = 7;
            seal(&mut s);
            let mut d = s.decisions[0].clone();
            d.task_ids = new_links.clone();
            if change_resolution {
                if let Nullable::Value(r) = &mut d.resolution {
                    r.option_id = "second".into();
                }
            }
            let next = apply(
                &s,
                &op(&s, DiscussionChange::UpdateDecision { decision: d }),
            )
            .unwrap();
            for i in [0, 2, 4] {
                assert!(matches!(&next.tasks[i].review, Nullable::Value(r) if r.needs_review));
            }
            for i in [1, 5] {
                if new_links.is_empty() {
                    assert_eq!(next.tasks[i], s.tasks[i]);
                } else {
                    assert!(matches!(&next.tasks[i].review, Nullable::Value(r) if r.needs_review));
                }
            }
            assert_eq!(next.tasks[3], s.tasks[3]);
            assert!(
                s.tasks
                    .iter()
                    .all(|t| matches!(&t.review, Nullable::Value(r) if !r.needs_review))
            );
        }
    }

    #[test]
    fn decision_changes_include_reverse_and_step_only_references() {
        for key in ["D001", "d001"] {
            let mut s = session(&[DecisionStatus::Confirmed]);
            s.tasks = vec![
                task(1, vec!["D001".into()], vec![]),
                task(2, vec![], vec![]),
                task(3, vec![], vec![]),
            ];
            s.tasks[1].steps = serde_json::from_value(json!([{"key":"choose","action":"Use decision","references":[{"kind":"decisions","key":key}]}])).unwrap();
            s.next_task_number = 4;
            seal(&mut s);
            let mut d = s.decisions[0].clone();
            if let Nullable::Value(r) = &mut d.resolution {
                r.option_id = "second".into();
            }
            let next = apply(
                &s,
                &op(&s, DiscussionChange::UpdateDecision { decision: d }),
            )
            .unwrap();
            assert!(
                next.tasks[..2]
                    .iter()
                    .all(|t| matches!(&t.review, Nullable::Value(r) if r.needs_review))
            );
            assert_eq!(next.tasks[2], s.tasks[2]);
        }
    }

    #[test]
    fn cycles_dangling_duplicate_and_reused_ids_are_rejected() {
        let s = session(&[DecisionStatus::Pending; 2]);
        for dependencies in [
            vec!["D001".into()],
            vec!["D999".into()],
            vec!["D002".into(), "D002".into()],
        ] {
            let mut d = s.decisions[0].clone();
            d.dependencies = dependencies;
            assert!(
                apply(
                    &s,
                    &op(&s, DiscussionChange::UpdateDecision { decision: d })
                )
                .is_err()
            );
        }
        let mut cycle = s.clone();
        cycle.decisions[0].dependencies = vec!["D002".into()];
        cycle.decisions[1].dependencies = vec!["D001".into()];
        seal(&mut cycle);
        assert_eq!(
            validate(&cycle).unwrap_err().0,
            "cyclic_discussion_dependency"
        );
        assert_eq!(
            apply(
                &s,
                &op(
                    &s,
                    DiscussionChange::AddDecision {
                        decision: decision(1, DecisionStatus::Pending)
                    }
                )
            )
            .unwrap_err()
            .0,
            "invalid_new_decision"
        );
        let next = apply(
            &s,
            &op(
                &s,
                DiscussionChange::AddDecision {
                    decision: decision(3, DecisionStatus::Pending),
                },
            ),
        )
        .unwrap();
        assert_eq!(progress(&next).known_total - progress(&s).known_total, 1);
        assert_eq!(next.decisions[2].added_reason, "Required choice");
    }

    #[test]
    fn authorization_stale_revision_fingerprint_and_question_are_rejected() {
        let s = session(&[DecisionStatus::Pending]);
        let operation = op(
            &s,
            DiscussionChange::UpdateContinuation {
                current_task_id: Nullable::Null,
            },
        );
        let mut stale = operation.clone();
        stale.expected_revision = 0;
        assert_eq!(
            apply(&s, &stale).unwrap_err().0,
            "stale_discussion_revision"
        );
        let mut damaged = s.clone();
        damaged.context.goal = "Changed".into();
        assert_eq!(
            apply(&damaged, &operation).unwrap_err().0,
            "discussion_content_integrity"
        );
        let mut denied = s.clone();
        denied.authorization.allowed_actions = vec![DiscussionAction::AddDecision];
        seal(&mut denied);
        assert_eq!(
            apply(&denied, &op(&denied, operation.change))
                .unwrap_err()
                .0,
            "discussion_save_not_authorized"
        );
        let q = SavedQuestion {
            decision_id: "D001".into(),
            question_version: 2,
            text: "Question 1?".into(),
            display_options: BTreeMap::from([("1".into(), "first".into())]),
        };
        assert_eq!(
            apply(
                &s,
                &op(&s, DiscussionChange::PrepareQuestion { question: q })
            )
            .unwrap_err()
            .0,
            "stale_discussion_question"
        );
        let answer = DiscussionChange::AnswerQuestion {
            decision_id: "D001".into(),
            question_version: 1,
            display_number: "1".into(),
            rationale: "Reason".into(),
            evidence: "User".into(),
        };
        assert_eq!(
            apply(&s, &op(&s, answer)).unwrap_err().0,
            "no_saved_discussion_question"
        );
        let mut altered = s.clone();
        altered.revision = 2;
        altered.commit.previous_sha256 = Nullable::Value("a".repeat(64));
        seal(&mut altered);
        assert_ne!(altered.commit.content_sha256, s.commit.content_sha256);
        assert_eq!(
            serde_json::to_value(view(&s).unwrap()).unwrap()["pending_decision_ids"],
            json!(["D001"])
        );
    }
    #[test]
    fn changing_task_content_invalidates_direct_and_transitive_reviews_only() {
        let mut s = session(&[]);
        s.tasks = vec![
            task(1, vec![], vec![]),
            task(2, vec![], vec!["TASK-001".into()]),
            task(3, vec![], vec![]),
        ];
        s.next_task_number = 4;
        seal(&mut s);
        let unrelated = s.tasks[2].clone();
        let mut changed = s.tasks[0].clone();
        changed.goal = "Changed outcome".into();
        let next = apply(
            &s,
            &op(
                &s,
                DiscussionChange::UpdatePlanning {
                    task: Box::new(changed),
                },
            ),
        )
        .unwrap();
        assert!(
            next.tasks[..2]
                .iter()
                .all(|task| matches!(&task.review,Nullable::Value(review) if review.needs_review))
        );
        assert_eq!(next.tasks[2], unrelated);
        let mut reviewed = next.tasks[0].clone();
        if let Nullable::Value(review) = &mut reviewed.review {
            review.needs_review = false;
            review.semantic_consistency_evidence = "Explicitly reviewed new outcome".into();
        }
        let rereviewed = apply(
            &next,
            &op(
                &next,
                DiscussionChange::UpdatePlanning {
                    task: Box::new(reviewed),
                },
            ),
        )
        .unwrap();
        assert!(
            matches!(&rereviewed.tasks[0].review,Nullable::Value(review) if !review.needs_review)
        );
        assert!(
            matches!(&rereviewed.tasks[1].review,Nullable::Value(review) if review.needs_review)
        );
    }
}
