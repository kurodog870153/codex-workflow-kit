//! Durable project file transactions using the existing requirement lock and runtime manifest.
use super::file_transaction_resources as resources;
use super::storage::LocalExecutionStorage;
use crate::files::{LocalFiles, require_runtime_same_filesystem, resolve_runtime_path};
use crate::transaction_storage::{
    prepare_runtime_transaction, runtime_manifest_directory, update_runtime_progress,
};
use crate::writer_lock::LocalWriterLock;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use work_feature::error::WorkError;
use work_feature::execution::ExecutionWriterContext;
use work_feature::execution::RuntimeExecutionReadiness;
use work_feature::execution::file_transaction::{FileTransactionRepository, error, prepare};
use work_feature::ports::{ArtifactStore, RuntimeWriterLock, with_runtime_writer};
use work_model::execution::file_transaction::*;
use work_model::runtime::{LockClass, RuntimeManifest, RuntimePhase};
use work_model::schema::PublicSchema;
use work_operations::derivation::{file_transaction as derive, fingerprint, publication};

pub struct LocalFileTransactions {
    root: PathBuf,
    owned: bool,
    instruction_root: Option<PathBuf>,
}

fn temporary_index(name: &str, count: usize) -> Result<(usize, bool), WorkError> {
    let (direction, index) = name
        .strip_suffix(".tmp")
        .and_then(|s| s.rsplit_once('-'))
        .ok_or_else(|| error("file_transaction_inventory_foreign"))?;
    let index: usize = index
        .parse()
        .map_err(|_| error("file_transaction_inventory_foreign"))?;
    if index >= count || name != format!("{direction}-{index}.tmp") {
        return Err(error("file_transaction_inventory_foreign"));
    }
    let publish = match direction {
        "publish" => true,
        "restore" => false,
        #[cfg(any(windows, target_os = "macos"))]
        "restore-metadata" => false,
        _ => return Err(error("file_transaction_inventory_foreign")),
    };
    Ok((index, publish))
}
fn temporary_prefix(expected: Option<&[u8]>, actual: &[u8]) -> Result<(), WorkError> {
    if !expected.is_some_and(|bytes| bytes.starts_with(actual)) {
        return Err(error("file_transaction_temporary_corrupt"));
    }
    Ok(())
}
impl LocalFileTransactions {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            owned: false,
            instruction_root: None,
        }
    }
    pub fn for_project(root: PathBuf, instruction_root: PathBuf) -> Self {
        Self {
            root,
            owned: false,
            instruction_root: Some(instruction_root),
        }
    }
    pub(crate) fn fixture_publish(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileTransactionPreview,
        position: usize,
    ) -> Result<FileTransactionResult, WorkError> {
        let owned = Self {
            root: self.root.clone(),
            owned: true,
            instruction_root: self.instruction_root.clone(),
        };
        owned.publish_scoped(
            context,
            preview,
            "Synthetic user approved complete publication",
            |n| {
                if n == position {
                    Err(error("fixture_interruption"))
                } else {
                    Ok(())
                }
            },
        )
    }
    fn path(&self, path: &str) -> Result<PathBuf, WorkError> {
        let file = resolve_runtime_path(&self.root, path)?;
        require_runtime_same_filesystem(&self.root, &file)?;
        Ok(file)
    }
    fn binding(manifest: &RuntimeManifest) -> Result<FileTransactionBinding, WorkError> {
        serde_json::from_value(manifest.business_identity.clone())
            .map_err(|_| error("file_transaction_binding_invalid"))
    }
    fn validate_context(
        &self,
        context: &ExecutionWriterContext,
        manifest: &RuntimeManifest,
    ) -> Result<(), WorkError> {
        let binding = Self::binding(manifest)?;
        if manifest.canonical_root != context.writer().canonical_project_root.to_string_lossy()
            || manifest.requirement_id != context.writer().requirement_id.as_str()
            || manifest.execution_dir != context.target().execution_dir
            || binding.task_id != context.target().task_id
        {
            return Err(error("file_transaction_context_mismatch"));
        }
        let raw = self.read_source(&format!("{}/index.json", context.target().execution_dir))?;
        let index = work_operations::canonical::parse_json_contract(&raw)
            .map_err(|_| error("file_transaction_index_invalid"))?;
        if index["lock"]["task_id"] != binding.task_id
            || index["lock"]["attempt_id"] != binding.attempt_id
        {
            return Err(error("file_transaction_attempt_mismatch"));
        }
        Ok(())
    }
    fn expected(
        manifest: &RuntimeManifest,
        binding: &FileTransactionBinding,
        after: bool,
    ) -> Result<BTreeMap<String, Option<FileState>>, WorkError> {
        manifest
            .targets
            .iter()
            .map(|t| {
                let bytes = if after { &t.after } else { &t.before };
                let metadata = if after {
                    &binding.after_metadata
                } else {
                    &binding.before_metadata
                };
                let state = match (bytes, metadata.get(&t.path)) {
                    (Some(b), Some(Some(m))) => {
                        #[cfg(target_os = "macos")]
                        super::project_file_metadata_macos::ownership(m)?;
                        Some(FileState {
                            bytes: b.bytes.clone(),
                            metadata: m.clone(),
                        })
                    }
                    (None, Some(None)) => None,
                    _ => return Err(error("file_transaction_metadata_inventory")),
                };
                Ok((t.path.clone(), state))
            })
            .collect()
    }
    fn observed(
        &self,
        manifest: &RuntimeManifest,
    ) -> Result<BTreeMap<String, Option<FileState>>, WorkError> {
        self.check_resources(
            &manifest
                .targets
                .iter()
                .map(|t| t.path.clone())
                .collect::<Vec<_>>(),
        )?;
        manifest
            .targets
            .iter()
            .map(|t| Ok((t.path.clone(), self.inspect(&t.path)?)))
            .collect()
    }
    fn directory(&self, requirement: &str, identity: &str) -> Result<PathBuf, WorkError> {
        let requirement = requirement
            .parse()
            .map_err(|_| error("file_transaction_requirement"))?;
        let path = publication::runtime_staging_path(
            &requirement,
            publication::RuntimeOperation::ProjectFiles,
            identity,
        )
        .map_err(|_| error("file_transaction_identity"))?;
        self.path(&path)
    }
    /// Exact inventory and frozen candidate verification, including interrupted temporary writes.
    fn load(&self, requirement: &str, identity: &str) -> Result<RuntimeManifest, WorkError> {
        let dir = self.directory(requirement, identity)?;
        let raw = resources::read(&dir.join("transaction.json"), resources::MAX_CONTROL_BYTES)?;
        self.load_bytes(requirement, identity, &raw)
    }
    fn load_bytes(
        &self,
        requirement: &str,
        identity: &str,
        raw: &[u8],
    ) -> Result<RuntimeManifest, WorkError> {
        let dir = self.directory(requirement, identity)?;
        let manifest: RuntimeManifest =
            serde_json::from_slice(raw).map_err(|_| error("file_transaction_manifest_invalid"))?;
        // Recovery may observe small after files while restoring much larger before
        // files. Budget the frozen set before cloning, serializing or writing it.
        let mut size = 0_u64;
        for target in &manifest.targets {
            size = size
                .checked_add(target.path.len() as u64 + 4096)
                .ok_or_else(|| error("file_transaction_resource_limit"))?;
            for state in [&target.before, &target.after].into_iter().flatten() {
                size = size
                    .checked_add(state.bytes.len() as u64)
                    .ok_or_else(|| error("file_transaction_resource_limit"))?;
            }
            if target.after.is_some() {
                size = size
                    .checked_add(target.path.len() as u64 + 4096)
                    .ok_or_else(|| error("file_transaction_resource_limit"))?;
            }
        }
        let available = fs4::available_space(&self.root)
            .map_err(|_| error("file_transaction_storage_unverifiable"))?;
        resources::verify(size, available)?;
        if serde_json::to_vec(&manifest).expect("manifest serializes") != raw
            || manifest.operation != "project-files"
            || manifest.transaction_identity != identity
            || manifest.requirement_id != requirement
            || runtime_manifest_directory(&self.root, &manifest)? != dir
            || derive::approval(&manifest) != manifest.approval_sha256
        {
            return Err(error("file_transaction_manifest_binding"));
        }
        let binding = Self::binding(&manifest)?;
        let frozen = derive::candidate_bytes(&binding, &manifest.targets);
        let payload = dir.join("candidate.json");
        // No target was touched until this full payload was written. A failed prepare retains
        // the manifest's complete before/after evidence and may be explicitly restored.
        if payload.exists() {
            let current = resources::read(&payload, resources::MAX_CONTROL_BYTES)?;
            if !frozen.starts_with(&current) {
                return Err(error("file_transaction_inventory_corrupt"));
            }
            if current != frozen && manifest.phase != RuntimePhase::Prepared {
                return Err(error("file_transaction_inventory_incomplete"));
            }
        } else if manifest.phase != RuntimePhase::Prepared {
            return Err(error("file_transaction_inventory_incomplete"));
        }
        if manifest.inventory.len() != 1
            || manifest.inventory[0].path != "candidate.json"
            || manifest.inventory[0].sha256 != fingerprint::raw(&frozen)
            || manifest.inventory[0].size_bytes != frozen.len() as u64
        {
            return Err(error("file_transaction_inventory_binding"));
        }
        Self::expected(&manifest, &binding, false)?;
        Self::expected(&manifest, &binding, true)?;
        let mut publish_authorized = false;
        let mut restore_authorized = false;
        for entry in fs::read_dir(&dir).map_err(|_| error("file_transaction_inventory_read"))? {
            let entry = entry.map_err(|_| error("file_transaction_inventory_read"))?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| error("file_transaction_inventory_foreign"))?
                .to_owned();
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| error("file_transaction_root"))?
                .to_string_lossy()
                .replace('\\', "/");
            let path = self.path(&relative)?;
            if !path.is_file() {
                return Err(error("file_transaction_inventory_foreign"));
            }
            if matches!(name.as_str(), "transaction.json" | "candidate.json") {
                continue;
            }
            if name.starts_with("authorization-") && name.ends_with(".json.tmp") {
                // An uncommitted authorization grants no authority. Retain it for exact
                // request revalidation; recovery may use a newly approved authorization.
                let final_name = name.strip_suffix(".tmp").expect("checked suffix");
                self.check_authorization_prefix(
                    &manifest,
                    final_name,
                    &resources::read(&path, resources::MAX_CONTROL_BYTES)?,
                )?;
                continue;
            }
            if name.starts_with("authorization-") && name.ends_with(".json") {
                let raw = resources::read(&path, resources::MAX_CONTROL_BYTES)?;
                let authorization: FileAuthorization = match serde_json::from_slice(&raw) {
                    Ok(value) => value,
                    Err(_) => {
                        // Older writers could leave a truncated final record. Do not
                        // overwrite it or count it as approval; require fresh evidence.
                        self.check_authorization_prefix(&manifest, &name, &raw)?;
                        continue;
                    }
                };
                if derive::authorization_path(&authorization) != name
                    || authorization.transaction_identity != manifest.transaction_identity
                    || authorization.publication_sha256 != manifest.approval_sha256
                    || authorization.authorization_evidence.trim().is_empty()
                {
                    return Err(error("file_transaction_authorization_binding"));
                }
                if let Some(recovery) = &authorization.recovery {
                    let mut frozen = manifest.clone();
                    frozen.phase = recovery.manifest.phase;
                    frozen.published_count = recovery.manifest.published_count;
                    if frozen != recovery.manifest
                        || recovery.approval_sha256
                            != derive::recovery_approval(&recovery.manifest, &recovery.observed)
                    {
                        return Err(error("file_transaction_authorization_binding"));
                    }
                    restore_authorized = true;
                } else {
                    publish_authorized = true;
                }
                continue;
            }
            if name == "transaction.json.tmp" {
                self.control_candidate(
                    &manifest,
                    &resources::read(&path, resources::MAX_CONTROL_BYTES)?,
                )?;
                continue;
            }
            let (n, publish) = temporary_index(&name, manifest.targets.len())?;
            let target = &manifest.targets[n];
            let bytes = if publish {
                &target.after
            } else {
                &target.before
            };
            let raw = resources::read(&path, resources::MAX_CONTROL_BYTES)?;
            temporary_prefix(bytes.as_ref().map(|b| b.bytes.as_slice()), &raw)?;
        }
        if (matches!(
            manifest.phase,
            RuntimePhase::Publishing | RuntimePhase::PublishedVerified
        ) && !publish_authorized)
            || (matches!(
                manifest.phase,
                RuntimePhase::Restoring | RuntimePhase::Restored
            ) && !restore_authorized)
        {
            return Err(error("file_transaction_authorization_missing"));
        }
        Ok(manifest)
    }
    fn control_candidate(
        &self,
        manifest: &RuntimeManifest,
        raw: &[u8],
    ) -> Result<RuntimeManifest, WorkError> {
        for phase in [
            RuntimePhase::Publishing,
            RuntimePhase::PublishedVerified,
            RuntimePhase::Restoring,
            RuntimePhase::Restored,
        ] {
            if derive::phase_rank(phase) < derive::phase_rank(manifest.phase) {
                continue;
            }
            for count in manifest.published_count..=manifest.targets.len() {
                let mut next = manifest.clone();
                next.phase = phase;
                next.published_count = count;
                if next.validate_shape().is_ok()
                    && serde_json::to_vec(&next)
                        .expect("manifest serializes")
                        .starts_with(raw)
                {
                    return Ok(next);
                }
            }
        }
        Err(error("file_transaction_control_corrupt"))
    }
    fn settle_control(&self, manifest: &RuntimeManifest) -> Result<RuntimeManifest, WorkError> {
        let dir = runtime_manifest_directory(&self.root, manifest)?;
        let path = dir.join("transaction.json.tmp");
        if !path.exists() {
            return Ok(manifest.clone());
        }
        let next = self.control_candidate(
            manifest,
            &resources::read(&path, resources::MAX_CONTROL_BYTES)?,
        )?;
        update_runtime_progress(
            &LocalFiles,
            &self.root,
            manifest,
            next.published_count,
            next.phase,
        )?;
        self.load(&manifest.requirement_id, &manifest.transaction_identity)
    }
    fn check_sources(
        &self,
        context: &ExecutionWriterContext,
        binding: &FileTransactionBinding,
        publishing: Option<&RuntimeManifest>,
    ) -> Result<(), WorkError> {
        for (p, h) in &binding.source_sha256 {
            if publishing
                .is_some_and(|manifest| manifest.targets.iter().any(|target| target.path == *p))
            {
                continue;
            }
            if fingerprint::raw(&self.read_source(p)?) != *h {
                return Err(error("file_transaction_source_drift"));
            }
        }
        for (target, staged) in &binding.staged_files {
            if self.inspect(staged)?.map(|state| state.metadata).as_ref()
                != binding.after_metadata.get(target).and_then(Option::as_ref)
            {
                return Err(error("file_transaction_source_drift"));
            }
        }
        let task = context.task_context().contract["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["id"] == binding.task_id)
            .ok_or_else(|| error("file_transaction_task_missing"))?;
        let attempt: serde_json::Value = serde_json::from_slice(&self.read_source(&format!(
            "{}/{}/{}/attempt.json",
            context.target().execution_dir,
            binding.task_id,
            binding.attempt_id
        ))?)
        .map_err(|_| error("file_transaction_attempt_invalid"))?;
        work_feature::execution::file_transaction::verify_execute(self, task, &attempt)?;
        Ok(())
    }
    fn authorize(
        &self,
        manifest: &RuntimeManifest,
        evidence: &str,
        recovery: Option<FileRecoveryPreview>,
    ) -> Result<(), WorkError> {
        let dir = runtime_manifest_directory(&self.root, manifest)?;
        let authorization = FileAuthorization {
            transaction_identity: manifest.transaction_identity.clone(),
            publication_sha256: manifest.approval_sha256.clone(),
            authorization_evidence: evidence.into(),
            recovery,
        };
        let path = dir.join(derive::authorization_path(&authorization));
        let bytes = serde_json::to_vec(&authorization).expect("authorization serializes");
        if path.exists() {
            if resources::read(&path, resources::MAX_CONTROL_BYTES)? != bytes {
                return Err(error("file_transaction_authorization_corrupt"));
            }
        } else {
            let temporary = path.with_file_name(format!(
                "{}.tmp",
                derive::authorization_path(&authorization)
            ));
            if temporary.exists() {
                let partial = resources::read(&temporary, resources::MAX_CONTROL_BYTES)?;
                if !bytes.starts_with(&partial) {
                    return Err(error("file_transaction_authorization_corrupt"));
                }
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(&temporary)
                    .map_err(|_| error("file_transaction_authorization_write"))?;
                file.write_all(&bytes[partial.len()..])
                    .and_then(|_| file.sync_all())
                    .map_err(|_| error("file_transaction_authorization_write"))?;
            } else {
                LocalFiles.create_new(&temporary, &bytes)?;
            }
            if resources::read(&temporary, resources::MAX_CONTROL_BYTES)? != bytes || path.exists()
            {
                return Err(error("file_transaction_authorization_corrupt"));
            }
            super::control_commit::new(&temporary, &path)?;
        }
        if resources::read(&path, resources::MAX_CONTROL_BYTES)? != bytes {
            return Err(error("file_transaction_authorization_corrupt"));
        }
        Ok(())
    }
    fn check_authorization_prefix(
        &self,
        manifest: &RuntimeManifest,
        name: &str,
        raw: &[u8],
    ) -> Result<(), WorkError> {
        let hash = name
            .strip_prefix("authorization-")
            .and_then(|s| s.strip_suffix(".json"))
            .filter(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or_else(|| error("file_transaction_authorization_corrupt"))?;
        let _ = hash;
        let header = format!(
            "{{\"transaction_identity\":{},\"publication_sha256\":{},\"authorization_evidence\":",
            serde_json::to_string(&manifest.transaction_identity).expect("identity serializes"),
            serde_json::to_string(&manifest.approval_sha256).expect("approval serializes")
        );
        let compared = raw.len().min(header.len());
        if raw[..compared] != header.as_bytes()[..compared] {
            return Err(error("file_transaction_authorization_corrupt"));
        }
        Ok(())
    }
    fn set_state(
        &self,
        dir: &Path,
        n: usize,
        path: &str,
        before: &Option<FileState>,
        after: &Option<FileState>,
        restore: bool,
    ) -> Result<(), WorkError> {
        if self.inspect(path)? != *before {
            return Err(error("file_transaction_target_drift"));
        }
        if before == after {
            return Ok(());
        }
        let target = self.path(path)?;
        match after {
            Some(state) => {
                let temporary = dir.join(format!(
                    "{}-{n}.tmp",
                    if restore { "restore" } else { "publish" }
                ));
                if temporary.exists() {
                    let current = resources::read(&temporary, resources::MAX_CONTROL_BYTES)?;
                    if !state.bytes.starts_with(&current) {
                        return Err(error("file_transaction_temporary_corrupt"));
                    }
                    let mut file = OpenOptions::new()
                        .append(true)
                        .open(&temporary)
                        .map_err(|_| error("file_transaction_temporary_write"))?;
                    file.write_all(&state.bytes[current.len()..])
                        .and_then(|_| file.sync_all())
                        .map_err(|_| error("file_transaction_temporary_write"))?;
                } else {
                    LocalFiles.create_new(&temporary, &state.bytes)?;
                }
                self.set_metadata(&temporary, &state.metadata)?;
                if self.inspect(path)? != *before {
                    return Err(error("file_transaction_target_drift"));
                }
                // Existing parents and same-volume temporary were verified before any target write.
                LocalFiles.replace(&temporary, &target)?;
            }
            None => {
                fs::remove_file(&target).map_err(|_| error("file_transaction_remove_failed"))?;
            }
        }
        if self.inspect(path)? != *after {
            return Err(error("file_transaction_readback"));
        }
        Ok(())
    }
    fn set_metadata(&self, path: &Path, metadata: &FileMetadata) -> Result<(), WorkError> {
        #[cfg(target_os = "macos")]
        {
            super::project_file_metadata_macos::write(path, metadata)?;
        }
        let mut permissions = fs::metadata(path)
            .map_err(|_| error("file_transaction_metadata_read"))?
            .permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(
                metadata
                    .unix_mode
                    .ok_or_else(|| error("file_transaction_metadata_unsupported"))?,
            );
        }
        #[cfg(windows)]
        {
            super::project_file_metadata::write(
                path,
                metadata
                    .windows_security
                    .as_deref()
                    .ok_or_else(|| error("file_transaction_security_missing"))?,
            )?;
            permissions.set_readonly(metadata.readonly);
        }
        fs::set_permissions(path, permissions).map_err(|_| error("file_transaction_metadata_write"))
    }
    fn publish_scoped(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileTransactionPreview,
        evidence: &str,
        mut checkpoint: impl FnMut(usize) -> Result<(), WorkError>,
    ) -> Result<FileTransactionResult, WorkError> {
        if evidence.trim().is_empty() {
            return Err(error("file_transaction_authorization_required"));
        }
        let binding = Self::binding(&preview.manifest)?;
        let current = prepare(self, context, &binding.attempt_id, &binding.staged_files)?;
        if current != *preview {
            return Err(error("file_transaction_approval_stale"));
        }
        let manifest = &preview.manifest;
        let payloads = BTreeMap::from([(
            "candidate.json".into(),
            derive::candidate_bytes(&binding, &manifest.targets),
        )]);
        let dir = prepare_runtime_transaction(&LocalFiles, &self.root, manifest, &payloads)?;
        if self.load(&manifest.requirement_id, &manifest.transaction_identity)? != *manifest {
            return Err(error("file_transaction_preparation_readback"));
        }
        self.check_sources(context, &binding, None)?;
        let before = Self::expected(manifest, &binding, false)?;
        let after = Self::expected(manifest, &binding, true)?;
        // Prove restoration of every supported metadata snapshot before the first target write.
        for (n, t) in manifest.targets.iter().enumerate() {
            for (direction, state) in [("publish", &after[&t.path]), ("restore", &before[&t.path])]
            {
                if let Some(state) = state {
                    let path = dir.join(format!("{direction}-{n}.tmp"));
                    LocalFiles.create_new(&path, &state.bytes)?;
                    self.set_metadata(&path, &state.metadata)?;
                }
            }
        }
        if self.observed(manifest)? != before {
            return Err(error("file_transaction_target_drift"));
        }
        self.authorize(manifest, evidence, None)?;
        update_runtime_progress(
            &LocalFiles,
            &self.root,
            manifest,
            0,
            RuntimePhase::Publishing,
        )?;
        checkpoint(0)?;
        for (n, t) in manifest.targets.iter().enumerate() {
            self.check_sources(context, &binding, Some(manifest))?;
            self.set_state(&dir, n, &t.path, &before[&t.path], &after[&t.path], false)?;
            checkpoint(n + 1)?;
            update_runtime_progress(
                &LocalFiles,
                &self.root,
                manifest,
                n + 1,
                RuntimePhase::Publishing,
            )?;
        }
        if self.observed(manifest)? != after {
            return Err(error("file_transaction_readback"));
        }
        update_runtime_progress(
            &LocalFiles,
            &self.root,
            manifest,
            manifest.targets.len(),
            RuntimePhase::PublishedVerified,
        )?;
        Ok(FileTransactionResult {
            schema: PublicSchema::WorkFileTransactionResult,
            transaction_identity: manifest.transaction_identity.clone(),
            phase: RuntimePhase::PublishedVerified,
        })
    }
    fn restore_scoped(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileRecoveryPreview,
        evidence: &str,
        mut checkpoint: impl FnMut(usize) -> Result<(), WorkError>,
    ) -> Result<FileTransactionResult, WorkError> {
        if evidence.trim().is_empty() {
            return Err(error("file_transaction_recovery_authorization_required"));
        }
        let current = self.recovery_preview(context, &preview.manifest.transaction_identity)?;
        if current != *preview {
            return Err(error("file_transaction_recovery_approval_stale"));
        }
        let binding = Self::binding(&current.manifest)?;
        let dir = runtime_manifest_directory(&self.root, &current.manifest)?;
        let frozen = derive::candidate_bytes(&binding, &current.manifest.targets);
        let path = dir.join("candidate.json");
        if path.exists() {
            let raw = resources::read(&path, resources::MAX_CONTROL_BYTES)?;
            if !frozen.starts_with(&raw) {
                return Err(error("file_transaction_inventory_corrupt"));
            }
            if raw != frozen {
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .map_err(|_| error("file_transaction_inventory_write"))?;
                file.write_all(&frozen[raw.len()..])
                    .and_then(|_| file.sync_all())
                    .map_err(|_| error("file_transaction_inventory_write"))?;
            }
        } else {
            LocalFiles.create_new(&path, &frozen)?;
        }
        self.authorize(&current.manifest, evidence, Some(current.clone()))?;
        let manifest = self.settle_control(&current.manifest)?;
        let binding = Self::binding(&manifest)?;
        let dir = runtime_manifest_directory(&self.root, &manifest)?;
        let before = Self::expected(&manifest, &binding, false)?;
        let after = Self::expected(&manifest, &binding, true)?;
        // Re-prove all restoration metadata before any target is changed, even if
        // the original publication preflight temporaries were lost or interrupted.
        #[cfg(any(windows, target_os = "macos"))]
        for (n, target) in manifest.targets.iter().enumerate() {
            if let Some(state) = &before[&target.path] {
                let path = dir.join(format!("restore-metadata-{n}.tmp"));
                if path.exists() {
                    let raw = resources::read(&path, resources::MAX_CONTROL_BYTES)?;
                    if !state.bytes.starts_with(&raw) {
                        return Err(error("file_transaction_temporary_corrupt"));
                    }
                    let mut file = OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .map_err(|_| error("file_transaction_temporary_write"))?;
                    file.write_all(&state.bytes[raw.len()..])
                        .and_then(|_| file.sync_all())
                        .map_err(|_| error("file_transaction_temporary_write"))?;
                } else {
                    LocalFiles.create_new(&path, &state.bytes)?;
                }
                self.set_metadata(&path, &state.metadata)?;
            }
        }
        let observed = self.observed(&manifest)?;
        for (p, s) in &observed {
            if *s != before[p] && *s != after[p] {
                return Err(error("file_transaction_external_change"));
            }
        }
        if manifest.phase != RuntimePhase::Restored {
            update_runtime_progress(
                &LocalFiles,
                &self.root,
                &manifest,
                manifest.published_count,
                RuntimePhase::Restoring,
            )?;
            checkpoint(0)?;
            for (n, t) in manifest.targets.iter().enumerate().rev() {
                self.set_state(&dir, n, &t.path, &observed[&t.path], &before[&t.path], true)?;
                checkpoint(n + 1)?;
            }
            if self.observed(&manifest)? != before {
                return Err(error("file_transaction_restore_readback"));
            }
            update_runtime_progress(
                &LocalFiles,
                &self.root,
                &manifest,
                manifest.published_count,
                RuntimePhase::Restored,
            )?;
        }
        Ok(FileTransactionResult {
            schema: PublicSchema::WorkFileTransactionResult,
            transaction_identity: manifest.transaction_identity,
            phase: RuntimePhase::Restored,
        })
    }
}

