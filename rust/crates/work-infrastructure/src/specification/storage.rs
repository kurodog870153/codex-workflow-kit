//! Specification transaction journal and safe local publication.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_operations::derivation::fingerprint;
#[cfg(test)]
use work_operations::derivation::fingerprint::raw as sha256_hex;
use work_operations::derivation::publication::completion_marker;
use work_operations::derivation::snapshot::decode_snapshot;
use work_operations::protocol::TASK_ID_PREFIX;
use work_operations::specification::transaction::render_transaction;

use crate::files::{LocalFiles, resolve_project_path};
use crate::transaction_storage::{Publication, publish_recoverable_sequence, replace_journal};

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn contract_error() -> WorkError {
    error(
        ExitCode::Contract,
        "invalid_contract_value",
        "The specification transaction is invalid.",
        json!({}),
    )
}

pub fn storage_path(root: &Path, relative: &str) -> Result<PathBuf, WorkError> {
    let (_, path) = resolve_project_path(root, relative)?;
    let mut candidate = root.to_path_buf();
    for part in relative.split('/') {
        candidate.push(part);
        match fs::symlink_metadata(&candidate) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || is_windows_reparse_point(&meta) {
                    return Err(error(
                        ExitCode::Contract,
                        "spec_update_link",
                        "Specification storage cannot contain links.",
                        json!({}),
                    ));
                }
                if candidate == path && meta.is_file() && hard_linked(&meta) {
                    return Err(error(
                        ExitCode::Contract,
                        "spec_update_alias",
                        "Specification storage cannot contain hard links.",
                        json!({}),
                    ));
                }
            }
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(error(
                    ExitCode::IoFailure,
                    "path_resolution_failed",
                    "The path could not be resolved.",
                    json!({"path": relative}),
                ));
            }
        }
    }
    Ok(path)
}

#[cfg(unix)]
fn hard_linked(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() != 1
}

#[cfg(not(unix))]
fn hard_linked(_: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn is_windows_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn read_journal(path: &Path) -> Result<(Value, Vec<u8>), WorkError> {
    let raw = LocalFiles.read_raw(path)?;
    let value: Value = serde_json::from_slice(&raw).map_err(|_| contract_error())?;
    let canonical = render_transaction(&value).map_err(|_| contract_error())?;
    if raw != canonical {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "noncanonical_json",
            "The transaction journal is not canonical JSON.",
            json!({}),
        ));
    }
    Ok((value, raw))
}

pub fn write_journal(root: &Path, relative: &str, value: &Value) -> Result<Vec<u8>, WorkError> {
    let raw = render_transaction(value).map_err(|_| contract_error())?;
    let path = storage_path(root, relative)?;
    LocalFiles.create_new(&path, &raw)?;
    Ok(raw)
}

#[derive(Debug)]
pub struct RetainedJournalEvidence {
    pub relative: String,
    pub contract: Value,
    pub raw: Vec<u8>,
    pub marker_raw: Option<Vec<u8>>,
    pub completion: work_operations::specification::transaction::CompletionState,
}

pub struct RetainedJournalRuntimeInput<'a> {
    pub context: &'a work_feature::ports::RequirementWriterContext,
    pub execution: &'a str,
    pub relative: &'a str,
    pub prepared_journal: &'a Value,
    pub recover: bool,
}

/// Candidate adapter; no normal entry is switched until all journal consumers are ready.
pub fn publish_retained_journal_runtime(
    input: RetainedJournalRuntimeInput<'_>,
    preflight: impl FnOnce() -> Result<(), WorkError>,
    after_stage: impl FnMut(crate::transaction_storage::JournalRuntimeStage) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    prepare_retained_journal_manifest(&input)?;
    crate::writer_lock::require_no_legacy_locks(input.context, Some(input.execution))?;
    work_feature::ports::with_runtime_writer(
        &crate::writer_lock::LocalWriterLock,
        input.context,
        work_model::runtime::LockClass::Execution,
        |owner| publish_retained_journal_with_owner(&input, owner, preflight, after_stage),
    )
}

