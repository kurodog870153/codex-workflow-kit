//! Immutable discussion snapshots with current as the sole logical commit point.

use std::path::{Path, PathBuf};

use serde_json::json;
use work_feature::discussion::repository::DiscussionRepository;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{ArtifactStore, WriterLock};
use work_model::discussion::DiscussionSession;
use work_operations::canonical::{canonical_json, parse_json_contract};
use work_operations::discussion::{self, DiscussionOperation};

use crate::files::LocalFiles;
use crate::specification::storage::storage_path;
use crate::writer_lock::LocalWriterLock;

#[derive(Debug, Clone)]
pub struct LocalDiscussionStorage<S = LocalFiles> {
    pub project_root: PathBuf,
    pub files: S,
}

fn error(reason: &str) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        reason,
        "Discussion state is not verified. Preserve evidence and check the last committed revision before continuing.",
        json!({"saved":false}),
    )
}

fn rule(e: discussion::DiscussionIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        e.0,
        "The discussion operation was rejected before publication.",
        json!({"saved":false}),
    )
}

fn bytes(session: &DiscussionSession) -> Vec<u8> {
    canonical_json(&serde_json::to_value(session).expect("Session serializes"))
        .expect("JSON serializes")
}

impl<S: ArtifactStore> LocalDiscussionStorage<S> {
    fn directory(&self, requirement: &str) -> Result<String, WorkError> {
        requirement
            .parse::<work_model::identifiers::RequirementId>()
            .map_err(|e| error(e.reason_code()))?;
        Ok(format!("outputs/work/discussions/{requirement}"))
    }

    fn path(&self, relative: &str) -> Result<PathBuf, WorkError> {
        storage_path(&self.project_root, relative)
    }

    fn decode(
        &self,
        relative: &str,
        requirement: &str,
        revision: Option<u64>,
    ) -> Result<DiscussionSession, WorkError> {
        let raw = self.files.read_raw(&self.path(relative)?)?;
        let value = parse_json_contract(&raw).map_err(|_| error("invalid_discussion_json"))?;
        let s: DiscussionSession =
            serde_json::from_value(value).map_err(|_| error("invalid_discussion_contract"))?;
        discussion::verify_integrity(&s).map_err(rule)?;
        if s.requirement_id != requirement
            || revision.is_some_and(|r| s.revision != r)
            || bytes(&s) != raw
        {
            return Err(error("discussion_snapshot_identity"));
        }
        Ok(s)
    }

    fn authorized_root(&self, session: &DiscussionSession) -> Result<(), WorkError> {
        discussion::validate_authorization(session, None).map_err(rule)?;
        let root = self
            .project_root
            .canonicalize()
            .map_err(|_| error("discussion_project_root"))?;
        // Authorization identifies the already resolved root, not an alias from the caller.
        if Path::new(&session.authorization.project_root) != root {
            return Err(error("discussion_save_scope_mismatch"));
        }
        self.path(&session.authorization.directory)?;
        Ok(())
    }

    fn committed(&self, requirement: &str) -> Result<Option<DiscussionSession>, WorkError> {
        let dir = self.directory(requirement)?;
        let relative = format!("{dir}/session.json");
        if !self.path(&relative)?.exists() {
            return Ok(None);
        }
        let current = self.decode(&relative, requirement, None)?;
        let history = self.decode(
            &format!("{dir}/history/{}/session.json", current.revision),
            requirement,
            Some(current.revision),
        )?;
        if current != history {
            return Err(error("discussion_current_history_mismatch"));
        }
        Ok(Some(current))
    }