impl FileTransactionRepository for LocalFileTransactions {
    fn check_resources(&self, paths: &[String]) -> Result<(), WorkError> {
        let mut size = 0_u64;
        for path in paths {
            let target = self.path(path)?;
            let length = match fs::metadata(&target) {
                Ok(metadata) => metadata.len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                Err(_) => return Err(error("file_transaction_file_read")),
            };
            size = size
                .checked_add(length)
                .and_then(|size| size.checked_add(path.len() as u64 + 4096))
                .ok_or_else(|| error("file_transaction_resource_limit"))?;
            resources::requirements(size)?;
        }
        let available = fs4::available_space(&self.root)
            .map_err(|_| error("file_transaction_storage_unverifiable"))?;
        resources::verify(size, available)
    }
    fn execute_selection(
        &self,
        task: &serde_json::Value,
        attempt: &serde_json::Value,
    ) -> Result<serde_json::Value, WorkError> {
        let root = self
            .instruction_root
            .as_ref()
            .ok_or_else(|| error("file_transaction_instruction_repository_required"))?;
        let selected: Vec<_> = task["instruction_selection"]["selected_paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect();
        let mut references = vec!["execute.general.execution-records".to_owned()];
        if attempt.get("continued_from").is_some() {
            references.push("execute.general.execution-recovery".into());
        }
        Ok(serde_json::json!(work_feature::instruction::select(
            &crate::hierarchy_catalog::LocalHierarchyCatalog {
                skill_root: root.clone()
            },
            "execute",
            &selected,
            &references
        )?))
    }
    fn read_source(&self, path: &str) -> Result<Vec<u8>, WorkError> {
        resources::read(&self.path(path)?, resources::MAX_CONTROL_BYTES)
    }
    fn inspect(&self, path: &str) -> Result<Option<FileState>, WorkError> {
        if !cfg!(any(windows, target_os = "macos")) {
            return Err(error("file_transaction_platform_unsupported"));
        }
        let target = self.path(path)?;
        if !target.parent().is_some_and(Path::is_dir) {
            return Err(error("file_transaction_parent_missing"));
        }
        let file = match OpenOptions::new().read(true).open(&target) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(error("file_transaction_file_read")),
        };
        let metadata = file
            .metadata()
            .map_err(|_| error("file_transaction_metadata_read"))?;
        if !metadata.is_file() || metadata.permissions().readonly() {
            return Err(error("file_transaction_metadata_unsupported"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & !(0x20 | 0x80) != 0 {
                return Err(error("file_transaction_metadata_unsupported"));
            }
        }
        #[cfg(unix)]
        let unix_mode = {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode())
        };
        #[cfg(not(unix))]
        let unix_mode = None;
        #[cfg(target_os = "macos")]
        let (unix_uid, unix_gid) = super::project_file_metadata_macos::read(&file)?;
        let permissions = FileMetadata {
            readonly: metadata.permissions().readonly(),
            unix_mode,
            #[cfg(target_os = "macos")]
            unix_uid: Some(unix_uid),
            #[cfg(not(target_os = "macos"))]
            unix_uid: None,
            #[cfg(target_os = "macos")]
            unix_gid: Some(unix_gid),
            #[cfg(not(target_os = "macos"))]
            unix_gid: None,
            #[cfg(windows)]
            windows_security: Some(super::project_file_metadata::read(&target)?),
            #[cfg(not(windows))]
            windows_security: None,
        };
        if permissions
            .windows_security
            .as_ref()
            .is_some_and(|security| security.len() > 4096)
        {
            return Err(error("file_transaction_metadata_resource_limit"));
        }
        let mut bytes = Vec::new();
        if metadata.len() > resources::MAX_PROJECT_BYTES {
            return Err(error("file_transaction_resource_limit"));
        }
        bytes
            .try_reserve_exact(metadata.len() as usize)
            .map_err(|_| error("file_transaction_memory_insufficient"))?;
        (&file)
            .take(resources::MAX_PROJECT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("file_transaction_file_read"))?;
        if target
            .metadata()
            .map_err(|_| error("file_transaction_metadata_read"))?
            .len()
            != bytes.len() as u64
            || bytes.len() as u64 > resources::MAX_PROJECT_BYTES
        {
            return Err(error("file_transaction_target_drift"));
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::PermissionsExt;
            if super::project_file_metadata_macos::read(&file)? != (unix_uid, unix_gid)
                || file
                    .metadata()
                    .map_err(|_| error("file_transaction_metadata_read"))?
                    .permissions()
                    .mode()
                    != unix_mode.unwrap()
            {
                return Err(error("file_transaction_target_drift"));
            }
        }
        Ok(Some(FileState {
            bytes,
            metadata: permissions,
        }))
    }
    fn check_prepare(&self, context: &ExecutionWriterContext) -> Result<(), WorkError> {
        if !self.owned {
            LocalWriterLock.require_runtime_idle(context.writer(), LockClass::Execution)?;
        }
        let storage = LocalExecutionStorage {
            project_root: self.root.clone(),
        };
        storage.check_command_context(context, false)?;
        let index: serde_json::Value = serde_json::from_slice(
            &self.read_source(&format!("{}/index.json", context.target().execution_dir))?,
        )
        .map_err(|_| error("file_transaction_index_invalid"))?;
        for manifest in check_retained(
            &self.root,
            context.writer().requirement_id.as_str(),
            context.target().execution_dir,
        )? {
            let binding = Self::binding(&manifest)?;
            if binding.task_id == context.target().task_id
                && index["lock"]["attempt_id"] == binding.attempt_id
            {
                return Err(error("file_transaction_attempt_already_used"));
            }
        }
        Ok(())
    }
    fn apply(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileTransactionPreview,
        evidence: &str,
    ) -> Result<FileTransactionResult, WorkError> {
        with_runtime_writer(
            &LocalWriterLock,
            context.writer(),
            LockClass::Execution,
            |_| {
                Self {
                    root: self.root.clone(),
                    owned: true,
                    instruction_root: self.instruction_root.clone(),
                }
                .publish_scoped(context, preview, evidence, |_| Ok(()))
            },
        )
    }
    fn recovery_preview(
        &self,
        context: &ExecutionWriterContext,
        identity: &str,
    ) -> Result<FileRecoveryPreview, WorkError> {
        if !self.owned {
            LocalWriterLock.require_runtime_idle(context.writer(), LockClass::Execution)?;
        }
        let manifest = self.load(context.writer().requirement_id.as_str(), identity)?;
        // Recovery owns only this transaction. Every unrelated pending allocation is a blocker.
        let namespace = format!(
            "outputs/work/runtime/staging/{}",
            context.writer().requirement_id.as_str()
        );
        for operation in fs::read_dir(self.path(&namespace)?)
            .map_err(|_| error("file_transaction_inventory_read"))?
        {
            let operation = operation.map_err(|_| error("file_transaction_inventory_read"))?;
            let relative = operation
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| error("file_transaction_root"))?
                .to_string_lossy()
                .replace('\\', "/");
            let path = self.path(&relative)?;
            for entry in fs::read_dir(path).map_err(|_| error("file_transaction_inventory_read"))? {
                let entry = entry.map_err(|_| error("file_transaction_inventory_read"))?;
                let name = entry.file_name();
                if operation.file_name() != "project-files" {
                    return Err(error("file_transaction_foreign_pending"));
                }
                let other = self.load(
                    context.writer().requirement_id.as_str(),
                    name.to_str()
                        .ok_or_else(|| error("file_transaction_identity"))?,
                )?;
                if other.transaction_identity != identity
                    && !matches!(
                        other.phase,
                        RuntimePhase::PublishedVerified | RuntimePhase::Restored
                    )
                {
                    return Err(error("file_transaction_foreign_pending"));
                }
            }
        }
        self.validate_context(context, &manifest)?;
        if manifest.phase == RuntimePhase::PublishedVerified {
            return Err(error("file_transaction_already_published"));
        }
        let binding = Self::binding(&manifest)?;
        let before = Self::expected(&manifest, &binding, false)?;
        let after = Self::expected(&manifest, &binding, true)?;
        let observed = self.observed(&manifest)?;
        for (p, s) in &observed {
            if *s != before[p] && *s != after[p] {
                return Err(error("file_transaction_external_change"));
            }
        }
        let approval_sha256 = derive::recovery_approval(&manifest, &observed);
        Ok(FileRecoveryPreview {
            schema: PublicSchema::WorkFileRecoveryPreview,
            manifest,
            observed,
            approval_sha256,
        })
    }
    fn restore(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileRecoveryPreview,
        evidence: &str,
    ) -> Result<FileTransactionResult, WorkError> {
        with_runtime_writer(
            &LocalWriterLock,
            context.writer(),
            LockClass::Execution,
            |_| {
                Self {
                    root: self.root.clone(),
                    owned: true,
                    instruction_root: self.instruction_root.clone(),
                }
                .restore_scoped(context, preview, evidence, |_| Ok(()))
            },
        )
    }
}

