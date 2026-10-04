use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::progress::{
    ProgressRepository, prepare_progress, preview_progress, read_progress, save_progress,
};

#[derive(Default)]
struct MemoryProgress {
    files: RefCell<BTreeMap<String, Vec<u8>>>,
}

impl ProgressRepository for MemoryProgress {
    fn read_current(&self, relative: &str) -> Result<Option<Vec<u8>>, WorkError> {
        Ok(self.files.borrow().get(relative).cloned())
    }

    fn read_history(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        self.files.borrow().get(relative).cloned().ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "progress_history_missing",
                "The saved progress history is missing.",
                json!({"path": relative}),
            )
        })
    }

    fn history_exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(self.files.borrow().contains_key(relative))
    }

    fn publish(
        &self,
        _directory: &str,
        history: &str,
        current: &str,
        raw: &[u8],
        _previous_sha256: Option<&str>,
    ) -> Result<(), WorkError> {
        let mut files = self.files.borrow_mut();
        files.insert(format!("{history}/progress.json"), raw.to_vec());
        files.insert(current.to_owned(), raw.to_vec());
        Ok(())
    }
}

fn progress(revision: u64, note: &str) -> Value {
    json!({
        "schema": "work-discussion-progress", "requirement_id": "example",
        "mode": "task", "revision": revision, "status": "discussion_only",
        "title": "Example", "request": "Example request.", "current_task_id": null,
        "context": {}, "source_status": [], "notes": [note],
        "confirmed_decisions": [], "tentative": [], "open_questions": [],
        "next_discussion_point": "Continue."
    })
}

#[test]
fn first_prepare_validates_before_repository_access_and_does_not_read_history() {
    #[derive(Default)]
    struct FirstRevisionRepository {
        calls: RefCell<Vec<&'static str>>,
    }

    impl ProgressRepository for FirstRevisionRepository {
        fn read_current(&self, _relative: &str) -> Result<Option<Vec<u8>>, WorkError> {
            self.calls.borrow_mut().push("read_current");
            Ok(None)
        }

        fn read_history(&self, _relative: &str) -> Result<Vec<u8>, WorkError> {
            panic!("first revision must not read saved progress history")
        }

        fn history_exists(&self, _relative: &str) -> Result<bool, WorkError> {
            self.calls.borrow_mut().push("history_exists");
            Ok(false)
        }

        fn publish(
            &self,
            _directory: &str,
            _history: &str,
            _current: &str,
            _raw: &[u8],
            _previous_sha256: Option<&str>,
        ) -> Result<(), WorkError> {
            panic!("prepare must not publish progress")
        }
    }

    let repo = FirstRevisionRepository::default();
    assert_eq!(
        prepare_progress(&repo, &json!({"unknown": true}), "example", "task", 0)
            .unwrap_err()
            .reason_code,
        "invalid_object_fields"
    );
    assert!(repo.calls.borrow().is_empty());

    let first = progress(1, "First discussion.");
    let mut semantic = first.as_object().unwrap().clone();
    for field in ["schema", "requirement_id", "mode", "revision", "status"] {
        semantic.remove(field);
    }
    let prepared = prepare_progress(&repo, &Value::Object(semantic), "example", "task", 0).unwrap();
    assert_eq!(prepared["schema"], "work-progress-prepare");
    assert_eq!(prepared["progress"]["revision"], 1);
    assert_eq!(
        repo.calls.borrow().as_slice(),
        ["read_current", "history_exists"]
    );
}

#[test]
fn preview_save_and_read_keep_revision_and_approval_boundaries() {
    let repo = MemoryProgress::default();
    assert_eq!(
        read_progress(&repo, "example", "task")
            .unwrap_err()
            .reason_code,
        "progress_not_saved"
    );

    let first = progress(1, "First discussion.");
    let preview = preview_progress(&repo, &first, 0).unwrap();
    let approved = preview["approved_sha256"].as_str().unwrap();
    let saved = save_progress(&repo, &first, 0, approved).unwrap();
    assert_eq!(saved["progress"], first);
    assert_eq!(
        read_progress(&repo, "example", "task").unwrap()["progress"],
        first
    );

    let second = progress(2, "Updated discussion.");
    assert_eq!(
        save_progress(&repo, &second, 1, approved)
            .unwrap_err()
            .reason_code,
        "progress_approval_changed"
    );
    let preview = preview_progress(&repo, &second, 1).unwrap();
    assert_ne!(preview["approved_sha256"], approved);
    let approved = preview["approved_sha256"].as_str().unwrap();
    let saved = save_progress(&repo, &second, 1, approved).unwrap();
    assert_eq!(saved["progress"], second);
    assert_eq!(
        read_progress(&repo, "example", "task").unwrap()["progress"],
        second
    );
    assert_eq!(repo.files.borrow().len(), 3);
}

