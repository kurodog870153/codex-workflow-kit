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

/// Supplied by a validated Source/Session/TASK context, never an execution-dir basename.
#[derive(Debug, Clone)]
pub struct RequirementWriterContext {
    pub canonical_project_root: std::path::PathBuf,
    pub requirement_id: work_model::identifiers::RequirementId,
}

pub trait RuntimeWriterGuard {
    fn owner(&self) -> &work_model::runtime::RuntimeOwner;
    fn release(self) -> Result<(), WorkError>;
}

/// Separate capability while all existing writers still use their original contract.
pub trait RuntimeWriterLock {
    type Guard: RuntimeWriterGuard;
    fn acquire_runtime(
        &self,
        context: &RequirementWriterContext,
        class: work_model::runtime::LockClass,
    ) -> Result<Self::Guard, WorkError>;
    fn require_runtime_idle(
        &self,
        context: &RequirementWriterContext,
        class: work_model::runtime::LockClass,
    ) -> Result<(), WorkError>;
}

pub trait RuntimeTransactionRepository {
    fn prepare_runtime(
        &self,
        manifest: &work_model::runtime::RuntimeManifest,
        payloads: &std::collections::BTreeMap<String, Vec<u8>>,
    ) -> Result<String, WorkError>;
    fn read_runtime(
        &self,
        expected: &work_model::runtime::RuntimeManifest,
    ) -> Result<work_model::runtime::RuntimeManifest, WorkError>;
    fn update_runtime(
        &self,
        expected: &work_model::runtime::RuntimeManifest,
        published_count: usize,
        phase: work_model::runtime::RuntimePhase,
    ) -> Result<(), WorkError>;
    fn cleanup_runtime(
        &self,
        expected: &work_model::runtime::RuntimeManifest,
    ) -> Result<(), WorkError>;
}

/// Explicit release covers ordinary success and every returned error from the critical section.
pub fn with_runtime_writer<T>(
    lock: &impl RuntimeWriterLock,
    context: &RequirementWriterContext,
    class: work_model::runtime::LockClass,
    write: impl FnOnce(&work_model::runtime::RuntimeOwner) -> Result<T, WorkError>,
) -> Result<T, WorkError> {
    let guard = lock.acquire_runtime(context, class)?;
    let owner = guard.owner();
    let operation = if owner.validate_shape().is_ok()
        && owner.canonical_root == context.canonical_project_root.to_string_lossy()
        && owner.requirement_id == context.requirement_id.as_str()
        && owner.class == class
    {
        write(owner)
    } else {
        Err(WorkError::new(
            crate::error::ExitCode::ArtifactIntegrity,
            "runtime_owner_context_mismatch",
            "Writer owner does not match the validated requirement context.",
            serde_json::json!({}),
        ))
    };
    let release = guard.release();
    match (operation, release) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(release)) => Err(release),
        (Err(operation), Ok(())) => Err(operation),
        (Err(mut operation), Err(release)) => {
            operation.details = serde_json::json!({"operation_details":operation.details,
                "writer_release":{"exit_code":release.exit_code as i32,"reason_code":release.reason_code,"message":release.message,"details":release.details}});
            Err(operation)
        }
    }
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

    struct FakeRuntimeGuard {
        owner: work_model::runtime::RuntimeOwner,
        releases: std::rc::Rc<std::cell::Cell<usize>>,
        fail_release: bool,
    }
    impl RuntimeWriterGuard for FakeRuntimeGuard {
        fn owner(&self) -> &work_model::runtime::RuntimeOwner {
            &self.owner
        }
        fn release(self) -> Result<(), WorkError> {
            self.releases.set(self.releases.get() + 1);
            if self.fail_release {
                Err(WorkError::new(
                    crate::error::ExitCode::IoFailure,
                    "release_failed",
                    "release failed",
                    serde_json::json!({"owner":self.owner.owner_identity}),
                ))
            } else {
                Ok(())
            }
        }
    }
    struct FakeRuntimeLock {
        releases: std::rc::Rc<std::cell::Cell<usize>>,
        fail_release: bool,
        wrong_owner: bool,
    }
    impl RuntimeWriterLock for FakeRuntimeLock {
        type Guard = FakeRuntimeGuard;
        fn acquire_runtime(
            &self,
            context: &RequirementWriterContext,
            class: work_model::runtime::LockClass,
        ) -> Result<Self::Guard, WorkError> {
            Ok(FakeRuntimeGuard {
                owner: work_model::runtime::RuntimeOwner {
                    canonical_root: context
                        .canonical_project_root
                        .to_string_lossy()
                        .into_owned(),
                    requirement_id: if self.wrong_owner {
                        "other".into()
                    } else {
                        context.requirement_id.as_str().into()
                    },
                    class,
                    instance_nonce: "a".repeat(64),
                    owner_identity: "b".repeat(64),
                },
                releases: self.releases.clone(),
                fail_release: self.fail_release,
            })
        }
        fn require_runtime_idle(
            &self,
            _: &RequirementWriterContext,
            _: work_model::runtime::LockClass,
        ) -> Result<(), WorkError> {
            Ok(())
        }
    }

    #[test]
    fn runtime_writer_releases_on_success_error_and_wrong_context_with_both_errors_retained() {
        use work_model::runtime::LockClass;
        let context = RequirementWriterContext {
            canonical_project_root: "/project".into(),
            requirement_id: "example".parse().unwrap(),
        };
        for (fail_operation, fail_release, wrong_owner) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, true, false),
            (false, false, true),
        ] {
            let releases = std::rc::Rc::new(std::cell::Cell::new(0));
            let lock = FakeRuntimeLock {
                releases: releases.clone(),
                fail_release,
                wrong_owner,
            };
            let calls = std::cell::Cell::new(0);
            let result = with_runtime_writer(&lock, &context, LockClass::Execution, |owner| {
                calls.set(calls.get() + 1);
                assert_eq!(owner.requirement_id, "example");
                if fail_operation {
                    Err(WorkError::new(
                        crate::error::ExitCode::Contract,
                        "operation_failed",
                        "operation failed",
                        serde_json::json!({"original":true}),
                    ))
                } else {
                    Ok(42)
                }
            });
            assert_eq!(releases.get(), 1);
            assert_eq!(calls.get(), usize::from(!wrong_owner));
            if wrong_owner {
                assert_eq!(
                    result.unwrap_err().reason_code,
                    "runtime_owner_context_mismatch"
                );
            } else if fail_operation {
                let error = result.unwrap_err();
                assert_eq!(error.reason_code, "operation_failed");
                assert_eq!(error.exit_code, crate::error::ExitCode::Contract);
                if fail_release {
                    assert_eq!(error.details["operation_details"]["original"], true);
                    assert_eq!(
                        error.details["writer_release"]["reason_code"],
                        "release_failed"
                    );
                } else {
                    assert_eq!(error.details["original"], true);
                }
            } else if fail_release {
                assert_eq!(result.unwrap_err().reason_code, "release_failed");
            } else {
                assert_eq!(result.unwrap(), 42);
            }
        }
    }

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
