//! Recoverable sequential storage primitives.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_operations::derivation::publication::completion_marker;

use crate::files::LocalFiles;

fn runtime_error(reason: &str, path: &Path) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        reason,
        "Runtime transaction evidence does not match the approved context.",
        json!({"path":path.to_string_lossy()}),
    )
}

fn runtime_operation(
    name: &str,
) -> Result<work_operations::derivation::publication::RuntimeOperation, WorkError> {
    use work_operations::derivation::publication::RuntimeOperation as O;
    Ok(match name {
        "project-files" => O::ProjectFiles,
        "attempt-start" => O::AttemptStart,
        "record-begin" => O::RecordBegin,
        "command-correction" => O::CommandCorrection,
        "record-finish" => O::RecordFinish,
        "deviation-record" => O::DeviationRecord,
        "attempt-close" => O::AttemptClose,
        "correction" => O::Correction,
        "specification-update" => O::SpecificationUpdate,
        "specification-migration" => O::SpecificationMigration,
        "specification-migration-item" => O::SpecificationMigrationItem,
        "specification-migration-reconcile" => O::SpecificationMigrationReconcile,
        "instruction-migration" => O::InstructionMigration,
        "source-refresh" => O::SourceRefresh,
        _ => return Err(runtime_error("runtime_operation_invalid", Path::new(name))),
    })
}

/// Resolve every formal/inventory path and verify the complete identity before mutation.
pub fn runtime_manifest_directory(
    root: &Path,
    manifest: &work_model::runtime::RuntimeManifest,
) -> Result<std::path::PathBuf, WorkError> {
    use work_operations::derivation::{identity, publication};
    manifest
        .validate_shape()
        .map_err(|reason| runtime_error(reason, root))?;
    let canonical = root
        .canonicalize()
        .map_err(|_| io_error("runtime_root_unavailable", root))?;
    if canonical.to_str() != Some(manifest.canonical_root.as_str()) {
        return Err(runtime_error("runtime_root_mismatch", root));
    }
    let requirement = manifest
        .requirement_id
        .parse()
        .map_err(|_| runtime_error("runtime_requirement_invalid", root))?;
    let targets: Vec<_> = manifest
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect();
    let identity = identity::runtime_transaction_identity(
        &manifest.canonical_root,
        &requirement,
        &manifest.operation,
        &manifest.approval_sha256,
        &manifest.business_identity,
        &targets,
    )
    .map_err(|_| runtime_error("runtime_identity_invalid", root))?;
    if identity != manifest.transaction_identity {
        return Err(runtime_error("runtime_identity_mismatch", root));
    }
    let operation = runtime_operation(&manifest.operation)?;
    let relative = publication::runtime_staging_path(&requirement, operation, &identity)
        .map_err(|_| runtime_error("runtime_identity_invalid", root))?;
    let directory = crate::files::resolve_runtime_path(root, &relative)?;
    crate::files::resolve_runtime_path(root, &manifest.execution_dir)?;
    let expected: std::collections::BTreeSet<_> =
        publication::runtime_inventory(operation, manifest.targets.len())
            .into_iter()
            .filter(|file| file != "transaction.json")
            .collect();
    let actual: std::collections::BTreeSet<_> = manifest
        .inventory
        .iter()
        .map(|file| file.path.clone())
        .collect();
    if actual != expected {
        return Err(runtime_error("runtime_inventory_incomplete", &directory));
    }
    for target in &manifest.targets {
        let path = crate::files::resolve_runtime_path(root, &target.path)?;
        crate::files::require_runtime_same_filesystem(&directory, &path)?;
        for evidence in target.before.iter().chain(target.after.iter()) {
            if !work_operations::derivation::fingerprint::verify_raw(
                &evidence.bytes,
                &evidence.sha256,
            ) {
                return Err(runtime_error("runtime_target_hash_mismatch", &path));
            }
        }
    }
    for file in &manifest.inventory {
        crate::files::resolve_runtime_path(root, &format!("{relative}/{}", file.path))?;
    }
    Ok(directory)
}

pub fn prepare_runtime_transaction(
    store: &impl ArtifactStore,
    root: &Path,
    manifest: &work_model::runtime::RuntimeManifest,
    payloads: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<std::path::PathBuf, WorkError> {
    let directory = runtime_manifest_directory(root, manifest)?;
    if manifest.phase != work_model::runtime::RuntimePhase::Prepared
        || payloads.len() != manifest.inventory.len()
    {
        return Err(runtime_error("runtime_prepare_invalid", &directory));
    }
    for file in &manifest.inventory {
        let raw = payloads
            .get(&file.path)
            .ok_or_else(|| runtime_error("runtime_inventory_incomplete", &directory))?;
        if raw.len() as u64 != file.size_bytes
            || !work_operations::derivation::fingerprint::verify_raw(raw, &file.sha256)
        {
            return Err(runtime_error("runtime_inventory_hash_mismatch", &directory));
        }
    }
    prepare_transaction_directory(&directory)?;
    let raw = serde_json::to_vec(manifest)
        .map_err(|_| runtime_error("runtime_manifest_encode", &directory))?;
    store.create_new(&directory.join("transaction.json"), &raw)?;
    for (path, raw) in payloads {
        let path = directory.join(path);
        store.create_directories(path.parent().expect("inventory path parent"))?;
        store.create_new(&path, raw)?;
        if store.read_raw(&path)? != *raw {
            return Err(runtime_error("runtime_prepare_readback", &path));
        }
    }
    Ok(directory)
}

pub fn read_runtime_manifest(
    store: &impl ArtifactStore,
    root: &Path,
    expected: &work_model::runtime::RuntimeManifest,
) -> Result<work_model::runtime::RuntimeManifest, WorkError> {
    let directory = runtime_manifest_directory(root, expected)?;
    let relative = directory
        .strip_prefix(&expected.canonical_root)
        .map_err(|_| runtime_error("runtime_manifest_context_mismatch", &directory))?
        .to_string_lossy()
        .replace('\\', "/");
    let path = crate::files::resolve_runtime_path(root, &format!("{relative}/transaction.json"))?;
    let actual: work_model::runtime::RuntimeManifest =
        serde_json::from_slice(&store.read_raw(&path)?)
            .map_err(|_| runtime_error("runtime_manifest_invalid", &path))?;
    runtime_manifest_directory(root, &actual)?;
    let mut identity = expected.clone();
    identity.phase = actual.phase;
    identity.published_count = actual.published_count;
    if identity != actual {
        return Err(runtime_error("runtime_manifest_context_mismatch", &path));
    }
    Ok(actual)
}

pub fn update_runtime_progress(
    store: &impl ArtifactStore,
    root: &Path,
    expected: &work_model::runtime::RuntimeManifest,
    published_count: usize,
    phase: work_model::runtime::RuntimePhase,
) -> Result<(), WorkError> {
    use work_model::runtime::RuntimePhase as P;
    let rank = |phase| match phase {
        P::Prepared => 0,
        P::Publishing => 1,
        P::PublishedVerified => 2,
        P::Cleaning => 3,
        P::Restoring => 4,
        P::Restored => 5,
    };
    let current = read_runtime_manifest(store, root, expected)?;
    if published_count < current.published_count || rank(phase) < rank(current.phase) {
        return Err(runtime_error("runtime_progress_regression", root));
    }
    let mut next = current.clone();
    next.published_count = published_count;
    next.phase = phase;
    next.validate_shape()
        .map_err(|reason| runtime_error(reason, root))?;
    let directory = runtime_manifest_directory(root, &next)?;
    let before = store.read_raw(&directory.join("transaction.json"))?;
    let after = serde_json::to_vec(&next)
        .map_err(|_| runtime_error("runtime_manifest_encode", &directory))?;
    replace_checked(
        store,
        &directory.join("transaction.json"),
        &before,
        &after,
        &directory.join("transaction.json.tmp"),
        true,
    )
}

fn runtime_entries(root: &Path, directory: &Path) -> Result<Vec<std::path::PathBuf>, WorkError> {
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(directory).map_err(|_| io_error("runtime_inventory_read_failed", directory))?
    {
        let entry = entry.map_err(|_| io_error("runtime_inventory_read_failed", directory))?;
        let path = entry.path();
        let canonical = root
            .canonicalize()
            .map_err(|_| io_error("runtime_root_unavailable", root))?;
        let relative = path
            .strip_prefix(&canonical)
            .map_err(|_| runtime_error("runtime_inventory_foreign", &path))?
            .to_string_lossy()
            .replace('\\', "/");
        crate::files::resolve_runtime_path(root, &relative)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| io_error("runtime_inventory_read_failed", &path))?;
        if metadata.file_type().is_symlink() {
            return Err(runtime_error("runtime_inventory_alias", &path));
        }
        if metadata.is_dir() {
            entries.push(path.clone());
            entries.extend(runtime_entries(root, &path)?);
        } else if metadata.is_file() {
            entries.push(path);
        } else {
            return Err(runtime_error("runtime_inventory_foreign", &path));
        }
    }
    Ok(entries)
}