#[test]
fn semantic_prepare_preserves_content_and_merges_only_requested_fields() {
    let repo = MemoryProgress::default();
    let mut first = progress(1, "First discussion.");
    first["context"] = json!({
        "plan_sha256": "a".repeat(64), "task_raw_sha256": "b".repeat(64),
        "status": "valid", "approved_sha256": "c".repeat(64)
    });
    let mut semantic = first.as_object().unwrap().clone();
    for field in ["schema", "requirement_id", "mode", "revision", "status"] {
        semantic.remove(field);
    }
    let prepared = prepare_progress(
        &repo,
        &Value::Object(semantic.clone()),
        "example",
        "task",
        0,
    )
    .unwrap();
    assert_eq!(prepared["progress"], first);
    assert_eq!(
        prepared["path"],
        "outputs/work/progress/example/task/progress.json"
    );
    assert_eq!(
        prepared["approved_sha256"],
        preview_progress(&repo, &first, 0).unwrap()["approved_sha256"]
    );
    assert_eq!(prepared["source_validation"], "not_checked");
    assert_eq!(prepared["evidence_trust"], "historical_context_only");
    assert_eq!(prepared["formal_readiness"], "not_established");
    assert!(repo.files.borrow().is_empty());
    save_progress(
        &repo,
        &first,
        0,
        prepared["approved_sha256"].as_str().unwrap(),
    )
    .unwrap();

    let changes = json!({"notes": ["Second discussion."], "open_questions": []});
    let second = prepare_progress(&repo, &changes, "example", "task", 1).unwrap();
    assert_eq!(second["progress"]["revision"], 2);
    assert_eq!(second["progress"]["notes"], changes["notes"]);
    assert_eq!(second["progress"]["title"], first["title"]);
    assert_eq!(second["progress"]["context"], first["context"]);
    assert_eq!(second["evidence_trust"], "historical_context_only");
    let previous_history =
        repo.files.borrow()["outputs/work/progress/example/task/history/1/progress.json"].clone();
    save_progress(
        &repo,
        &second["progress"],
        1,
        second["approved_sha256"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_progress(&repo, "example", "task").unwrap()["progress"],
        second["progress"]
    );
    assert_eq!(
        repo.files.borrow()["outputs/work/progress/example/task/history/1/progress.json"],
        previous_history
    );
    assert_eq!(
        prepare_progress(&repo, &json!({}), "example", "task", 1)
            .unwrap_err()
            .reason_code,
        "progress_prepare_empty_change"
    );
}

#[test]
fn prepare_rejects_machine_fields_missing_content_and_reserved_history() {
    let repo = MemoryProgress::default();
    let first = progress(1, "First discussion.");
    let mut semantic = first.as_object().unwrap().clone();
    for field in ["schema", "requirement_id", "mode", "revision", "status"] {
        semantic.remove(field);
    }
    for field in ["schema", "requirement_id", "mode", "revision", "status"] {
        let mut invalid = semantic.clone();
        invalid.insert(field.to_owned(), first[field].clone());
        assert_eq!(
            prepare_progress(&repo, &Value::Object(invalid), "example", "task", 0)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
    }
    let mut missing = semantic.clone();
    missing.remove("tentative");
    assert_eq!(
        prepare_progress(&repo, &Value::Object(missing), "example", "task", 0)
            .unwrap_err()
            .reason_code,
        "invalid_object_fields"
    );
    let mut wrong_identity = semantic.clone();
    wrong_identity.insert("current_task_id".into(), json!("TASK-1"));
    assert_eq!(
        prepare_progress(&repo, &Value::Object(wrong_identity), "example", "task", 0)
            .unwrap_err()
            .reason_code,
        "invalid_progress_task"
    );
    repo.files.borrow_mut().insert(
        "outputs/work/progress/example/task/history/1".into(),
        Vec::new(),
    );
    assert_eq!(
        preview_progress(&repo, &first, 0).unwrap_err().reason_code,
        "progress_save_pending"
    );
    assert!(
        repo.files
            .borrow()
            .get("outputs/work/progress/example/task/progress.json")
            .is_none()
    );
}

#[test]
fn read_rejects_changed_history_and_preserves_current_bytes() {
    let repo = MemoryProgress::default();
    let first = progress(1, "First discussion.");
    let approved = preview_progress(&repo, &first, 0).unwrap()["approved_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    save_progress(&repo, &first, 0, &approved).unwrap();
    let current_path = "outputs/work/progress/example/task/progress.json";
    let current = repo.files.borrow().get(current_path).unwrap().clone();
    repo.files.borrow_mut().insert(
        "outputs/work/progress/example/task/history/1/progress.json".into(),
        b"changed evidence".to_vec(),
    );
    assert_eq!(
        read_progress(&repo, "example", "task")
            .unwrap_err()
            .reason_code,
        "progress_history_mismatch"
    );
    assert_eq!(repo.files.borrow().get(current_path).unwrap(), &current);
}

#[test]
fn interrupted_publication_keeps_previous_commit_and_blocks_retry() {
    #[derive(Default)]
    struct InterruptedProgress {
        base: MemoryProgress,
        interrupt_next: Cell<bool>,
    }

    impl ProgressRepository for InterruptedProgress {
        fn read_current(&self, relative: &str) -> Result<Option<Vec<u8>>, WorkError> {
            self.base.read_current(relative)
        }

        fn read_history(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
            self.base.read_history(relative)
        }

        fn history_exists(&self, relative: &str) -> Result<bool, WorkError> {
            self.base.history_exists(relative)
        }

        fn publish(
            &self,
            directory: &str,
            history: &str,
            current: &str,
            raw: &[u8],
            previous_sha256: Option<&str>,
        ) -> Result<(), WorkError> {
            if self.interrupt_next.replace(false) {
                self.base
                    .files
                    .borrow_mut()
                    .insert(history.into(), Vec::new());
                return Err(WorkError::new(
                    ExitCode::IoFailure,
                    "progress_save_interrupted",
                    "Progress publication was interrupted.",
                    json!({}),
                ));
            }
            self.base
                .publish(directory, history, current, raw, previous_sha256)
        }
    }

    let repo = InterruptedProgress::default();
    let first = progress(1, "First discussion.");
    let first_approval = preview_progress(&repo, &first, 0).unwrap()["approved_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    save_progress(&repo, &first, 0, &first_approval).unwrap();
    let previous = read_progress(&repo, "example", "task").unwrap();
    let mut second = first.clone();
    second["revision"] = json!(2);
    second["notes"] = json!(["Updated discussion."]);
    let second_approval = preview_progress(&repo, &second, 1).unwrap()["approved_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    repo.interrupt_next.set(true);
    let error = save_progress(&repo, &second, 1, &second_approval).unwrap_err();
    assert_eq!(error.exit_code, ExitCode::IoFailure);
    assert_eq!(error.reason_code, "progress_save_interrupted");
    assert_eq!(read_progress(&repo, "example", "task").unwrap(), previous);
    let before = repo.base.files.borrow().clone();
    let error = save_progress(&repo, &second, 1, &second_approval).unwrap_err();
    assert_eq!(error.exit_code, ExitCode::WorkflowState);
    assert_eq!(error.reason_code, "progress_save_pending");
    assert_eq!(*repo.base.files.borrow(), before);
}

#[test]
fn prepare_rejects_corrupt_current_without_replacing_it() {
    let repo = MemoryProgress::default();
    let first = progress(1, "First discussion.");
    let approved = preview_progress(&repo, &first, 0).unwrap()["approved_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    save_progress(&repo, &first, 0, &approved).unwrap();
    let current_path = "outputs/work/progress/example/task/progress.json";
    repo.files
        .borrow_mut()
        .insert(current_path.into(), b"corrupt".to_vec());
    assert_eq!(
        prepare_progress(&repo, &json!({"notes": ["Changed"]}), "example", "task", 1)
            .unwrap_err()
            .reason_code,
        "invalid_json"
    );
    assert_eq!(repo.files.borrow()[current_path], b"corrupt");
}

#[test]
fn plan_progress_is_rejected_without_publication() {
    let repo = MemoryProgress::default();
    let mut legacy = progress(1, "Historical discussion");
    legacy["mode"] = json!("plan");
    assert_eq!(
        read_progress(&repo, "example", "plan")
            .unwrap_err()
            .reason_code,
        "invalid_progress_mode"
    );
    assert_eq!(
        prepare_progress(&repo, &json!({}), "example", "plan", 0)
            .unwrap_err()
            .reason_code,
        "invalid_progress_mode"
    );
    assert_eq!(
        preview_progress(&repo, &legacy, 0).unwrap_err().reason_code,
        "invalid_progress_mode"
    );
    assert_eq!(
        save_progress(&repo, &legacy, 0, &"0".repeat(64))
            .unwrap_err()
            .reason_code,
        "invalid_progress_mode"
    );
    assert!(repo.files.borrow().is_empty());
    assert!(serde_json::from_value::<work_model::progress::DiscussionProgress>(legacy).is_err());
}
