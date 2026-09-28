//! Durable prepared bytes and verified replacement for recovery.

use std::path::Path;

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;

pub fn prepare_recovery_target(
    store: &impl ArtifactStore,
    path: &Path,
    expected: &[u8],
) -> Result<(), WorkError> {
    if path.exists() {
        if store.read_raw(path)? == expected {
            return Ok(());
        }
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            json!({"path": path.to_string_lossy()}),
        ));
    }
    store.create_new(path, expected).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "execution_recovery_prepare_failed",
            "The canonical recovery target could not be prepared.",
            json!({"path": path.to_string_lossy()}),
        )
    })?;
    if store.read_raw(path)? != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            json!({"path": path.to_string_lossy(), "recovery_required": true, "transaction_stage": "recovery_target_prepared"}),
        ));
    }
    Ok(())
}

pub fn install_recovery_target(
    store: &impl ArtifactStore,
    temporary: &Path,
    target: &Path,
    expected: &[u8],
    source_bytes: &[u8],
    stage: &str,
) -> Result<(), WorkError> {
    if store.read_raw(temporary)? != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_prepared_bytes_mismatch",
            "The prepared transaction bytes do not match the canonical target.",
            json!({"path": temporary.to_string_lossy()}),
        ));
    }
    if store.read_raw(target)? != source_bytes {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_source_changed",
            "A recovery source changed before replacement.",
            json!({"path": target.to_string_lossy()}),
        ));
    }
    store.replace(temporary, target).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "execution_recovery_replace_failed",
            "The verified recovery target could not be installed.",
            json!({"path": temporary.to_string_lossy(), "recovery_required": true, "transaction_stage": stage}),
        )
    })?;
    if store.read_raw(target)? != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execution_recovery_stored_bytes_mismatch",
            "The installed recovery bytes do not match the verified target.",
            json!({"path": target.to_string_lossy(), "recovery_required": true, "transaction_stage": stage}),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::LocalFiles;
    use std::fs;

    struct FailedReplace;

    impl ArtifactStore for FailedReplace {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }

        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, bytes)
        }

        fn replace(&self, _temporary: &Path, _target: &Path) -> Result<(), WorkError> {
            Err(WorkError::new(
                ExitCode::IoFailure,
                "injected_replace_failure",
                "The injected replacement failed.",
                json!({}),
            ))
        }

        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)
        }

        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
    }

    fn root() -> std::path::PathBuf {
        loop {
            let mut random = [0_u8; 8];
            getrandom::fill(&mut random).unwrap();
            let suffix = u64::from_le_bytes(random);
            let root = std::env::temp_dir().join(format!(
                "work-rust-recovery-{}-{suffix:016x}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => return root,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create recovery test directory: {error}"),
            }
        }
    }

    #[test]
    fn prepare_is_durable_idempotent_and_conflict_safe() {
        let root = root();
        let temporary = root.join("prepared.tmp");
        let store = LocalFiles;
        prepare_recovery_target(&store, &temporary, b"expected").unwrap();
        prepare_recovery_target(&store, &temporary, b"expected").unwrap();
        assert_eq!(store.read_raw(&temporary).unwrap(), b"expected");
        assert_eq!(
            prepare_recovery_target(&store, &temporary, b"different")
                .unwrap_err()
                .reason_code,
            "execution_recovery_prepared_bytes_mismatch"
        );
        assert_eq!(store.read_raw(&temporary).unwrap(), b"expected");
    }

    #[test]
    fn changed_source_is_rejected_before_replace() {
        let root = root();
        let temporary = root.join("prepared.tmp");
        let target = root.join("target.json");
        let store = LocalFiles;
        store.create_new(&temporary, b"expected").unwrap();
        store.create_new(&target, b"changed").unwrap();
        assert_eq!(
            install_recovery_target(
                &store,
                &temporary,
                &target,
                b"expected",
                b"original",
                "index_update"
            )
            .unwrap_err()
            .reason_code,
            "execution_recovery_source_changed"
        );
        assert_eq!(store.read_raw(&target).unwrap(), b"changed");
        assert!(temporary.exists());
    }

    #[test]
    fn failed_replace_preserves_prepared_bytes_target_and_stage() {
        let root = root();
        let temporary = root.join("prepared.tmp");
        let target = root.join("target.json");
        LocalFiles.create_new(&temporary, b"expected").unwrap();
        LocalFiles.create_new(&target, b"original").unwrap();
        let error = install_recovery_target(
            &FailedReplace,
            &temporary,
            &target,
            b"expected",
            b"original",
            "index_update",
        )
        .unwrap_err();
        assert_eq!(error.exit_code, ExitCode::IoFailure);
        assert_eq!(error.reason_code, "execution_recovery_replace_failed");
        assert_eq!(error.details["transaction_stage"], "index_update");
        assert_eq!(LocalFiles.read_raw(&temporary).unwrap(), b"expected");
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"original");
    }
}