    fn publish(
        &self,
        session: &DiscussionSession,
        recover: bool,
    ) -> Result<DiscussionSession, WorkError> {
        let dir = self.directory(&session.requirement_id)?;
        let history = format!("{dir}/history/{}", session.revision);
        let history_path = self.path(&history)?;
        let snapshot = format!("{history}/session.json");
        let raw = bytes(session);
        if history_path.exists() {
            if !recover {
                return Err(error("discussion_recovery_required"));
            }
            let saved = self.decode(&snapshot, &session.requirement_id, Some(session.revision))?;
            if saved != *session {
                return Err(error("discussion_prepared_operation_conflict"));
            }
        } else {
            self.files.create_directories(&history_path)?;
            self.files.create_new(&self.path(&snapshot)?, &raw)?;
        }
        let saved = self.decode(&snapshot, &session.requirement_id, Some(session.revision))?;
        if saved != *session {
            return Err(error("discussion_history_write_mismatch"));
        }
        let pending = self.path(&format!("{history}/session.pending"))?;
        if pending.exists() {
            if !recover || self.files.read_raw(&pending)? != raw {
                return Err(error("discussion_pending_operation_conflict"));
            }
        } else {
            self.files.create_new(&pending, &raw)?;
        }
        if self.files.read_raw(&pending)? != raw {
            return Err(error("discussion_pending_write_mismatch"));
        }
        // Not a physical multi-file transaction: this replacement alone commits.
        self.files
            .replace(&pending, &self.path(&format!("{dir}/session.json"))?)?;
        let reread = self
            .committed(&session.requirement_id)?
            .ok_or_else(|| error("discussion_commit_unknown"))?;
        if reread != *session {
            return Err(error("discussion_commit_unknown"));
        }
        Ok(reread)
    }

    fn replay(
        &self,
        current: &DiscussionSession,
        operation: &DiscussionOperation,
    ) -> Result<Option<DiscussionSession>, WorkError> {
        let dir = self.directory(&current.requirement_id)?;
        let digest = discussion::operation_sha256(operation);
        let mut previous = work_model::common::Nullable::Null;
        let mut result = None;
        // Only history anchored in current is eligible for deduplication.
        for revision in 1..=current.revision {
            let s = self.decode(
                &format!("{dir}/history/{revision}/session.json"),
                &current.requirement_id,
                Some(revision),
            )?;
            if s.commit.previous_sha256 != previous {
                return Err(error("discussion_history_chain_mismatch"));
            }
            previous = work_model::common::Nullable::Value(s.commit.content_sha256.clone());
            if s.commit.operation_id == operation.operation_id {
                if s.commit.operation_sha256 != digest {
                    return Err(error("discussion_operation_id_conflict"));
                }
                result = Some(s);
            }
        }
        Ok(result)
    }
}

impl<S: ArtifactStore> DiscussionRepository for LocalDiscussionStorage<S> {
    fn read_current(&self, requirement: &str) -> Result<Option<DiscussionSession>, WorkError> {
        self.committed(requirement)
    }

    fn read_history(
        &self,
        requirement: &str,
        revision: u64,
    ) -> Result<DiscussionSession, WorkError> {
        let current = self
            .committed(requirement)?
            .ok_or_else(|| error("discussion_not_initialized"))?;
        if revision == 0 || revision > current.revision {
            return Err(error("discussion_history_not_committed"));
        }
        let dir = self.directory(requirement)?;
        let mut saved = current;
        while saved.revision > revision {
            let predecessor = self.decode(
                &format!("{dir}/history/{}/session.json", saved.revision - 1),
                requirement,
                Some(saved.revision - 1),
            )?;
            if saved.commit.previous_sha256
                != work_model::common::Nullable::Value(predecessor.commit.content_sha256.clone())
            {
                return Err(error("discussion_history_chain_mismatch"));
            }
            saved = predecessor;
        }
        Ok(saved)
    }

    fn initialize(
        &self,
        session: &DiscussionSession,
        recover: bool,
    ) -> Result<DiscussionSession, WorkError> {
        discussion::verify_integrity(session).map_err(rule)?;
        self.authorized_root(session)?;
        if session.revision != 1 {
            return Err(error("discussion_initial_revision"));
        }
        let dir = self.directory(&session.requirement_id)?;
        self.files.create_directories(&self.path(&dir)?)?;
        let _guard =
            LocalWriterLock.acquire(&self.path(&format!("{dir}/.work-state-writer.lock"))?)?;
        if let Some(current) = self.committed(&session.requirement_id)? {
            let original = self.read_history(&session.requirement_id, 1)?;
            if original != *session {
                return Err(error("discussion_already_initialized"));
            }
            self.authorized_root(&current)?;
            return Ok(original);
        }
        self.publish(session, recover)
    }

