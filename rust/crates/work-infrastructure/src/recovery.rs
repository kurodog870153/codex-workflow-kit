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
    let journal = read_approved_instruction_journal(root, relative, approved_sha256)?;
    let requirement_id = journal["metadata"]["request"]["requirement_id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "instruction_recovery_layout",
                "The approved requirement identity is invalid.",
                json!({"journal":relative}),
            )
        })?;
    let context = work_feature::ports::RequirementWriterContext {
        canonical_project_root: root.canonicalize().map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "instruction_recovery_root",
                "The project root could not be resolved.",
                json!({}),
            )
        })?,
        requirement_id,
    };
    recover_instruction_journal_with_runtime(&context, relative, approved_sha256)
}

/// Candidate recovery scope binds the complete approved journal to a trusted requirement.
pub fn recover_instruction_journal_with_runtime(
    context: &work_feature::ports::RequirementWriterContext,
    relative: &str,
    approved_sha256: &str,
) -> Result<serde_json::Value, WorkError> {
    let root = &context.canonical_project_root;
    let journal = read_approved_instruction_journal(root, relative, approved_sha256)?;
    if journal["metadata"]["request"]["requirement_id"] != context.requirement_id.as_str() {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "instruction_recovery_context_mismatch",
            "The approved journal does not match the trusted writer context.",
            json!({}),
        ));
    }
    let execution = journal["metadata"]["artifacts"]["execution"]
        .as_str()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "instruction_recovery_layout",
                "The approved execution path is required.",
                json!({"journal":relative}),
            )
        })?;
    crate::writer_lock::require_no_legacy_locks(context, Some(execution))?;
    work_feature::ports::with_runtime_writer(
        &crate::writer_lock::LocalWriterLock,
        context,
        work_model::runtime::LockClass::Execution,
        |owner| recover_retained_instruction_owned(context, relative, approved_sha256, owner),
    )
}

fn recover_retained_instruction_owned(
    context: &work_feature::ports::RequirementWriterContext,
    relative: &str,
    approved: &str,
    owner: &work_model::runtime::RuntimeOwner,
) -> Result<serde_json::Value, WorkError> {
    let root = &context.canonical_project_root;
    let mut original = read_approved_instruction_journal_with_layout(root, relative, approved)?;
    original["state"] = json!("prepared");
    original["published_count"] = json!(0);
    let execution = original["metadata"]["artifacts"]["execution"]
        .as_str()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "instruction_recovery_layout",
                "The execution scope is required.",
                json!({"journal":relative}),
            )
        })?;
    for (path, expected) in original["metadata"]["source_sha256"]
        .as_object()
        .expect("validated source map")
    {
        if original["metadata"]["candidate_sha256"][path] == *expected
            && work_operations::derivation::fingerprint::raw(
                &crate::files::LocalFiles
                    .read_raw(&crate::files::resolve_runtime_path(root, path)?)?,
            ) != *expected
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "instruction_recovery_source_changed",
                "Immutable Source evidence changed after approval.",
                json!({"journal":relative}),
            ));
        }
    }
    let input = crate::specification::storage::RetainedJournalRuntimeInput {
        context,
        execution,
        relative,
        prepared_journal: &original,
        recover: true,
    };
    crate::specification::storage::publish_retained_journal_with_owner(
        &input,
        owner,
        || Ok(()),
        |_| Ok(()),
    )?;
    let raw =
        crate::files::LocalFiles.read_raw(&crate::files::resolve_runtime_path(root, relative)?)?;
    work_operations::canonical::parse_json_contract(&raw).map_err(|_| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "invalid_contract_value",
            "The published journal is invalid.",
            json!({"journal":relative}),
        )
    })
}

fn read_approved_instruction_journal(
    root: &Path,
    relative: &str,
    approved_sha256: &str,
) -> Result<serde_json::Value, WorkError> {
    read_approved_instruction_journal_with_layout(root, relative, approved_sha256)
}