fn verify_runtime_formal(
    store: &impl ArtifactStore,
    root: &Path,
    manifest: &work_model::runtime::RuntimeManifest,
) -> Result<(), WorkError> {
    for target in &manifest.targets {
        let path = crate::files::resolve_runtime_path(root, &target.path)?;
        match &target.after {
            Some(after) if store.read_raw(&path)? == after.bytes => {}
            None if fs::symlink_metadata(&path)
                .is_err_and(|issue| issue.kind() == std::io::ErrorKind::NotFound) => {}
            _ => return Err(runtime_error("runtime_cleanup_formal_mismatch", &path)),
        }
    }
    Ok(())
}

/// Only a fully published approved transaction can remove its own runtime payloads.
pub fn cleanup_runtime_transaction(
    store: &impl ArtifactStore,
    root: &Path,
    expected: &work_model::runtime::RuntimeManifest,
) -> Result<(), WorkError> {
    use work_model::runtime::RuntimePhase as P;
    let directory = runtime_manifest_directory(root, expected)?;
    if fs::symlink_metadata(directory.join("transaction.json"))
        .is_err_and(|issue| issue.kind() == std::io::ErrorKind::NotFound)
    {
        if !matches!(expected.phase, P::PublishedVerified | P::Cleaning) {
            return Err(runtime_error(
                "runtime_cleanup_identity_missing",
                &directory,
            ));
        }
        verify_runtime_formal(store, root, expected)?;
        match fs::read_dir(&directory) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(runtime_error(
                        "runtime_cleanup_identity_missing",
                        &directory,
                    ));
                }
                return fs::remove_dir(&directory)
                    .map_err(|_| io_error("runtime_cleanup_failed", &directory));
            }
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            _ => {
                return Err(runtime_error(
                    "runtime_cleanup_identity_missing",
                    &directory,
                ));
            }
        }
    }
    let current = read_runtime_manifest(store, root, expected)?;
    if !matches!(current.phase, P::PublishedVerified | P::Cleaning) {
        return Err(runtime_error("runtime_cleanup_unpublished", &directory));
    }
    verify_runtime_formal(store, root, &current)?;
    let inventory: std::collections::BTreeMap<_, _> = current
        .inventory
        .iter()
        .map(|file| (directory.join(&file.path), file))
        .collect();
    if current.phase != P::Cleaning {
        for (path, file) in &inventory {
            let raw = store.read_raw(path)?;
            if raw.len() as u64 != file.size_bytes
                || !work_operations::derivation::fingerprint::verify_raw(&raw, &file.sha256)
            {
                return Err(runtime_error("runtime_inventory_hash_mismatch", path));
            }
        }
    }
    for path in runtime_entries(root, &directory)? {
        let relative = path
            .strip_prefix(
                root.canonicalize()
                    .map_err(|_| io_error("runtime_root_unavailable", root))?,
            )
            .map_err(|_| runtime_error("runtime_inventory_foreign", &path))?
            .to_string_lossy()
            .replace('\\', "/");
        crate::files::resolve_runtime_path(root, &relative)?;
        if path.is_dir()
            && path == directory.join("targets")
            && inventory.keys().any(|file| file.starts_with(&path))
        {
            continue;
        }
        if path == directory.join("transaction.json")
            || path == directory.join("transaction.json.tmp")
        {
            continue;
        }
        let file = inventory
            .get(&path)
            .ok_or_else(|| runtime_error("runtime_inventory_foreign", &path))?;
        let raw = store.read_raw(&path)?;
        if raw.len() as u64 != file.size_bytes
            || !work_operations::derivation::fingerprint::verify_raw(&raw, &file.sha256)
        {
            return Err(runtime_error("runtime_inventory_hash_mismatch", &path));
        }
    }
    update_runtime_progress(store, root, expected, current.targets.len(), P::Cleaning)?;
    for path in inventory.keys() {
        match fs::symlink_metadata(path) {
            Ok(_) => store.remove(path)?,
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(io_error("runtime_cleanup_failed", path)),
        }
    }
    let targets = directory.join("targets");
    if targets.is_dir() {
        fs::remove_dir(&targets).map_err(|_| io_error("runtime_cleanup_failed", &targets))?;
    }
    let cleaning = read_runtime_manifest(store, root, expected)?;
    verify_runtime_formal(store, root, &cleaning)?;
    let manifest_path = directory.join("transaction.json");
    let raw = serde_json::to_vec(&cleaning)
        .map_err(|_| runtime_error("runtime_manifest_encode", &directory))?;
    if store.read_raw(&manifest_path)? != raw {
        return Err(runtime_error(
            "runtime_manifest_context_mismatch",
            &directory,
        ));
    }
    store.remove(&manifest_path)?;
    if fs::remove_dir(&directory).is_err() {
        let primary = io_error("runtime_cleanup_failed", &directory);
        let restore = || -> Result<(), WorkError> {
            if runtime_manifest_directory(root, &cleaning)? != directory {
                return Err(runtime_error(
                    "runtime_manifest_context_mismatch",
                    &directory,
                ));
            }
            // Never replace a concurrently introduced file, including another manifest.
            store.create_new(&manifest_path, &raw)?;
            if store.read_raw(&manifest_path)? != raw {
                return Err(runtime_error("runtime_prepare_readback", &manifest_path));
            }
            Ok(())
        };
        if let Err(secondary) = restore() {
            return Err(WorkError::new(
                ExitCode::IoFailure,
                "runtime_cleanup_manifest_restore_failed",
                "Directory cleanup failed and its recovery identity could not be restored.",
                json!({"primary":{"reason_code":primary.reason_code,"details":primary.details},
                    "restore":{"reason_code":secondary.reason_code,"details":secondary.details}}),
            ));
        }
        return Err(primary);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalRuntimeStage {
    Prepared,
    JournalInitialized,
    TargetWritten(usize),
    TargetVerified(usize),
    ProgressWritten(usize),
    JournalReplaced,
    MarkerWritten,
    PublishedVerified,
    BeforeCleanup,
}

/// Independent candidate: callers supply domain/namespace revalidation inside the owner scope.
pub fn publish_prepared_journal_runtime(
    store: &impl ArtifactStore,
    context: &work_feature::ports::RequirementWriterContext,
    manifest: &work_model::runtime::RuntimeManifest,
    recover: bool,
    preflight: impl FnOnce() -> Result<(), WorkError>,
    after_stage: impl FnMut(JournalRuntimeStage) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    if manifest.requirement_id != context.requirement_id.as_str()
        || manifest.canonical_root != context.canonical_project_root.to_string_lossy()
    {
        return Err(runtime_error(
            "runtime_manifest_context_mismatch",
            &context.canonical_project_root,
        ));
    }
    crate::writer_lock::require_no_legacy_locks(context, Some(&manifest.execution_dir))?;
    work_feature::ports::with_runtime_writer(
        &crate::writer_lock::LocalWriterLock,
        context,
        work_model::runtime::LockClass::Execution,
        |_| {
            preflight()?;
            publish_journal_staging_scoped(
                store,
                &context.canonical_project_root,
                manifest,
                recover,
                after_stage,
            )
        },
    )
}

pub(crate) fn publish_journal_staging_scoped(
    store: &impl ArtifactStore,
    root: &Path,
    expected: &work_model::runtime::RuntimeManifest,
    recover: bool,
    after_stage: impl FnMut(JournalRuntimeStage) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    publish_journal_staging_bytes(store, root, expected, recover, after_stage, false).map_err(
        |mut error| {
            if let Ok(current) = read_runtime_manifest(store, root, expected) {
                let mut details = error.details.as_object().cloned().unwrap_or_else(|| {
                    serde_json::Map::from_iter([("primary_details".into(), error.details.clone())])
                });
                let requirement = expected
                    .requirement_id
                    .parse()
                    .expect("verified runtime requirement");
                let operation =
                    runtime_operation(&expected.operation).expect("verified runtime operation");
                let directory = work_operations::derivation::publication::runtime_staging_path(
                    &requirement,
                    operation,
                    &expected.transaction_identity,
                )
                .expect("verified runtime identity");
                details.insert("recovery_required".into(), json!(true));
                details.insert("transaction_dir".into(), json!(directory));
                details.insert("phase".into(), json!(current.phase));
                details.insert("published_count".into(), json!(current.published_count));
                error.details = Value::Object(details);
            }
            error
        },
    )
}

pub(crate) fn inspect_journal_staging_graph(
    root: &Path,
    expected: &work_model::runtime::RuntimeManifest,
) -> Result<(), WorkError> {
    publish_journal_staging_bytes(&LocalFiles, root, expected, true, |_| Ok(()), true).map(|_| ())
}

fn publish_journal_staging_bytes(
    store: &impl ArtifactStore,
    root: &Path,
    expected: &work_model::runtime::RuntimeManifest,
    recover: bool,
    mut after_stage: impl FnMut(JournalRuntimeStage) -> Result<(), WorkError>,
    inspect_only: bool,
) -> Result<Value, WorkError> {
    use work_model::runtime::RuntimePhase as P;
    let prepared = work_operations::derivation::transaction::restore_journal_staging(expected)
        .map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
    let directory = runtime_manifest_directory(root, expected)?;
    let count = expected.targets.len() - 2;
    let journal = &expected.targets[count];
    let marker = &expected.targets[count + 1];
    let read = |relative: &str| -> Result<Option<Vec<u8>>, WorkError> {
        let path = crate::files::resolve_runtime_path(root, relative)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => Ok(Some(store.read_raw(&path)?)),
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(io_error("runtime_target_read_failed", &path)),
        }
    };
    let mut originals = Vec::new();
    let mut after_count = 0;
    let mut before_seen = false;
    let mut changed_after = false;
    for target in &expected.targets[..count] {
        let raw = read(&target.path)?;
        if target.before == target.after {
            if raw.as_deref() != target.after.as_ref().map(|bytes| bytes.bytes.as_slice()) {
                return Err(runtime_error("journal_runtime_formal_conflict", &directory));
            }
            if !before_seen {
                after_count += 1;
            }
        } else if raw.as_deref() == target.after.as_ref().map(|bytes| bytes.bytes.as_slice()) {
            if before_seen {
                return Err(runtime_error(
                    "journal_runtime_publication_order",
                    &directory,
                ));
            }
            after_count += 1;
            changed_after = true;
        } else if raw.as_deref() == target.before.as_ref().map(|bytes| bytes.bytes.as_slice())
            || (recover
                && target.before.is_none()
                && raw
                    .as_ref()
                    .zip(target.after.as_ref())
                    .is_some_and(|(raw, after)| after.bytes.starts_with(raw))
                && !before_seen)
        {
            before_seen = true;
        } else {
            return Err(runtime_error("journal_runtime_formal_conflict", &directory));
        }
        originals.push(raw);
    }
    let journal_raw = read(&journal.path)?;
    let final_journal = journal.after.as_ref().expect("restored journal");
    let journal_done = journal_raw.as_deref() == Some(final_journal.bytes.as_slice());
    if journal_raw.as_deref().is_some_and(|raw| {
        raw != prepared.prepared_journal
            && raw != final_journal.bytes
            && !(recover && prepared.prepared_journal.starts_with(raw))
    }) || (journal_done && after_count != count)
    {
        return Err(runtime_error("journal_runtime_formal_conflict", &directory));
    }
    if changed_after
        && journal_raw.as_deref() != Some(prepared.prepared_journal.as_slice())
        && !journal_done
    {
        return Err(runtime_error(
            "journal_runtime_publication_order",
            &directory,
        ));
    }
    let marker_raw = read(&marker.path)?;
    let final_marker = marker.after.as_ref().expect("restored marker");
    let marker_done = marker_raw.as_deref() == Some(final_marker.bytes.as_slice());
    if marker_raw.as_ref().is_some_and(|raw| {
        !journal_done
            || (raw != &final_marker.bytes && !(recover && final_marker.bytes.starts_with(raw)))
    }) {
        return Err(runtime_error("journal_runtime_marker_conflict", &directory));
    }
    let observed_count = after_count + usize::from(journal_done) + usize::from(marker_done);
    if !recover
        && (changed_after || journal_raw.is_some() || marker_raw.is_some() || directory.exists())
    {
        return Err(runtime_error(
            "journal_runtime_transaction_present",
            &directory,
        ));
    }
    // Changed formal targets use their frozen before proof; all untouched evidence is reread.
    for field in ["source_sha256", "history_sha256"] {
        for (path, hash) in expected.business_identity["original_journal"]["metadata"][field]
            .as_object()
            .expect("restored evidence map")
        {
            if let Some(target) = expected.targets[..count]
                .iter()
                .find(|target| target.path == *path)
            {
                if target.before.as_ref().map(|bytes| bytes.sha256.as_str()) != hash.as_str() {
                    return Err(runtime_error("journal_runtime_source_conflict", &directory));
                }
            } else if !read(path)?.as_ref().is_some_and(|raw| {
                work_operations::derivation::fingerprint::raw(raw) == hash.as_str().unwrap_or("")
            }) {
                return Err(runtime_error("journal_runtime_source_conflict", &directory));
            }
        }
    }
    let existing = directory.exists();
    let mut current = if existing {
        read_runtime_manifest(store, root, expected)?
    } else {
        expected.clone()
    };
    if current.published_count > observed_count
        || (matches!(current.phase, P::PublishedVerified | P::Cleaning) && !marker_done)
    {
        return Err(runtime_error(
            "journal_runtime_progress_conflict",
            &directory,
        ));
    }
    let relative = directory
        .strip_prefix(&expected.canonical_root)
        .map_err(|_| runtime_error("runtime_manifest_context_mismatch", &directory))?
        .to_string_lossy()
        .replace('\\', "/");
    let control_path =
        crate::files::resolve_runtime_path(root, &format!("{relative}/transaction.json.tmp"))?;
    let control = if existing && control_path.exists() {
        let raw = store.read_raw(&control_path)?;
        let candidates =
            work_operations::derivation::transaction::journal_control_candidates(&current, &raw)
                .map_err(|_| runtime_error("runtime_control_temporary_invalid", &control_path))?;
        let candidate = candidates
            .into_iter()
            .find(|candidate| {
                candidate.published_count <= observed_count
                    && (!matches!(candidate.phase, P::PublishedVerified | P::Cleaning)
                        || marker_done)
            })
            .ok_or_else(|| runtime_error("journal_runtime_progress_conflict", &control_path))?;
        if !recover {
            return Err(runtime_error(
                "runtime_control_temporary_present",
                &control_path,
            ));
        }
        Some(candidate)
    } else {
        None
    };
    if existing {
        for path in runtime_entries(root, &directory)? {
            if (path == directory.join("targets") && path.is_dir())
                || path == directory.join("transaction.json")
                || path == control_path
            {
                continue;
            }
            let relative = path
                .strip_prefix(&directory)
                .map_err(|_| runtime_error("runtime_inventory_foreign", &path))?
                .to_string_lossy()
                .replace('\\', "/");
            let frozen = prepared
                .payloads
                .get(&relative)
                .ok_or_else(|| runtime_error("runtime_inventory_foreign", &path))?;
            let raw = store.read_raw(&path)?;
            if raw != *frozen && !(recover && frozen.starts_with(&raw)) {
                return Err(runtime_error("runtime_inventory_hash_mismatch", &path));
            }
        }
    }
    if recover && !existing {
        if !marker_done {
            return Err(runtime_error(
                "runtime_cleanup_identity_missing",
                &directory,
            ));
        }
        work_operations::specification::transaction::verify_retained_journal_commit(
            &expected.business_identity["original_journal"],
            journal_raw.as_ref().unwrap(),
            marker_raw.as_deref(),
        )
        .map_err(|_| runtime_error("journal_runtime_commit_conflict", &directory))?;
        return Ok(json!({"status":"already_published","published_count":count}));
    }
    if inspect_only {
        return Ok(json!({"status":"verified"}));
    }
    (|| {
        if !existing {
            prepare_runtime_transaction(store, root, expected, &prepared.payloads)?;
            after_stage(JournalRuntimeStage::Prepared)?;
        }
        if let Some(candidate) = control {
            let before = store.read_raw(&directory.join("transaction.json"))?;
            let after = serde_json::to_vec(&candidate)
                .map_err(|_| runtime_error("runtime_manifest_encode", &directory))?;
            replace_checked(
                store,
                &directory.join("transaction.json"),
                &before,
                &after,
                &control_path,
                true,
            )?;
            current = candidate;
        }
        if !matches!(current.phase, P::PublishedVerified | P::Cleaning) {
            for (file, frozen) in &prepared.payloads {
                let path = directory.join(file);
                if !path.exists() {
                    store.create_directories(path.parent().expect("prepared parent"))?;
                    store.create_new(&path, frozen)?;
                } else if store.read_raw(&path)? != *frozen {
                    if !recover {
                        return Err(runtime_error("runtime_inventory_hash_mismatch", &path));
                    }
                    complete_write(&path, frozen)?;
                }
            }
            let journal_path = crate::files::resolve_runtime_path(root, &journal.path)?;
            if let Some(raw) = &journal_raw {
                if !journal_done && raw != &prepared.prepared_journal {
                    complete_write(&journal_path, &prepared.prepared_journal)?;
                }
            } else {
                store.create_directories(journal_path.parent().expect("journal parent"))?;
                store.create_new(&journal_path, &prepared.prepared_journal)?;
            }
            if !journal_done && store.read_raw(&journal_path)? != prepared.prepared_journal {
                return Err(runtime_error(
                    "journal_runtime_formal_conflict",
                    &journal_path,
                ));
            }
            after_stage(JournalRuntimeStage::JournalInitialized)?;
            for (position, target) in expected.targets[..count].iter().enumerate() {
                let path = crate::files::resolve_runtime_path(root, &target.path)?;
                let raw = read(&target.path)?;
                let before = target.before.as_ref().map(|bytes| bytes.bytes.as_slice());
                let after = target.after.as_ref().map(|bytes| bytes.bytes.as_slice());
                if raw.as_deref() != after {
                    if raw != originals[position] {
                        return Err(runtime_error("journal_runtime_formal_conflict", &path));
                    }
                    match (before, after) {
                        (None, Some(after)) => {
                            store.create_directories(path.parent().expect("target parent"))?;
                            if raw.is_some() {
                                complete_write(&path, after)?;
                            } else {
                                store.create_new(&path, after)?;
                            }
                        }
                        (Some(before), Some(after)) => replace_checked(
                            store,
                            &path,
                            before,
                            after,
                            &directory.join(format!("targets/{position}.tmp")),
                            recover,
                        )?,
                        (Some(_), None) => store.remove(&path)?,
                        _ => return Err(runtime_error("journal_runtime_formal_conflict", &path)),
                    }
                }
                after_stage(JournalRuntimeStage::TargetWritten(position))?;
                if read(&target.path)?.as_deref() != after {
                    return Err(runtime_error("journal_runtime_readback", &path));
                }
                after_stage(JournalRuntimeStage::TargetVerified(position))?;
                let completed = current.published_count.max(position + 1);
                update_runtime_progress(store, root, expected, completed, P::Publishing)?;
                current = read_runtime_manifest(store, root, expected)?;
                after_stage(JournalRuntimeStage::ProgressWritten(position))?;
            }
            if !journal_done {
                replace_checked(
                    store,
                    &journal_path,
                    &prepared.prepared_journal,
                    &prepared.published_journal,
                    &directory.join("journal.json.tmp"),
                    recover,
                )?;
            }
            if store.read_raw(&journal_path)? != prepared.published_journal {
                return Err(runtime_error("journal_runtime_readback", &journal_path));
            }
            after_stage(JournalRuntimeStage::JournalReplaced)?;
            update_runtime_progress(
                store,
                root,
                expected,
                current.published_count.max(count + 1),
                P::Publishing,
            )?;
            let marker_path = crate::files::resolve_runtime_path(root, &marker.path)?;
            if !marker_done {
                if marker_raw.is_some() {
                    complete_write(&marker_path, &final_marker.bytes)?;
                } else {
                    store.create_new(&marker_path, &final_marker.bytes)?;
                }
            }
            after_stage(JournalRuntimeStage::MarkerWritten)?;
            work_operations::specification::transaction::verify_retained_journal_commit(
                &expected.business_identity["original_journal"],
                &store.read_raw(&journal_path)?,
                Some(&store.read_raw(&marker_path)?),
            )
            .map_err(|_| runtime_error("journal_runtime_commit_conflict", &directory))?;
            verify_runtime_formal(store, root, expected)?;
            // Checked replacements consume prepared files; restore their frozen proof before commit.
            for (file, frozen) in &prepared.payloads {
                let path = directory.join(file);
                if !path.exists() {
                    store.create_new(&path, frozen)?;
                }
                if store.read_raw(&path)? != *frozen {
                    return Err(runtime_error("runtime_inventory_hash_mismatch", &path));
                }
            }
            update_runtime_progress(
                store,
                root,
                expected,
                expected.targets.len(),
                P::PublishedVerified,
            )?;
            after_stage(JournalRuntimeStage::PublishedVerified)?;
        }
        verify_runtime_formal(store, root, expected)?;
        after_stage(JournalRuntimeStage::BeforeCleanup)?;
        cleanup_runtime_transaction(store, root, expected)?;
        Ok(json!({"status":"published","published_count":count}))
    })()
}

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

    fn journal_runtime_case(
        kind: usize,
    ) -> (
        work_feature::ports::RequirementWriterContext,
        work_operations::derivation::transaction::PreparedJournalStaging,
    ) {
        use work_operations::derivation::{publication, transaction::*};
        let root = root().canonicalize().unwrap();
        fs::create_dir_all(root.join("formal")).unwrap();
        fs::create_dir_all(root.join("history")).unwrap();
        fs::write(root.join("formal/change.txt"), b"old\r\n").unwrap();
        fs::write(root.join("formal/remove.txt"), b"removed old bytes").unwrap();
        fs::write(
            root.join("history/record.json"),
            b"immutable historical bytes\r\n",
        )
        .unwrap();
        let context = work_feature::ports::RequirementWriterContext {
            canonical_project_root: root.clone(),
            requirement_id: "example".parse().unwrap(),
        };
        let execution = "custom execution/例";
        let journal = TransactionDeriver::derive(TransactionInput {
            kind: TransactionKind::Update,
            order: PublicationOrder::Flat,
            request: json!({"requirement_id":"example"}),
            artifacts: json!({"execution":execution}),
            affected_task_ids: vec![],
            history: std::collections::BTreeMap::from([(
                "history/record.json".into(),
                b"immutable historical bytes\r\n".to_vec(),
            )]),
            source: std::collections::BTreeMap::from([
                ("formal/change.txt".into(), b"old\r\n".to_vec()),
                ("formal/remove.txt".into(), b"removed old bytes".to_vec()),
            ]),
            candidate: std::collections::BTreeMap::from([
                ("formal/add.txt".into(), b"new artifact".to_vec()),
                ("formal/change.txt".into(), b"new\r\n".to_vec()),
            ]),
        })
        .unwrap()
        .journal;
        let approved = "ab".repeat(32);
        let kind = match kind {
            0 => publication::JournalKind::SpecificationUpdate(
                journal["transaction_id"].as_str().unwrap(),
            ),
            1 => publication::JournalKind::SpecificationMigration(&approved),
            2 => publication::JournalKind::SpecificationMigrationItem {
                approved: &approved,
                position: 2,
            },
            3 => publication::JournalKind::SpecificationMigrationReconcile(&approved),
            4 => publication::JournalKind::InstructionMigration(&approved),
            5 => publication::JournalKind::SourceRefresh(&approved),
            _ => unreachable!(),
        };
        let relative = publication::retained_journal_path(execution, kind).unwrap();
        let prepared = build_journal_staging(JournalStagingInput {
            canonical_root: root.to_str().unwrap(),
            requirement: &context.requirement_id,
            execution_dir: execution,
            journal_path: &relative,
            kind,
            journal: &journal,
        })
        .unwrap();
        (context, prepared)
    }

    #[test]
    fn journal_runtime_all_six_kinds_recover_each_boundary_without_rewriting_history() {
        use JournalRuntimeStage as S;
        let mut boundaries = vec![
            S::Prepared,
            S::JournalInitialized,
            S::JournalReplaced,
            S::MarkerWritten,
            S::PublishedVerified,
            S::BeforeCleanup,
        ];
        for position in 0..3 {
            boundaries.extend([
                S::TargetWritten(position),
                S::TargetVerified(position),
                S::ProgressWritten(position),
            ]);
        }
        for kind in 0..6 {
            for boundary in &boundaries {
                let (context, prepared) = journal_runtime_case(kind);
                let root = &context.canonical_project_root;
                let failure = publish_prepared_journal_runtime(
                    &LocalFiles,
                    &context,
                    &prepared.manifest,
                    false,
                    || Ok(()),
                    |stage| {
                        if stage == *boundary {
                            Err(io_error("injected_journal_boundary", root))
                        } else {
                            Ok(())
                        }
                    },
                )
                .unwrap_err();
                assert_eq!(failure.reason_code, "injected_journal_boundary");
                assert_eq!(failure.details["recovery_required"], true);
                assert!(
                    !root
                        .join("outputs/work/runtime/locks/example/execution.lock")
                        .exists()
                );
                let directory = runtime_manifest_directory(root, &prepared.manifest).unwrap();
                assert!(directory.join("transaction.json").is_file());
                publish_prepared_journal_runtime(
                    &LocalFiles,
                    &context,
                    &prepared.manifest,
                    true,
                    || Ok(()),
                    |_| Ok(()),
                )
                .unwrap();
                assert!(!directory.exists());
                assert_eq!(
                    fs::read(root.join(&prepared.manifest.targets[3].path)).unwrap(),
                    prepared.published_journal
                );
                assert_eq!(
                    fs::read(root.join(&prepared.manifest.targets[4].path)).unwrap(),
                    completion_marker(&prepared.published_journal)
                );
                assert_eq!(
                    fs::read(root.join("formal/add.txt")).unwrap(),
                    b"new artifact"
                );
                assert_eq!(
                    fs::read(root.join("formal/change.txt")).unwrap(),
                    b"new\r\n"
                );
                assert!(!root.join("formal/remove.txt").exists());
                assert_eq!(
                    fs::read(root.join("history/record.json")).unwrap(),
                    b"immutable historical bytes\r\n"
                );
                let already = publish_prepared_journal_runtime(
                    &LocalFiles,
                    &context,
                    &prepared.manifest,
                    true,
                    || Ok(()),
                    |_| Ok(()),
                )
                .unwrap();
                assert_eq!(already["status"], "already_published");
            }
        }
    }

    struct ImmutableJournalTargetStore {
        target: std::path::PathBuf,
    }
    impl ArtifactStore for ImmutableJournalTargetStore {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            assert_ne!(path, self.target);
            LocalFiles.create_directories(path)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
            assert_ne!(path, self.target);
            LocalFiles.create_new(path, bytes)
        }
        fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
            assert_ne!(target, self.target);
            LocalFiles.replace(temporary, target)
        }
        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            assert_ne!(path, self.target);
            LocalFiles.remove(path)
        }
    }

    #[test]
    fn journal_runtime_neutral_targets_never_write_or_imply_changed_progress_at_any_position() {
        use work_operations::derivation::{publication, transaction::*};
        let mut boundaries = vec![
            JournalRuntimeStage::Prepared,
            JournalRuntimeStage::PublishedVerified,
        ];
        for step in 0..3 {
            boundaries.extend([
                JournalRuntimeStage::TargetWritten(step),
                JournalRuntimeStage::ProgressWritten(step),
            ]);
        }
        for neutral in 0..3 {
            for boundary in &boundaries {
                let root = root().canonicalize().unwrap();
                fs::create_dir_all(root.join("formal")).unwrap();
                let context = work_feature::ports::RequirementWriterContext {
                    canonical_project_root: root.clone(),
                    requirement_id: "example".parse().unwrap(),
                };
                let mut source = std::collections::BTreeMap::new();
                let mut candidate = std::collections::BTreeMap::new();
                for position in 0..3 {
                    let path = format!("formal/{}.txt", (b'a' + position as u8) as char);
                    let before = format!("original {position}\r\n").into_bytes();
                    fs::write(root.join(&path), &before).unwrap();
                    candidate.insert(
                        path.clone(),
                        if position == neutral {
                            before.clone()
                        } else {
                            format!("approved {position}\r\n").into_bytes()
                        },
                    );
                    source.insert(path, before);
                }
                let approved = "a".repeat(64);
                let journal = TransactionDeriver::derive(TransactionInput {
                    kind: TransactionKind::Migration,
                    order: PublicationOrder::Migration,
                    request: json!({"migration":{},"preview_fingerprint":approved}),
                    artifacts: json!({"execution":"execution"}),
                    affected_task_ids: vec![],
                    history: std::collections::BTreeMap::new(),
                    source: source.clone(),
                    candidate: candidate.clone(),
                })
                .unwrap()
                .journal;
                assert_eq!(journal["files"].as_array().unwrap().len(), 3);
                let kind = publication::JournalKind::SpecificationMigration(&approved);
                let relative = publication::retained_journal_path("execution", kind).unwrap();
                let prepared = build_journal_staging(JournalStagingInput {
                    canonical_root: root.to_str().unwrap(),
                    requirement: &context.requirement_id,
                    execution_dir: "execution",
                    journal_path: &relative,
                    kind,
                    journal: &journal,
                })
                .unwrap();
                let neutral_path = format!("formal/{}.txt", (b'a' + neutral as u8) as char);
                let store = ImmutableJournalTargetStore {
                    target: root.join(&neutral_path),
                };
                let failure = publish_prepared_journal_runtime(
                    &store,
                    &context,
                    &prepared.manifest,
                    false,
                    || Ok(()),
                    |stage| {
                        if &stage == boundary {
                            Err(io_error("injected_neutral_target_boundary", &root))
                        } else {
                            Ok(())
                        }
                    },
                )
                .unwrap_err();
                assert_eq!(failure.reason_code, "injected_neutral_target_boundary");
                assert_eq!(fs::read(&store.target).unwrap(), source[&neutral_path]);
                publish_prepared_journal_runtime(
                    &store,
                    &context,
                    &prepared.manifest,
                    true,
                    || Ok(()),
                    |_| Ok(()),
                )
                .unwrap();
                for (path, after) in candidate {
                    assert_eq!(fs::read(root.join(path)).unwrap(), after);
                }
                assert_eq!(fs::read(&store.target).unwrap(), source[&neutral_path]);
                assert!(
                    !runtime_manifest_directory(&root, &prepared.manifest)
                        .unwrap()
                        .exists()
                );
            }
        }
    }

    struct JournalFaultStore {
        fault: &'static str,
        removals: std::cell::Cell<usize>,
    }
    impl ArtifactStore for JournalFaultStore {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
        fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
            let name = path.file_name().unwrap().to_string_lossy();
            let partial = (self.fault == "payload" && name == "journal.json.tmp")
                || (self.fault == "artifact" && name == "add.txt")
                || (self.fault == "journal" && name == "journal.json")
                || (self.fault == "marker" && name == "committed.sha256");
            if partial {
                LocalFiles.create_new(path, &bytes[..7])?;
                return Err(io_error("injected_journal_partial", path));
            }
            LocalFiles.create_new(path, bytes)
        }
        fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
            let name = temporary.file_name().unwrap().to_string_lossy();
            if (self.fault == "control" && name == "transaction.json.tmp")
                || (self.fault == "journal-replace" && name == "journal.json.tmp")
            {
                let bytes = fs::read(temporary).unwrap();
                fs::write(temporary, &bytes[..7]).unwrap();
                return Err(io_error("injected_journal_partial", temporary));
            }
            LocalFiles.replace(temporary, target)
        }
        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            if self.fault == "cleanup" && path.to_string_lossy().contains("runtime") {
                if self.removals.get() == 1 {
                    return Err(io_error("injected_journal_cleanup", path));
                }
                self.removals.set(self.removals.get() + 1);
            }
            LocalFiles.remove(path)
        }
    }

    #[test]
    fn journal_runtime_recovers_partial_payload_journal_marker_control_and_cleanup() {
        for fault in [
            "payload",
            "artifact",
            "journal",
            "marker",
            "control",
            "journal-replace",
            "cleanup",
        ] {
            let (context, prepared) = journal_runtime_case(2);
            let store = JournalFaultStore {
                fault,
                removals: std::cell::Cell::new(0),
            };
            let failure = publish_prepared_journal_runtime(
                &store,
                &context,
                &prepared.manifest,
                false,
                || Ok(()),
                |_| Ok(()),
            )
            .unwrap_err();
            assert_eq!(
                failure.details["recovery_required"], true,
                "{fault}: {failure:?}"
            );
            let directory =
                runtime_manifest_directory(&context.canonical_project_root, &prepared.manifest)
                    .unwrap();
            assert!(directory.join("transaction.json").is_file());
            if fault == "control" {
                fs::remove_file(directory.join("targets/1.tmp")).unwrap();
            }
            publish_prepared_journal_runtime(
                &LocalFiles,
                &context,
                &prepared.manifest,
                true,
                || Ok(()),
                |_| Ok(()),
            )
            .unwrap();
            assert!(!directory.exists(), "{fault}");
            assert_eq!(
                fs::read(
                    context
                        .canonical_project_root
                        .join(&prepared.manifest.targets[3].path)
                )
                .unwrap(),
                prepared.published_journal
            );
            assert_eq!(
                fs::read(context.canonical_project_root.join("history/record.json")).unwrap(),
                b"immutable historical bytes\r\n"
            );
        }
    }

    #[test]
    fn journal_runtime_refuses_unknown_formal_source_inventory_and_premature_control_before_writes()
    {
        for fault in [
            "formal",
            "source",
            "inventory",
            "control",
            "missing-journal",
        ] {
            let (context, prepared) = journal_runtime_case(0);
            let root = &context.canonical_project_root;
            publish_prepared_journal_runtime(
                &LocalFiles,
                &context,
                &prepared.manifest,
                false,
                || Ok(()),
                |stage| {
                    if stage == JournalRuntimeStage::JournalInitialized {
                        Err(io_error("injected_pause", root))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            let directory = runtime_manifest_directory(root, &prepared.manifest).unwrap();
            match fault {
                "formal" => {
                    fs::write(root.join("formal/change.txt"), b"foreign formal bytes").unwrap()
                }
                "source" => {
                    fs::write(root.join("history/record.json"), b"foreign history bytes").unwrap()
                }
                "inventory" => {
                    fs::write(directory.join("foreign.txt"), b"foreign runtime bytes").unwrap()
                }
                "control" => {
                    let mut future = prepared.manifest.clone();
                    future.phase = work_model::runtime::RuntimePhase::PublishedVerified;
                    future.published_count = future.targets.len();
                    fs::write(
                        directory.join("transaction.json.tmp"),
                        serde_json::to_vec(&future).unwrap(),
                    )
                    .unwrap();
                    fs::remove_file(directory.join("targets/0.tmp")).unwrap();
                }
                "missing-journal" => {
                    fs::write(root.join("formal/add.txt"), b"new artifact").unwrap();
                    fs::remove_file(root.join(&prepared.manifest.targets[3].path)).unwrap();
                }
                _ => unreachable!(),
            }
            let before_manifest = fs::read(directory.join("transaction.json")).unwrap();
            let before_formal = fs::read(root.join("formal/change.txt")).unwrap();
            let before_history = fs::read(root.join("history/record.json")).unwrap();
            let failure = publish_prepared_journal_runtime(
                &LocalFiles,
                &context,
                &prepared.manifest,
                true,
                || Ok(()),
                |_| Ok(()),
            )
            .unwrap_err();
            assert_eq!(
                failure.exit_code,
                ExitCode::ArtifactIntegrity,
                "{fault}: {failure:?}"
            );
            assert_eq!(
                fs::read(directory.join("transaction.json")).unwrap(),
                before_manifest
            );
            assert_eq!(
                fs::read(root.join("formal/change.txt")).unwrap(),
                before_formal
            );
            assert_eq!(
                fs::read(root.join("history/record.json")).unwrap(),
                before_history
            );
            assert!(!root.join(&prepared.manifest.targets[4].path).exists());
            if fault == "control" {
                assert!(!directory.join("targets/0.tmp").exists());
            }
            assert!(
                !root
                    .join("outputs/work/runtime/locks/example/execution.lock")
                    .exists()
            );
        }
    }

    fn runtime_case(
        root: &Path,
    ) -> (
        work_model::runtime::RuntimeManifest,
        std::collections::BTreeMap<String, Vec<u8>>,
    ) {
        use work_model::runtime::*;
        let raw = b"approved".to_vec();
        let hash = work_operations::derivation::fingerprint::raw(&raw);
        let mut manifest = RuntimeManifest {
            schema: "work-runtime-transaction".into(),
            canonical_root: root.canonicalize().unwrap().to_string_lossy().into_owned(),
            requirement_id: "example".into(),
            execution_dir: "custom/execution".into(),
            operation: "record-finish".into(),
            transaction_identity: String::new(),
            approval_sha256: "a".repeat(64),
            business_identity: json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001"}),
            targets: vec![RuntimeTarget {
                path: "custom/execution/index.json".into(),
                before: None,
                after: Some(RuntimeBytes {
                    bytes: raw.clone(),
                    sha256: hash.clone(),
                }),
            }],
            inventory: vec![
                RuntimeFile {
                    path: "attempt.json.tmp".into(),
                    sha256: hash.clone(),
                    size_bytes: raw.len() as u64,
                },
                RuntimeFile {
                    path: "index.json.tmp".into(),
                    sha256: hash,
                    size_bytes: raw.len() as u64,
                },
            ],
            published_count: 0,
            phase: RuntimePhase::Prepared,
        };
        manifest.transaction_identity =
            work_operations::derivation::identity::runtime_transaction_identity(
                &manifest.canonical_root,
                &"example".parse().unwrap(),
                &manifest.operation,
                &manifest.approval_sha256,
                &manifest.business_identity,
                &[manifest.targets[0].path.clone()],
            )
            .unwrap();
        (
            manifest,
            std::collections::BTreeMap::from([
                ("attempt.json.tmp".into(), raw.clone()),
                ("index.json.tmp".into(), raw),
            ]),
        )
    }

    struct FailRuntimeRemove {
        removed: std::cell::Cell<usize>,
    }
    impl ArtifactStore for FailRuntimeRemove {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &Path, raw: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, raw)
        }
        fn replace(&self, source: &Path, target: &Path) -> Result<(), WorkError> {
            LocalFiles.replace(source, target)
        }
        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            if self.removed.get() == 1 {
                return Err(io_error("injected_runtime_cleanup", path));
            }
            LocalFiles.remove(path)?;
            self.removed.set(self.removed.get() + 1);
            Ok(())
        }
    }

    struct ForeignAfterManifestRemoval {
        foreign_manifest: bool,
    }
    impl ArtifactStore for ForeignAfterManifestRemoval {
        fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
            LocalFiles.read_raw(path)
        }
        fn create_new(&self, path: &Path, raw: &[u8]) -> Result<(), WorkError> {
            LocalFiles.create_new(path, raw)
        }
        fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
            LocalFiles.replace(temporary, target)
        }
        fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.create_directories(path)
        }
        fn remove(&self, path: &Path) -> Result<(), WorkError> {
            LocalFiles.remove(path)?;
            if path
                .file_name()
                .is_some_and(|name| name == "transaction.json")
            {
                let foreign = if self.foreign_manifest {
                    path.to_path_buf()
                } else {
                    path.parent().unwrap().join("foreign-after-cleanup")
                };
                fs::write(foreign, b"foreign exact bytes").unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn final_directory_cleanup_failure_preserves_cleaning_identity_or_foreign_manifest_without_overwrite()
     {
        use work_model::runtime::RuntimePhase as P;
        for foreign_manifest in [false, true] {
            let root = root();
            let (manifest, payloads) = runtime_case(&root);
            let directory =
                prepare_runtime_transaction(&LocalFiles, &root, &manifest, &payloads).unwrap();
            let formal = root.join(&manifest.targets[0].path);
            fs::create_dir_all(formal.parent().unwrap()).unwrap();
            fs::write(&formal, b"approved").unwrap();
            update_runtime_progress(&LocalFiles, &root, &manifest, 1, P::PublishedVerified)
                .unwrap();
            let issue = cleanup_runtime_transaction(
                &ForeignAfterManifestRemoval { foreign_manifest },
                &root,
                &manifest,
            )
            .unwrap_err();
            if foreign_manifest {
                assert_eq!(issue.reason_code, "runtime_cleanup_manifest_restore_failed");
                assert_eq!(
                    issue.details["primary"]["reason_code"],
                    "runtime_cleanup_failed"
                );
                assert_eq!(
                    fs::read(directory.join("transaction.json")).unwrap(),
                    b"foreign exact bytes"
                );
            } else {
                assert_eq!(issue.reason_code, "runtime_cleanup_failed");
                let preserved = read_runtime_manifest(&LocalFiles, &root, &manifest).unwrap();
                assert_eq!(preserved.phase, P::Cleaning);
                assert_eq!(preserved.published_count, 1);
                let foreign = directory.join("foreign-after-cleanup");
                assert_eq!(fs::read(&foreign).unwrap(), b"foreign exact bytes");
                assert_eq!(
                    cleanup_runtime_transaction(&LocalFiles, &root, &manifest)
                        .unwrap_err()
                        .reason_code,
                    "runtime_inventory_foreign"
                );
                // Only the isolated actor removes the exact foreign file it introduced.
                fs::remove_file(foreign).unwrap();
                cleanup_runtime_transaction(&LocalFiles, &root, &manifest).unwrap();
                assert!(!directory.exists());
            }
            assert_eq!(fs::read(formal).unwrap(), b"approved");
        }
    }

    #[test]
    fn runtime_cleanup_reenters_after_partial_failure_and_preserves_formal_and_foreign_transactions()
     {
        use work_model::runtime::RuntimePhase as P;
        let root = root();
        let (manifest, payloads) = runtime_case(&root);
        let directory =
            prepare_runtime_transaction(&LocalFiles, &root, &manifest, &payloads).unwrap();
        assert!(prepare_runtime_transaction(&LocalFiles, &root, &manifest, &payloads).is_err());
        assert!(cleanup_runtime_transaction(&LocalFiles, &root, &manifest).is_err());
        let formal = root.join(&manifest.targets[0].path);
        fs::create_dir_all(formal.parent().unwrap()).unwrap();
        fs::write(&formal, b"approved").unwrap();
        let mut other = manifest.clone();
        other.business_identity["attempt_id"] = json!("ATTEMPT-002");
        other.transaction_identity =
            work_operations::derivation::identity::runtime_transaction_identity(
                &other.canonical_root,
                &"example".parse().unwrap(),
                &other.operation,
                &other.approval_sha256,
                &other.business_identity,
                &[other.targets[0].path.clone()],
            )
            .unwrap();
        let foreign = prepare_runtime_transaction(&LocalFiles, &root, &other, &payloads).unwrap();
        update_runtime_progress(&LocalFiles, &root, &manifest, 1, P::PublishedVerified).unwrap();
        assert_eq!(
            cleanup_runtime_transaction(
                &FailRuntimeRemove {
                    removed: std::cell::Cell::new(0)
                },
                &root,
                &manifest
            )
            .unwrap_err()
            .reason_code,
            "injected_runtime_cleanup"
        );
        assert!(directory.join("transaction.json").is_file());
        assert_eq!(
            read_runtime_manifest(&LocalFiles, &root, &manifest)
                .unwrap()
                .phase,
            P::Cleaning
        );
        cleanup_runtime_transaction(&LocalFiles, &root, &manifest).unwrap();
        assert!(!directory.exists());
        let mut finished = manifest.clone();
        finished.phase = P::Cleaning;
        finished.published_count = 1;
        cleanup_runtime_transaction(&LocalFiles, &root, &finished).unwrap();
        fs::create_dir(&directory).unwrap();
        cleanup_runtime_transaction(&LocalFiles, &root, &finished).unwrap();
        assert_eq!(fs::read(formal).unwrap(), b"approved");
        for (path, raw) in payloads {
            assert_eq!(fs::read(foreign.join(path)).unwrap(), raw);
        }
    }

    #[test]
    fn runtime_cleanup_rejects_foreign_entries_corruption_and_changed_approval_before_removal() {
        use work_model::runtime::RuntimePhase as P;
        let root = root();
        let (manifest, payloads) = runtime_case(&root);
        let directory =
            prepare_runtime_transaction(&LocalFiles, &root, &manifest, &payloads).unwrap();
        let formal = root.join(&manifest.targets[0].path);
        fs::create_dir_all(formal.parent().unwrap()).unwrap();
        fs::write(&formal, b"approved").unwrap();
        update_runtime_progress(&LocalFiles, &root, &manifest, 1, P::PublishedVerified).unwrap();
        fs::create_dir(directory.join("foreign-empty")).unwrap();
        assert_eq!(
            cleanup_runtime_transaction(&LocalFiles, &root, &manifest)
                .unwrap_err()
                .reason_code,
            "runtime_inventory_foreign"
        );
        fs::remove_dir(directory.join("foreign-empty")).unwrap();
        fs::write(directory.join("index.json.tmp"), b"corrupt").unwrap();
        assert_eq!(
            cleanup_runtime_transaction(&LocalFiles, &root, &manifest)
                .unwrap_err()
                .reason_code,
            "runtime_inventory_hash_mismatch"
        );
        assert_eq!(
            fs::read(directory.join("attempt.json.tmp")).unwrap(),
            b"approved"
        );
        let mut different = manifest.clone();
        different.approval_sha256.replace_range(63..64, "b");
        assert!(read_runtime_manifest(&LocalFiles, &root, &different).is_err());
        assert_eq!(fs::read(formal).unwrap(), b"approved");
        assert!(directory.join("transaction.json").is_file());
    }

    #[test]
    fn runtime_control_temporary_accepts_only_expected_next_phase_bytes() {
        use work_model::runtime::RuntimePhase as P;
        for corrupt in [false, true] {
            let project = root();
            let (manifest, payloads) = runtime_case(&project);
            let directory =
                prepare_runtime_transaction(&LocalFiles, &project, &manifest, &payloads).unwrap();
            let formal = project.join(&manifest.targets[0].path);
            fs::create_dir_all(formal.parent().unwrap()).unwrap();
            fs::write(&formal, b"approved").unwrap();
            update_runtime_progress(&LocalFiles, &project, &manifest, 1, P::PublishedVerified)
                .unwrap();
            let mut cleaning = manifest.clone();
            cleaning.phase = P::Cleaning;
            cleaning.published_count = 1;
            let next = serde_json::to_vec(&cleaning).unwrap();
            fs::write(
                directory.join("transaction.json.tmp"),
                if corrupt {
                    b"foreign".as_slice()
                } else {
                    &next[..next.len() / 2]
                },
            )
            .unwrap();
            let result = cleanup_runtime_transaction(&LocalFiles, &project, &manifest);
            if corrupt {
                assert!(result.is_err());
                assert_eq!(
                    fs::read(directory.join("attempt.json.tmp")).unwrap(),
                    b"approved"
                );
                assert!(directory.join("transaction.json").is_file());
            } else {
                result.unwrap();
                assert!(!directory.exists());
            }
            assert_eq!(fs::read(formal).unwrap(), b"approved");
        }
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
