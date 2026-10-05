use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde_json::json;
use work_feature::discussion::{self, repository::DiscussionRepository};
use work_feature::error::{ExitCode, WorkError};
use work_model::common::Nullable;
use work_model::discussion::*;
use work_operations::derivation::fingerprint;
use work_operations::discussion::{DiscussionChange, DiscussionOperation};

#[derive(Default)]
struct Repository {
    history: RefCell<BTreeMap<u64, DiscussionSession>>,
    reads: Cell<usize>,
    writes: Cell<usize>,
    fail_save: Cell<bool>,
    fail_read: Cell<bool>,
}

fn error(reason: &str) -> WorkError {
    WorkError::new(
        ExitCode::IoFailure,
        reason,
        "Injected failure",
        json!({"saved":false}),
    )
}

impl DiscussionRepository for Repository {
    fn read_current(&self, _: &str) -> Result<Option<DiscussionSession>, WorkError> {
        self.reads.set(self.reads.get() + 1);
        if self.fail_read.get() {
            return Err(error("read_failed"));
        }
        Ok(self
            .history
            .borrow()
            .last_key_value()
            .map(|(_, s)| s.clone()))
    }
    fn read_history(&self, _: &str, r: u64) -> Result<DiscussionSession, WorkError> {
        self.history
            .borrow()
            .get(&r)
            .cloned()
            .ok_or_else(|| error("missing_history"))
    }
    fn initialize(&self, s: &DiscussionSession, _: bool) -> Result<DiscussionSession, WorkError> {
        work_operations::discussion::verify_integrity(s).map_err(|e| error(e.0))?;
        self.writes.set(self.writes.get() + 1);
        self.history.borrow_mut().insert(s.revision, s.clone());
        Ok(s.clone())
    }
    fn submit(
        &self,
        requirement: &str,
        op: &DiscussionOperation,
        _: bool,
    ) -> Result<DiscussionSession, WorkError> {
        if self.fail_save.get() {
            return Err(error("save_failed"));
        }
        let current = self.read_current(requirement)?.unwrap();
        let next = work_operations::discussion::apply(&current, op).map_err(|e| error(e.0))?;
        self.writes.set(self.writes.get() + 1);
        self.history
            .borrow_mut()
            .insert(next.revision, next.clone());
        Ok(next)
    }
}

fn initial(count: usize) -> DiscussionSession {
    let decisions:Vec<_>=(1..=count).map(|n|json!({"id":format!("D{n:03}"),"question":format!("Question {n}?"),"question_version":1,
        "options":[{"id":"first","label":"First","explanation":"Reason"}],"status":if n==count{"pending"}else{"confirmed"},
        "tentative_option_id":null,"resolution":if n==count{json!(null)}else{json!({"option_id":"first","rationale":format!("Reason {n}"),"confirmation_evidence":"User"})},
        "status_reason":"","status_evidence":"","added_reason":"Required choice","task_ids":[],"dependencies":[],"source_references":[format!("source-{n}.txt")]})).collect();
    let mut s:DiscussionSession=serde_json::from_value(json!({"schema":"work-discussion-session","requirement_id":"example","revision":1,
        "context":{"original_references":["source.txt"],"goal":"Deliver","scope":["Scope"],"constraints":[],
            "acceptance_criteria":[{"id":"ACCEPTANCE-001","criterion":"Verified"}],"confirmed_source":null,"work_type":"task","revision":1},
        "authorization":{"requirement_id":"example","project_root":"/project","directory":"outputs/work/discussions/example",
            "allowed_actions":["prepare_question","answer_question","update_decision"],"evidence":"User approved bounded save","revoked_reason":null},
        "tasks":[],"retired_task_ids":[],"decisions":decisions,"next_task_number":1,"next_decision_number":count+1,
        "continuation":{"current_task_id":null,"question":null},
        "commit":{"operation_id":"init","operation_sha256":"a".repeat(64),"previous_sha256":null,"content_sha256":"0".repeat(64)}})).unwrap();
    s.commit.content_sha256 = fingerprint::discussion_session(&s);
    s
}

fn question(n: usize) -> SavedQuestion {
    SavedQuestion {
        decision_id: format!("D{n:03}"),
        question_version: 1,
        text: format!("Question {n}?"),
        display_options: BTreeMap::from([("1".into(), "first".into())]),
    }
}

fn answer(
    view: &work_operations::discussion::DiscussionView,
    n: usize,
    version: u64,
) -> DiscussionOperation {
    DiscussionOperation {
        operation_id: "answer".into(),
        expected_revision: view.revision,
        previous_sha256: view.content_sha256.clone(),
        change: DiscussionChange::AnswerQuestion {
            decision_id: format!("D{n:03}"),
            question_version: version,
            display_number: "1".into(),
            rationale: "User selected first".into(),
            evidence: "Answer 1".into(),
        },
    }
}

#[test]
fn restores_question_before_answering_one_without_replaying_seven_decisions() {
    let repo = Repository::default();
    let initial = initial(8);
    discussion::initialize(&repo, &initial, false).unwrap();
    let saved = discussion::prepare_question(&repo, "example", "prepare", question(8)).unwrap();
    assert!(saved.saved);
    assert_eq!(saved.view.progress.confirmed, 7);
    assert_eq!(saved.view.decisions.len(), 1);
    let restored = discussion::read(&repo, "example").unwrap();
    let answered = discussion::submit(&repo, "example", &answer(&restored, 8, 2), false).unwrap();
    assert_eq!(answered.view.progress.confirmed, 8);
    assert_eq!(answered.view.continuation.question, Nullable::Null);
    let current = repo.read_current("example").unwrap().unwrap();
    assert_eq!(&current.decisions[..7], &initial.decisions[..7]);
    assert!(repo.reads.get() >= 8);
    assert_eq!(
        discussion::decision_detail(&repo, "example", "D001", Some(1)).unwrap(),
        initial.decisions[0]
    );
}

