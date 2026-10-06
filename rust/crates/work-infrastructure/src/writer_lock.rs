//! Cross-process writer mutexes.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalWriterLock;

/// Reject persistent legacy locks even when their old OS lock is no longer held.
/// Explicit custom execution paths come from verified TASK/Session/candidate contexts.
pub fn require_no_legacy_locks(
    context: &work_feature::ports::RequirementWriterContext,
    execution_dir: Option<&str>,
) -> Result<(), WorkError> {
    let (root, _) = runtime_lock_context(context, work_model::runtime::LockClass::Execution)?;
    let paths = work_feature::artifact_paths::default_artifact_paths(&context.requirement_id);
    let requirement = context.requirement_id.as_str();
    let mut locks = vec![
        format!("{}/.work-source-writer.lock", paths.source),
        format!("outputs/work/discussions/{requirement}/.work-state-writer.lock"),
        format!("outputs/work/discussions/{requirement}/.task-publication.lock"),
        format!("{}/.work-state-writer.lock", paths.execution),
    ];
    if let Some(execution) = execution_dir {
        if !work_operations::derivation::identity::runtime_relative_path(execution) {
            return Err(runtime_lock_error("runtime_lock_context_invalid", &root));
        }
        locks.push(format!("{execution}/.work-state-writer.lock"));
    }
    locks.sort();
    locks.dedup();
    let mut legacy = Vec::new();
    for relative in locks {
        let path = crate::files::resolve_runtime_path(&root, &relative)?;
        match std::fs::symlink_metadata(&path) {
            Ok(_) => legacy.push(relative),
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(runtime_lock_error("legacy_writer_lock_probe_failed", &path)),
        }
    }
    if legacy.is_empty() {
        Ok(())
    } else {
        Err(WorkError::new(
            ExitCode::LockConflict,
            "legacy_writer_lock_present",
            "Legacy writer evidence requires reviewed offline migration before current writers can run.",
            json!({"requirement_id":requirement,"legacy_locks":legacy}),
        ))
    }
}

/// Current owner-bound lock protocol with explicit release.
pub struct RuntimeFileGuard {
    file: Option<File>,
    root: PathBuf,
    relative: String,
    owner: work_model::runtime::RuntimeOwner,
    owner_raw: Vec<u8>,
    released: bool,
}

impl RuntimeFileGuard {
    fn release_owned(&mut self) -> Result<(), WorkError> {
        self.release_owned_with(|path| std::fs::remove_file(path))
    }

    fn release_owned_with(
        &mut self,
        remove: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), WorkError> {
        // Windows must close the writing handle before removing the lock entry.
        self.file.take();
        let path = crate::files::resolve_runtime_path(&self.root, &self.relative)?;
        let raw = std::fs::read(&path)
            .map_err(|_| runtime_lock_error("runtime_lock_release_read_failed", &path))?;
        if raw != self.owner_raw {
            return Err(runtime_lock_error("runtime_lock_owner_changed", &path));
        }
        let current: work_model::runtime::RuntimeOwner = serde_json::from_slice(&raw)
            .map_err(|_| runtime_lock_error("runtime_lock_owner_changed", &path))?;
        if current != self.owner || current.validate_shape().is_err() {
            return Err(runtime_lock_error("runtime_lock_owner_changed", &path));
        }
        remove(&path).map_err(|_| runtime_lock_error("runtime_lock_release_failed", &path))?;
        self.released = true;
        Ok(())
    }
}

impl work_feature::ports::RuntimeWriterGuard for RuntimeFileGuard {
    fn owner(&self) -> &work_model::runtime::RuntimeOwner {
        &self.owner
    }
    fn release(mut self) -> Result<(), WorkError> {
        self.release_owned()
    }
}

impl Drop for RuntimeFileGuard {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.release_owned();
        }
    }
}

fn runtime_lock_error(reason: &str, path: &Path) -> WorkError {
    WorkError::new(
        ExitCode::IoFailure,
        reason,
        "The requirement writer lock could not be safely acquired or released.",
        json!({"path":path.to_string_lossy()}),
    )
}

fn runtime_lock_context(
    context: &work_feature::ports::RequirementWriterContext,
    class: work_model::runtime::LockClass,
) -> Result<(PathBuf, String), WorkError> {
    let root = context.canonical_project_root.canonicalize().map_err(|_| {
        runtime_lock_error(
            "runtime_lock_root_unavailable",
            &context.canonical_project_root,
        )
    })?;
    if root != context.canonical_project_root || root.to_str().is_none() {
        return Err(runtime_lock_error(
            "runtime_lock_context_not_canonical",
            &context.canonical_project_root,
        ));
    }
    let relative = work_operations::derivation::publication::runtime_lock_path(
        &context.requirement_id,
        class.as_str(),
    )
    .map_err(|_| runtime_lock_error("runtime_lock_context_invalid", &root))?;
    Ok((root, relative))
}