fn prepare_retained_journal_manifest(
    input: &RetainedJournalRuntimeInput<'_>,
) -> Result<work_model::runtime::RuntimeManifest, WorkError> {
    use work_operations::derivation::publication::JournalKind;
    let kind = work_operations::specification::transaction::verified_retained_journal_kind(
        input.execution,
        input.relative,
        input.prepared_journal,
    )
    .map_err(|issue| {
        error(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if !input.recover
        && matches!(
            kind,
            JournalKind::InstructionMigration(_) | JournalKind::SourceRefresh(_)
        )
    {
        return Err(retained_error("journal_recover_only", input.relative));
    }
    let request = &input.prepared_journal["metadata"]["request"];
    for requirement in [
        request.get("requirement_id"),
        request
            .get("task_index")
            .and_then(|index| index.get("requirement_id")),
    ]
    .into_iter()
    .flatten()
    {
        if requirement != input.context.requirement_id.as_str() {
            return Err(retained_error(
                "journal_requirement_identity",
                input.relative,
            ));
        }
    }
    let prepared = work_operations::derivation::transaction::build_journal_staging(
        work_operations::derivation::transaction::JournalStagingInput {
            canonical_root: input
                .context
                .canonical_project_root
                .to_str()
                .ok_or_else(|| retained_error("journal_root_identity", input.relative))?,
            requirement: &input.context.requirement_id,
            execution_dir: input.execution,
            journal_path: input.relative,
            kind,
            journal: input.prepared_journal,
        },
    )
    .map_err(|issue| {
        error(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    Ok(prepared.manifest)
}

pub(crate) fn publish_retained_journal_with_owner(
    input: &RetainedJournalRuntimeInput<'_>,
    owner: &work_model::runtime::RuntimeOwner,
    preflight: impl FnOnce() -> Result<(), WorkError>,
    after_stage: impl FnMut(crate::transaction_storage::JournalRuntimeStage) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    publish_retained_journal_with_batch_owner(input, owner, None, preflight, after_stage)
}

pub(crate) struct RetainedJournalBatch {
    manifests: BTreeMap<String, work_model::runtime::RuntimeManifest>,
}

pub(crate) fn prepare_retained_journal_batch(
    context: &work_feature::ports::RequirementWriterContext,
    execution: &str,
    approved: &str,
    journals: &BTreeMap<String, Value>,
) -> Result<RetainedJournalBatch, WorkError> {
    let mut manifests = BTreeMap::new();
    let mut targets = std::collections::BTreeSet::new();
    if !work_operations::protocol::valid_sha256(approved) {
        return Err(retained_error("journal_batch_identity", execution));
    }
    for (relative, journal) in journals {
        let manifest = prepare_retained_journal_manifest(&RetainedJournalRuntimeInput {
            context,
            execution,
            relative,
            prepared_journal: journal,
            recover: true,
        })?;
        if manifest.operation != "specification-migration-item"
            || journal["metadata"]["request"]["request_sha256"] != approved
            || manifest.business_identity["layout_identity"] != approved
            || manifest.targets[..manifest.targets.len() - 2]
                .iter()
                .any(|target| !targets.insert(target.path.clone()))
        {
            return Err(retained_error("journal_batch_identity", relative));
        }
        manifests.insert(relative.clone(), manifest);
    }
    Ok(RetainedJournalBatch { manifests })
}

pub(crate) fn prepare_retained_batch_history(
    root: &Path,
    execution: &str,
    approved: &str,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    // Provisional derivation only: every excluded leaf must later match the complete batch proof
    // under the Native owner before this snapshot can authorize any write.
    use work_operations::derivation::publication::{self, JournalKind};
    let current = publication::retained_journal_path(
        execution,
        JournalKind::SpecificationMigration(approved),
    )
    .map_err(|_| retained_error("journal_batch_identity", execution))?;
    let mut history = execution_history_bytes_with_receipts(root, execution)?;
    for relative in retained_journal_paths(root, execution)? {
        if !publication::retained_journal_history_includes(execution, &relative, Some(&current))
            .map_err(|_| retained_error("journal_batch_identity", &relative))?
        {
            continue;
        }
        let evidence = read_retained_journal(root, execution, &relative)?;
        if evidence.completion
            != work_operations::specification::transaction::CompletionState::Completed
        {
            return Err(retained_error("spec_update_pending", &relative));
        }
        let marker = publication::retained_journal_marker(&relative)
            .map_err(|_| retained_error("journal_layout_identity", &relative))?;
        history.insert(relative, evidence.raw);
        history.insert(marker, evidence.marker_raw.expect("verified marker"));
    }
    Ok(history)
}

pub(crate) fn publish_retained_journal_with_batch_owner(
    input: &RetainedJournalRuntimeInput<'_>,
    owner: &work_model::runtime::RuntimeOwner,
    batch: Option<&RetainedJournalBatch>,
    preflight: impl FnOnce() -> Result<(), WorkError>,
    after_stage: impl FnMut(crate::transaction_storage::JournalRuntimeStage) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    let manifest = prepare_retained_journal_manifest(input)?;
    retained_journal_history_with_batch_owner(input, owner, batch)?;
    preflight()?;
    crate::transaction_storage::publish_journal_staging_scoped(
        &LocalFiles,
        &input.context.canonical_project_root,
        &manifest,
        input.recover,
        after_stage,
    )
}

fn require_retained_native_scope(
    input: &RetainedJournalRuntimeInput<'_>,
    owner: &work_model::runtime::RuntimeOwner,
    manifest: &work_model::runtime::RuntimeManifest,
    batch: Option<&RetainedJournalBatch>,
) -> Result<std::collections::BTreeSet<String>, WorkError> {
    let root = &input.context.canonical_project_root;
    let requirement = &input.context.requirement_id;
    if owner.validate_shape().is_err()
        || owner.class != work_model::runtime::LockClass::Execution
        || owner.canonical_root != root.to_string_lossy()
        || owner.requirement_id != requirement.as_str()
        || root
            .canonicalize()
            .map_err(|_| retained_error("journal_owner_identity", input.relative))?
            != *root
    {
        return Err(retained_error("journal_owner_identity", input.relative));
    }
    let identity = work_operations::derivation::identity::runtime_owner_identity(
        &owner.canonical_root,
        requirement,
        "execution",
        &owner.instance_nonce,
    )
    .map_err(|_| retained_error("journal_owner_identity", input.relative))?;
    let lock =
        work_operations::derivation::publication::runtime_lock_path(requirement, "execution")
            .map_err(|_| retained_error("journal_owner_identity", input.relative))?;
    let lock = crate::files::resolve_runtime_path(root, &lock)?;
    if owner.owner_identity != identity
        || LocalFiles.read_raw(&lock)?
            != serde_json::to_vec(owner)
                .map_err(|_| retained_error("journal_owner_identity", input.relative))?
    {
        return Err(retained_error("journal_owner_identity", input.relative));
    }
    crate::writer_lock::require_no_legacy_locks(input.context, Some(input.execution))?;
    require_no_legacy_journal_layout(root, input.execution)?;
    let inventory = crate::execution::storage::LocalExecutionStorage {
        project_root: root.clone(),
    }
    .retained_requirement_inventory(input.context, input.execution)?;
    let mut present = std::collections::BTreeSet::new();
    for item in inventory {
        let relative = item.manifest.business_identity["journal_path"]
            .as_str()
            .unwrap_or("");
        let mut expected = if relative == input.relative {
            manifest.clone()
        } else if let Some(member) = batch.and_then(|batch| batch.manifests.get(relative)) {
            member.clone()
        } else {
            return Err(retained_error(
                "runtime_execution_transaction_present",
                &item.transaction_dir,
            ));
        };
        expected.phase = item.manifest.phase;
        expected.published_count = item.manifest.published_count;
        if (relative == input.relative && !input.recover) || expected != item.manifest {
            return Err(retained_error(
                "runtime_execution_transaction_present",
                &item.transaction_dir,
            ));
        }
        if relative != input.relative {
            crate::transaction_storage::inspect_journal_staging_graph(root, &expected)?;
        }
        present.insert(relative.to_owned());
    }
    Ok(present)
}

pub(crate) fn retained_journal_history_with_owner(
    input: &RetainedJournalRuntimeInput<'_>,
    owner: &work_model::runtime::RuntimeOwner,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    retained_journal_history_with_batch_owner(input, owner, None)
}

pub(crate) fn retained_journal_history_with_batch_owner(
    input: &RetainedJournalRuntimeInput<'_>,
    owner: &work_model::runtime::RuntimeOwner,
    batch: Option<&RetainedJournalBatch>,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    use work_operations::derivation::publication::{self, JournalKind};
    let manifest = prepare_retained_journal_manifest(input)?;
    if let Some(batch) = batch {
        for member in batch.manifests.values() {
            if member.canonical_root != manifest.canonical_root
                || member.requirement_id != manifest.requirement_id
                || member.execution_dir != manifest.execution_dir
                || member.business_identity["layout_identity"]
                    != manifest.business_identity["layout_identity"]
            {
                return Err(retained_error("journal_batch_identity", input.relative));
            }
        }
    }
    let runtime = require_retained_native_scope(input, owner, &manifest, batch)?;
    let root = &input.context.canonical_project_root;
    let paths = retained_journal_paths(root, input.execution)?;
    let own_path = crate::files::resolve_runtime_path(root, input.relative)?;
    if !input.recover && (own_path.exists() || paths.iter().any(|path| path == input.relative)) {
        return Err(retained_error(
            "journal_publication_reserved",
            input.relative,
        ));
    }
    let mut history = execution_history_bytes_with_receipts(root, input.execution)?;
    for relative in paths {
        let member = batch.and_then(|batch| batch.manifests.get(&relative));
        if runtime.contains(&relative) && (relative == input.relative || member.is_some()) {
            // The exact native manifest authorizes only this leaf. Byte graph validation remains
            // in the engine, including missing/partial journal and marker recovery.
            let path = crate::files::resolve_runtime_path(root, &relative)?;
            let proof = if let Some(member) = member {
                work_operations::derivation::transaction::restore_journal_staging(member)
                    .map_err(|_| retained_error("journal_batch_identity", &relative))?
            } else {
                work_operations::derivation::transaction::restore_journal_staging(&manifest)
                    .map_err(|_| retained_error("journal_batch_identity", &relative))?
            };
            if path.exists() {
                let raw = LocalFiles.read_raw(&path)?;
                if raw != proof.prepared_journal
                    && raw != proof.published_journal
                    && !(proof.prepared_journal.starts_with(&raw)
                        || proof.published_journal.starts_with(&raw))
                {
                    return Err(retained_error(
                        "journal_history_evidence_changed",
                        &relative,
                    ));
                }
            }
            continue;
        }
        let include = publication::retained_journal_history_includes(
            input.execution,
            &relative,
            Some(input.relative),
        )
        .map_err(|_| retained_error("journal_history_scope_invalid", &relative))?;
        let evidence = if include {
            read_retained_journal(root, input.execution, &relative)?
        } else {
            read_retained_journal_for_recovery(root, input.execution, &relative)?
        };
        if !include {
            let kind = work_operations::specification::transaction::verified_retained_journal_kind(
                input.execution,
                &relative,
                &evidence.contract,
            )
            .map_err(|_| retained_error("journal_history_scope_invalid", &relative))?;
            let identity = match kind {
                JournalKind::SpecificationUpdate(value)
                | JournalKind::SpecificationMigration(value)
                | JournalKind::SpecificationMigrationReconcile(value)
                | JournalKind::InstructionMigration(value)
                | JournalKind::SourceRefresh(value)
                | JournalKind::SpecificationMigrationItem {
                    approved: value, ..
                } => value,
            };
            if manifest.business_identity["layout_identity"] != identity {
                return Err(retained_error(
                    "journal_history_approval_mismatch",
                    &relative,
                ));
            }
            if batch.is_some() {
                if matches!(kind, JournalKind::SpecificationMigrationItem { .. })
                    && member.is_none()
                {
                    return Err(retained_error("journal_batch_identity", &relative));
                }
                if evidence.completion
                    != work_operations::specification::transaction::CompletionState::Completed
                {
                    return Err(retained_error(
                        "runtime_cleanup_identity_missing",
                        &relative,
                    ));
                }
            }
            if relative == input.relative {
                work_operations::specification::transaction::verify_retained_journal_progress(
                    input.prepared_journal,
                    &evidence.raw,
                )
                .map_err(|_| retained_error("journal_history_evidence_changed", &relative))?;
            } else if let Some(member) = member {
                work_operations::specification::transaction::verify_retained_journal_progress(
                    &member.business_identity["original_journal"],
                    &evidence.raw,
                )
                .map_err(|_| retained_error("journal_history_evidence_changed", &relative))?;
            }
            continue;
        }
        if evidence.completion
            != work_operations::specification::transaction::CompletionState::Completed
        {
            return Err(retained_error("spec_update_pending", &relative));
        }
        let marker = publication::retained_journal_marker(&relative)
            .map_err(|_| retained_error("journal_layout_identity", &relative))?;
        history.insert(relative, evidence.raw);
        history.insert(
            marker,
            evidence.marker_raw.expect("verified completed marker"),
        );
    }
    let expected = input.prepared_journal["metadata"]["history_sha256"]
        .as_object()
        .ok_or_else(|| retained_error("journal_history_scope_invalid", input.relative))?;
    let mut targets = manifest.targets[..manifest.targets.len() - 2]
        .iter()
        .collect::<Vec<_>>();
    if let Some(batch) = batch {
        for member in batch.manifests.values() {
            targets.extend(member.targets[..member.targets.len() - 2].iter());
        }
    }
    for target in targets {
        if history.contains_key(&target.path) {
            if let Some(before) = &target.before {
                history.insert(target.path.clone(), before.bytes.clone());
            } else {
                history.remove(&target.path);
            }
        }
    }
    for relative in expected.keys() {
        if !history.contains_key(relative) {
            let raw = if let Some(before) = manifest
                .targets
                .iter()
                .find(|target| target.path == *relative)
                .and_then(|target| target.before.as_ref())
            {
                before.bytes.clone()
            } else {
                LocalFiles.read_raw(&crate::files::resolve_runtime_path(root, relative)?)?
            };
            history.insert(relative.clone(), raw);
        }
    }
    let actual = history
        .iter()
        .map(|(path, raw)| (path.clone(), json!(fingerprint::history(raw))))
        .collect::<serde_json::Map<_, _>>();
    if &actual != expected {
        return Err(retained_error(
            "spec_update_history_changed",
            input.relative,
        ));
    }
    Ok(history)
}

fn retained_error(reason: &str, relative: &str) -> WorkError {
    error(
        ExitCode::ArtifactIntegrity,
        reason,
        "Retained journal evidence requires complete canonical layout and approval bindings.",
        json!({"path":relative,"recovery_required":true}),
    )
}

pub(crate) fn retained_journal_original_for_recovery(
    context: &work_feature::ports::RequirementWriterContext,
    execution: &str,
    relative: &str,
) -> Result<Value, WorkError> {
    let inventory = crate::execution::storage::LocalExecutionStorage {
        project_root: context.canonical_project_root.clone(),
    }
    .retained_requirement_inventory(context, execution)?;
    if let Some(item) = inventory.first() {
        if inventory.len() != 1 || item.manifest.business_identity["journal_path"] != relative {
            return Err(retained_error(
                "runtime_execution_transaction_present",
                &item.transaction_dir,
            ));
        }
        return Ok(item.manifest.business_identity["original_journal"].clone());
    }
    let mut original =
        read_retained_journal_for_recovery(&context.canonical_project_root, execution, relative)?
            .contract;
    original["state"] = json!("prepared");
    original["published_count"] = json!(0);
    Ok(original)
}

fn retained_children(root: &Path, relative: &str) -> Result<Vec<(String, bool)>, WorkError> {
    let path = crate::files::resolve_runtime_path(root, relative)?;
    let mut result = Vec::new();
    for entry in
        fs::read_dir(&path).map_err(|_| retained_error("journal_layout_unreadable", relative))?
    {
        let entry = entry.map_err(|_| retained_error("journal_layout_unreadable", relative))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| retained_error("journal_layout_foreign", relative))?;
        let child = format!("{relative}/{name}");
        let physical = crate::files::resolve_runtime_path(root, &child)?;
        let metadata = fs::symlink_metadata(&physical)
            .map_err(|_| retained_error("journal_layout_unreadable", &child))?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(retained_error("journal_layout_foreign", &child));
        }
        result.push((name, metadata.is_dir()));
    }
    result.sort();
    Ok(result)
}

pub fn require_no_legacy_journal_layout(root: &Path, execution: &str) -> Result<(), WorkError> {
    let path = crate::files::resolve_runtime_path(root, execution)?;
    if fs::symlink_metadata(&path).is_err_and(|issue| issue.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    let legacy = retained_children(root, execution)?
        .into_iter()
        .filter_map(|(name, _)| {
            [
                ".work-spec-update-",
                ".work-spec-migration-",
                ".work-instruction-migration-",
                ".work-source-refresh-",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            .then(|| format!("{execution}/{name}"))
        })
        .collect::<Vec<_>>();
    if !legacy.is_empty() {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "legacy_journal_layout_present",
            "Legacy journal evidence requires reviewed offline migration before current publication.",
            json!({"legacy_journals":legacy}),
        ));
    }
    Ok(())
}

fn retained_leaf_paths(
    root: &Path,
    execution: &str,
    directory: &str,
    paths: &mut Vec<String>,
) -> Result<(), WorkError> {
    let journal = format!("{directory}/journal.json");
    work_operations::derivation::publication::parse_retained_journal_path(execution, &journal)
        .map_err(|_| retained_error("journal_layout_identity", &journal))?;
    for (name, is_directory) in retained_children(root, directory)? {
        if is_directory || !matches!(name.as_str(), "journal.json" | "committed.sha256") {
            return Err(retained_error(
                "journal_layout_foreign",
                &format!("{directory}/{name}"),
            ));
        }
    }
    // An allocated empty/marker-only leaf is evidence requiring recovery, never an absent journal.
    paths.push(journal);
    Ok(())
}

pub fn retained_journal_paths(root: &Path, execution: &str) -> Result<Vec<String>, WorkError> {
    require_no_legacy_journal_layout(root, execution)?;
    let journals = format!("{execution}/journals");
    let physical = crate::files::resolve_runtime_path(root, &journals)?;
    if fs::symlink_metadata(&physical)
        .is_err_and(|issue| issue.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(vec![]);
    }
    let mut paths = Vec::new();
    for (kind, is_directory) in retained_children(root, &journals)? {
        if !is_directory
            || !matches!(
                kind.as_str(),
                "specification-update"
                    | "specification-migration"
                    | "instruction-migration"
                    | "source-refresh"
            )
        {
            return Err(retained_error(
                "journal_layout_foreign",
                &format!("{journals}/{kind}"),
            ));
        }
        let kind_path = format!("{journals}/{kind}");
        for (identity, is_directory) in retained_children(root, &kind_path)? {
            let family = format!("{kind_path}/{identity}");
            let main = format!("{family}/journal.json");
            if !is_directory {
                return Err(retained_error("journal_layout_foreign", &family));
            }
            work_operations::derivation::publication::parse_retained_journal_path(execution, &main)
                .map_err(|_| retained_error("journal_layout_identity", &main))?;
            if kind != "specification-migration" {
                retained_leaf_paths(root, execution, &family, &mut paths)?;
                continue;
            }
            let before = paths.len();
            let mut has_main = false;
            for (name, is_directory) in retained_children(root, &family)? {
                match (name.as_str(), is_directory) {
                    ("journal.json" | "committed.sha256", false) => has_main = true,
                    ("reconcile", true) => retained_leaf_paths(
                        root,
                        execution,
                        &format!("{family}/reconcile"),
                        &mut paths,
                    )?,
                    ("items", true) => {
                        for (position, is_directory) in
                            retained_children(root, &format!("{family}/items"))?
                        {
                            let leaf = format!("{family}/items/{position}");
                            if !is_directory {
                                return Err(retained_error("journal_layout_foreign", &leaf));
                            }
                            retained_leaf_paths(root, execution, &leaf, &mut paths)?;
                        }
                    }
                    _ => {
                        return Err(retained_error(
                            "journal_layout_foreign",
                            &format!("{family}/{name}"),
                        ));
                    }
                }
            }
            if has_main || paths.len() == before {
                paths.push(main);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn read_retained_journal(
    root: &Path,
    execution: &str,
    relative: &str,
) -> Result<RetainedJournalEvidence, WorkError> {
    read_retained_journal_variant(root, execution, relative, false)
}

pub fn read_retained_journal_for_recovery(
    root: &Path,
    execution: &str,
    relative: &str,
) -> Result<RetainedJournalEvidence, WorkError> {
    read_retained_journal_variant(root, execution, relative, true)
}

fn read_retained_journal_variant(
    root: &Path,
    execution: &str,
    relative: &str,
    recover: bool,
) -> Result<RetainedJournalEvidence, WorkError> {
    let address =
        work_operations::derivation::publication::parse_retained_journal_path(execution, relative)
            .map_err(|_| retained_error("journal_layout_identity", relative))?;
    let path = crate::files::resolve_runtime_path(root, relative)?;
    if fs::symlink_metadata(&path).is_err_and(|issue| issue.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(error(
            ExitCode::LockConflict,
            "spec_update_pending",
            "An allocated journal leaf requires separately reviewed recovery evidence.",
            json!({"recovery_required":true,"record":relative}),
        ));
    }
    let (contract, raw) = read_journal(&path)?;
    work_operations::specification::transaction::verify_retained_journal_layout(
        execution, relative, &contract,
    )
    .map_err(|issue| {
        error(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let marker = crate::files::resolve_runtime_path(root, &address.marker)?;
    let marker_raw = match fs::symlink_metadata(&marker) {
        Ok(_) => Some(LocalFiles.read_raw(&marker)?),
        Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(retained_error("journal_marker_unreadable", &address.marker)),
    };
    let completion =
        work_operations::specification::transaction::completion_state(&raw, marker_raw.as_deref());
    if marker_raw.is_some()
        && (contract["state"] != "published"
            || completion
                != work_operations::specification::transaction::CompletionState::Completed)
        && !(recover
            && contract["state"] == "published"
            && marker_raw
                .as_ref()
                .is_some_and(|marker| completion_marker(&raw).starts_with(marker)))
    {
        return Err(retained_error("journal_commit_evidence", &address.marker));
    }
    Ok(RetainedJournalEvidence {
        relative: relative.to_owned(),
        contract,
        raw,
        marker_raw,
        completion,
    })
}

/// Exclusions carry the complete reviewed journal, never a caller-selected path alone.
pub struct RetainedJournalHistoryScope<'a> {
    pub relative: &'a str,
    pub prepared: &'a Value,
}

pub fn execution_history_bytes_for_journal(
    root: &Path,
    execution: &str,
    relative: &str,
    prepared: &Value,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    {
        execution_history_bytes_with_journals(
            root,
            execution,
            Some(RetainedJournalHistoryScope { relative, prepared }),
        )
    }
}

pub fn execution_history_bytes_with_journals(
    root: &Path,
    execution: &str,
    current: Option<RetainedJournalHistoryScope<'_>>,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    retained_history_bytes(root, execution, current, true)
}

fn retained_history_bytes(
    root: &Path,
    execution: &str,
    current: Option<RetainedJournalHistoryScope<'_>>,
    include_receipts: bool,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    use work_operations::derivation::publication::{self, JournalKind};
    use work_operations::specification::transaction::{self, CompletionState};
    let identity = |kind: JournalKind<'_>| match kind {
        JournalKind::SpecificationUpdate(value)
        | JournalKind::SpecificationMigration(value)
        | JournalKind::SpecificationMigrationReconcile(value)
        | JournalKind::InstructionMigration(value)
        | JournalKind::SourceRefresh(value)
        | JournalKind::SpecificationMigrationItem {
            approved: value, ..
        } => value.to_owned(),
    };
    let approved = current
        .as_ref()
        .map(|scope| {
            if scope.prepared["state"] != "prepared" || scope.prepared["published_count"] != 0 {
                return Err(retained_error(
                    "journal_history_scope_invalid",
                    scope.relative,
                ));
            }
            transaction::verified_retained_journal_kind(execution, scope.relative, scope.prepared)
                .map(identity)
                .map_err(|_| retained_error("journal_history_scope_invalid", scope.relative))
        })
        .transpose()?;
    let mut result = if include_receipts {
        execution_history_bytes_with_receipts(root, execution)?
    } else {
        BTreeMap::new()
    };
    for relative in retained_journal_paths(root, execution)? {
        let include = publication::retained_journal_history_includes(
            execution,
            &relative,
            current.as_ref().map(|scope| scope.relative),
        )
        .map_err(|_| retained_error("journal_history_scope_invalid", &relative))?;
        let evidence = if include {
            read_retained_journal(root, execution, &relative)?
        } else {
            read_retained_journal_for_recovery(root, execution, &relative)?
        };
        if !include {
            let kind = transaction::verified_retained_journal_kind(
                execution,
                &relative,
                &evidence.contract,
            )
            .map_err(|_| retained_error("journal_history_scope_invalid", &relative))?;
            if Some(identity(kind)) != approved {
                return Err(retained_error(
                    "journal_history_approval_mismatch",
                    &relative,
                ));
            }
            if let Some(scope) = current.as_ref().filter(|scope| scope.relative == relative) {
                transaction::verify_retained_journal_progress(scope.prepared, &evidence.raw)
                    .map_err(|_| retained_error("journal_history_evidence_changed", &relative))?;
            }
            continue;
        }
        if evidence.completion != CompletionState::Completed {
            return Err(error(
                ExitCode::LockConflict,
                "spec_update_pending",
                "An incomplete retained journal requires reviewed recovery.",
                json!({"recovery_required":true,"record":relative}),
            ));
        }
        let marker = publication::retained_journal_marker(&relative)
            .map_err(|_| retained_error("journal_layout_identity", &relative))?;
        result.insert(relative, evidence.raw);
        result.insert(
            marker,
            evidence.marker_raw.expect("verified completed marker"),
        );
    }
    Ok(result)
}

fn snapshot(row: &Value, side: &str) -> Result<Option<Vec<u8>>, WorkError> {
    row.get(side)
        .map(|value| decode_snapshot(value).map_err(|_| contract_error()))
        .transpose()
}

pub fn publish_journal(
    root: &Path,
    journal_relative: &str,
    marker_relative: &str,
) -> Result<Value, WorkError> {
    let journal = storage_path(root, journal_relative)?;
    let marker = storage_path(root, marker_relative)?;
    let (mut contract, mut raw) = read_journal(&journal)?;
    if marker.exists() {
        let marker_raw = LocalFiles.read_raw(&marker)?;
        if contract["state"] == "published" && marker_raw == completion_marker(&raw) {
            return Ok(
                json!({"status":"already_published","published_count":contract["published_count"]}),
            );
        }
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "spec_transaction_marker_conflict",
            "The completion marker does not match the final journal.",
            json!({}),
        ));
    }
    let files = contract["files"]
        .as_array()
        .ok_or_else(contract_error)?
        .clone();
    let start = contract["published_count"]
        .as_u64()
        .ok_or_else(contract_error)? as usize;
    for (index, row) in files.iter().enumerate().skip(start) {
        let relative = row["path"].as_str().ok_or_else(contract_error)?;
        let target = storage_path(root, relative)?;
        let before = snapshot(row, "before")?;
        let after = snapshot(row, "after")?;
        let temporary = PathBuf::from(format!("{}.{}.tmp", journal.display(), index));
        publish_recoverable_sequence(
            &LocalFiles,
            &[Publication {
                target: &target,
                before: before.as_deref(),
                after: after.as_deref(),
                temporary: &temporary,
            }],
            |_| Ok(()),
        )?;
        contract["published_count"] = json!(index + 1);
        contract["state"] = json!(if index + 1 == files.len() {
            "published"
        } else {
            "publishing"
        });
        raw = render_transaction(&contract).map_err(|_| contract_error())?;
        replace_journal(&journal, &raw)?;
    }
    LocalFiles.create_new(&marker, &completion_marker(&raw))?;
    Ok(json!({"status":"published","published_count":files.len()}))
}

/// Readiness inspection grants no publication authority and never allocates missing paths.
pub fn require_retained_journal_readiness(
    root: &Path,
    execution: &str,
    ignored_record: Option<&str>,
) -> Result<(), WorkError> {
    let mut current = ignored_record
        .map(|relative| {
            let path = crate::files::resolve_runtime_path(root, relative)?;
            match fs::symlink_metadata(&path) {
                Ok(_) => read_retained_journal_for_recovery(root, execution, relative)
                    .map(|evidence| Some(evidence.contract)),
                Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err(retained_error("journal_layout_unreadable", relative)),
            }
        })
        .transpose()?
        .flatten();
    if let Some(journal) = current.as_mut() {
        journal["state"] = json!("prepared");
        journal["published_count"] = json!(0);
    }
    retained_history_bytes(
        root,
        execution,
        ignored_record
            .zip(current.as_ref())
            .map(|(relative, prepared)| RetainedJournalHistoryScope { relative, prepared }),
        false,
    )?;
    Ok(())
}

pub fn require_no_spec_update(
    root: &Path,
    execution_dir: &str,
    ignored_record: Option<&str>,
) -> Result<(), WorkError> {
    require_retained_journal_readiness(root, execution_dir, ignored_record)
}

pub fn execution_history_fingerprints(
    root: &Path,
    execution: &str,
) -> Result<BTreeMap<String, String>, WorkError> {
    Ok(execution_history_bytes(root, execution)?
        .iter()
        .map(|(path, raw)| (path.clone(), fingerprint::history(raw)))
        .collect())
}

pub fn execution_history_bytes(
    root: &Path,
    execution: &str,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    execution_history_bytes_with_journals(root, execution, None)
}

/// Candidate history scan keeps exact receipt bytes while refusing aliases and legacy evidence.
pub fn execution_history_bytes_with_receipts(
    root: &Path,
    execution: &str,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    let directory = crate::files::resolve_runtime_path(root, execution)?;
    if !directory.exists() {
        return Ok(BTreeMap::new());
    }
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(&directory).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "file_read_failed",
            "Execution history could not be read.",
            json!({}),
        )
    })? {
        let entry = entry.map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "Execution history could not be read.",
                json!({}),
            )
        })?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            error(
                ExitCode::ArtifactIntegrity,
                "spec_update_history_layout",
                "Execution history names must be portable.",
                json!({}),
            )
        })?;
        if name.starts_with(TASK_ID_PREFIX) {
            let relative = format!("{execution}/{name}");
            let path = crate::files::resolve_runtime_path(root, &relative)?;
            if !path.is_dir() {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "spec_update_history_layout",
                    "Execution TASK entries must be directories.",
                    json!({"path":relative}),
                ));
            }
            collect_receipt_history(root, execution, &relative, &mut result)?;
        }
    }
    Ok(result)
}