#[test]
fn save_and_read_failures_never_return_a_question_or_advance() {
    let repo = Repository::default();
    discussion::initialize(&repo, &initial(1), false).unwrap();
    repo.fail_save.set(true);
    let failure =
        discussion::prepare_question(&repo, "example", "prepare", question(1)).unwrap_err();
    assert_eq!(failure.reason_code, "save_failed");
    assert_eq!(failure.details["saved"], false);
    assert_eq!(failure.details["last_verified_revision"], 1);
    assert_eq!(failure.details["operation_id"], "prepare");
    assert_eq!(
        repo.read_current("example")
            .unwrap()
            .unwrap()
            .continuation
            .question,
        Nullable::Null
    );
    assert_eq!(repo.writes.get(), 1);
    repo.fail_save.set(false);
    repo.fail_read.set(true);
    assert!(discussion::read(&repo, "example").is_err());
    assert!(discussion::prepare_question(&repo, "example", "prepare", question(1)).is_err());
    assert_eq!(repo.writes.get(), 1);
}

#[test]
fn unauthorized_scope_and_stale_answer_are_rejected_before_writes() {
    let repo = Repository::default();
    let mut denied = initial(1);
    denied.authorization.evidence.clear();
    denied.commit.content_sha256 = fingerprint::discussion_session(&denied);
    assert!(discussion::initialize(&repo, &denied, false).is_err());
    assert_eq!(repo.writes.get(), 0);
    discussion::initialize(&repo, &initial(1), false).unwrap();
    let saved = discussion::prepare_question(&repo, "example", "prepare", question(1)).unwrap();
    assert_eq!(
        discussion::submit(&repo, "example", &answer(&saved.view, 1, 1), false)
            .unwrap_err()
            .reason_code,
        "stale_discussion_question"
    );
    assert_eq!(repo.writes.get(), 2);
    assert_eq!(
        discussion::read(&repo, "example")
            .unwrap()
            .progress
            .confirmed,
        0
    );
}

#[test]
fn large_session_projection_only_includes_pending_and_required_dependencies() {
    let repo = Repository::default();
    let mut s = initial(300);
    s.decisions[299].dependencies = vec!["D001".into()];
    s.commit.content_sha256 = fingerprint::discussion_session(&s);
    discussion::initialize(&repo, &s, false).unwrap();
    let view = discussion::read(&repo, "example").unwrap();
    assert_eq!(view.progress.known_total, 300);
    assert_eq!(view.decisions.len(), 2);
    let raw = serde_json::to_value(&view).unwrap();
    assert!(!raw.to_string().contains("confirmation_evidence"));
    assert_eq!(
        discussion::decision_detail(&repo, "example", "D250", None)
            .unwrap()
            .resolution,
        s.decisions[249].resolution
    );
}

#[test]
fn staged_request_context_and_resume_follow_only_committed_state() {
    struct NoPublication;
    impl discussion::assembly::DiscussionPublicationRepository for NoPublication {
        fn preview(&self, _: &str, _: &serde_json::Value) -> Result<serde_json::Value, WorkError> {
            panic!("read must not generate a collection")
        }
        fn publish(
            &self,
            _: discussion::assembly::PublicationRequest<'_>,
        ) -> Result<serde_json::Value, WorkError> {
            panic!("save or read must not publish")
        }
    }
    let repo = Repository::default();
    let session = initial(8);
    discussion::initialize(&repo, &session, false).unwrap();
    let read: work_model::discussion::request::DiscussionRequest = serde_json::from_value(json!({
        "schema":"work-discussion-request","requirement_id":"example","command":{"kind":"read"}}))
    .unwrap();
    let context = discussion::request::context(&repo, &read).unwrap();
    discussion::request::verify_context(&repo, &read, &context).unwrap();
    let result = discussion::request::dispatch(&repo, &NoPublication, &read).unwrap();
    assert_eq!(result["data"]["progress"]["confirmed"], 7);
    assert_eq!(
        discussion::request::resume(&repo, "example").unwrap()["next_action"],
        "update"
    );
    discussion::prepare_question(&repo, "example", "ask-d008", question(8)).unwrap();
    assert_eq!(
        discussion::request::verify_context(&repo, &read, &context)
            .unwrap_err()
            .reason_code,
        "stale_discussion_context"
    );
    let resume = discussion::request::resume(&repo, "example").unwrap();
    assert_eq!(resume["next_action"], "answer");
    assert_eq!(
        resume["view"]["continuation"]["question"]["display_options"]["1"],
        "first"
    );
    assert_eq!(resume["planning_ready"], false);
    for name in work_model::discussion::contracts::COMMANDS {
        let definition = discussion::request::command_definition(name).unwrap();
        assert_eq!(definition["request_contract"], "work-discussion-request");
        assert_eq!(definition["execute_authorized"], false);
    }
    assert!(discussion::request::command_definition("draft-save").is_none());
    let ambiguous = serde_json::from_value(
        json!({"schema":"work-discussion-request","requirement_id":"example",
        "command":{"kind":"recover","session":null,"operation":null}}),
    )
    .unwrap();
    let writes = repo.writes.get();
    assert_eq!(
        discussion::request::dispatch(&repo, &NoPublication, &ambiguous)
            .unwrap_err()
            .reason_code,
        "ambiguous_discussion_recovery"
    );
    assert_eq!(repo.writes.get(), writes);
}