/// Retained terminal evidence remains in runtime; pending or corrupt evidence blocks all writers.
pub fn check_retained(
    root: &Path,
    requirement: &str,
    execution: &str,
) -> Result<Vec<RuntimeManifest>, WorkError> {
    let store = LocalFileTransactions::new(root.to_path_buf());
    let namespace = format!("outputs/work/runtime/staging/{requirement}/project-files");
    let path = store.path(&namespace)?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| error("file_transaction_inventory_read"))? {
        let name = entry
            .map_err(|_| error("file_transaction_inventory_read"))?
            .file_name();
        let name = name
            .to_str()
            .ok_or_else(|| error("file_transaction_identity"))?;
        let manifest = store.load(requirement, name)?;
        if manifest.execution_dir != execution {
            return Err(error("file_transaction_context_mismatch"));
        }
        if !matches!(
            manifest.phase,
            RuntimePhase::PublishedVerified | RuntimePhase::Restored
        ) {
            return Err(error("file_transaction_recovery_required"));
        }
        result.push(manifest);
    }
    Ok(result)
}

pub fn require_published(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<(), WorkError> {
    let store = LocalFileTransactions::new(context.writer().canonical_project_root.clone());
    let manifests = check_retained(
        &store.root,
        context.writer().requirement_id.as_str(),
        context.target().execution_dir,
    )?;
    let mut matching = Vec::new();
    for manifest in manifests {
        let b = LocalFileTransactions::binding(&manifest)?;
        if b.task_id == context.target().task_id && b.attempt_id == attempt_id {
            if manifest.phase != RuntimePhase::PublishedVerified {
                return Err(error("file_transaction_restored_attempt"));
            }
            matching.push((manifest, b));
        }
    }
    if matching.len() != 1 {
        return Err(error("file_transaction_publication_required"));
    }
    let (m, b) = &matching[0];
    if store.observed(m)? != LocalFileTransactions::expected(m, b, true)? {
        return Err(error("file_transaction_target_drift"));
    }
    Ok(())
}

/// Old approved record transactions retain forward recovery. A retained project-file
/// transaction for this Attempt must still prove its complete outcome during closure recovery.
pub fn require_retained_publication(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<(), WorkError> {
    for manifest in check_retained(
        &context.writer().canonical_project_root,
        context.writer().requirement_id.as_str(),
        context.target().execution_dir,
    )? {
        let binding = LocalFileTransactions::binding(&manifest)?;
        if binding.task_id == context.target().task_id && binding.attempt_id == attempt_id {
            return require_published(context, attempt_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_inventory_names_and_prefixes_are_bound_before_any_file_write() {
        assert_eq!(temporary_index("publish-1.tmp", 4).unwrap(), (1, true));
        assert_eq!(temporary_index("restore-2.tmp", 4).unwrap(), (2, false));
        #[cfg(any(windows, target_os = "macos"))]
        assert_eq!(
            temporary_index("restore-metadata-2.tmp", 4).unwrap(),
            (2, false)
        );
        for name in [
            "restore-metadata-02.tmp",
            "restore-metadata-4.tmp",
            "restore-metadata--1.tmp",
            "restore-metadata-0.tmp.extra",
            "restore-metadata-0-other.tmp",
            "foreign-0.tmp",
        ] {
            assert_eq!(
                temporary_index(name, 4).unwrap_err().reason_code,
                "file_transaction_inventory_foreign"
            );
        }
        let before = b"complete before evidence";
        for length in 0..=before.len() {
            temporary_prefix(Some(before), &before[..length]).unwrap();
        }
        for (expected, observed) in [
            (Some(before.as_slice()), b"corrupt".as_slice()),
            (None, b"".as_slice()),
        ] {
            assert_eq!(
                temporary_prefix(expected, observed)
                    .unwrap_err()
                    .reason_code,
                "file_transaction_temporary_corrupt"
            );
        }
    }
    #[cfg(target_os = "macos")]
    fn mac_command(program: &str, args: &[&str], path: &Path) {
        let status = std::process::Command::new(program)
            .args(args)
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success(), "metadata fixture failed: {program}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_ownership_survives_publication_and_restoration() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let (store, context, staged) = setup();
        let target = store.root.join("c-existing.txt");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let original = store.inspect("c-existing.txt").unwrap();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let binding = LocalFileTransactions::binding(&preview.manifest).unwrap();
        let metadata = binding.before_metadata["c-existing.txt"].as_ref().unwrap();
        assert_eq!(
            metadata.unix_uid,
            Some(fs::metadata(&target).unwrap().uid())
        );
        assert_eq!(
            metadata.unix_gid,
            Some(fs::metadata(&target).unwrap().gid())
        );
        // All targets were published, but the terminal verification was interrupted.
        // A PublishedVerified transaction is deliberately ineligible for Recovery.
        assert_eq!(
            store
                .fixture_publish(&context, &preview, 4)
                .unwrap_err()
                .reason_code,
            "fixture_interruption"
        );
        let published = store.inspect("c-existing.txt").unwrap().unwrap();
        assert_eq!(published.metadata, original.as_ref().unwrap().metadata);
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        store
            .restore(&context, &recovery, "approve complete restoration")
            .unwrap();
        assert_eq!(store.inspect("c-existing.txt").unwrap(), original);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unsupported_metadata_rejects_before_any_target_write() {
        for (program, args) in [
            (
                "/usr/bin/xattr",
                vec!["-w", "com.work.transaction-test", "evidence"],
            ),
            ("/bin/chmod", vec!["+a", "everyone allow read"]),
            ("/usr/bin/chflags", vec!["hidden"]),
        ] {
            let (store, context, staged) = setup();
            let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
            mac_command(program, &args, &store.root.join("c-existing.txt"));
            assert!(store.apply(&context, &preview, "approve").is_err());
            assert_original(&store);

            let (store, context, staged) = setup();
            mac_command(program, &args, &store.root.join(&staged["c-existing.txt"]));
            assert!(prepare(&store, &context, "ATTEMPT-001", &staged).is_err());
            assert_original(&store);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_inherited_acl_blocks_full_publish_and_restore_preflight() {
        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let staging = store.root.join("outputs/work/runtime/staging");
        fs::create_dir_all(&staging).unwrap();
        mac_command(
            "/bin/chmod",
            &["+a", "everyone allow read,file_inherit,directory_inherit"],
            &staging,
        );
        assert!(store.apply(&context, &preview, "approve").is_err());
        assert_original(&store);

        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        assert_eq!(
            store
                .fixture_publish(&context, &preview, 2)
                .unwrap_err()
                .reason_code,
            "fixture_interruption"
        );
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        let before_restore = store.observed(&preview.manifest).unwrap();
        let dir = runtime_manifest_directory(&store.root, &preview.manifest).unwrap();
        mac_command(
            "/bin/chmod",
            &["+a", "everyone allow read,file_inherit"],
            &dir,
        );
        assert!(
            store
                .restore(&context, &recovery, "approve restore")
                .is_err()
        );
        assert_eq!(store.observed(&preview.manifest).unwrap(), before_restore);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restore_metadata_inventory_validates_exact_names_indices_and_before_prefixes() {
        for (suffix, contents, expected) in [
            ("valid", None, None),
            (
                "corrupt",
                Some(b"foreign data".as_slice()),
                Some("file_transaction_temporary_corrupt"),
            ),
            ("index", None, Some("file_transaction_inventory_foreign")),
            ("name", None, Some("file_transaction_inventory_foreign")),
        ] {
            let (store, context, staged) = setup();
            let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
            assert!(store.fixture_publish(&context, &preview, 0).is_err());
            let index = preview
                .manifest
                .targets
                .iter()
                .position(|t| t.before.is_some())
                .unwrap();
            let bytes = &preview.manifest.targets[index]
                .before
                .as_ref()
                .unwrap()
                .bytes;
            let name = match suffix {
                "index" => format!("restore-metadata-{}.tmp", preview.manifest.targets.len()),
                "name" => format!("restore-metadata-0{index}.tmp"),
                _ => format!("restore-metadata-{index}.tmp"),
            };
            let dir = runtime_manifest_directory(&store.root, &preview.manifest).unwrap();
            let path = dir.join(name);
            let raw = contents.unwrap_or(&bytes[..bytes.len() / 2]);
            LocalFiles.create_new(&path, raw).unwrap();
            let result = store.recovery_preview(&context, &preview.manifest.transaction_identity);
            if let Some(reason) = expected {
                assert_eq!(result.unwrap_err().reason_code, reason);
            } else {
                let recovery = result.unwrap();
                store
                    .restore(&context, &recovery, "Fresh restore approval")
                    .unwrap();
                // Preflight resumes the saved prefix, without treating it as authority.
                assert_eq!(fs::read(&path).unwrap(), *bytes);
            }
            if expected.is_some() {
                assert_eq!(fs::read(&path).unwrap(), raw);
            }
            assert_original(&store);
        }
    }
    fn setup() -> (
        LocalFileTransactions,
        ExecutionWriterContext,
        BTreeMap<String, String>,
    ) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "work-project-files-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let skills = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"))
            .canonicalize()
            .unwrap();
        let context = crate::fixture_support::file_transaction_fixture(&root, &skills).unwrap();
        let staged = ["b-moved.txt", "c-existing.txt", "d-new.txt"]
            .into_iter()
            .map(|p| {
                (
                    p.into(),
                    format!("outputs/work/transactions/example/file-test/staging/{p}"),
                )
            })
            .collect();
        (
            LocalFileTransactions::for_project(root, skills),
            context,
            staged,
        )
    }
    fn assert_original(store: &LocalFileTransactions) {
        assert_eq!(
            store.read_source("a-old.txt").unwrap(),
            b"original move\r\n"
        );
        assert_eq!(
            store.read_source("c-existing.txt").unwrap(),
            b"original modify\0bytes\n"
        );
        assert!(store.inspect("b-moved.txt").unwrap().is_none());
        assert!(store.inspect("d-new.txt").unwrap().is_none());
    }
    #[test]
    fn prepare_is_read_only_and_exact_apply_preserves_whole_set() {
        let (store, context, staged) = setup();
        let runtime = store
            .root
            .join("outputs/work/runtime/staging/example/project-files");
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        assert!(!runtime.exists());
        assert_original(&store);
        let result = store
            .apply(&context, &preview, "User approved full candidate")
            .unwrap();
        assert_eq!(result.phase, RuntimePhase::PublishedVerified);
        assert!(store.inspect("a-old.txt").unwrap().is_none());
        assert_eq!(store.read_source("b-moved.txt").unwrap(), b"final move\n");
        assert_eq!(
            store.read_source("c-existing.txt").unwrap(),
            b"final modify\0\n"
        );
        require_published(&context, "ATTEMPT-001").unwrap();
        assert_eq!(
            store
                .recovery_preview(&context, &result.transaction_identity)
                .unwrap_err()
                .reason_code,
            "file_transaction_already_published"
        );
        assert!(prepare(&store, &context, "ATTEMPT-001", &staged).is_err());
    }
    #[test]
    fn every_publication_boundary_restores_and_every_restore_boundary_reenters() {
        for publish_fault in 0..=4 {
            for restore_fault in 0..=4 {
                let (store, context, staged) = setup();
                let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
                let owned = LocalFileTransactions {
                    root: store.root.clone(),
                    owned: true,
                    instruction_root: store.instruction_root.clone(),
                };
                let result = with_runtime_writer(
                    &LocalWriterLock,
                    context.writer(),
                    LockClass::Execution,
                    |_| {
                        owned.publish_scoped(&context, &preview, "Publish authorization", |n| {
                            if n == publish_fault {
                                Err(error("injected"))
                            } else {
                                Ok(())
                            }
                        })
                    },
                );
                assert!(result.is_err());
                assert!(
                    check_retained(&store.root, "example", context.target().execution_dir).is_err()
                );
                assert!(require_published(&context, "ATTEMPT-001").is_err());
                let recovery = store
                    .recovery_preview(&context, &preview.manifest.transaction_identity)
                    .unwrap();
                let result = with_runtime_writer(
                    &LocalWriterLock,
                    context.writer(),
                    LockClass::Execution,
                    |_| {
                        owned.restore_scoped(
                            &context,
                            &recovery,
                            "Fresh restore authorization",
                            |n| {
                                if n == restore_fault {
                                    Err(error("injected"))
                                } else {
                                    Ok(())
                                }
                            },
                        )
                    },
                );
                assert!(result.is_err());
                let recovery = store
                    .recovery_preview(&context, &preview.manifest.transaction_identity)
                    .unwrap();
                assert_eq!(
                    store
                        .restore(&context, &recovery, "Fresh reentry authorization")
                        .unwrap()
                        .phase,
                    RuntimePhase::Restored
                );
                assert_original(&store);
                assert!(require_published(&context, "ATTEMPT-001").is_err());
                let again = store
                    .recovery_preview(&context, &preview.manifest.transaction_identity)
                    .unwrap();
                store
                    .restore(&context, &again, "Confirm already restored state")
                    .unwrap();
                assert_original(&store);
            }
        }
    }
    #[test]
    fn stale_approval_foreign_inventory_and_external_changes_never_overwrite_targets() {
        let (store, context, mut staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        staged.remove("d-new.txt");
        assert_eq!(
            prepare(&store, &context, "ATTEMPT-001", &staged)
                .unwrap_err()
                .reason_code,
            "file_transaction_incomplete_set"
        );
        fs::write(store.root.join("c-existing.txt"), b"external").unwrap();
        assert_eq!(
            store
                .apply(&context, &preview, "Approved old preview")
                .unwrap_err()
                .reason_code,
            "file_transaction_approval_stale"
        );
        assert!(
            !store
                .root
                .join("outputs/work/runtime/staging/example/project-files")
                .exists()
        );
        assert_eq!(store.read_source("c-existing.txt").unwrap(), b"external");
        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let owned = LocalFileTransactions {
            root: store.root.clone(),
            owned: true,
            instruction_root: store.instruction_root.clone(),
        };
        assert!(
            with_runtime_writer(
                &LocalWriterLock,
                context.writer(),
                LockClass::Execution,
                |_| owned.publish_scoped(&context, &preview, "Publish", |n| if n == 2 {
                    Err(error("injected"))
                } else {
                    Ok(())
                })
            )
            .is_err()
        );
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        fs::write(
            store.root.join("c-existing.txt"),
            b"external changed after preview",
        )
        .unwrap();
        assert!(
            store
                .restore(&context, &recovery, "Restore approval")
                .is_err()
        );
        assert_eq!(
            store.read_source("c-existing.txt").unwrap(),
            b"external changed after preview"
        );
        let dir = runtime_manifest_directory(&store.root, &preview.manifest).unwrap();
        fs::write(dir.join("foreign.txt"), b"foreign evidence").unwrap();
        assert!(
            store
                .recovery_preview(&context, &preview.manifest.transaction_identity)
                .is_err()
        );
        assert_eq!(
            fs::read(dir.join("foreign.txt")).unwrap(),
            b"foreign evidence"
        );
    }
    #[test]
    fn aliases_links_missing_parents_and_readonly_are_rejected_before_publication() {
        let (store, context, staged) = setup();
        assert!(store.inspect("missing-parent/file").is_err());
        fs::hard_link(
            store.root.join("c-existing.txt"),
            store.root.join("alias.txt"),
        )
        .unwrap();
        assert!(prepare(&store, &context, "ATTEMPT-001", &staged).is_err());
        assert!(store.inspect("alias.txt").is_err());
        let (store, context, staged) = setup();
        let path = store.root.join("c-existing.txt");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).unwrap();
        assert!(prepare(&store, &context, "ATTEMPT-001", &staged).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn named_stream_is_rejected_without_changing_targets_or_evidence() {
        let (store, context, staged) = setup();
        let original = fs::read(store.root.join("c-existing.txt")).unwrap();
        let stream = store.root.join("c-existing.txt:extra");
        fs::write(&stream, b"retained alternate stream").unwrap();
        assert!(prepare(&store, &context, "ATTEMPT-001", &staged).is_err());
        assert_eq!(
            fs::read(store.root.join("c-existing.txt")).unwrap(),
            original
        );
        assert_eq!(fs::read(stream).unwrap(), b"retained alternate stream");
        assert!(
            !store
                .root
                .join("outputs/work/runtime/staging/example/project-files")
                .exists()
        );
    }

    #[test]
    fn interrupted_control_prefix_restores_from_observed_set_instead_of_progress_count() {
        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let owned = LocalFileTransactions {
            root: store.root.clone(),
            owned: true,
            instruction_root: store.instruction_root.clone(),
        };
        with_runtime_writer(
            &LocalWriterLock,
            context.writer(),
            LockClass::Execution,
            |_| {
                owned.publish_scoped(&context, &preview, "Publish approval", |_| {
                    Err(error("injected"))
                })
            },
        )
        .unwrap_err();
        let dir = runtime_manifest_directory(&store.root, &preview.manifest).unwrap();
        let mut next = store
            .load("example", &preview.manifest.transaction_identity)
            .unwrap();
        next.published_count = 1;
        let raw = serde_json::to_vec(&next).unwrap();
        fs::write(dir.join("transaction.json.tmp"), &raw[..raw.len() / 2]).unwrap();
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        store
            .restore(
                &context,
                &recovery,
                "New approval of interrupted control and actual targets",
            )
            .unwrap();
        assert_original(&store);
        assert!(require_published(&context, "ATTEMPT-001").is_err());
    }

    #[test]
    fn interrupted_authorization_is_not_approval_and_fresh_restore_preserves_it() {
        for legacy in [false, true] {
            let (store, context, staged) = setup();
            let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
            let binding = LocalFileTransactions::binding(&preview.manifest).unwrap();
            let payloads = BTreeMap::from([(
                "candidate.json".into(),
                derive::candidate_bytes(&binding, &preview.manifest.targets),
            )]);
            let dir =
                prepare_runtime_transaction(&LocalFiles, &store.root, &preview.manifest, &payloads)
                    .unwrap();
            let authorization = FileAuthorization {
                transaction_identity: preview.manifest.transaction_identity.clone(),
                publication_sha256: preview.manifest.approval_sha256.clone(),
                authorization_evidence: "Interrupted approval".into(),
                recovery: None,
            };
            let raw = serde_json::to_vec(&authorization).unwrap();
            let name = derive::authorization_path(&authorization);
            let path = dir.join(if legacy {
                name.clone()
            } else {
                format!("{name}.tmp")
            });
            let prefix = &raw[..raw.len() - 7];
            LocalFiles.create_new(&path, prefix).unwrap();
            let recovery = store
                .recovery_preview(&context, &preview.manifest.transaction_identity)
                .unwrap();
            assert_eq!(recovery.manifest.phase, RuntimePhase::Prepared);
            assert!(require_published(&context, "ATTEMPT-001").is_err());
            store
                .restore(&context, &recovery, "Fresh complete restore approval")
                .unwrap();
            assert_eq!(LocalFiles.read_raw(&path).unwrap(), prefix);
            assert_original(&store);
        }
    }

    #[test]
    fn authorization_temporary_resumes_only_exact_bytes_and_foreign_data_is_retained() {
        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let binding = LocalFileTransactions::binding(&preview.manifest).unwrap();
        let payloads = BTreeMap::from([(
            "candidate.json".into(),
            derive::candidate_bytes(&binding, &preview.manifest.targets),
        )]);
        let dir =
            prepare_runtime_transaction(&LocalFiles, &store.root, &preview.manifest, &payloads)
                .unwrap();
        let authorization = FileAuthorization {
            transaction_identity: preview.manifest.transaction_identity.clone(),
            publication_sha256: preview.manifest.approval_sha256.clone(),
            authorization_evidence: "Exact approval".into(),
            recovery: None,
        };
        let raw = serde_json::to_vec(&authorization).unwrap();
        let name = derive::authorization_path(&authorization);
        let tmp = dir.join(format!("{name}.tmp"));
        LocalFiles.create_new(&tmp, &raw[..raw.len() / 2]).unwrap();
        assert!(!dir.join(&name).exists());
        store
            .authorize(&preview.manifest, "Exact approval", None)
            .unwrap();
        assert_eq!(LocalFiles.read_raw(&dir.join(&name)).unwrap(), raw);
        LocalFiles.create_new(&tmp, b"unknown user data").unwrap();
        assert!(
            store
                .load("example", &preview.manifest.transaction_identity)
                .is_err()
        );
        assert_eq!(LocalFiles.read_raw(&tmp).unwrap(), b"unknown user data");
        assert_original(&store);
    }

    #[test]
    fn large_staged_file_is_rejected_before_loading_or_runtime_creation() {
        let (store, context, staged) = setup();
        let path = store.path(&staged["d-new.txt"]).unwrap();
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(resources::MAX_PROJECT_BYTES + 1)
            .unwrap();
        assert_eq!(
            prepare(&store, &context, "ATTEMPT-001", &staged)
                .unwrap_err()
                .reason_code,
            "file_transaction_resource_limit"
        );
        assert_original(&store);
        assert!(
            !store
                .root
                .join("outputs/work/runtime/staging/example/project-files")
                .exists()
        );
    }

    #[test]
    fn oversized_preserved_before_state_stops_recovery_before_target_writes() {
        let (store, context, staged) = setup();
        let mut manifest = prepare(&store, &context, "ATTEMPT-001", &staged)
            .unwrap()
            .manifest;
        manifest
            .targets
            .iter_mut()
            .find_map(|target| target.before.as_mut())
            .unwrap()
            .bytes
            .resize(resources::MAX_PROJECT_BYTES as usize + 1, 0);
        let dir = store
            .directory("example", &manifest.transaction_identity)
            .unwrap();
        fs::create_dir_all(&dir).unwrap();
        let raw = serde_json::to_vec(&manifest).unwrap();
        LocalFiles
            .create_new(&dir.join("transaction.json"), &raw)
            .unwrap();
        assert_eq!(
            store
                // Exercise the decoded frozen-evidence budget independently from
                // the preceding host-dependent control-JSON allocation gate.
                .load_bytes("example", &manifest.transaction_identity, &raw)
                .unwrap_err()
                .reason_code,
            "file_transaction_resource_limit"
        );
        assert_eq!(
            LocalFiles.read_raw(&dir.join("transaction.json")).unwrap(),
            raw
        );
        assert_original(&store);
    }

    #[test]
    fn interrupted_candidate_restores_but_unknown_manifest_is_preserved() {
        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let binding = LocalFileTransactions::binding(&preview.manifest).unwrap();
        let frozen = derive::candidate_bytes(&binding, &preview.manifest.targets);
        let dir = store
            .directory("example", &preview.manifest.transaction_identity)
            .unwrap();
        fs::create_dir_all(&dir).unwrap();
        LocalFiles
            .create_new(
                &dir.join("transaction.json"),
                &serde_json::to_vec(&preview.manifest).unwrap(),
            )
            .unwrap();
        LocalFiles
            .create_new(&dir.join("candidate.json"), &frozen[..frozen.len() / 2])
            .unwrap();
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        store
            .restore(
                &context,
                &recovery,
                "Fresh approval after candidate interruption",
            )
            .unwrap();
        assert_original(&store);
        let manifest = dir.join("transaction.json");
        let raw = LocalFiles.read_raw(&manifest).unwrap();
        fs::write(&manifest, &raw[..raw.len() / 2]).unwrap();
        assert!(
            store
                .recovery_preview(&context, &preview.manifest.transaction_identity)
                .is_err()
        );
        assert_eq!(
            LocalFiles.read_raw(&manifest).unwrap(),
            raw[..raw.len() / 2]
        );
        assert_original(&store);
    }
}