fn collect_receipt_history(
    root: &Path,
    execution: &str,
    relative: &str,
    result: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), WorkError> {
    let directory = crate::files::resolve_runtime_path(root, relative)?;
    let receipt_root = relative.ends_with("/receipts");
    let instance_root = relative
        .strip_prefix(&format!("{execution}/"))
        .is_some_and(|tail| {
            let parts: Vec<_> = tail.split('/').collect();
            parts.len() == 4 && parts[2] == "receipts"
        });
    let mut count = 0;
    for entry in fs::read_dir(&directory).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "file_read_failed",
            "Execution history could not be read.",
            json!({"path":relative}),
        )
    })? {
        let entry = entry.map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "Execution history could not be read.",
                json!({"path":relative}),
            )
        })?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            error(
                ExitCode::ArtifactIntegrity,
                "spec_update_history_layout",
                "History names must be portable.",
                json!({"path":relative}),
            )
        })?;
        let child = format!("{relative}/{name}");
        let path = crate::files::resolve_runtime_path(root, &child)?;
        if name.starts_with(".work-command-")
            && (name.ends_with(".started.json") || name.ends_with(".finished.json"))
        {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "legacy_command_receipt_present",
                "Legacy command evidence requires reviewed offline handling.",
                json!({"path":child}),
            ));
        }
        if receipt_root {
            work_operations::derivation::publication::command_receipt_instance(name).map_err(
                |_| {
                    error(
                        ExitCode::ArtifactIntegrity,
                        "command_receipt_history_layout",
                        "Receipt instance names must be canonical.",
                        json!({"path":child}),
                    )
                },
            )?;
            if !path.is_dir() {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "command_receipt_history_layout",
                    "Receipt instances must be directories.",
                    json!({"path":child}),
                ));
            }
        }
        if instance_root && (!matches!(name, "started.json" | "finished.json") || !path.is_file()) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "command_receipt_history_layout",
                "Receipt inventories contain only regular started/finished evidence.",
                json!({"path":child}),
            ));
        }
        if path.is_dir() {
            collect_receipt_history(root, execution, &child, result)?;
        } else if path.is_file() {
            let raw = LocalFiles.read_raw(&path)?;
            if instance_root {
                verify_receipt_history_identity(execution, relative, name, &raw)?;
            }
            result.insert(child, raw);
        } else {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "command_receipt_history_layout",
                "History entries must be real files or directories.",
                json!({"path":child}),
            ));
        }
        count += 1;
    }
    if instance_root && count == 0 {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_receipt_history_incomplete",
            "An empty allocated receipt instance requires diagnosis.",
            json!({"path":relative}),
        ));
    }
    Ok(())
}