fn read_approved_instruction_journal_with_layout(
    root: &Path,
    relative: &str,
    approved_sha256: &str,
) -> Result<serde_json::Value, WorkError> {
    {
        if let Some(original) = read_staged_instruction_original(root, relative, approved_sha256)? {
            return Ok(original);
        }
    }
    let path = crate::files::resolve_runtime_path(root, relative)?;
    let raw = crate::files::LocalFiles.read_raw(&path)?;
    let journal = work_operations::canonical::parse_json_contract(&raw).map_err(|_| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "invalid_contract_value",
            "The journal is invalid.",
            json!({}),
        )
    })?;
    let rendered = work_operations::specification::transaction::render_transaction(&journal)
        .map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
    if rendered != raw {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "noncanonical_json",
            "The journal is not canonical JSON.",
            json!({"journal":relative}),
        ));
    }
    if !work_operations::protocol::valid_sha256(approved_sha256)
        || journal["approval_sha256"] != approved_sha256
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "instruction_recovery_approval_changed",
            "The approved transaction fingerprint does not match.",
            json!({"journal":relative}),
        ));
    }
    {
        let execution = journal["metadata"]["artifacts"]["execution"]
            .as_str()
            .ok_or_else(|| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "instruction_recovery_layout",
                    "The journal must identify its execution scope.",
                    json!({"journal":relative}),
                )
            })?;
        crate::specification::storage::read_retained_journal_for_recovery(
            root, execution, relative,
        )?;
    }
    Ok(journal)
}

