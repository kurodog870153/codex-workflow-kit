//! I/O capabilities requested by Work use cases.

use std::path::Path;
use std::time::Duration;

use crate::error::WorkError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotBytes {
    pub manifest: work_model::source::snapshot::SourceSnapshot,
    pub bytes: Vec<u8>,
}

pub trait SourceSnapshotWriter {
    fn capture(
        &self,
        requirement_id: &work_model::identifiers::RequirementId,
        source: &work_model::source::snapshot::SnapshotSource,
        content_path: &work_model::source::snapshot::SourceContentPath,
        bytes: &[u8],
        captured_at: &str,
    ) -> Result<SnapshotBytes, WorkError>;
}

pub trait SourceSnapshotReader {
    fn read_snapshot_at(
        &self,
        requirement_id: &work_model::identifiers::RequirementId,
        source_id: &work_model::identifiers::SourceId,
        source_root: &str,
    ) -> Result<SnapshotBytes, WorkError>;
    fn read_snapshot(
        &self,
        requirement_id: &work_model::identifiers::RequirementId,
        source_id: &work_model::identifiers::SourceId,
    ) -> Result<SnapshotBytes, WorkError>;
}

pub trait ArtifactStore {
    fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError>;
    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError>;
    fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError>;
    fn remove(&self, path: &Path) -> Result<(), WorkError>;
    fn create_directories(&self, path: &Path) -> Result<(), WorkError>;
}

pub trait DocumentRepository {
    fn read_json(&self, path: &Path) -> Result<serde_json::Value, WorkError>;
    fn write_json_new(&self, path: &Path, value: &serde_json::Value) -> Result<(), WorkError>;
    fn read_yaml(&self, path: &Path) -> Result<serde_json::Value, WorkError>;
}

pub trait WriterGuard {}

pub trait WriterLock {
    type Guard: WriterGuard;
    fn acquire(&self, path: &Path) -> Result<Self::Guard, WorkError>;
    fn require_idle(&self, path: &Path) -> Result<(), WorkError>;
}

pub trait Git {
    fn read_only(&self, root: &Path, args: &[String]) -> Result<Vec<u8>, WorkError>;
    fn mutate(&self, root: &Path, args: &[String]) -> Result<Vec<u8>, WorkError>;
}

#[derive(Debug, Clone)]
pub struct CommandRequest {
    pub argv: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    Exited,
    TimedOut,
    LaunchFailed,
}

#[derive(Debug, Clone)]
pub struct CommandOutcome {
    pub status: CommandStatus,
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stdout_truncated: bool,
    pub stderr_tail: String,
    pub stderr_truncated: bool,
}

pub trait CommandRunner {
    fn run(&self, request: &CommandRequest) -> CommandOutcome;
}

pub trait ClockAndId {
    fn utc_timestamp(&self) -> String;
    fn random_suffix(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct FakeStore {
        files: RefCell<HashMap<PathBuf, Vec<u8>>>,
    }

    impl ArtifactStore for FakeStore {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            self.files.borrow().get(path).cloned().ok_or_else(|| {
                WorkError::new(
                    crate::error::ExitCode::ArtifactIntegrity,
                    "file_not_found",
                    "The required file does not exist or is not a regular file.",
                    serde_json::json!({"path": path.to_string_lossy()}),
                )
            })
        }

        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
            let mut files = self.files.borrow_mut();
            if files.contains_key(path) {
                return Err(WorkError::new(
                    crate::error::ExitCode::LockConflict,
                    "transaction_present",
                    "A transaction is already present.",
                    serde_json::json!({}),
                ));
            }
            files.insert(path.to_path_buf(), bytes.to_vec());
            Ok(())
        }

        fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
            let mut files = self.files.borrow_mut();
            let bytes = files.remove(temporary).ok_or_else(|| {
                WorkError::new(
                    crate::error::ExitCode::ArtifactIntegrity,
                    "file_not_found",
                    "The required file does not exist or is not a regular file.",
                    serde_json::json!({}),
                )
            })?;
            files.insert(target.to_path_buf(), bytes);
            Ok(())
        }

        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            self.files.borrow_mut().remove(path);
            Ok(())
        }

        fn create_directories(&self, _path: &Path) -> Result<(), WorkError> {
            Ok(())
        }
    }

    struct FakeCommand;

    impl CommandRunner for FakeCommand {
        fn run(&self, _request: &CommandRequest) -> CommandOutcome {
            CommandOutcome {
                status: CommandStatus::Exited,
                exit_code: Some(0),
                stdout_tail: "ok".into(),
                stdout_truncated: false,
                stderr_tail: String::new(),
                stderr_truncated: false,
            }
        }
    }

    #[test]
    fn use_case_can_run_against_fakes_without_cli_or_io() {
        fn publish(
            store: &impl ArtifactStore,
            path: &Path,
            bytes: &[u8],
        ) -> Result<Vec<u8>, WorkError> {
            store.create_new(path, bytes)?;
            store.read_raw(path)
        }
        let store = FakeStore::default();
        let output = publish(&store, Path::new("plan.json"), b"{}\n").unwrap();
        assert_eq!(output, b"{}\n");
        let command = FakeCommand.run(&CommandRequest {
            argv: vec!["example".into()],
            cwd: PathBuf::from("."),
            timeout: Duration::from_secs(1),
        });
        assert_eq!(command.status, CommandStatus::Exited);
    }
}