fn verify_receipt_history_identity(
    execution: &str,
    directory: &str,
    name: &str,
    raw: &[u8],
) -> Result<(), WorkError> {
    let Ok(value) = work_operations::canonical::parse_json_contract(raw) else {
        // Partial bytes are execution evidence even when the outcome is unknown.
        return Ok(());
    };
    let parts: Vec<_> = directory
        .strip_prefix(&format!("{execution}/"))
        .ok_or_else(|| {
            error(
                ExitCode::ArtifactIntegrity,
                "command_receipt_history_identity",
                "Receipt history must stay within its execution.",
                json!({}),
            )
        })?
        .split('/')
        .collect();
    let record = work_operations::derivation::publication::command_receipt_instance(parts[3])
        .map_err(|_| {
            error(
                ExitCode::ArtifactIntegrity,
                "command_receipt_history_identity",
                "Receipt identity is invalid.",
                json!({}),
            )
        })?;
    let mismatch = if name == "started.json" && value["schema"] == "work-command-started" {
        let preview = &value["preview"];
        preview["receipt_dir"] != directory
            || preview["task_id"] != parts[0]
            || preview["attempt_id"] != parts[1]
            || preview["record_id"] != record
    } else if name == "finished.json" && value["schema"] == "work-command-result" {
        value["record_id"] != record
            || value
                .get("receipt_dir")
                .is_some_and(|path| !path.is_null() && *path != directory)
    } else {
        false
    };
    if mismatch {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "command_receipt_history_identity",
            "Receipt evidence belongs to a different command instance.",
            json!({"path":directory}),
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn synthetic_retained_journal(
    root: &Path,
    execution: &str,
    position: usize,
) -> (String, Value) {
    use work_operations::derivation::{
        publication::{self, JournalKind},
        transaction::*,
    };
    let approved = "ab".repeat(32);
    let request = match position {
        0 => {
            json!({"schema":"work-spec-update-request","task_index":{"requirement_id":"example"}})
        }
        1 => json!({"migration":{},"preview_fingerprint":approved}),
        2 => {
            json!({"request_sha256":approved,"analysis_fingerprint":"c".repeat(64),"item_id":"ITEM-003"})
        }
        3 => json!({"request_sha256":approved,"phase":"reconciliation"}),
        4 => {
            json!({"kind":"instruction_migration","requirement_id":"example","preview_fingerprint":approved})
        }
        5 => {
            json!({"kind":"source_refresh","requirement_id":"example","preview_fingerprint":approved})
        }
        6 => {
            json!({"reconciliation_fingerprint":approved,"attempt_path":format!("{execution}/TASK-001/ATTEMPT-001/attempt.json")})
        }
        _ => unreachable!(),
    };
    let transaction_kind = match position {
        1 | 2 => TransactionKind::Migration,
        3 | 6 => TransactionKind::Reconciliation,
        4 => TransactionKind::InstructionMigration,
        5 => TransactionKind::SourceRefresh,
        _ => TransactionKind::Update,
    };
    let mut journal = TransactionDeriver::derive(TransactionInput {
        kind: transaction_kind,
        order: PublicationOrder::Flat,
        request,
        artifacts: if position == 6 {
            json!({})
        } else {
            json!({"execution":execution})
        },
        affected_task_ids: vec![],
        history: BTreeMap::new(),
        source: BTreeMap::new(),
        candidate: BTreeMap::from([(
            format!("target-{position}.json"),
            format!("data {position}").into_bytes(),
        )]),
    })
    .unwrap()
    .journal;
    let kind = match position {
        0 => JournalKind::SpecificationUpdate(journal["transaction_id"].as_str().unwrap()),
        1 | 6 => JournalKind::SpecificationMigration(&approved),
        2 => JournalKind::SpecificationMigrationItem {
            approved: &approved,
            position: 2,
        },
        3 => JournalKind::SpecificationMigrationReconcile(&approved),
        4 => JournalKind::InstructionMigration(&approved),
        5 => JournalKind::SourceRefresh(&approved),
        _ => unreachable!(),
    };
    let relative = publication::retained_journal_path(execution, kind).unwrap();
    let path = root.join(&relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    journal["state"] = json!("published");
    journal["published_count"] = json!(1);
    let raw = render_transaction(&journal).unwrap();
    fs::write(&path, &raw).unwrap();
    fs::write(
        root.join(publication::retained_journal_marker(&relative).unwrap()),
        completion_marker(&raw),
    )
    .unwrap();
    (relative, journal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retained_case(root: &Path, execution: &str, position: usize) -> (String, Value) {
        synthetic_retained_journal(root, execution, position)
    }

    #[test]
    fn retained_history_preserves_bytes_and_requires_full_reviewed_family_identity() {
        use work_operations::derivation::transaction::*;
        let root = policy_test_root("retained-history");
        let execution = "custom execution/例";
        let mut expected = BTreeMap::new();
        let mut migration = None;
        let mut item = None;
        for position in 0..6 {
            let (relative, journal) = retained_case(&root, execution, position);
            let marker =
                work_operations::derivation::publication::retained_journal_marker(&relative)
                    .unwrap();
            expected.insert(relative.clone(), fs::read(root.join(&relative)).unwrap());
            expected.insert(marker.clone(), fs::read(root.join(marker)).unwrap());
            if position == 1 {
                migration = Some((relative.clone(), journal.clone()));
            }
            if position == 2 {
                item = Some((relative, journal));
            }
        }
        assert_eq!(
            execution_history_bytes_with_journals(&root, execution, None).unwrap(),
            expected
        );
        let (relative, mut prepared) = migration.unwrap();
        prepared["state"] = json!("prepared");
        prepared["published_count"] = json!(0);
        let scope = || {
            Some(RetainedJournalHistoryScope {
                relative: &relative,
                prepared: &prepared,
            })
        };
        let history = execution_history_bytes_with_journals(&root, execution, scope()).unwrap();
        let excluded = format!("{execution}/journals/specification-migration/");
        expected.retain(|path, _| !path.starts_with(&excluded));
        assert_eq!(history, expected);
        let (item_path, original_item) = item.unwrap();
        let mut request = original_item["metadata"]["request"].clone();
        request["request_sha256"] = json!(format!("{}{}", "ab".repeat(6), "cd".repeat(26)));
        let foreign = TransactionDeriver::derive(TransactionInput {
            kind: TransactionKind::Migration,
            order: PublicationOrder::Flat,
            request,
            artifacts: json!({"execution":execution}),
            affected_task_ids: vec![],
            history: BTreeMap::new(),
            source: BTreeMap::new(),
            candidate: BTreeMap::from([("target-2.json".into(), b"data 2".to_vec())]),
        })
        .unwrap()
        .journal;
        let marker =
            work_operations::derivation::publication::retained_journal_marker(&item_path).unwrap();
        fs::remove_file(root.join(&marker)).unwrap();
        fs::write(root.join(&item_path), render_transaction(&foreign).unwrap()).unwrap();
        assert_eq!(
            execution_history_bytes_with_journals(&root, execution, scope())
                .unwrap_err()
                .reason_code,
            "journal_history_approval_mismatch"
        );
        fs::write(
            root.join(&item_path),
            render_transaction(&original_item).unwrap(),
        )
        .unwrap();
        assert_eq!(
            execution_history_bytes_with_journals(&root, execution, None)
                .unwrap_err()
                .reason_code,
            "spec_update_pending"
        );
        assert!(execution_history_bytes_with_journals(&root, execution, scope()).is_ok());
        fs::write(
            root.join(format!("{execution}/journals/unknown.tmp")),
            b"foreign",
        )
        .unwrap();
        assert_eq!(
            execution_history_bytes_with_journals(&root, execution, scope())
                .unwrap_err()
                .reason_code,
            "journal_layout_foreign"
        );
        assert!(!root.join("outputs/work/runtime").exists());
    }

    #[test]
    fn retained_verification_requires_exact_scope_marker_and_installed_bytes() {
        for position in 0..4 {
            let root = policy_test_root("retained-verification");
            let execution = "custom execution/例";
            let (relative, journal) = retained_case(&root, execution, position);
            let target = format!("target-{position}.json");
            fs::write(root.join(&target), format!("data {position}")).unwrap();
            let verify = || {
                crate::specification::migration_verification::verify_published_retained_transaction(
                    &root,
                    execution,
                    &relative,
                    Some(&journal["metadata"]["request"]),
                )
            };
            assert_eq!(
                verify().unwrap()["approval_sha256"],
                journal["approval_sha256"]
            );
            assert!(crate::specification::migration_verification::verify_published_retained_transaction(
                &root, "foreign execution", &relative, None).is_err());
            let original = fs::read(root.join(&relative)).unwrap();
            fs::write(root.join(&target), b"changed after publication").unwrap();
            assert_eq!(
                verify().unwrap_err().reason_code,
                "migration_verify_installed_mismatch"
            );
            assert_eq!(fs::read(root.join(&relative)).unwrap(), original);
            fs::write(root.join(&target), format!("data {position}")).unwrap();
            let marker =
                work_operations::derivation::publication::retained_journal_marker(&relative)
                    .unwrap();
            fs::write(root.join(&marker), b"wrong marker").unwrap();
            assert_eq!(
                verify().unwrap_err().reason_code,
                "migration_verify_marker_mismatch"
            );
            assert!(read_retained_journal_for_recovery(&root, execution, &relative).is_err());
            let prefix = &completion_marker(&original)[..9];
            fs::write(root.join(&marker), prefix).unwrap();
            assert_eq!(
                verify().unwrap_err().reason_code,
                "migration_verify_marker_mismatch"
            );
            let recovering =
                read_retained_journal_for_recovery(&root, execution, &relative).unwrap();
            assert_eq!(
                recovering.completion,
                work_operations::specification::transaction::CompletionState::Corrupt
            );
            let mut prepared = journal.clone();
            prepared["state"] = json!("prepared");
            prepared["published_count"] = json!(0);
            assert!(
                execution_history_bytes_with_journals(
                    &root,
                    execution,
                    Some(RetainedJournalHistoryScope {
                        relative: &relative,
                        prepared: &prepared
                    })
                )
                .is_ok()
            );
            assert!(execution_history_bytes_with_journals(&root, execution, None).is_err());
            assert_eq!(fs::read(root.join(marker)).unwrap(), prefix);
            assert_eq!(fs::read(root.join(&relative)).unwrap(), original);
        }
    }

    #[test]
    fn retained_readiness_distinguishes_absent_from_allocated_ignored_leaf_without_writes() {
        let root = policy_test_root("retained-readiness");
        let execution = "custom execution/例";
        let relative =
            format!("{execution}/journals/specification-migration/AAAAAAAAAAAA/journal.json");
        require_retained_journal_readiness(&root, execution, Some(&relative)).unwrap();
        assert!(!root.join(execution).exists());
        fs::create_dir_all(root.join(&relative).parent().unwrap()).unwrap();
        assert_eq!(
            require_retained_journal_readiness(&root, execution, Some(&relative))
                .unwrap_err()
                .reason_code,
            "spec_update_pending"
        );
        assert!(!root.join(&relative).exists());
        assert!(!root.join("outputs/work/runtime").exists());
    }

    #[test]
    fn retained_reader_collects_all_kinds_nested_and_ledger_only_without_writes() {
        let root = policy_test_root("retained");
        let execution = "custom execution/例";
        let mut expected = Vec::new();
        for position in 0..6 {
            let (relative, value) = retained_case(&root, execution, position);
            let original = fs::read(root.join(&relative)).unwrap();
            let evidence = read_retained_journal(&root, execution, &relative).unwrap();
            assert_eq!(evidence.raw, original);
            assert_eq!(evidence.contract, value);
            assert_eq!(
                evidence.completion,
                work_operations::specification::transaction::CompletionState::Completed
            );
            assert_eq!(fs::read(root.join(&relative)).unwrap(), original);
            expected.push(relative);
        }
        expected.sort();
        assert_eq!(retained_journal_paths(&root, execution).unwrap(), expected);
        assert!(!root.join("outputs/work/runtime").exists());
        let ledger = policy_test_root("retained-ledger");
        let (relative, _) = retained_case(&ledger, execution, 6);
        assert_eq!(
            retained_journal_paths(&ledger, execution).unwrap(),
            vec![relative.clone()]
        );
        assert_eq!(
            read_retained_journal(&ledger, execution, &relative)
                .unwrap()
                .completion,
            work_operations::specification::transaction::CompletionState::Completed
        );
    }

    #[test]
    fn retained_reader_preserves_unknown_partial_marker_and_empty_allocated_evidence() {
        let execution = "custom execution/例";
        for damage in [
            "missing-marker",
            "partial-marker",
            "empty-marker",
            "missing-journal",
            "empty-leaf",
            "foreign",
            "legacy",
        ] {
            let root = policy_test_root(damage);
            let (relative, _) = retained_case(&root, execution, 2);
            let marker =
                work_operations::derivation::publication::retained_journal_marker(&relative)
                    .unwrap();
            match damage {
                "missing-marker" => fs::remove_file(root.join(&marker)).unwrap(),
                "partial-marker" => fs::write(root.join(&marker), b"partial marker").unwrap(),
                "empty-marker" => fs::write(root.join(&marker), b"").unwrap(),
                "missing-journal" => fs::remove_file(root.join(&relative)).unwrap(),
                "empty-leaf" => {
                    fs::remove_file(root.join(&relative)).unwrap();
                    fs::remove_file(root.join(&marker)).unwrap();
                }
                "foreign" => fs::write(
                    root.join(&relative).parent().unwrap().join("foreign.json"),
                    b"foreign raw bytes",
                )
                .unwrap(),
                "legacy" => fs::write(
                    root.join(execution)
                        .join(".work-spec-migration-ABABABABABAB.json.done"),
                    b"old raw bytes",
                )
                .unwrap(),
                _ => unreachable!(),
            }
            let raw = fs::read(root.join(&relative)).ok();
            let marker_raw = fs::read(root.join(&marker)).ok();
            if matches!(damage, "foreign" | "legacy") {
                assert!(retained_journal_paths(&root, execution).is_err());
            } else {
                assert_eq!(
                    retained_journal_paths(&root, execution).unwrap(),
                    vec![relative.clone()]
                );
                let evidence = read_retained_journal(&root, execution, &relative);
                if damage == "missing-marker" {
                    assert_eq!(
                        evidence.unwrap().completion,
                        work_operations::specification::transaction::CompletionState::Incomplete
                    );
                } else {
                    assert!(evidence.is_err());
                }
            }
            assert_eq!(fs::read(root.join(&relative)).ok(), raw);
            assert_eq!(fs::read(root.join(&marker)).ok(), marker_raw);
            assert!(!root.join("outputs/work/runtime").exists());
        }
    }

    #[test]
    fn retained_owned_adapter_rejects_released_foreign_and_mixed_owners_before_publication() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        for damage in ["released", "foreign", "mixed"] {
            let root = policy_test_root("retained-owned").canonicalize().unwrap();
            let execution = "custom execution/例";
            let (relative, mut journal) = retained_case(&root, execution, 0);
            let marker =
                work_operations::derivation::publication::retained_journal_marker(&relative)
                    .unwrap();
            fs::remove_file(root.join(&relative)).unwrap();
            fs::remove_file(root.join(&marker)).unwrap();
            fs::remove_dir(root.join(&relative).parent().unwrap()).unwrap();
            journal["state"] = json!("prepared");
            journal["published_count"] = json!(0);
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.clone(),
                requirement_id: "example".parse().unwrap(),
            };
            let input = RetainedJournalRuntimeInput {
                context: &context,
                execution,
                relative: &relative,
                prepared_journal: &journal,
                recover: false,
            };
            if damage == "mixed" {
                let (other, mut contract) = retained_case(&root, execution, 1);
                let other_marker =
                    work_operations::derivation::publication::retained_journal_marker(&other)
                        .unwrap();
                fs::remove_file(root.join(&other)).unwrap();
                fs::remove_file(root.join(other_marker)).unwrap();
                fs::remove_dir(root.join(&other).parent().unwrap()).unwrap();
                contract["state"] = json!("prepared");
                contract["published_count"] = json!(0);
                let manifest = prepare_retained_journal_manifest(&RetainedJournalRuntimeInput {
                    context: &context,
                    execution,
                    relative: &other,
                    prepared_journal: &contract,
                    recover: false,
                })
                .unwrap();
                let frozen =
                    work_operations::derivation::transaction::restore_journal_staging(&manifest)
                        .unwrap();
                crate::transaction_storage::prepare_runtime_transaction(
                    &LocalFiles,
                    &root,
                    &manifest,
                    &frozen.payloads,
                )
                .unwrap();
            }
            let guard = crate::writer_lock::LocalWriterLock
                .acquire_runtime(&context, work_model::runtime::LockClass::Execution)
                .unwrap();
            let mut owner = guard.owner().clone();
            if damage == "released" {
                guard.release().unwrap();
                assert!(
                    publish_retained_journal_with_owner(&input, &owner, || Ok(()), |_| Ok(()))
                        .is_err()
                );
            } else {
                if damage == "foreign" {
                    owner.instance_nonce = "d".repeat(64);
                    owner.owner_identity =
                        work_operations::derivation::identity::runtime_owner_identity(
                            &owner.canonical_root,
                            &context.requirement_id,
                            "execution",
                            &owner.instance_nonce,
                        )
                        .unwrap();
                }
                let error =
                    publish_retained_journal_with_owner(&input, &owner, || Ok(()), |_| Ok(()))
                        .unwrap_err();
                assert_eq!(
                    error.reason_code,
                    if damage == "mixed" {
                        "runtime_execution_transaction_present"
                    } else {
                        "journal_owner_identity"
                    }
                );
                guard.release().unwrap();
            }
            assert!(!root.join(&relative).exists());
            assert!(!root.join(marker).exists());
            assert!(!root.join("target-0.json").exists());
            assert!(
                !root
                    .join("outputs/work/runtime/locks/example/execution.lock")
                    .exists()
            );
        }
    }

    #[test]
    fn retained_owned_adapter_reuses_native_scope_and_recovers_its_exact_manifest() {
        let root = policy_test_root("retained-owned-recovery")
            .canonicalize()
            .unwrap();
        let execution = "custom execution/例";
        let (relative, mut journal) = retained_case(&root, execution, 0);
        let marker =
            work_operations::derivation::publication::retained_journal_marker(&relative).unwrap();
        fs::remove_file(root.join(&relative)).unwrap();
        fs::remove_file(root.join(&marker)).unwrap();
        fs::remove_dir(root.join(&relative).parent().unwrap()).unwrap();
        journal["state"] = json!("prepared");
        journal["published_count"] = json!(0);
        let context = work_feature::ports::RequirementWriterContext {
            canonical_project_root: root.clone(),
            requirement_id: "example".parse().unwrap(),
        };
        let input = |recover| RetainedJournalRuntimeInput {
            context: &context,
            execution,
            relative: &relative,
            prepared_journal: &journal,
            recover,
        };
        let fail = work_feature::ports::with_runtime_writer(
            &crate::writer_lock::LocalWriterLock,
            &context,
            work_model::runtime::LockClass::Execution,
            |owner| {
                publish_retained_journal_with_owner(
                    &input(false),
                    owner,
                    || Ok(()),
                    |stage| {
                        if stage == crate::transaction_storage::JournalRuntimeStage::Prepared {
                            Err(error(
                                ExitCode::IoFailure,
                                "injected_journal_fault",
                                "injected",
                                json!({}),
                            ))
                        } else {
                            Ok(())
                        }
                    },
                )
            },
        )
        .unwrap_err();
        assert_eq!(fail.reason_code, "injected_journal_fault");
        assert!(!root.join(&relative).exists());
        assert!(
            !root
                .join("outputs/work/runtime/locks/example/execution.lock")
                .exists()
        );
        let pending = crate::execution::storage::LocalExecutionStorage {
            project_root: root.clone(),
        }
        .retained_requirement_inventory(&context, execution)
        .unwrap();
        assert_eq!(pending.len(), 1);
        work_feature::ports::with_runtime_writer(
            &crate::writer_lock::LocalWriterLock,
            &context,
            work_model::runtime::LockClass::Execution,
            |owner| publish_retained_journal_with_owner(&input(true), owner, || Ok(()), |_| Ok(())),
        )
        .unwrap();
        assert!(!root.join(&pending[0].transaction_dir).exists());
        assert_eq!(fs::read(root.join("target-0.json")).unwrap(), b"data 0");
        assert_eq!(
            read_retained_journal(&root, execution, &relative)
                .unwrap()
                .completion,
            work_operations::specification::transaction::CompletionState::Completed
        );
        assert!(
            !root
                .join("outputs/work/runtime/locks/example/execution.lock")
                .exists()
        );
    }

    #[test]
    fn retained_adapter_publishes_under_owner_and_never_adds_recover_only_writers() {
        let execution = "custom execution/例";
        for position in [0, 4, 5] {
            let root = policy_test_root("retained-adapter").canonicalize().unwrap();
            let (relative, mut journal) = retained_case(&root, execution, position);
            let marker =
                work_operations::derivation::publication::retained_journal_marker(&relative)
                    .unwrap();
            fs::remove_file(root.join(&relative)).unwrap();
            fs::remove_file(root.join(&marker)).unwrap();
            fs::remove_dir(root.join(&relative).parent().unwrap()).unwrap();
            journal["state"] = json!("prepared");
            journal["published_count"] = json!(0);
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.clone(),
                requirement_id: "example".parse().unwrap(),
            };
            let result = publish_retained_journal_runtime(
                RetainedJournalRuntimeInput {
                    context: &context,
                    execution,
                    relative: &relative,
                    prepared_journal: &journal,
                    recover: false,
                },
                || {
                    assert!(
                        root.join("outputs/work/runtime/locks/example/execution.lock")
                            .is_file()
                    );
                    Ok(())
                },
                |_| Ok(()),
            );
            if position == 0 {
                result.unwrap();
                assert_eq!(
                    read_retained_journal(&root, execution, &relative)
                        .unwrap()
                        .completion,
                    work_operations::specification::transaction::CompletionState::Completed
                );
                assert_eq!(fs::read(root.join("target-0.json")).unwrap(), b"data 0");
            } else {
                assert_eq!(result.unwrap_err().reason_code, "journal_recover_only");
                assert!(!root.join(&relative).exists());
                assert!(!root.join("outputs/work/runtime").exists());
            }
            assert!(
                !root
                    .join("outputs/work/runtime/locks/example/execution.lock")
                    .exists()
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn retained_reader_refuses_links_and_copied_foreign_metadata_before_consumption() {
        use std::os::unix::fs::symlink;
        let execution = "custom execution/例";
        for link in ["journal", "marker", "directory"] {
            let root = policy_test_root("retained-links");
            let (relative, _) = retained_case(&root, execution, 0);
            let marker =
                work_operations::derivation::publication::retained_journal_marker(&relative)
                    .unwrap();
            match link {
                "journal" => {
                    fs::remove_file(root.join(&relative)).unwrap();
                    symlink(root.join(&marker), root.join(&relative)).unwrap();
                }
                "marker" => {
                    fs::remove_file(root.join(&marker)).unwrap();
                    fs::hard_link(root.join(&relative), root.join(&marker)).unwrap();
                }
                "directory" => {
                    let directory = root.join(&relative).parent().unwrap().to_path_buf();
                    let moved = root.join("actor-evidence");
                    fs::rename(&directory, &moved).unwrap();
                    symlink(&moved, &directory).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(retained_journal_paths(&root, execution).is_err());
            assert!(read_retained_journal(&root, execution, &relative).is_err());
            assert!(!root.join("outputs/work/runtime").exists());
        }
        let root = policy_test_root("retained-copied");
        let (relative, _) = retained_case(&root, execution, 2);
        let copied = relative.replace(execution, "foreign execution");
        let target = root.join(&copied);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let raw = fs::read(root.join(&relative)).unwrap();
        fs::write(&target, &raw).unwrap();
        assert_eq!(
            read_retained_journal(&root, "foreign execution", &copied)
                .unwrap_err()
                .reason_code,
            "journal_layout_metadata"
        );
        assert_eq!(fs::read(target).unwrap(), raw);
    }
    use work_operations::derivation::identity::derived_transaction_id;
    use work_operations::derivation::snapshot::encode_snapshot;
    use work_operations::derivation::transaction::approval_sha256;

    fn policy_test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-spec-policy-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn storage_rejects_hard_and_symbolic_aliases_without_changing_source() {
        let root = policy_test_root("aliases");
        fs::write(root.join("original"), b"preserve").unwrap();
        fs::hard_link(root.join("original"), root.join("alias")).unwrap();
        assert_eq!(
            storage_path(&root, "alias").unwrap_err().reason_code,
            "spec_update_alias"
        );
        std::os::unix::fs::symlink(root.join("original"), root.join("link")).unwrap();
        assert_eq!(
            storage_path(&root, "link").unwrap_err().reason_code,
            "spec_update_link"
        );
        assert_eq!(fs::read(root.join("original")).unwrap(), b"preserve");
    }

    #[test]
    fn guard_requires_valid_markers_for_all_specification_record_types() {
        let root = policy_test_root("pending");
        fs::create_dir(root.join("execution")).unwrap();
        {
            for kind in 0..6 {
                let (relative, _) = retained_case(&root, "execution", kind);
                let raw = fs::read(root.join(&relative)).unwrap();
                let marker = root.join(
                    work_operations::derivation::publication::retained_journal_marker(&relative)
                        .unwrap(),
                );
                fs::remove_file(&marker).unwrap();
                assert_eq!(
                    require_no_spec_update(&root, "execution", None)
                        .unwrap_err()
                        .reason_code,
                    "spec_update_pending"
                );
                fs::write(&marker, b"invalid").unwrap();
                assert_eq!(
                    require_no_spec_update(&root, "execution", None)
                        .unwrap_err()
                        .reason_code,
                    "journal_commit_evidence"
                );
                assert_eq!(fs::read(root.join(&relative)).unwrap(), raw);
                fs::write(&marker, completion_marker(&raw)).unwrap();
                require_no_spec_update(&root, "execution", None).unwrap();
            }
        }
    }

    #[test]
    fn guard_preserves_underlying_record_read_failure() {
        let root = policy_test_root("read-failure");
        fs::create_dir_all(root.join("execution/.work-spec-update-example.json")).unwrap();
        let failure = require_no_spec_update(&root, "execution", None).unwrap_err();
        assert_eq!(failure.exit_code, ExitCode::ArtifactIntegrity);
        assert_eq!(failure.reason_code, { "legacy_journal_layout_present" });
    }

    #[test]
    fn journal_publication_and_resume_match_current_contract_progress() {
        let root = std::env::temp_dir().join(format!(
            "work-spec-journal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let metadata = json!({"request":{},"artifacts":{},"affected_task_ids":[],"history_sha256":{},"source_sha256":{},"candidate_sha256":{}});
        let files = json!([{"phase":10,"path":"one.json","operation":"add","after":encode_snapshot(b"one")},{"phase":20,"path":"two.json","operation":"add","after":encode_snapshot(b"two")}]);
        let approval = approval_sha256(&files, &metadata);
        let value = json!({"schema":"work-spec-transaction","transaction_id":derived_transaction_id("UPDATE", &approval).unwrap(),"approval_sha256":approval,"state":"prepared","published_count":0,"metadata":metadata,"files":files});
        write_journal(&root, "journal.json", &value).unwrap();
        assert_eq!(
            publish_journal(&root, "journal.json", "journal.json.done").unwrap()["status"],
            "published"
        );
        assert_eq!(fs::read(root.join("one.json")).unwrap(), b"one");
        assert_eq!(fs::read(root.join("two.json")).unwrap(), b"two");
        assert_eq!(
            publish_journal(&root, "journal.json", "journal.json.done").unwrap()["status"],
            "already_published"
        );
        let mut resumed = value;
        resumed["published_count"] = json!(1);
        resumed["state"] = json!("publishing");
        fs::write(
            root.join("resume.json"),
            render_transaction(&resumed).unwrap(),
        )
        .unwrap();
        fs::write(root.join("one.json"), b"one").unwrap();
        assert_eq!(
            publish_journal(&root, "resume.json", "resume.json.done").unwrap()["status"],
            "published"
        );
        assert_eq!(
            fs::read(root.join("resume.json.done")).unwrap(),
            completion_marker(&fs::read(root.join("resume.json")).unwrap())
        );
    }

    #[test]
    fn receipt_history_rejects_copied_cross_attempt_identity_without_rewriting_evidence() {
        let root = policy_test_root("receipt-identity");
        let relative = "execution/TASK-001/ATTEMPT-002/receipts/CMD-001";
        let directory = root.join(relative);
        fs::create_dir_all(&directory).unwrap();
        let raw = serde_json::to_vec(&json!({"schema":"work-command-started",
            "preview":{"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"CMD-001",
                "receipt_dir":"execution/TASK-001/ATTEMPT-001/receipts/CMD-001"}}))
        .unwrap();
        fs::write(directory.join("started.json"), &raw).unwrap();
        assert_eq!(
            execution_history_bytes_with_receipts(&root, "execution")
                .unwrap_err()
                .reason_code,
            "command_receipt_history_identity"
        );
        assert_eq!(fs::read(directory.join("started.json")).unwrap(), raw);
    }

    #[test]
    fn receipt_history_keeps_partial_and_finished_only_bytes_and_rejects_unknown_or_legacy_layout()
    {
        let root = policy_test_root("receipts");
        let directory = root.join("custom execution/TASK-001/ATTEMPT-002/receipts/CMD-001-retry-2");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("finished.json"), b"{partial outcome").unwrap();
        let history = execution_history_bytes_with_receipts(&root, "custom execution").unwrap();
        assert_eq!(
            history["custom execution/TASK-001/ATTEMPT-002/receipts/CMD-001-retry-2/finished.json"],
            b"{partial outcome"
        );
        fs::write(directory.join("started.json"), b"raw start\r\n").unwrap();
        assert_eq!(
            execution_history_bytes_with_receipts(&root, "custom execution")
                .unwrap()
                .len(),
            2
        );
        fs::write(directory.join("foreign.json"), b"foreign").unwrap();
        assert_eq!(
            execution_history_bytes_with_receipts(&root, "custom execution")
                .unwrap_err()
                .reason_code,
            "command_receipt_history_layout"
        );
        assert_eq!(
            fs::read(directory.join("finished.json")).unwrap(),
            b"{partial outcome"
        );
        fs::remove_file(directory.join("foreign.json")).unwrap();
        let legacy = directory
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".work-command-CMD-002.started.json");
        fs::write(&legacy, b"original legacy proof").unwrap();
        assert_eq!(
            execution_history_bytes_with_receipts(&root, "custom execution")
                .unwrap_err()
                .reason_code,
            "legacy_command_receipt_present"
        );
        assert_eq!(fs::read(legacy).unwrap(), b"original legacy proof");
    }

    #[test]
    fn receipt_history_refuses_empty_instance_and_hardlink_alias_without_cleanup() {
        let root = policy_test_root("receipt-alias");
        let directory = root.join("execution/TASK-001/ATTEMPT-001/receipts/CMD-001");
        fs::create_dir_all(&directory).unwrap();
        assert_eq!(
            execution_history_bytes_with_receipts(&root, "execution")
                .unwrap_err()
                .reason_code,
            "command_receipt_history_incomplete"
        );
        let foreign = root.join("foreign-evidence");
        fs::write(&foreign, b"foreign bytes").unwrap();
        fs::hard_link(&foreign, directory.join("started.json")).unwrap();
        assert_eq!(
            execution_history_bytes_with_receipts(&root, "execution")
                .unwrap_err()
                .reason_code,
            "runtime_path_alias"
        );
        assert_eq!(fs::read(foreign).unwrap(), b"foreign bytes");
    }

    #[test]
    fn history_fingerprints_include_nested_attempts_and_reject_bad_task_layout() {
        let root = std::env::temp_dir().join(format!(
            "work-spec-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("execution/TASK-001/ATTEMPT-001")).unwrap();
        fs::write(
            root.join("execution/TASK-001/ATTEMPT-001/attempt.json"),
            b"{}\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("execution/TASK-002/ATTEMPT-001/corrections")).unwrap();
        fs::write(
            root.join("execution/TASK-002/ATTEMPT-001/corrections/CORRECTION-001.json"),
            b"second\n",
        )
        .unwrap();
        let history = execution_history_fingerprints(&root, "execution").unwrap();
        assert_eq!(
            history["execution/TASK-001/ATTEMPT-001/attempt.json"],
            sha256_hex(b"{}\n")
        );
        assert_eq!(
            history["execution/TASK-002/ATTEMPT-001/corrections/CORRECTION-001.json"],
            sha256_hex(b"second\n")
        );
        assert_eq!(history.len(), 2);
        fs::write(root.join("execution/TASK-003"), b"bad").unwrap();
        assert_eq!(
            execution_history_fingerprints(&root, "execution")
                .unwrap_err()
                .reason_code,
            "spec_update_history_layout"
        );
    }
}