impl work_feature::ports::RuntimeWriterLock for LocalWriterLock {
    type Guard = RuntimeFileGuard;

    fn acquire_runtime(
        &self,
        context: &work_feature::ports::RequirementWriterContext,
        class: work_model::runtime::LockClass,
    ) -> Result<Self::Guard, WorkError> {
        let (root, relative) = runtime_lock_context(context, class)?;
        require_no_legacy_locks(context, None)?;
        let inspect = || {
            crate::files::resolve_runtime_path(&root, &relative).map_err(|error| {
                if error.reason_code == "runtime_path_changed" {
                    busy()
                } else {
                    error
                }
            })
        };
        let path = inspect()?;
        std::fs::create_dir_all(path.parent().expect("derived lock parent"))
            .map_err(|_| runtime_lock_error("runtime_lock_parent_failed", &path))?;
        let path = inspect()?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|issue| {
                if issue.kind() == std::io::ErrorKind::AlreadyExists {
                    busy()
                } else {
                    runtime_lock_error("runtime_lock_create_failed", &path)
                }
            })?;
        let mut nonce_bytes = [0u8; 32];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|_| runtime_lock_error("runtime_lock_nonce_failed", &path))?;
        let mut nonce = String::with_capacity(nonce_bytes.len() * 2);
        for byte in nonce_bytes {
            write!(&mut nonce, "{byte:02x}").expect("writing to a String cannot fail");
        }
        let identity = work_operations::derivation::identity::runtime_owner_identity(
            &root.to_string_lossy(),
            &context.requirement_id,
            class.as_str(),
            &nonce,
        )
        .map_err(|_| runtime_lock_error("runtime_lock_context_invalid", &path))?;
        let owner = work_model::runtime::RuntimeOwner {
            canonical_root: root.to_string_lossy().into_owned(),
            requirement_id: context.requirement_id.as_str().into(),
            class,
            instance_nonce: nonce,
            owner_identity: identity,
        };
        let owner_raw = serde_json::to_vec(&owner)
            .map_err(|_| runtime_lock_error("runtime_lock_owner_encode_failed", &path))?;
        file.write_all(&owner_raw)
            .and_then(|_| file.sync_all())
            .map_err(|_| runtime_lock_error("runtime_lock_owner_write_failed", &path))?;
        if std::fs::read(&path)
            .map_err(|_| runtime_lock_error("runtime_lock_owner_read_failed", &path))?
            != owner_raw
        {
            return Err(runtime_lock_error(
                "runtime_lock_owner_readback_failed",
                &path,
            ));
        }
        Ok(RuntimeFileGuard {
            file: Some(file),
            root,
            relative,
            owner,
            owner_raw,
            released: false,
        })
    }

    fn require_runtime_idle(
        &self,
        context: &work_feature::ports::RequirementWriterContext,
        class: work_model::runtime::LockClass,
    ) -> Result<(), WorkError> {
        let (root, relative) = runtime_lock_context(context, class)?;
        require_no_legacy_locks(context, None)?;
        let path = crate::files::resolve_runtime_path(&root, &relative)?;
        match std::fs::symlink_metadata(&path) {
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(busy()),
            Err(_) => Err(runtime_lock_error("runtime_lock_probe_failed", &path)),
        }
    }
}

fn busy() -> WorkError {
    WorkError::new(
        ExitCode::LockConflict,
        "work_state_writer_busy",
        "Another Work command is updating this requirement.",
        json!({}),
    )
}

// Historical lock actors exist only to exercise legacy rejection in unit tests.
#[cfg(test)]
mod legacy_test_lock {
    use super::*;
    use fs4::FileExt;

    pub trait WriterLock {
        type Guard;
        fn acquire(&self, path: &Path) -> Result<Self::Guard, WorkError>;
        fn require_idle(&self, path: &Path) -> Result<(), WorkError>;
    }

    pub struct FileGuard(File);

    impl Drop for FileGuard {
        fn drop(&mut self) {
            let _ = FileExt::unlock(&self.0);
        }
    }

    impl WriterLock for LocalWriterLock {
        type Guard = FileGuard;