    fn submit(
        &self,
        requirement: &str,
        operation: &DiscussionOperation,
        recover: bool,
    ) -> Result<DiscussionSession, WorkError> {
        let current = self
            .committed(requirement)?
            .ok_or_else(|| error("discussion_not_initialized"))?;
        self.authorized_root(&current)?;
        discussion::validate_authorization(&current, Some(operation.change.action()))
            .map_err(rule)?;
        let dir = self.directory(requirement)?;
        let _guard =
            LocalWriterLock.acquire(&self.path(&format!("{dir}/.work-state-writer.lock"))?)?;
        let current = self
            .committed(requirement)?
            .ok_or_else(|| error("discussion_not_initialized"))?;
        self.authorized_root(&current)?;
        discussion::validate_authorization(&current, Some(operation.change.action()))
            .map_err(rule)?;
        if let Some(original) = self.replay(&current, operation)? {
            return Ok(original);
        }
        let next = discussion::apply(&current, operation).map_err(rule)?;
        self.publish(&next, recover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};
    use work_model::common::Nullable;
    use work_operations::derivation::fingerprint;
    use work_operations::discussion::{DiscussionChange, DiscussionOperation};

    fn root() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "work-discussion-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        p.canonicalize().unwrap()
    }

    fn session(root: &Path) -> DiscussionSession {
        let mut s:DiscussionSession=serde_json::from_value(json!({
            "schema":"work-discussion-session","requirement_id":"example","revision":1,
            "context":{"original_references":["source.txt"],"goal":"Deliver","scope":["Scope"],"constraints":[],
                "acceptance_criteria":[{"id":"ACCEPTANCE-001","criterion":"Verified"}],"confirmed_source":null,"work_type":"task","revision":1},
            "authorization":{"requirement_id":"example","project_root":root.to_string_lossy(),"directory":"outputs/work/discussions/example",
                "allowed_actions":["update_continuation","add_decision"],"evidence":"User approved bounded save","revoked_reason":null},
            "tasks":[],"retired_task_ids":[],"decisions":[],"next_task_number":1,"next_decision_number":1,
            "continuation":{"current_task_id":null,"question":null},
            "commit":{"operation_id":"init","operation_sha256":"a".repeat(64),"previous_sha256":null,"content_sha256":"0".repeat(64)}
        })).unwrap();
        s.commit.content_sha256 = fingerprint::discussion_session(&s);
        s
    }

    fn operation(s: &DiscussionSession, id: &str) -> DiscussionOperation {
        DiscussionOperation {
            operation_id: id.into(),
            expected_revision: s.revision,
            previous_sha256: s.commit.content_sha256.clone(),
            change: DiscussionChange::UpdateContinuation {
                current_task_id: Nullable::Null,
            },
        }
    }

    fn local(root: &Path) -> LocalDiscussionStorage {
        LocalDiscussionStorage {
            project_root: root.into(),
            files: LocalFiles,
        }
    }

    #[test]
    fn discussion_deduplicates_older_committed_operations_and_rejects_collision() {
        let root = root();
        let store = local(&root);
        let first = store.initialize(&session(&root), false).unwrap();
        let op = operation(&first, "update-1");
        let second = store.submit("example", &op, false).unwrap();
        let third = store
            .submit("example", &operation(&second, "update-2"), false)
            .unwrap();
        assert_eq!(store.submit("example", &op, false).unwrap(), second);
        assert_eq!(store.read_current("example").unwrap().unwrap(), third);
        let mut collision = op.clone();
        collision.expected_revision = 2;
        assert_eq!(
            store
                .submit("example", &collision, false)
                .unwrap_err()
                .reason_code,
            "discussion_operation_id_conflict"
        );
        assert_eq!(
            store
                .submit("example", &operation(&first, "stale"), false)
                .unwrap_err()
                .reason_code,
            "stale_discussion_revision"
        );
        assert_eq!(store.read_history("example", 1).unwrap(), first);
        assert_eq!(store.initialize(&session(&root), false).unwrap(), first);
    }

