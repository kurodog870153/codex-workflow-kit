//! Immutable progress history with a single current-snapshot commit point.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{ArtifactStore, WriterLock};
use work_feature::progress::ProgressRepository;
use work_operations::derivation::fingerprint;

use crate::files::LocalFiles;
use crate::specification::storage::storage_path;
use crate::writer_lock::LocalWriterLock;

#[derive(Debug, Clone)]
pub struct LocalProgressStorage {
    pub project_root: PathBuf,
}

fn error(code: ExitCode, reason: &str, message: &str, details: serde_json::Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

impl LocalProgressStorage {
    fn path(&self, relative: &str) -> Result<PathBuf, WorkError> {
        storage_path(&self.project_root, relative)
    }

    fn interrupted(current: &str, history: &str) -> WorkError {
        error(
            ExitCode::IoFailure,
            "progress_save_interrupted",
            "Saving was interrupted. Preserve all files and read the last committed progress before deciding how to continue.",
            json!({"path":current,"history":history}),
        )
    }

    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(path).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "progress_read_failed",
                "The progress file could not be read.",
                json!({"path":path.to_string_lossy()}),
            )
        })
    }
}

impl ProgressRepository for LocalProgressStorage {
    fn read_current(&self, relative: &str) -> Result<Option<Vec<u8>>, WorkError> {
        let path = self.path(relative)?;
        if !path.exists() {
            Ok(None)
        } else {
            self.read_bytes(&path).map(Some)
        }
    }

    fn read_history(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        self.read_bytes(&self.path(relative)?)
    }

