//! Durable prepared bytes and verified replacement for recovery.

use std::path::Path;

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;

/// Resume an approved current instruction journal without deriving a candidate.
/// Both router and source interruptions use the shared transaction contract.
pub fn recover_instruction_journal(
    root: &Path,
    relative: &str,
    approved_sha256: &str,
) -> Result<serde_json::Value, WorkError> {
    use crate::files::LocalFiles;
    use crate::specification::storage::{
        execution_history_fingerprints, publish_journal, require_no_spec_update, storage_path,
    };
    use crate::writer_lock::{LocalWriterLock, WriterLock};
    use work_operations::canonical::parse_json_contract;
    use work_operations::derivation::{fingerprint, publication, snapshot};
    use work_operations::specification::transaction::render_transaction;

    let fail = |reason, message| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            reason,
            message,
            json!({"journal":relative}),
        )
    };
    let execution = relative
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(|| {
            fail(
                "instruction_recovery_layout",
                "A journal execution directory is required.",
            )
        })?;
    let journal_path = storage_path(root, relative)?;
    // Acquire the existing requirement mutex before reading approved evidence.
    let lock = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&lock)?;
    let raw = LocalFiles.read_raw(&journal_path)?;
    let journal = parse_json_contract(&raw)
        .map_err(|_| fail("invalid_contract_value", "The current journal is invalid."))?;
    if render_transaction(&journal)
        .map_err(|_| fail("invalid_contract_value", "The current journal is invalid."))?
        != raw
    {
        return Err(fail(
            "noncanonical_json",
            "The journal is not canonical JSON.",
        ));
    }
    if !work_operations::protocol::valid_sha256(approved_sha256)
        || journal["approval_sha256"] != approved_sha256
    {
        return Err(fail(
            "instruction_recovery_approval_changed",
            "The approved transaction fingerprint does not match.",
        ));
    }
    let request = &journal["metadata"]["request"];
    let preview = request["preview_fingerprint"]
        .as_str()
        .filter(|digest| work_operations::protocol::valid_sha256(digest))
        .ok_or_else(|| {
            fail(
                "instruction_recovery_request",
                "The approved request fingerprint is invalid.",
            )
        })?;
    let kind = match request["kind"].as_str() {
        Some("source_refresh") => publication::JournalKind::SourceRefresh(preview),
        Some("instruction_migration") => publication::JournalKind::InstructionMigration(preview),
        _ => {
            return Err(fail(
                "instruction_recovery_request",
                "Only current instruction journals can be recovered here.",
            ));
        }
    };
    if publication::journal_path(execution, kind) != relative
        || journal["metadata"]["artifacts"]["execution"] != execution
        || request["requirement_id"].as_str().is_none_or(|id| {
            id.parse::<work_operations::identifiers::RequirementId>()
                .is_err()
        })
    {
        return Err(fail(
            "instruction_recovery_layout",
            "The journal location does not match its approved request.",
        ));
    }
    require_no_spec_update(root, execution, Some(relative))?;
    let metadata = &journal["metadata"];
    let actual_history = serde_json::to_value(execution_history_fingerprints(root, execution)?)
        .expect("fingerprints serialize");
    if metadata["history_sha256"] != actual_history {
        return Err(fail(
            "instruction_recovery_history_changed",
            "Immutable execution history changed after approval.",
        ));
    }
    for (path, expected) in metadata["source_sha256"]
        .as_object()
        .expect("validated map")
    {
        if metadata["candidate_sha256"][path] == *expected
            && fingerprint::raw(&LocalFiles.read_raw(&storage_path(root, path)?)?) != *expected
        {
            return Err(fail(
                "instruction_recovery_source_changed",
                "Immutable Source evidence changed after approval.",
            ));
        }
    }
    let published = journal["published_count"]
        .as_u64()
        .expect("validated count") as usize;
    for row in journal["files"]
        .as_array()
        .expect("validated files")
        .iter()
        .take(published)
    {
        let path = storage_path(root, row["path"].as_str().expect("validated path"))?;
        let matches = if let Some(after) = row.get("after") {
            let expected = snapshot::decode_snapshot(after).map_err(|_| {
                fail(
                    "invalid_contract_value",
                    "The approved snapshot is invalid.",
                )
            })?;
            LocalFiles.read_raw(&path)? == expected
        } else {
            !path.exists()
        };
        if !matches {
            return Err(fail(
                "instruction_recovery_published_changed",
                "An already published target changed after approval.",
            ));
        }
    }
    let marker = publication::completion_marker_path(relative);
    publish_journal(root, relative, &marker)?;
    let final_raw = LocalFiles.read_raw(&journal_path)?;
    parse_json_contract(&final_raw).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The published journal is invalid.",
        )
    })
}

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

    fn instruction_fixture(kind: &str) -> (std::path::PathBuf, String, String) {
        use std::collections::BTreeMap;
        use work_operations::derivation::transaction::{
            PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
        };
        use work_operations::derivation::{publication, snapshot};
        let root = root();
        fs::create_dir_all(root.join("execution")).unwrap();
        fs::write(root.join("source.txt"), b"immutable source").unwrap();
        fs::write(root.join("one.json"), b"before one").unwrap();
        fs::write(root.join("two.json"), b"before two").unwrap();
        let preview = "a".repeat(64);
        let (transaction_kind, journal_kind) = if kind == "source_refresh" {
            (
                TransactionKind::SourceRefresh,
                publication::JournalKind::SourceRefresh(&preview),
            )
        } else {
            (
                TransactionKind::InstructionMigration,
                publication::JournalKind::InstructionMigration(&preview),
            )
        };
        let derived = TransactionDeriver::derive(TransactionInput {
            kind: transaction_kind,
            order: PublicationOrder::Flat,
            request: json!({"kind":kind,"requirement_id":"example","preview_fingerprint":preview}),
            artifacts: json!({"execution":"execution"}),
            affected_task_ids: vec![],
            history: BTreeMap::new(),
            source: BTreeMap::from([
                ("source.txt".into(), b"immutable source".to_vec()),
                ("one.json".into(), b"before one".to_vec()),
                ("two.json".into(), b"before two".to_vec()),
            ]),
            candidate: BTreeMap::from([
                ("source.txt".into(), b"immutable source".to_vec()),
                ("one.json".into(), b"after one".to_vec()),
                ("two.json".into(), b"after two".to_vec()),
            ]),
        })
        .unwrap();
        let mut journal = derived.journal;
        let first = &journal["files"][0];
        fs::write(
            root.join(first["path"].as_str().unwrap()),
            snapshot::decode_snapshot(&first["after"]).unwrap(),
        )
        .unwrap();
        journal["state"] = json!("publishing");
        journal["published_count"] = json!(1);
        let relative = publication::journal_path("execution", journal_kind);
        crate::specification::storage::write_journal(&root, &relative, &journal).unwrap();
        (root, relative, derived.approval_sha256)
    }

    #[test]
    fn instruction_recovery_resumes_only_approved_bytes_and_is_idempotent() {
        for kind in ["source_refresh", "instruction_migration"] {
            let (root, path, approval) = instruction_fixture(kind);
            assert_eq!(
                recover_instruction_journal(&root, &path, &"0".repeat(64))
                    .unwrap_err()
                    .reason_code,
                "instruction_recovery_approval_changed"
            );
            let result = recover_instruction_journal(&root, &path, &approval).unwrap();
            assert_eq!(result["state"], "published");
            assert_eq!(result["published_count"], 2);
            assert_eq!(
                fs::read(root.join("source.txt")).unwrap(),
                b"immutable source"
            );
            assert_eq!(fs::read(root.join("two.json")).unwrap(), b"after two");
            let bytes = fs::read(root.join(&path)).unwrap();
            assert_eq!(
                recover_instruction_journal(&root, &path, &approval).unwrap(),
                result
            );
            assert_eq!(fs::read(root.join(&path)).unwrap(), bytes);
            fs::write(root.join(format!("{path}.done")), b"wrong marker").unwrap();
            assert_eq!(
                recover_instruction_journal(&root, &path, &approval)
                    .unwrap_err()
                    .reason_code,
                "spec_transaction_marker_conflict"
            );
        }
    }

    #[test]
    fn instruction_recovery_rejects_source_target_history_drift_and_busy_writer() {
        use crate::writer_lock::{LocalWriterLock, WriterLock};
        for kind in ["source_refresh", "instruction_migration"] {
            for (file, expected) in [
                ("source.txt", "instruction_recovery_source_changed"),
                ("one.json", "instruction_recovery_published_changed"),
                (
                    "execution/TASK-001/ATTEMPT-001/attempt.json",
                    "instruction_recovery_history_changed",
                ),
            ] {
                let (root, path, approval) = instruction_fixture(kind);
                let target = root.join(file);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(target, b"unexpected drift").unwrap();
                assert_eq!(
                    recover_instruction_journal(&root, &path, &approval)
                        .unwrap_err()
                        .reason_code,
                    expected
                );
                assert_eq!(fs::read(root.join("two.json")).unwrap(), b"before two");
            }
            let (root, path, approval) = instruction_fixture(kind);
            let _guard = LocalWriterLock
                .acquire(&root.join("execution/.work-state-writer.lock"))
                .unwrap();
            assert_eq!(
                recover_instruction_journal(&root, &path, &approval)
                    .unwrap_err()
                    .reason_code,
                "work_state_writer_busy"
            );
            assert_eq!(fs::read(root.join("two.json")).unwrap(), b"before two");
        }
    }
}