    #[test]
    fn discussion_rejects_unauthorized_and_unsafe_paths_without_creating_outputs() {
        let root = root();
        let store = local(&root);
        let mut s = session(&root);
        s.authorization.project_root = "/different-project".into();
        s.commit.content_sha256 = fingerprint::discussion_session(&s);
        assert_eq!(
            store.initialize(&s, false).unwrap_err().reason_code,
            "discussion_save_scope_mismatch"
        );
        assert!(!root.join("outputs").exists());
        for id in ["../escape", "con", "EXAMPLE", "a/b", "nul", "a?b"] {
            assert!(store.read_current(id).is_err());
        }
        let mut s = session(&root);
        s.authorization.evidence.clear();
        s.commit.content_sha256 = fingerprint::discussion_session(&s);
        assert!(store.initialize(&s, false).is_err());
        assert!(!root.join("outputs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn discussion_rejects_symlink_escape() {
        let outside = root();
        let root = root();
        fs::create_dir(root.join("outputs")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("outputs/work")).unwrap();
        assert!(local(&root).initialize(&session(&root), false).is_err());
        assert!(!outside.join("discussions").exists());
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Fault {
        HistoryDirectory,
        HistoryWrite,
        HistoryReadback,
        PendingWrite,
        Replace,
        CurrentReadback,
        PartialHistory,
        HistoryFlush,
        CrashBefore,
        CrashAfter,
    }

    struct FaultFiles {
        fault: Fault,
        fired: Cell<bool>,
        replaced: Cell<bool>,
    }
    impl FaultFiles {
        fn fail(&self) -> Result<(), WorkError> {
            self.fired.set(true);
            Err(error("injected_discussion_io_failure"))
        }
    }
    impl ArtifactStore for FaultFiles {
        fn read_raw(&self, p: &Path) -> Result<Vec<u8>, WorkError> {
            if !self.fired.get()
                && ((self.fault == Fault::HistoryReadback && p.ends_with("history/2/session.json"))
                    || (self.fault == Fault::CurrentReadback
                        && self.replaced.get()
                        && p.ends_with("discussions/example/session.json")))
            {
                self.fail()?;
            }
            LocalFiles.read_raw(p)
        }
        fn create_new(&self, p: &Path, b: &[u8]) -> Result<(), WorkError> {
            if !self.fired.get()
                && self.fault == Fault::HistoryFlush
                && p.ends_with("2/session.json")
            {
                LocalFiles.create_new(p, b)?;
                self.fail()?;
            }
            if !self.fired.get()
                && self.fault == Fault::PartialHistory
                && p.ends_with("2/session.json")
            {
                LocalFiles.create_new(p, &b[..b.len() / 2])?;
                self.fail()?;
            }
            if !self.fired.get()
                && ((self.fault == Fault::HistoryWrite && p.ends_with("2/session.json"))
                    || (self.fault == Fault::PendingWrite && p.ends_with("session.pending")))
            {
                self.fail()?;
            }
            LocalFiles.create_new(p, b)
        }
        fn replace(&self, p: &Path, t: &Path) -> Result<(), WorkError> {
            if self.fault == Fault::CrashBefore {
                std::process::exit(17);
            }
            if !self.fired.get() && self.fault == Fault::Replace {
                self.fail()?;
            }
            LocalFiles.replace(p, t)?;
            if self.fault == Fault::CrashAfter {
                std::process::exit(18);
            }
            self.replaced.set(true);
            Ok(())
        }
        fn remove(&self, _: &Path) -> Result<(), WorkError> {
            panic!("Recovery must never remove evidence")
        }
        fn create_directories(&self, p: &Path) -> Result<(), WorkError> {
            if !self.fired.get()
                && self.fault == Fault::HistoryDirectory
                && p.ends_with("history/2")
            {
                self.fail()?;
            }
            LocalFiles.create_directories(p)
        }
    }

    #[test]
    fn discussion_faults_keep_old_current_or_verified_commit_and_recover_same_operation() {
        for fault in [
            Fault::HistoryDirectory,
            Fault::HistoryWrite,
            Fault::HistoryFlush,
            Fault::HistoryReadback,
            Fault::PendingWrite,
            Fault::Replace,
            Fault::CurrentReadback,
            Fault::PartialHistory,
        ] {
            let root = root();
            let store = local(&root);
            let first = store.initialize(&session(&root), false).unwrap();
            let op = operation(&first, "update");
            let failing = LocalDiscussionStorage {
                project_root: root.clone(),
                files: FaultFiles {
                    fault,
                    fired: Cell::new(false),
                    replaced: Cell::new(false),
                },
            };
            assert!(
                failing.submit("example", &op, false).is_err(),
                "{fault:?} must reject the submission"
            );
            assert!(failing.files.fired.get(), "{fault:?} must be injected");
            let current = store.read_current("example").unwrap().unwrap();
            assert_eq!(
                current.revision,
                if fault == Fault::CurrentReadback {
                    2
                } else {
                    1
                }
            );
            if matches!(fault, Fault::HistoryWrite | Fault::PartialHistory) {
                assert!(store.submit("example", &op, true).is_err());
                assert_eq!(store.read_current("example").unwrap().unwrap().revision, 1);
            } else {
                let recovered = store.submit("example", &op, true).unwrap();
                assert_eq!(recovered.revision, 2);
                assert_eq!(store.submit("example", &op, false).unwrap(), recovered);
            }
            assert_eq!(store.read_history("example", 1).unwrap(), first);
        }
    }

    #[test]
    fn discussion_uncommitted_history_never_becomes_current_and_conflicting_recovery_is_rejected() {
        let root = root();
        let store = local(&root);
        let first = store.initialize(&session(&root), false).unwrap();
        let op = operation(&first, "update");
        let failing = LocalDiscussionStorage {
            project_root: root.clone(),
            files: FaultFiles {
                fault: Fault::Replace,
                fired: Cell::new(false),
                replaced: Cell::new(false),
            },
        };
        assert!(failing.submit("example", &op, false).is_err());
        assert_eq!(store.read_current("example").unwrap().unwrap(), first);
        assert_eq!(
            store.read_history("example", 2).unwrap_err().reason_code,
            "discussion_history_not_committed"
        );
        assert!(
            store
                .submit("example", &operation(&first, "different"), true)
                .is_err()
        );
        assert_eq!(
            store.submit("example", &op, false).unwrap_err().reason_code,
            "discussion_recovery_required"
        );
    }

    #[test]
    fn discussion_history_tampering_blocks_current_read() {
        let root = root();
        let store = local(&root);
        store.initialize(&session(&root), false).unwrap();
        let p = root.join("outputs/work/discussions/example/history/1/session.json");
        let mut raw = fs::read(&p).unwrap();
        raw.push(b' ');
        fs::write(p, raw).unwrap();
        assert_eq!(
            store.read_current("example").unwrap_err().reason_code,
            "discussion_snapshot_identity"
        );
    }

    #[test]
    fn discussion_process_worker() {
        let Ok(root) = std::env::var("WORK_DISCUSSION_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mode = std::env::var("WORK_DISCUSSION_TEST_MODE").unwrap();
        let store = local(&root);
        if mode == "hold" {
            let _guard = LocalWriterLock
                .acquire(&root.join("outputs/work/discussions/example/.work-state-writer.lock"))
                .unwrap();
            fs::write(root.join("ready"), b"ready").unwrap();
            while !root.join("release").exists() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        } else {
            let s = store.read_history("example", 1).unwrap();
            let op = operation(&s, &mode);
            if matches!(mode.as_str(), "crash-before" | "crash-after") {
                let crashing = LocalDiscussionStorage {
                    project_root: root.clone(),
                    files: FaultFiles {
                        fault: if mode == "crash-before" {
                            Fault::CrashBefore
                        } else {
                            Fault::CrashAfter
                        },
                        fired: Cell::new(false),
                        replaced: Cell::new(false),
                    },
                };
                crashing.submit("example", &op, false).unwrap();
                panic!("Expected process termination inside commit");
            }
            let r = store.submit("example", &op, false);
            fs::write(
                root.join(format!("{mode}.result")),
                r.map(|s| s.revision.to_string())
                    .unwrap_or_else(|e| e.reason_code),
            )
            .unwrap();
        }
    }

    fn spawn(root: &Path, mode: &str) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "discussion::storage::tests::discussion_process_worker",
                "--nocapture",
            ])
            .env("WORK_DISCUSSION_TEST_ROOT", root)
            .env("WORK_DISCUSSION_TEST_MODE", mode)
            .spawn()
            .unwrap()
    }

    #[test]
    fn discussion_two_processes_respect_lock_and_stale_writers() {
        let root = root();
        local(&root).initialize(&session(&root), false).unwrap();
        let mut holding = spawn(&root, "hold");
        let start = std::time::Instant::now();
        while !root.join("ready").exists() {
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(spawn(&root, "blocked").wait().unwrap().success());
        assert_eq!(
            fs::read_to_string(root.join("blocked.result")).unwrap(),
            "work_state_writer_busy"
        );
        fs::write(root.join("release"), b"release").unwrap();
        assert!(holding.wait().unwrap().success());
        assert!(spawn(&root, "first").wait().unwrap().success());
        assert!(spawn(&root, "stale").wait().unwrap().success());
        assert_eq!(fs::read_to_string(root.join("first.result")).unwrap(), "2");
        assert_eq!(
            fs::read_to_string(root.join("stale.result")).unwrap(),
            "stale_discussion_revision"
        );
    }

    #[test]
    fn discussion_process_termination_before_and_after_commit_preserves_recovery() {
        for mode in ["crash-before", "crash-after"] {
            let root = root();
            let store = local(&root);
            let first = store.initialize(&session(&root), false).unwrap();
            let status = spawn(&root, mode).wait().unwrap();
            assert_eq!(
                status.code(),
                Some(if mode == "crash-before" { 17 } else { 18 })
            );
            assert_eq!(
                store.read_current("example").unwrap().unwrap().revision,
                if mode == "crash-before" { 1 } else { 2 }
            );
            let op = operation(&first, mode);
            let recovered = store.submit("example", &op, true).unwrap();
            assert_eq!(recovered.revision, 2);
            assert_eq!(store.submit("example", &op, false).unwrap(), recovered);
            assert_eq!(store.read_history("example", 1).unwrap(), first);
        }
    }

    #[derive(Default)]
    struct CountedFiles {
        history_reads: Cell<usize>,
        history_bytes: Cell<usize>,
    }

    impl ArtifactStore for CountedFiles {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            let raw = LocalFiles.read_raw(path)?;
            if path.file_name() == Some(std::ffi::OsStr::new("session.json"))
                && path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    == Some(std::ffi::OsStr::new("history"))
            {
                self.history_reads.set(self.history_reads.get() + 1);
                self.history_bytes.set(self.history_bytes.get() + raw.len());
            }
            Ok(raw)
        }
        fn create_new(&self, path: &Path, raw: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, raw)
        }
        fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
            LocalFiles.replace(temporary, target)
        }
        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)
        }
        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
    }

    #[test]
    #[ignore = "explicit release-mode history cost measurement"]
    fn discussion_submission_history_cost_measurement() {
        println!(
            "discussion_cost_profile debug_assertions={}",
            cfg!(debug_assertions)
        );
        for decision_count in [1, 100] {
            for revisions in [10, 100, 1000] {
                let root = root();
                let store = LocalDiscussionStorage {
                    project_root: root.clone(),
                    files: CountedFiles::default(),
                };
                let mut current = session(&root);
                current.decisions = (1..=decision_count).map(|n| serde_json::from_value(json!({
                    "id":format!("D{n:03}"),"question":"Select the data format?","question_version":1,
                    "options":[{"id":"option_a","label":"JSON","explanation":"Keep structured fields"}],
                    "status":"confirmed","tentative_option_id":null,
                    "resolution":{"option_id":"option_a","rationale":"Structured fields meet the confirmed requirement", "confirmation_evidence":"User approved"},
                    "status_reason":"","status_evidence":"","added_reason":"Confirm format","task_ids":[],"dependencies":[],"source_references":["source.txt"]
                })).unwrap()).collect();
                current.next_decision_number = decision_count + 1;
                current.commit.content_sha256 = fingerprint::discussion_session(&current);
                store.initialize(&current, false).unwrap();
                // Build verified committed snapshots without the quadratic cost of measuring every prefix.
                for n in 2..=revisions {
                    current =
                        discussion::apply(&current, &operation(&current, &format!("setup-{n}")))
                            .unwrap();
                    store.publish(&current, false).unwrap();
                }
                let snapshot_bytes = bytes(&current).len();
                let mut timings = Vec::new();
                for sample in 0..5 {
                    store.files.history_reads.set(0);
                    store.files.history_bytes.set(0);
                    let op = operation(&current, &format!("measured-{sample}"));
                    let start = std::time::Instant::now();
                    let saved =
                        work_feature::discussion::submit(&store, "example", &op, false).unwrap();
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    // Six snapshot readbacks surround the full N-revision deduplication scan:
                    // feature read, storage reads before/after locking, prepared history,
                    // committed history after replacement, and feature result readback.
                    assert_eq!(store.files.history_reads.get() as u64, current.revision + 6);
                    println!(
                        "discussion_cost decisions={decision_count} base_revisions={revisions} current_revision={} sample={sample} snapshot_bytes={snapshot_bytes} history_reads={} history_bytes={} elapsed_ms={elapsed:.3}",
                        current.revision,
                        store.files.history_reads.get(),
                        store.files.history_bytes.get()
                    );
                    timings.push(elapsed);
                    current = store.read_current("example").unwrap().unwrap();
                    assert_eq!(saved.operation_revision, current.revision);
                }
                timings.sort_by(f64::total_cmp);
                println!(
                    "discussion_cost_summary decisions={decision_count} base_revisions={revisions} median_ms={:.3} max_ms={:.3}",
                    timings[2], timings[4]
                );
            }
        }
    }
}
