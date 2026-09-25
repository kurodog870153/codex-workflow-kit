//! Recoverable sequential storage primitives.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_operations::canonical::sha256_hex;

use crate::files::LocalFiles;

pub struct Publication<'a> {
    pub target: &'a Path,
    pub before: Option<&'a [u8]>,
    pub after: Option<&'a [u8]>,
    pub temporary: &'a Path,
}

pub fn publish_recoverable_sequence(
    store: &impl ArtifactStore,
    steps: &[Publication<'_>],
    mut record_progress: impl FnMut(usize) -> Result<(), WorkError>,
) -> Result<(), WorkError> {
    for (index, step) in steps.iter().enumerate() {
        let current = if step.target.is_file() {
            Some(store.read_raw(step.target)?)
        } else {
            None
        };
        if current.as_deref() != step.after {
            if current.as_deref() != step.before {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "spec_transaction_concurrent_change",
                    "An artifact changed outside the transaction.",
                    json!({"path": step.target.to_string_lossy()}),
                ));
            }
            match (step.before, step.after) {
                (None, Some(after)) => {
                    if let Some(parent) = step.target.parent() {
                        store.create_directories(parent)?;
                    }
                    store.create_new(step.target, after)?;
                }
                (Some(before), Some(after)) => {
                    replace_checked(store, step.target, before, after, step.temporary, true)?;
                }
                (Some(_), None) => {
                    store.remove(step.target)?;
                }
                (None, None) => {}
            }
        }
        record_progress(index + 1)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionState {
    Incomplete,
    Completed,
    Corrupt,
}

pub fn completion_marker(record: &[u8]) -> Vec<u8> {
    format!("{}\n", sha256_hex(record)).into_bytes()
}

pub fn completion_state(record: &[u8], marker: Option<&[u8]>) -> CompletionState {
    match marker {
        None => CompletionState::Incomplete,
        Some(bytes) if bytes == completion_marker(record) => CompletionState::Completed,
        Some(_) => CompletionState::Corrupt,
    }
}

pub fn complete_write(path: &Path, target: &[u8]) -> Result<(), WorkError> {
    if !path.exists() {
        return LocalFiles.create_new(path, target);
    }
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|_| io_error("spec_update_partial_read_failed", path))?;
    let mut current = Vec::new();
    file.read_to_end(&mut current)
        .map_err(|_| io_error("spec_update_partial_read_failed", path))?;
    if !target.starts_with(&current) {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "spec_update_partial_conflict",
            "Partial transaction bytes conflict with approval.",
            json!({}),
        ));
    }
    if current != target {
        file.write_all(&target[current.len()..])
            .and_then(|_| file.sync_all())
            .map_err(|_| io_error("spec_update_partial_write_failed", path))?;
    }
    Ok(())
}

fn io_error(reason: &str, path: &Path) -> WorkError {
    WorkError::new(
        ExitCode::IoFailure,
        reason,
        "Transaction storage could not be updated.",
        json!({"path": path.to_string_lossy()}),
    )
}

pub fn replace_checked(
    store: &impl ArtifactStore,
    target: &Path,
    expected_before: &[u8],
    expected_after: &[u8],
    temporary: &Path,
    recover: bool,
) -> Result<(), WorkError> {
    if recover {
        complete_write(temporary, expected_after)?;
    } else if temporary.exists() {
        if store.read_raw(temporary)? != expected_after {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "spec_update_temporary_changed",
                "Prepared specification bytes changed.",
                json!({}),
            ));
        }
    } else {
        store.create_new(temporary, expected_after)?;
    }
    if store.read_raw(target)? != expected_before {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "spec_update_concurrent_change",
            "An artifact changed before publication.",
            json!({}),
        ));
    }
    store
        .replace(temporary, target)
        .map_err(|_| io_error("spec_update_replace_failed", temporary))?;
    if store.read_raw(target)? != expected_after {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "spec_update_write_mismatch",
            "Published specification bytes differ from the approved candidate.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn replace_journal(path: &Path, raw: &[u8]) -> Result<(), WorkError> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
    ));
    if temporary.exists() {
        if LocalFiles.read_raw(&temporary)? != raw {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "spec_transaction_journal_temporary_changed",
                "Prepared journal bytes changed.",
                json!({}),
            ));
        }
    } else {
        LocalFiles.create_new(&temporary, raw)?;
    }
    LocalFiles.replace(&temporary, path)
}

pub fn read_completion_state(journal: &Path, marker: &Path) -> Result<CompletionState, WorkError> {
    let record = LocalFiles.read_raw(journal)?;
    let marker_bytes = if marker.exists() {
        Some(LocalFiles.read_raw(marker)?)
    } else {
        None
    };
    Ok(completion_state(&record, marker_bytes.as_deref()))
}

pub fn write_completion_marker(journal: &Path, marker: &Path) -> Result<(), WorkError> {
    let raw = LocalFiles.read_raw(journal)?;
    LocalFiles.create_new(marker, &completion_marker(&raw))
}