    fn history_exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(self.path(relative)?.exists())
    }

    fn publish(
        &self,
        directory: &str,
        history: &str,
        current: &str,
        raw: &[u8],
        previous_sha256: Option<&str>,
    ) -> Result<(), WorkError> {
        let directory_path = self.path(directory)?;
        fs::create_dir_all(&directory_path).map_err(|_| Self::interrupted(current, history))?;
        let lock_path = self.path(&format!("{directory}/.work-state-writer.lock"))?;
        let _guard = LocalWriterLock.acquire(&lock_path)?;
        let current_path = self.path(current)?;
        let current_raw = if current_path.exists() {
            Some(self.read_bytes(&current_path)?)
        } else {
            None
        };
        if current_raw
            .as_ref()
            .map(|raw| fingerprint::raw(raw))
            .as_deref()
            != previous_sha256
        {
            return Err(error(
                ExitCode::WorkflowState,
                "progress_revision_conflict",
                "Read and review current progress before saving the next revision.",
                json!({}),
            ));
        }
        let history_path = self.path(history)?;
        if history_path.exists() {
            return Err(error(
                ExitCode::WorkflowState,
                "progress_save_pending",
                "An existing uncommitted revision requires review; do not retry or overwrite it.",
                json!({"path":history}),
            ));
        }
        fs::create_dir_all(history_path.parent().expect("history has parent"))
            .map_err(|_| Self::interrupted(current, history))?;
        fs::create_dir(&history_path).map_err(|_| Self::interrupted(current, history))?;
        let history_file = self.path(&format!("{history}/progress.json"))?;
        LocalFiles
            .create_new(&history_file, raw)
            .map_err(|_| Self::interrupted(current, history))?;
        if self.read_bytes(&history_file)? != raw {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "progress_write_mismatch",
                "Saved progress differs from the reviewed bytes.",
                json!({}),
            ));
        }
        let pending = self.path(&format!("{history}/progress.pending"))?;
        LocalFiles
            .create_new(&pending, raw)
            .map_err(|_| Self::interrupted(current, history))?;
        if self.read_bytes(&pending)? != raw {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "progress_write_mismatch",
                "Saved progress differs from the reviewed bytes.",
                json!({}),
            ));
        }
        LocalFiles
            .replace(&pending, &current_path)
            .map_err(|_| Self::interrupted(current, history))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_feature::progress::{
        prepare_progress, prepare_progress_document, preview_progress, preview_progress_raw,
        read_progress, save_progress, save_progress_raw,
    };

    fn example() -> serde_json::Value {
        json!({"schema":"work-discussion-progress","requirement_id":"example","mode":"task","revision":1,"status":"discussion_only","title":"Example","request":"Example request.","current_task_id":null,"context":{},"source_status":[],"notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue."})
    }

    #[test]
    fn preview_save_resume_and_revision_conflict() {
        let root = std::env::temp_dir().join(format!(
            "work-progress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalProgressStorage { project_root: root };
        let candidate = example();
        let mut skipped_revision = candidate.clone();
        skipped_revision["revision"] = json!(2);
        assert_eq!(
            preview_progress(&storage, &skipped_revision, 0)
                .unwrap_err()
                .reason_code,
            "progress_revision_conflict"
        );
        let preview = preview_progress(&storage, &candidate, 0).unwrap();
        assert_eq!(
            preview["approved_sha256"],
            "a0082819b36e8120797f5edc5f4b4e33116f7adbd0c3b2971ea83555a0769516"
        );
        let approved = preview["approved_sha256"].as_str().unwrap();
        let saved = save_progress(&storage, &candidate, 0, approved).unwrap();
        assert_eq!(
            saved["sha256"],
            "b43e8b4fe1b6a66a2f2353f62a59082c56b9f1ae45511890a26d3fcb9d8f7d5e"
        );
        assert_eq!(
            read_progress(&storage, "example", "task").unwrap()["progress"],
            candidate
        );
        assert_eq!(
            preview_progress(&storage, &candidate, 0)
                .unwrap_err()
                .reason_code,
            "progress_revision_conflict"
        );
        let prepared =
            prepare_progress(&storage, &json!({"title":"Revised"}), "example", "task", 1).unwrap();
        assert_eq!(prepared["schema"], "work-progress-prepare");
        let second = save_progress(
            &storage,
            &prepared["progress"],
            1,
            prepared["approved_sha256"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(second["progress"]["revision"], 2);
        assert_eq!(
            fs::read(
                storage
                    .project_root
                    .join("outputs/work/progress/example/task/history/1/progress.json")
            )
            .unwrap(),
            work_operations::progress::render_progress(&candidate).unwrap()
        );
        let mut task = candidate.clone();
        task["requirement_id"] = json!("task-discussion");
        task["current_task_id"] = json!("TASK-001");
        let task_approval = preview_progress(&storage, &task, 0).unwrap()["approved_sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        save_progress(&storage, &task, 0, &task_approval).unwrap();
        let mut other_requirement = candidate.clone();
        other_requirement["requirement_id"] = json!("other");
        let other_approval =
            preview_progress(&storage, &other_requirement, 0).unwrap()["approved_sha256"]
                .as_str()
                .unwrap()
                .to_owned();
        save_progress(&storage, &other_requirement, 0, &other_approval).unwrap();
        assert_eq!(
            read_progress(&storage, "example", "task").unwrap()["progress"],
            second["progress"]
        );
        assert_eq!(
            read_progress(&storage, "task-discussion", "task").unwrap()["progress"],
            task
        );
        assert_eq!(
            read_progress(&storage, "other", "task").unwrap()["progress"],
            other_requirement
        );
    }

    #[test]
    fn changed_approval_pending_history_and_corrupt_history_preserve_current() {
        let root = std::env::temp_dir().join(format!(
            "work-progress-conflict-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalProgressStorage { project_root: root };
        let first = example();
        let approved = preview_progress(&storage, &first, 0).unwrap()["approved_sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut changed = first.clone();
        changed["notes"] = json!(["Unreviewed"]);
        assert_eq!(
            save_progress(&storage, &changed, 0, &approved)
                .unwrap_err()
                .reason_code,
            "progress_approval_changed"
        );
        assert!(!storage.project_root.join("outputs/work").exists());
        save_progress(&storage, &first, 0, &approved).unwrap();
        let mut next = first.clone();
        next["revision"] = json!(2);
        let approved_next = preview_progress(&storage, &next, 1).unwrap()["approved_sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let current = storage
            .project_root
            .join("outputs/work/progress/example/task/progress.json");
        let original = fs::read(&current).unwrap();
        fs::write(&current, b"incomplete unrelated edit").unwrap();
        assert_eq!(
            read_progress(&storage, "example", "task")
                .unwrap_err()
                .exit_code,
            ExitCode::InputFormat
        );
        assert_eq!(
            preview_progress(&storage, &next, 1).unwrap_err().exit_code,
            ExitCode::InputFormat
        );
        fs::write(&current, &original).unwrap();
        let history_file = storage
            .project_root
            .join("outputs/work/progress/example/task/history/1/progress.json");
        let mut changed_baseline = first.clone();
        changed_baseline["notes"] = json!(["Changed after approval"]);
        let changed_raw = work_operations::progress::render_progress(&changed_baseline).unwrap();
        fs::write(&current, &changed_raw).unwrap();
        fs::write(&history_file, &changed_raw).unwrap();
        assert_eq!(
            save_progress(&storage, &next, 1, &approved_next)
                .unwrap_err()
                .reason_code,
            "progress_approval_changed"
        );
        assert_eq!(fs::read(&current).unwrap(), changed_raw);
        fs::write(&current, &original).unwrap();
        fs::write(&history_file, &original).unwrap();
        let history = storage
            .project_root
            .join("outputs/work/progress/example/task/history/2");
        fs::create_dir(&history).unwrap();
        fs::write(history.join("progress.pending"), b"partial").unwrap();
        assert_eq!(
            save_progress(&storage, &next, 1, &approved_next)
                .unwrap_err()
                .reason_code,
            "progress_save_pending"
        );
        assert_eq!(
            read_progress(&storage, "example", "task").unwrap()["progress"],
            first
        );
        let old_history = storage
            .project_root
            .join("outputs/work/progress/example/task/history/1/progress.json");
        fs::write(old_history, b"changed history").unwrap();
        assert_eq!(
            read_progress(&storage, "example", "task")
                .unwrap_err()
                .reason_code,
            "progress_history_mismatch"
        );
    }

    #[test]
    fn unrelated_formal_sources_do_not_block_progress_and_writer_lock_does() {
        let root = std::env::temp_dir().join(format!(
            "work-progress-isolation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let formal = [
            (
                "outputs/work/plans/example.json",
                b"invalid Plan".as_slice(),
            ),
            (
                "outputs/work/tasks/example/index.json",
                b"invalid TASK".as_slice(),
            ),
            (
                "outputs/work/executions/example/index.json",
                b"invalid execution".as_slice(),
            ),
        ];
        for (relative, bytes) in formal {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let storage = LocalProgressStorage {
            project_root: root.clone(),
        };
        let mut candidate = example();
        candidate["mode"] = json!("task");
        candidate["current_task_id"] = json!("TASK-001");
        let approval = preview_progress(&storage, &candidate, 0).unwrap()["approved_sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let directory = root.join("outputs/work/progress/example/task");
        fs::create_dir_all(&directory).unwrap();
        let guard = LocalWriterLock
            .acquire(&directory.join(".work-state-writer.lock"))
            .unwrap();
        assert_eq!(
            save_progress(&storage, &candidate, 0, &approval)
                .unwrap_err()
                .reason_code,
            "work_state_writer_busy"
        );
        assert!(!directory.join("progress.json").exists());
        assert!(!directory.join("history").exists());
        drop(guard);
        save_progress(&storage, &candidate, 0, &approval).unwrap();
        assert_eq!(
            read_progress(&storage, "example", "task").unwrap()["progress"],
            candidate
        );
        for (relative, bytes) in formal {
            assert_eq!(fs::read(root.join(relative)).unwrap(), bytes);
        }
    }

    #[test]
    fn raw_context_order_survives_preview_save_and_read() {
        let root = std::env::temp_dir().join(format!(
            "work-progress-order-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalProgressStorage { project_root: root };
        let raw = br#"{"schema":"work-discussion-progress","requirement_id":"example","mode":"task","revision":1,"status":"discussion_only","title":"Example","request":"Example request.","current_task_id":null,"context":{"z":1,"a":2},"source_status":[],"notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue."}"#;
        let preview = preview_progress_raw(&storage, raw, 0).unwrap();
        assert_eq!(
            preview["approved_sha256"],
            "3ed93c18d99a0c5acf64a2b6a0d3f642395037548d302b369de42b574c85b56a"
        );
        let saved = save_progress_raw(
            &storage,
            raw,
            0,
            preview["approved_sha256"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            saved["sha256"],
            "b6643764e52be97a860af6d91387cbc853c3168b9218353af294a7a041c328ca"
        );
        assert_eq!(
            read_progress(&storage, "example", "task").unwrap()["sha256"],
            saved["sha256"]
        );
        let current = fs::read(
            storage
                .project_root
                .join("outputs/work/progress/example/task/progress.json"),
        )
        .unwrap();
        assert!(
            String::from_utf8(current)
                .unwrap()
                .contains("\"z\": 1,\n    \"a\": 2")
        );
        let prepared =
            prepare_progress_document(&storage, br#"{"title":"Revised"}"#, "example", "task", 1)
                .unwrap();
        assert!(
            String::from_utf8(prepared.candidate_raw.clone())
                .unwrap()
                .contains("\"z\": 1,\n    \"a\": 2")
        );
        let saved = save_progress_raw(
            &storage,
            &prepared.candidate_raw,
            1,
            prepared.response["approved_sha256"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(saved["progress"]["title"], "Revised");
    }

    #[test]
    fn unreadable_current_progress_has_specific_error_and_path() {
        let root = std::env::temp_dir().join(format!(
            "work-progress-read-error-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let current = root.join("outputs/work/progress/example/task/progress.json");
        fs::create_dir_all(&current).unwrap();
        let storage = LocalProgressStorage { project_root: root };
        let error = read_progress(&storage, "example", "task").unwrap_err();
        assert_eq!(error.reason_code, "progress_read_failed");
        assert_eq!(
            error.details["path"],
            fs::canonicalize(&current)
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );
    }

    #[cfg(unix)]
    #[test]
    fn progress_storage_rejects_linked_directory_and_current_file() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "work-progress-links-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let storage = LocalProgressStorage {
            project_root: root.clone(),
        };
        let target = root.join("other-storage");
        fs::create_dir(&target).unwrap();
        let directory = root.join("outputs/work/progress/example/task");
        fs::create_dir_all(directory.parent().unwrap()).unwrap();
        symlink(&target, &directory).unwrap();
        assert_eq!(
            preview_progress(&storage, &example(), 0)
                .unwrap_err()
                .exit_code,
            ExitCode::Contract
        );
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);

        let other_root = root.join("separate-project");
        fs::create_dir(&other_root).unwrap();
        let storage = LocalProgressStorage {
            project_root: other_root.clone(),
        };
        let candidate = example();
        let preview = preview_progress(&storage, &candidate, 0).unwrap();
        save_progress(
            &storage,
            &candidate,
            0,
            preview["approved_sha256"].as_str().unwrap(),
        )
        .unwrap();
        let current = other_root.join("outputs/work/progress/example/task/progress.json");
        fs::hard_link(&current, other_root.join("alias.json")).unwrap();
        assert_eq!(
            read_progress(&storage, "example", "task")
                .unwrap_err()
                .exit_code,
            ExitCode::Contract
        );
    }

    #[test]
    fn partial_first_progress_history_is_never_committed() {
        let root = std::env::temp_dir().join(format!(
            "work-progress-partial-first-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let history = root.join("outputs/work/progress/example/task/history/1");
        fs::create_dir_all(&history).unwrap();
        fs::write(history.join("progress.json"), b"{\"schema\":").unwrap();
        let storage = LocalProgressStorage { project_root: root };
        assert_eq!(
            read_progress(&storage, "example", "task")
                .unwrap_err()
                .reason_code,
            "progress_not_saved"
        );
        assert_eq!(
            preview_progress(&storage, &example(), 0)
                .unwrap_err()
                .reason_code,
            "progress_save_pending"
        );
    }
}