        fn acquire(&self, path: &Path) -> Result<Self::Guard, WorkError> {
            let file = OpenOptions::new()
                .read(true)
                .append(true)
                .create(true)
                .open(path)
                .map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "writer_lock_open_failed",
                        "The Work writer lock could not be opened.",
                        json!({"path": path.to_string_lossy()}),
                    )
                })?;
            FileExt::try_lock(&file).map_err(|_| busy())?;
            Ok(FileGuard(file))
        }

        fn require_idle(&self, path: &Path) -> Result<(), WorkError> {
            if !path.exists() {
                return Ok(());
            }
            let file = File::open(path).map_err(|_| busy())?;
            FileExt::try_lock(&file).map_err(|_| busy())?;
            FileExt::unlock(&file).map_err(|_| busy())
        }
    }
}

#[cfg(test)]
pub(crate) use legacy_test_lock::WriterLock;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn runtime_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-runtime-lock-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root.canonicalize().unwrap()
    }
    fn context(root: &Path) -> work_feature::ports::RequirementWriterContext {
        work_feature::ports::RequirementWriterContext {
            canonical_project_root: root.to_path_buf(),
            requirement_id: "example".parse().unwrap(),
        }
    }

    #[test]
    fn runtime_owner_release_is_explicit_and_unknown_metadata_stays_blocked() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        use work_model::runtime::LockClass;
        let root = runtime_root();
        let context = context(&root);
        let lock = LocalWriterLock;
        lock.require_runtime_idle(&context, LockClass::Execution)
            .unwrap();
        assert!(!root.join("outputs").exists());
        let guard = lock
            .acquire_runtime(&context, LockClass::Execution)
            .unwrap();
        let first = guard.owner().instance_nonce.clone();
        assert!(
            lock.acquire_runtime(&context, LockClass::Execution)
                .is_err()
        );
        assert!(
            lock.require_runtime_idle(&context, LockClass::Execution)
                .is_err()
        );
        let relative = work_operations::derivation::publication::runtime_lock_path(
            &context.requirement_id,
            "execution",
        )
        .unwrap();
        let path = root.join(relative);
        guard.release().unwrap();
        assert!(!path.exists());
        let next = lock
            .acquire_runtime(&context, LockClass::Execution)
            .unwrap();
        assert_ne!(first, next.owner().instance_nonce);
        next.release().unwrap();
        fs::write(&path, b"partial owner").unwrap();
        assert!(
            lock.require_runtime_idle(&context, LockClass::Execution)
                .is_err()
        );
        assert!(
            lock.acquire_runtime(&context, LockClass::Execution)
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"partial owner");
    }

    #[test]
    fn runtime_release_failure_retains_exact_owner_until_successful_reentry() {
        use work_feature::ports::RuntimeWriterLock;
        use work_model::runtime::LockClass;
        let root = runtime_root();
        let context = context(&root);
        let mut guard = LocalWriterLock
            .acquire_runtime(&context, LockClass::Source)
            .unwrap();
        let path = root.join(
            work_operations::derivation::publication::runtime_lock_path(
                &context.requirement_id,
                "source",
            )
            .unwrap(),
        );
        let raw = fs::read(&path).unwrap();
        assert_eq!(
            guard
                .release_owned_with(|_| Err(std::io::Error::from(
                    std::io::ErrorKind::PermissionDenied
                )))
                .unwrap_err()
                .reason_code,
            "runtime_lock_release_failed"
        );
        assert_eq!(fs::read(&path).unwrap(), raw);
        assert!(
            LocalWriterLock
                .require_runtime_idle(&context, LockClass::Source)
                .is_err()
        );
        guard.release_owned().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn runtime_gate_preserves_every_legacy_lock_and_creates_nothing() {
        use work_feature::ports::RuntimeWriterLock;
        use work_model::runtime::LockClass;
        for relative in [
            "outputs/work/sources/example/.work-source-writer.lock",
            "outputs/work/discussions/example/.work-state-writer.lock",
            "outputs/work/discussions/example/.task-publication.lock",
            "outputs/work/executions/example/.work-state-writer.lock",
            "自訂 空白/execution/.work-state-writer.lock",
        ] {
            let root = runtime_root();
            let context = context(&root);
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"legacy evidence").unwrap();
            assert_eq!(
                require_no_legacy_locks(&context, Some("自訂 空白/execution"))
                    .unwrap_err()
                    .reason_code,
                "legacy_writer_lock_present"
            );
            if !relative.starts_with("自訂") {
                for class in [
                    LockClass::Source,
                    LockClass::Discussion,
                    LockClass::Execution,
                ] {
                    assert!(LocalWriterLock.acquire_runtime(&context, class).is_err());
                    assert!(
                        LocalWriterLock
                            .require_runtime_idle(&context, class)
                            .is_err()
                    );
                }
            }
            assert_eq!(fs::read(&path).unwrap(), b"legacy evidence");
            assert!(!root.join("outputs/work/runtime").exists());
        }
    }

    #[test]
    fn runtime_release_preserves_foreign_owner_and_reports_primary_and_cleanup_errors() {
        use work_feature::ports::with_runtime_writer;
        use work_model::runtime::LockClass;
        let root = runtime_root();
        let context = context(&root);
        let path = root.join(
            work_operations::derivation::publication::runtime_lock_path(
                &context.requirement_id,
                "execution",
            )
            .unwrap(),
        );
        let result: Result<(), WorkError> =
            with_runtime_writer(&LocalWriterLock, &context, LockClass::Execution, |owner| {
                let mut foreign = owner.clone();
                foreign.instance_nonce = "b".repeat(64);
                foreign.owner_identity =
                    work_operations::derivation::identity::runtime_owner_identity(
                        &foreign.canonical_root,
                        &context.requirement_id,
                        "execution",
                        &foreign.instance_nonce,
                    )
                    .unwrap();
                fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
                Err(WorkError::new(
                    ExitCode::WorkflowState,
                    "controlled_failure",
                    "controlled failure",
                    json!({"original":true}),
                ))
            });
        let error = result.unwrap_err();
        assert_eq!(error.reason_code, "controlled_failure");
        assert_eq!(error.exit_code, ExitCode::WorkflowState);
        assert_eq!(
            error.details["writer_release"]["reason_code"],
            "runtime_lock_owner_changed"
        );
        let foreign: work_model::runtime::RuntimeOwner =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(foreign.instance_nonce, "b".repeat(64));
    }

    #[test]
    fn runtime_lock_worker() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        use work_model::runtime::LockClass;
        let Some(root) = std::env::var_os("WORK_RUNTIME_LOCK_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let context = context(&root);
        let guard = LocalWriterLock
            .acquire_runtime(&context, LockClass::Execution)
            .unwrap();
        fs::write(root.join("child-ready"), b"ready").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !root.join("child-release").is_file() {
            assert!(
                std::time::Instant::now() < deadline,
                "parent did not finish lock test"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        guard.release().unwrap();
    }

    #[test]
    fn runtime_lock_excludes_other_processes_and_killed_owner_is_never_assumed_idle() {
        use work_feature::ports::RuntimeWriterLock;
        use work_model::runtime::LockClass;
        for kill in [false, true] {
            let root = runtime_root();
            let context = context(&root);
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "writer_lock::tests::runtime_lock_worker",
                    "--nocapture",
                ])
                .env("WORK_RUNTIME_LOCK_TEST_ROOT", &root)
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !root.join("child-ready").is_file() {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "worker exited before acquiring lock"
                );
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker did not acquire lock"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(
                LocalWriterLock
                    .acquire_runtime(&context, LockClass::Execution)
                    .is_err()
            );
            if kill {
                let path = root.join(
                    work_operations::derivation::publication::runtime_lock_path(
                        &context.requirement_id,
                        "execution",
                    )
                    .unwrap(),
                );
                let owner = fs::read(&path).unwrap();
                child.kill().unwrap();
                assert!(!child.wait().unwrap().success());
                assert!(
                    LocalWriterLock
                        .require_runtime_idle(&context, LockClass::Execution)
                        .is_err()
                );
                assert!(
                    LocalWriterLock
                        .acquire_runtime(&context, LockClass::Execution)
                        .is_err()
                );
                assert_eq!(fs::read(path).unwrap(), owner);
            } else {
                fs::write(root.join("child-release"), b"release").unwrap();
                assert!(child.wait().unwrap().success());
                LocalWriterLock
                    .require_runtime_idle(&context, LockClass::Execution)
                    .unwrap();
            }
        }
    }

    #[test]
    fn idle_probe_does_not_create_file_and_guard_releases() {
        let root = std::env::temp_dir().join(format!(
            "work-rust-lock-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join(".work-state-writer.lock");
        let lock = LocalWriterLock;
        lock.require_idle(&path).unwrap();
        assert!(!path.exists());
        let guard = lock.acquire(&path).unwrap();
        assert_eq!(
            lock.require_idle(&path).unwrap_err().reason_code,
            "work_state_writer_busy"
        );
        drop(guard);
        lock.require_idle(&path).unwrap();
    }
}