fn read_staged_instruction_original(
    root: &Path,
    relative: &str,
    approved: &str,
) -> Result<Option<serde_json::Value>, WorkError> {
    let fail = |reason| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            reason,
            "Instruction recovery requires complete scope-bound evidence.",
            json!({"journal":relative}),
        )
    };
    if !work_operations::protocol::valid_sha256(approved) {
        return Err(fail("instruction_recovery_approval_changed"));
    }
    let staging = "outputs/work/runtime/staging";
    let directory = crate::files::resolve_runtime_path(root, staging)?;
    if !directory.exists() {
        return Ok(None);
    }
    let mut result = None;
    for requirement in
        std::fs::read_dir(&directory).map_err(|_| fail("runtime_inventory_read_failed"))?
    {
        let requirement = requirement.map_err(|_| fail("runtime_inventory_read_failed"))?;
        let name = requirement
            .file_name()
            .into_string()
            .map_err(|_| fail("runtime_requirement_invalid"))?;
        let requirement_id: work_operations::identifiers::RequirementId = name
            .parse()
            .map_err(|_| fail("runtime_requirement_invalid"))?;
        crate::files::resolve_runtime_path(root, &format!("{staging}/{name}"))?;
        for operation in ["instruction-migration", "source-refresh"] {
            let namespace = format!("{staging}/{name}/{operation}");
            let directory = crate::files::resolve_runtime_path(root, &namespace)?;
            if !directory.exists() {
                continue;
            }
            for entry in
                std::fs::read_dir(&directory).map_err(|_| fail("runtime_inventory_read_failed"))?
            {
                let entry = entry.map_err(|_| fail("runtime_inventory_read_failed"))?;
                let identity = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| fail("runtime_identity_invalid"))?;
                let path = crate::files::resolve_runtime_path(
                    root,
                    &format!("{namespace}/{identity}/transaction.json"),
                )?;
                let raw = crate::files::LocalFiles.read_raw(&path)?;
                let manifest: work_model::runtime::RuntimeManifest =
                    serde_json::from_slice(&raw).map_err(|_| fail("runtime_manifest_invalid"))?;
                if serde_json::to_vec(&manifest).map_err(|_| fail("runtime_manifest_invalid"))?
                    != raw
                    || crate::transaction_storage::runtime_manifest_directory(root, &manifest)?
                        != path.parent().expect("manifest parent")
                    || manifest.requirement_id != name
                    || manifest.operation != operation
                {
                    return Err(fail("runtime_manifest_scope_mismatch"));
                }
                let proof =
                    work_operations::derivation::transaction::restore_journal_staging(&manifest)
                        .map_err(|_| fail("runtime_manifest_identity"))?;
                if proof.manifest.business_identity["journal_path"] != relative {
                    continue;
                }
                let original = &proof.manifest.business_identity["original_journal"];
                if original["approval_sha256"] != approved {
                    return Err(fail("instruction_recovery_approval_changed"));
                }
                let context = work_feature::ports::RequirementWriterContext {
                    canonical_project_root: root
                        .canonicalize()
                        .map_err(|_| fail("instruction_recovery_root"))?,
                    requirement_id: requirement_id.clone(),
                };
                crate::execution::storage::LocalExecutionStorage {
                    project_root: context.canonical_project_root.clone(),
                }
                .retained_requirement_inventory(&context, &manifest.execution_dir)?;
                if result.replace(original.clone()).is_some() {
                    return Err(fail("runtime_execution_transaction_present"));
                }
            }
        }
    }
    Ok(result)
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
        let fixture = instruction_fixture_with_layout(kind, "execution", true);
        {
            let (root, relative, _) = &fixture;
            let mut original: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join(relative)).unwrap()).unwrap();
            original["state"] = json!("prepared");
            original["published_count"] = json!(0);
            let preview = original["metadata"]["request"]["preview_fingerprint"]
                .as_str()
                .unwrap();
            let journal_kind = if kind == "source_refresh" {
                work_operations::derivation::publication::JournalKind::SourceRefresh(preview)
            } else {
                work_operations::derivation::publication::JournalKind::InstructionMigration(preview)
            };
            let staged = work_operations::derivation::transaction::build_journal_staging(
                work_operations::derivation::transaction::JournalStagingInput {
                    canonical_root: root.canonicalize().unwrap().to_str().unwrap(),
                    requirement: &"example".parse().unwrap(),
                    execution_dir: "execution",
                    journal_path: relative,
                    kind: journal_kind,
                    journal: &original,
                },
            )
            .unwrap();
            fs::write(root.join(relative), &staged.prepared_journal).unwrap();
            crate::transaction_storage::prepare_runtime_transaction(
                &LocalFiles,
                root,
                &staged.manifest,
                &staged.payloads,
            )
            .unwrap();
        }
        fixture
    }

    fn instruction_fixture_with_layout(
        kind: &str,
        execution: &str,
        retained: bool,
    ) -> (std::path::PathBuf, String, String) {
        use std::collections::BTreeMap;
        use work_operations::derivation::transaction::{
            PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
        };
        use work_operations::derivation::{publication, snapshot};
        let root = root();
        fs::create_dir_all(root.join(execution)).unwrap();
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
            artifacts: json!({"execution":execution}),
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
        let relative = if retained {
            publication::retained_journal_path(execution, journal_kind).unwrap()
        } else {
            work_operations::derivation::legacy_layout::legacy_journal_path(execution, journal_kind)
        };
        fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
        crate::specification::storage::write_journal(&root, &relative, &journal).unwrap();
        (root, relative, derived.approval_sha256)
    }

    #[test]
    fn retained_instruction_reader_binds_metadata_scope_before_owner_or_any_mutation() {
        for kind in ["source_refresh", "instruction_migration"] {
            let execution = "custom execution/例";
            let (root, relative, approval) = instruction_fixture_with_layout(kind, execution, true);
            let raw = fs::read(root.join(&relative)).unwrap();
            let journal =
                read_approved_instruction_journal_with_layout(&root, &relative, &approval).unwrap();
            assert_eq!(journal["metadata"]["artifacts"]["execution"], execution);
            assert_eq!(journal["metadata"]["request"]["kind"], kind);
            assert!(
                read_approved_instruction_journal_with_layout(&root, &relative, &"0".repeat(64))
                    .is_err()
            );
            let foreign = relative.replacen(execution, "foreign execution", 1);
            fs::create_dir_all(root.join(&foreign).parent().unwrap()).unwrap();
            fs::write(root.join(&foreign), &raw).unwrap();
            assert!(
                read_approved_instruction_journal_with_layout(&root, &foreign, &approval).is_err()
            );
            assert_eq!(fs::read(root.join(&relative)).unwrap(), raw);
            assert_eq!(fs::read(root.join("two.json")).unwrap(), b"before two");
            assert!(!root.join("outputs/work/runtime").exists());
        }
    }

    #[test]
    fn retained_instruction_recovery_requires_full_proof_for_missing_partial_and_foreign_evidence()
    {
        use work_operations::derivation::{publication, transaction};
        for kind in ["source_refresh", "instruction_migration"] {
            for mode in 0..4 {
                let execution = "custom execution/例";
                let (root, relative, approval) =
                    instruction_fixture_with_layout(kind, execution, true);
                let context = work_feature::ports::RequirementWriterContext {
                    canonical_project_root: root.canonicalize().unwrap(),
                    requirement_id: "example".parse().unwrap(),
                };
                let mut original: serde_json::Value =
                    serde_json::from_slice(&fs::read(root.join(&relative)).unwrap()).unwrap();
                original["state"] = json!("prepared");
                original["published_count"] = json!(0);
                fs::write(root.join("one.json"), b"before one").unwrap();
                let preview = original["metadata"]["request"]["preview_fingerprint"]
                    .as_str()
                    .unwrap();
                let journal_kind = if kind == "source_refresh" {
                    publication::JournalKind::SourceRefresh(preview)
                } else {
                    publication::JournalKind::InstructionMigration(preview)
                };
                let staged = transaction::build_journal_staging(transaction::JournalStagingInput {
                    canonical_root: context.canonical_project_root.to_str().unwrap(),
                    requirement: &context.requirement_id,
                    execution_dir: execution,
                    journal_path: &relative,
                    kind: journal_kind,
                    journal: &original,
                })
                .unwrap();
                fs::write(root.join(&relative), &staged.prepared_journal).unwrap();
                if mode != 3 {
                    crate::transaction_storage::prepare_runtime_transaction(
                        &LocalFiles,
                        &root,
                        &staged.manifest,
                        &staged.payloads,
                    )
                    .unwrap();
                }
                match mode {
                    1 | 3 => fs::remove_file(root.join(&relative)).unwrap(),
                    2 => fs::write(root.join(&relative), &staged.prepared_journal[..17]).unwrap(),
                    _ => {}
                }
                let recovered = work_feature::ports::with_runtime_writer(
                    &crate::writer_lock::LocalWriterLock,
                    &context,
                    work_model::runtime::LockClass::Execution,
                    |owner| {
                        recover_retained_instruction_owned(&context, &relative, &approval, owner)
                    },
                );
                if mode == 3 {
                    assert!(recovered.is_err());
                    assert_eq!(fs::read(root.join("one.json")).unwrap(), b"before one");
                    assert_eq!(fs::read(root.join("two.json")).unwrap(), b"before two");
                    continue;
                }
                assert_eq!(recovered.unwrap()["state"], "published");
                assert_eq!(
                    fs::read(root.join(&relative)).unwrap(),
                    staged.published_journal
                );
                assert_eq!(
                    fs::read(root.join("source.txt")).unwrap(),
                    b"immutable source"
                );
                assert!(
                    crate::execution::storage::LocalExecutionStorage {
                        project_root: root.clone()
                    }
                    .retained_requirement_inventory(&context, execution)
                    .unwrap()
                    .is_empty()
                );
                let again = work_feature::ports::with_runtime_writer(
                    &crate::writer_lock::LocalWriterLock,
                    &context,
                    work_model::runtime::LockClass::Execution,
                    |owner| {
                        recover_retained_instruction_owned(&context, &relative, &approval, owner)
                    },
                )
                .unwrap();
                assert_eq!(again["state"], "published");
                assert_eq!(
                    fs::read(root.join(&relative)).unwrap(),
                    staged.published_journal
                );
            }
        }
    }

    #[test]
    fn runtime_instruction_recovery_binds_approval_and_releases_on_drift() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        for kind in ["source_refresh", "instruction_migration"] {
            let (root, path, approval) = instruction_fixture(kind);
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.canonicalize().unwrap(),
                requirement_id: "example".parse().unwrap(),
            };
            let lock = crate::writer_lock::LocalWriterLock;
            let foreign = work_feature::ports::RequirementWriterContext {
                requirement_id: "foreign".parse().unwrap(),
                ..context.clone()
            };
            assert!(recover_instruction_journal_with_runtime(&foreign, &path, &approval).is_err());
            assert!(
                !root
                    .join(
                        work_operations::derivation::publication::runtime_lock_path(
                            &context.requirement_id,
                            "execution"
                        )
                        .unwrap()
                    )
                    .exists()
            );
            assert_eq!(fs::read(root.join("two.json")).unwrap(), b"before two");
            let held = lock
                .acquire_runtime(&context, work_model::runtime::LockClass::Execution)
                .unwrap();
            assert!(recover_instruction_journal_with_runtime(&context, &path, &approval).is_err());
            held.release().unwrap();
            fs::write(root.join("source.txt"), b"drift").unwrap();
            assert_eq!(
                recover_instruction_journal_with_runtime(&context, &path, &approval)
                    .unwrap_err()
                    .reason_code,
                "instruction_recovery_source_changed"
            );
            lock.require_runtime_idle(&context, work_model::runtime::LockClass::Execution)
                .unwrap();
            fs::write(root.join("source.txt"), b"immutable source").unwrap();
            let result =
                recover_instruction_journal_with_runtime(&context, &path, &approval).unwrap();
            assert_eq!(result["state"], "published");
            assert_eq!(
                recover_instruction_journal_with_runtime(&context, &path, &approval).unwrap(),
                result
            );
            lock.require_runtime_idle(&context, work_model::runtime::LockClass::Execution)
                .unwrap();
        }
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
            fs::write(
                root.join(work_operations::derivation::publication::completion_marker_path(&path)),
                b"wrong marker",
            )
            .unwrap();
            assert_eq!(
                recover_instruction_journal(&root, &path, &approval)
                    .unwrap_err()
                    .reason_code,
                { "journal_commit_evidence" }
            );
        }
    }

    #[test]
    fn instruction_recovery_rejects_source_target_history_drift_and_busy_writer() {
        use crate::writer_lock::LocalWriterLock;
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
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
                let expected = {
                    match file {
                        "one.json" => "journal_runtime_formal_conflict",
                        "execution/TASK-001/ATTEMPT-001/attempt.json" => {
                            "spec_update_history_changed"
                        }
                        _ => expected,
                    }
                };
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

            let journal = read_approved_instruction_journal(&root, &path, &approval).unwrap();
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.canonicalize().unwrap(),
                requirement_id: journal["metadata"]["request"]["requirement_id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
            };
            let runtime_guard = {
                LocalWriterLock
                    .acquire_runtime(&context, work_model::runtime::LockClass::Execution)
                    .unwrap()
            };

            assert_eq!(
                recover_instruction_journal(&root, &path, &approval)
                    .unwrap_err()
                    .reason_code,
                "work_state_writer_busy"
            );
            assert_eq!(fs::read(root.join("two.json")).unwrap(), b"before two");
            {
                let guard = runtime_guard;
                guard.release().unwrap();
                recover_instruction_journal(&root, &path, &approval).unwrap();
            }
        }
    }
}