pub fn prepare_transaction_directory(path: &Path) -> Result<(), WorkError> {
    if path.exists() {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "transaction_workspace_exists",
            "A generated transaction workspace already exists.",
            json!({"path": path.to_string_lossy()}),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io_error("transaction_workspace_create_failed", path))?;
    fs::create_dir_all(parent)
        .map_err(|_| io_error("transaction_workspace_create_failed", path))?;
    fs::create_dir(path).map_err(|_| io_error("transaction_workspace_create_failed", path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-rust-transaction-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn completion_marker_and_partial_write_are_exact() {
        for raw in [
            b"".as_slice(),
            b"{}\n",
            b"{}\r\n",
            b"\xef\xbb\xbf{}\n",
            b"\xff",
        ] {
            let marker = completion_marker(raw);
            assert_eq!(marker.len(), 65);
            assert_eq!(marker.last(), Some(&b'\n'));
            assert_eq!(completion_state(raw, None), CompletionState::Incomplete);
            assert_eq!(
                completion_state(raw, Some(&marker)),
                CompletionState::Completed
            );
            for invalid in [
                Vec::new(),
                marker[..marker.len() - 1].to_vec(),
                [marker.as_slice(), b"\n"].concat(),
                [marker[..marker.len() - 1].as_ref(), b"\r\n"].concat(),
                marker.to_ascii_uppercase(),
            ] {
                assert_eq!(
                    completion_state(raw, Some(&invalid)),
                    CompletionState::Corrupt
                );
            }
            assert_eq!(
                completion_state(&[raw, b"\n"].concat(), Some(&marker)),
                CompletionState::Corrupt
            );
        }
        let root = root();
        let journal = root.join("journal.json");
        let marker = root.join("journal.json.done");
        complete_write(&journal, b"pre").unwrap();
        complete_write(&journal, b"prepared").unwrap();
        assert_eq!(LocalFiles.read_raw(&journal).unwrap(), b"prepared");
        assert_eq!(
            complete_write(&journal, b"different")
                .unwrap_err()
                .reason_code,
            "spec_update_partial_conflict"
        );
        assert_eq!(
            read_completion_state(&journal, &marker).unwrap(),
            CompletionState::Incomplete
        );
        write_completion_marker(&journal, &marker).unwrap();
        assert_eq!(
            read_completion_state(&journal, &marker).unwrap(),
            CompletionState::Completed
        );
        assert_eq!(
            completion_state(b"changed", Some(&LocalFiles.read_raw(&marker).unwrap())),
            CompletionState::Corrupt
        );
    }

    #[test]
    fn checked_replace_preserves_changed_source() {
        let root = root();
        let target = root.join("target.json");
        let temporary = root.join("target.tmp");
        LocalFiles.create_new(&target, b"changed").unwrap();
        assert_eq!(
            replace_checked(&LocalFiles, &target, b"before", b"after", &temporary, false)
                .unwrap_err()
                .reason_code,
            "spec_update_concurrent_change"
        );
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"changed");
        assert_eq!(LocalFiles.read_raw(&temporary).unwrap(), b"after");
        replace_checked(&LocalFiles, &target, b"changed", b"after", &temporary, true).unwrap();
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"after");
        let direct_target = root.join("direct.json");
        let direct_temp = root.join("direct.tmp");
        LocalFiles.create_new(&direct_target, b"old").unwrap();
        replace_checked(
            &LocalFiles,
            &direct_target,
            b"old",
            b"new",
            &direct_temp,
            false,
        )
        .unwrap();
        assert_eq!(LocalFiles.read_raw(&direct_target).unwrap(), b"new");
        assert!(!direct_temp.exists());
        let changed_target = root.join("changed-temp.json");
        let changed_temp = root.join("changed-temp.tmp");
        LocalFiles.create_new(&changed_target, b"old").unwrap();
        LocalFiles.create_new(&changed_temp, b"unknown").unwrap();
        assert_eq!(
            replace_checked(
                &LocalFiles,
                &changed_target,
                b"old",
                b"new",
                &changed_temp,
                false
            )
            .unwrap_err()
            .reason_code,
            "spec_update_temporary_changed"
        );
        assert_eq!(LocalFiles.read_raw(&changed_target).unwrap(), b"old");
        assert_eq!(LocalFiles.read_raw(&changed_temp).unwrap(), b"unknown");
    }

    struct FaultyReplaceStore {
        corrupt_after_replace: bool,
    }

    impl ArtifactStore for FaultyReplaceStore {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, bytes)
        }
        fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
            if !self.corrupt_after_replace {
                return Err(io_error("injected_replace_failure", temporary));
            }
            LocalFiles.replace(temporary, target)?;
            fs::write(target, b"external").map_err(|_| io_error("injected_write_failure", target))
        }
        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)
        }
        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
    }

    #[test]
    fn checked_replace_keeps_evidence_after_failure_and_detects_post_write_change() {
        let root = root();
        let target = root.join("target.json");
        let temporary = root.join("prepared.tmp");
        LocalFiles.create_new(&target, b"old").unwrap();
        assert_eq!(
            replace_checked(
                &FaultyReplaceStore {
                    corrupt_after_replace: false,
                },
                &target,
                b"old",
                b"new",
                &temporary,
                false,
            )
            .unwrap_err()
            .reason_code,
            "spec_update_replace_failed"
        );
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"old");
        assert_eq!(LocalFiles.read_raw(&temporary).unwrap(), b"new");
        assert_eq!(
            replace_checked(
                &FaultyReplaceStore {
                    corrupt_after_replace: true,
                },
                &target,
                b"old",
                b"new",
                &temporary,
                false,
            )
            .unwrap_err()
            .reason_code,
            "spec_update_write_mismatch"
        );
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"external");
    }

    struct FailingReadStore {
        target: std::path::PathBuf,
    }

    impl ArtifactStore for FailingReadStore {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            if path == self.target {
                return Err(io_error("injected_read_failure", path));
            }
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, bytes)
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
    fn checked_replace_preserves_source_on_prepare_or_read_failure() {
        let root = root();
        let target = root.join("target.json");
        LocalFiles.create_new(&target, b"old").unwrap();
        let missing_parent = root.join("missing").join("prepared.tmp");
        let failed = replace_checked(&LocalFiles, &target, b"old", b"new", &missing_parent, false)
            .unwrap_err();
        assert_eq!(failed.exit_code, ExitCode::IoFailure);
        assert!(!missing_parent.exists());
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"old");

        let temporary = root.join("prepared.tmp");
        let failed = replace_checked(
            &FailingReadStore {
                target: target.clone(),
            },
            &target,
            b"old",
            b"new",
            &temporary,
            false,
        )
        .unwrap_err();
        assert_eq!(failed.reason_code, "injected_read_failure");
        assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"old");
        assert_eq!(LocalFiles.read_raw(&temporary).unwrap(), b"new");
    }

    #[test]
    fn sequential_publish_resumes_after_progress_failure() {
        let root = root();
        let first = root.join("first.json");
        let second = root.join("second.json");
        let first_temp = root.join("first.tmp");
        let second_temp = root.join("second.tmp");
        LocalFiles.create_new(&first, b"old").unwrap();
        LocalFiles.create_new(&second, b"old").unwrap();
        let steps = [
            Publication {
                target: &first,
                before: Some(b"old"),
                after: Some(b"new"),
                temporary: &first_temp,
            },
            Publication {
                target: &second,
                before: Some(b"old"),
                after: Some(b"new"),
                temporary: &second_temp,
            },
        ];
        let failed = publish_recoverable_sequence(&LocalFiles, &steps, |_| {
            Err(io_error("injected_journal_failure", &first))
        });
        assert_eq!(failed.unwrap_err().reason_code, "injected_journal_failure");
        assert_eq!(LocalFiles.read_raw(&first).unwrap(), b"new");
        assert_eq!(LocalFiles.read_raw(&second).unwrap(), b"old");
        fs::write(&second, b"external").unwrap();
        assert_eq!(
            publish_recoverable_sequence(&LocalFiles, &steps, |_| Ok(()))
                .unwrap_err()
                .reason_code,
            "spec_transaction_concurrent_change"
        );
        assert_eq!(LocalFiles.read_raw(&first).unwrap(), b"new");
        assert_eq!(LocalFiles.read_raw(&second).unwrap(), b"external");
        fs::write(&second, b"old").unwrap();
        let mut progress = Vec::new();
        publish_recoverable_sequence(&LocalFiles, &steps, |count| {
            progress.push(count);
            Ok(())
        })
        .unwrap();
        assert_eq!(progress, [1, 2]);
        assert_eq!(LocalFiles.read_raw(&second).unwrap(), b"new");
        publish_recoverable_sequence(&LocalFiles, &steps, |_| Ok(())).unwrap();
        assert_eq!(LocalFiles.read_raw(&first).unwrap(), b"new");
        assert_eq!(LocalFiles.read_raw(&second).unwrap(), b"new");
    }

    #[test]
    fn replacement_with_open_source_handle_is_verified_on_macos() {
        let root = root();
        let target = root.join("target.json");
        let temporary = root.join("target.tmp");
        LocalFiles.create_new(&target, b"old").unwrap();
        let held = std::fs::File::open(&target).unwrap();
        let result = replace_checked(&LocalFiles, &target, b"old", b"new", &temporary, false);
        #[cfg(not(windows))]
        {
            result.unwrap();
            assert_eq!(LocalFiles.read_raw(&target).unwrap(), b"new");
        }
        #[cfg(windows)]
        {
            // Windows handle sharing must be exercised in T26's Windows runner.
            let _ = result;
        }
        drop(held);
    }
}
