//! Durable project file transactions using the existing requirement lock and runtime manifest.
use super::resources;
use crate::execution::storage::LocalExecutionStorage;
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

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalEvidence {
    transaction_identity: String,
    phase: RuntimePhase,
    path_identities: BTreeMap<String, Option<String>>,
}
fn terminal_prefix(phase: RuntimePhase) -> Result<&'static str, WorkError> {
    match phase {
        RuntimePhase::PublishedVerified => Ok("publication"),
        RuntimePhase::Restored => Ok("restoration"),
        _ => Err(error("file_transaction_terminal_evidence_invalid")),
    }
}

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
        if manifest.business_identity.get("staged_files").is_some() {
            return Err(error("file_transaction_migration_required"));
        }
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
                        crate::execution::project_file_metadata::macos::ownership(m)?;
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
        for metadata in binding
            .before_metadata
            .values()
            .chain(binding.after_metadata.values())
            .flatten()
        {
            size = size
                .checked_add(
                    serde_json::to_vec(metadata)
                        .map_err(|_| error("file_transaction_metadata_read"))?
                        .len() as u64,
                )
                .ok_or_else(|| error("file_transaction_resource_limit"))?;
        }
        resources::verify(size, available)?;
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
        let paths: std::collections::BTreeSet<_> =
            manifest.targets.iter().map(|t| &t.path).collect();
        if !binding
            .before_metadata
            .keys()
            .collect::<std::collections::BTreeSet<_>>()
            .eq(&paths)
            || !binding
                .after_metadata
                .keys()
                .collect::<std::collections::BTreeSet<_>>()
                .eq(&paths)
            || !binding
                .path_identities
                .keys()
                .collect::<std::collections::BTreeSet<_>>()
                .eq(&paths)
            || manifest
                .targets
                .iter()
                .any(|t| binding.path_identities[&t.path].is_some() != t.before.is_some())
        {
            return Err(error("file_transaction_metadata_inventory"));
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
            if name.starts_with("publication-") || name.starts_with("restoration-") {
                self.terminal_record(
                    &manifest,
                    &name,
                    &resources::read(&path, resources::MAX_CONTROL_BYTES)?,
                )?;
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
                            != derive::recovery_approval(
                                &recovery.manifest,
                                &recovery.observed,
                                &recovery.observed_identities,
                            )
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
        if matches!(
            manifest.phase,
            RuntimePhase::PublishedVerified | RuntimePhase::Restored
        ) {
            self.terminal_identities(&manifest)?;
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
    fn terminal_record(
        &self,
        manifest: &RuntimeManifest,
        name: &str,
        raw: &[u8],
    ) -> Result<Option<TerminalEvidence>, WorkError> {
        let final_name = name.strip_suffix(".tmp").unwrap_or(name);
        let (prefix, digest) = final_name
            .strip_suffix(".json")
            .and_then(|n| n.split_once('-'))
            .ok_or_else(|| error("file_transaction_terminal_evidence_invalid"))?;
        let phase = match prefix {
            "publication" => RuntimePhase::PublishedVerified,
            "restoration" => RuntimePhase::Restored,
            _ => return Err(error("file_transaction_terminal_evidence_invalid")),
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(error("file_transaction_terminal_evidence_invalid"));
        }
        if name.ends_with(".tmp") {
            let header = format!(
                "{{\"transaction_identity\":{},\"phase\":{},\"path_identities\":",
                serde_json::to_string(&manifest.transaction_identity).expect("identity serializes"),
                serde_json::to_string(&phase).expect("phase serializes")
            );
            let compared = raw.len().min(header.len());
            if raw[..compared] != header.as_bytes()[..compared] {
                return Err(error("file_transaction_terminal_evidence_invalid"));
            }
            return Ok(None);
        }
        let receipt: TerminalEvidence = serde_json::from_slice(raw)
            .map_err(|_| error("file_transaction_terminal_evidence_invalid"))?;
        let binding = Self::binding(manifest)?;
        let expected =
            Self::expected(manifest, &binding, phase == RuntimePhase::PublishedVerified)?;
        if receipt.transaction_identity != manifest.transaction_identity
            || receipt.phase != phase
            || serde_json::to_vec(&receipt).expect("receipt serializes") != raw
            || fingerprint::raw(raw) != digest
            || !receipt.path_identities.keys().eq(expected.keys())
            || expected.iter().any(|(p, s)| {
                receipt.path_identities[p].is_some() != s.is_some()
                    || receipt.path_identities[p]
                        .as_ref()
                        .is_some_and(|id| id.trim().is_empty())
            })
        {
            return Err(error("file_transaction_terminal_evidence_invalid"));
        }
        Ok(Some(receipt))
    }
    fn terminal_identities(
        &self,
        manifest: &RuntimeManifest,
    ) -> Result<BTreeMap<String, Option<String>>, WorkError> {
        let dir = runtime_manifest_directory(&self.root, manifest)?;
        let prefix = format!("{}-", terminal_prefix(manifest.phase)?);
        let mut result = None;
        for entry in fs::read_dir(&dir).map_err(|_| error("file_transaction_inventory_read"))? {
            let entry = entry.map_err(|_| error("file_transaction_inventory_read"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".json") {
                let receipt = self
                    .terminal_record(
                        manifest,
                        &name,
                        &resources::read(
                            &self.path(
                                &entry
                                    .path()
                                    .strip_prefix(&self.root)
                                    .map_err(|_| error("file_transaction_root"))?
                                    .to_string_lossy()
                                    .replace('\\', "/"),
                            )?,
                            resources::MAX_CONTROL_BYTES,
                        )?,
                    )?
                    .ok_or_else(|| error("file_transaction_terminal_evidence_invalid"))?;
                if result.replace(receipt.path_identities).is_some() {
                    return Err(error("file_transaction_terminal_evidence_duplicate"));
                }
            }
        }
        result.ok_or_else(|| error("file_transaction_terminal_evidence_missing"))
    }
    fn commit_terminal(
        &self,
        manifest: &RuntimeManifest,
        phase: RuntimePhase,
        identities: &BTreeMap<String, Option<String>>,
    ) -> Result<(), WorkError> {
        let receipt = TerminalEvidence {
            transaction_identity: manifest.transaction_identity.clone(),
            phase,
            path_identities: identities.clone(),
        };
        let raw = serde_json::to_vec(&receipt).expect("receipt serializes");
        let dir = runtime_manifest_directory(&self.root, manifest)?;
        let name = format!(
            "{}-{}.json",
            terminal_prefix(phase)?,
            fingerprint::raw(&raw)
        );
        let path = dir.join(&name);
        if path.exists() {
            if resources::read(&path, resources::MAX_CONTROL_BYTES)? != raw {
                return Err(error("file_transaction_terminal_evidence_invalid"));
            }
        } else {
            let temporary = dir.join(format!("{name}.tmp"));
            if temporary.exists() {
                let prefix = resources::read(&temporary, resources::MAX_CONTROL_BYTES)?;
                if !raw.starts_with(&prefix) {
                    return Err(error("file_transaction_terminal_evidence_invalid"));
                }
                let mut file = OpenOptions::new()
                    .append(true)
                    .open(&temporary)
                    .map_err(|_| error("file_transaction_terminal_evidence_write"))?;
                file.write_all(&raw[prefix.len()..])
                    .and_then(|_| file.sync_all())
                    .map_err(|_| error("file_transaction_terminal_evidence_write"))?;
            } else {
                LocalFiles.create_new(&temporary, &raw)?;
            }
            if resources::read(&temporary, resources::MAX_CONTROL_BYTES)? != raw {
                return Err(error("file_transaction_terminal_evidence_invalid"));
            }
            crate::execution::control_commit::new(&temporary, &path)?;
        }
        if resources::read(&path, resources::MAX_CONTROL_BYTES)? != raw {
            return Err(error("file_transaction_terminal_evidence_invalid"));
        }
        Ok(())
    }
    fn check_sources(
        &self,
        context: &ExecutionWriterContext,
        binding: &FileTransactionBinding,
        publishing: Option<&RuntimeManifest>,
    ) -> Result<(), WorkError> {
        for path in &binding.directory_inputs {
            if !self.is_directory(path)? {
                return Err(error("file_transaction_source_drift"));
            }
        }
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
            crate::execution::control_commit::new(&temporary, &path)?;
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
            crate::execution::project_file_metadata::macos::write(path, metadata)?;
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
            crate::execution::project_file_metadata::windows::write(
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
        self.publish_tracked(context, preview, evidence, |count, _| checkpoint(count))
    }
    fn publish_tracked(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileTransactionPreview,
        evidence: &str,
        mut checkpoint: impl FnMut(usize, &BTreeMap<String, Option<String>>) -> Result<(), WorkError>,
    ) -> Result<FileTransactionResult, WorkError> {
        if evidence.trim().is_empty() {
            return Err(error("file_transaction_authorization_required"));
        }
        let binding = Self::binding(&preview.manifest)?;
        let current = prepare(
            self,
            context,
            &binding.attempt_id,
            &binding.changes,
            binding.baseline_confirmation.as_ref(),
        )?;
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
        let check_identities = || -> Result<(), WorkError> {
            if !binding.path_identities.keys().eq(before.keys()) {
                return Err(error("file_transaction_path_identity"));
            }
            for (path, expected) in &binding.path_identities {
                if self.path_identity(path)? != *expected {
                    return Err(error("file_transaction_source_drift"));
                }
            }
            Ok(())
        };
        check_identities()?;
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
        check_identities()?;
        self.authorize(manifest, evidence, None)?;
        update_runtime_progress(
            &LocalFiles,
            &self.root,
            manifest,
            0,
            RuntimePhase::Publishing,
        )?;
        let mut frontier = before.clone();
        let mut identities = binding.path_identities.clone();
        checkpoint(0, &identities)?;
        for (n, t) in manifest.targets.iter().enumerate() {
            if self.observed(manifest)? != frontier {
                return Err(error("file_transaction_target_drift"));
            }
            for (path, expected) in &identities {
                if self.path_identity(path)? != *expected {
                    return Err(error("file_transaction_source_drift"));
                }
            }
            self.check_sources(context, &binding, Some(manifest))?;
            self.set_state(&dir, n, &t.path, &before[&t.path], &after[&t.path], false)?;
            frontier.insert(t.path.clone(), after[&t.path].clone());
            identities.insert(t.path.clone(), self.path_identity(&t.path)?);
            checkpoint(n + 1, &identities)?;
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
        for (path, expected) in &identities {
            if self.path_identity(path)? != *expected {
                return Err(error("file_transaction_source_drift"));
            }
        }
        self.commit_terminal(manifest, RuntimePhase::PublishedVerified, &identities)?;
        for (path, expected) in &identities {
            if self.path_identity(path)? != *expected {
                return Err(error("file_transaction_external_change"));
            }
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
    fn publish_with_rollback(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileTransactionPreview,
        evidence: &str,
        mut checkpoint: impl FnMut(usize) -> Result<(), WorkError>,
    ) -> Result<FileTransactionResult, WorkError> {
        let binding = Self::binding(&preview.manifest)?;
        let new_allocation = !runtime_manifest_directory(&self.root, &preview.manifest)?.exists();
        let attempt_path = format!(
            "{}/{}/{}/attempt.json",
            context.target().execution_dir,
            context.target().task_id,
            binding.attempt_id
        );
        let attempt_raw = self.read_source(&attempt_path)?;
        let attempt: serde_json::Value = serde_json::from_slice(&attempt_raw)
            .map_err(|_| error("file_transaction_attempt_invalid"))?;
        let allowed = attempt["authorization"]["scope"]["rollback_on_write_failure"] == true;
        let mut verified = Self::expected(&preview.manifest, &binding, false)?;
        let after = Self::expected(&preview.manifest, &binding, true)?;
        let mut identities = binding.path_identities.clone();
        let result =
            self.publish_tracked(context, preview, evidence, |count, writer_identities| {
                let mut frontier = Self::expected(&preview.manifest, &binding, false)?;
                for target in preview.manifest.targets.iter().take(count) {
                    frontier.insert(target.path.clone(), after[&target.path].clone());
                }
                if self.observed(&preview.manifest)? != frontier {
                    return Err(error("file_transaction_external_change"));
                }
                for (path, expected) in writer_identities {
                    if self.path_identity(path)? != *expected {
                        return Err(error("file_transaction_external_change"));
                    }
                }
                verified = frontier;
                identities = writer_identities.clone();
                checkpoint(count)
            });
        let Err(original) = result else {
            return result;
        };
        if !allowed
            || !new_allocation
            || !runtime_manifest_directory(&self.root, &preview.manifest)?.exists()
        {
            return Err(original);
        }
        let rollback = || -> Result<(), WorkError> {
            if self.read_source(&attempt_path)? != attempt_raw
                || self.observed(&preview.manifest)? != verified
            {
                return Err(error("file_transaction_external_change"));
            }
            for (path, expected) in &identities {
                if self.path_identity(path)? != *expected {
                    return Err(error("file_transaction_external_change"));
                }
            }
            let recovery =
                self.recovery_preview(context, &preview.manifest.transaction_identity)?;
            if recovery.observed != verified {
                return Err(error("file_transaction_external_change"));
            }
            self.restore_scoped(context, &recovery, evidence, |_| Ok(()))?;
            Ok(())
        };
        match rollback() {
            Ok(()) => Err(WorkError::new(
                work_feature::error::ExitCode::ArtifactIntegrity,
                "file_transaction_write_failed_restored",
                "The current write failed and only this invocation's changes were restored.",
                serde_json::json!({"original_reason":original.reason_code,"phase":"restored","recovery_required":false,
                    "transaction_identity":preview.manifest.transaction_identity}),
            )),
            Err(failure) => Err(WorkError::new(
                work_feature::error::ExitCode::ArtifactIntegrity,
                "file_transaction_rollback_blocked",
                "Preserve all evidence and current contents; the current write cannot be safely restored.",
                serde_json::json!({"original_reason":original.reason_code,"rollback_reason":failure.reason_code,
                    "recovery_required":true,"transaction_identity":preview.manifest.transaction_identity}),
            )),
        }
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
        let restore_identities = current.observed_identities.clone();
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
        for (path, expected) in &restore_identities {
            if self.path_identity(path)? != *expected {
                return Err(error("file_transaction_external_change"));
            }
        }
        let mut frontier = observed.clone();
        let mut identities = restore_identities;
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
                if self.observed(&manifest)? != frontier {
                    return Err(error("file_transaction_external_change"));
                }
                for (path, expected) in &identities {
                    if self.path_identity(path)? != *expected {
                        return Err(error("file_transaction_external_change"));
                    }
                }
                self.set_state(&dir, n, &t.path, &observed[&t.path], &before[&t.path], true)?;
                frontier.insert(t.path.clone(), before[&t.path].clone());
                identities.insert(t.path.clone(), self.path_identity(&t.path)?);
                checkpoint(n + 1)?;
            }
            if self.observed(&manifest)? != before {
                return Err(error("file_transaction_restore_readback"));
            }
            self.commit_terminal(&manifest, RuntimePhase::Restored, &identities)?;
            for (path, expected) in &identities {
                if self.path_identity(path)? != *expected {
                    return Err(error("file_transaction_external_change"));
                }
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
    fn input_paths(
        &self,
        context: &ExecutionWriterContext,
        task: &serde_json::Value,
        attempt: &str,
    ) -> Result<Vec<String>, WorkError> {
        resolved_input_paths(context, task, attempt)
    }
    fn migration_review_sources(
        &self,
        context: &ExecutionWriterContext,
    ) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
        crate::specification::migration::verification::task_review_sources(context)
    }
    fn is_directory(&self, path: &str) -> Result<bool, WorkError> {
        let metadata = fs::symlink_metadata(self.path(path)?)
            .map_err(|_| error("file_transaction_input_unverified"))?;
        Ok(metadata.is_dir())
    }
    fn accepted_file_scopes(
        &self,
        context: &ExecutionWriterContext,
    ) -> Result<BTreeMap<String, Vec<String>>, WorkError> {
        let mut scopes = BTreeMap::<String, Vec<String>>::new();
        let storage = LocalExecutionStorage {
            project_root: self.root.clone(),
        };
        for task in context.task_context().collection["tasks"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let id = task["id"]
                .as_str()
                .ok_or_else(|| error("file_transaction_task_missing"))?;
            for name in work_feature::execution::AttemptStartRepository::attempt_names(
                &storage,
                context.target().execution_dir,
                id,
            )? {
                let path = format!(
                    "{}/{id}/{name}/attempt.json",
                    context.target().execution_dir
                );
                let raw = self.read_source(&path)?;
                let saved = work_operations::canonical::parse_json_contract(&raw)
                    .map_err(|_| error("file_transaction_source_invalid"))?;
                work_operations::execution::attempt::validate_attempt_bytes(&saved, &raw)
                    .map_err(|e| error(e.reason_code))?;
                for deviation in saved["execution_deviations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if deviation["decision"]["outcome"] == "approved"
                        && work_operations::execution::deviation_reconciliation_target(
                            &deviation["proposal"],
                        ) == "task_only"
                    {
                        scopes.entry(id.into()).or_default().extend(
                            deviation["supplemental_authorization"]["modifiable_files"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|p| p.as_str().map(str::to_owned)),
                        );
                    }
                }
            }
        }
        for manifest in check_retained(
            &self.root,
            context.writer().requirement_id.as_str(),
            context.target().execution_dir,
        )? {
            let binding = Self::binding(&manifest)?;
            // Restored attempts also accepted this scope; recovery does not erase ordering obligations.
            let paths = scopes.entry(binding.task_id).or_default();
            for target in manifest.targets {
                if !paths.contains(&target.path) {
                    paths.push(target.path);
                }
            }
        }
        Ok(scopes)
    }
    fn history(
        &self,
        context: &ExecutionWriterContext,
        attempt_id: &str,
    ) -> Result<work_operations::execution::file_transaction::history::Current, WorkError> {
        retained_history(context, attempt_id)
    }
    fn new_file_metadata(&self, path: &str) -> Result<FileMetadata, WorkError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let target = self.path(path)?;
        let parent = target
            .parent()
            .filter(|p| p.is_dir())
            .ok_or_else(|| error("file_transaction_parent_missing"))?;
        let name = format!(
            ".work-file-metadata-{}-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| error("file_transaction_metadata_unavailable"))?
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let probe = parent.join(name);
        let relative = probe
            .strip_prefix(&self.root)
            .map_err(|_| error("file_transaction_root"))?
            .to_string_lossy()
            .replace('\\', "/");
        LocalFiles.create_new(&probe, &[])?;
        let identity = self.path_identity(&relative)?;
        let measured = self.inspect(&relative);
        // Only this exact still-empty file created by this invocation is disposable.
        if self.path_identity(&relative)? != identity
            || fs::metadata(&probe)
                .map_err(|_| error("file_transaction_metadata_read"))?
                .len()
                != 0
        {
            return Err(error("file_transaction_metadata_probe_changed"));
        }
        fs::remove_file(&probe).map_err(|_| error("file_transaction_metadata_probe_cleanup"))?;
        measured?
            .map(|s| s.metadata)
            .ok_or_else(|| error("file_transaction_metadata_unavailable"))
    }

    fn check_complete_resources(&self, weighted_bytes: u64) -> Result<(), WorkError> {
        let available = fs4::available_space(&self.root)
            .map_err(|_| error("file_transaction_storage_unverifiable"))?;
        resources::verify(weighted_bytes, available)
    }
    fn path_identity(&self, path: &str) -> Result<Option<String>, WorkError> {
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = path;
            Err(error("file_transaction_platform_unsupported"))
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            let path = self.path(path)?;
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.custom_flags(0x00200000);
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(0x20000000 | 0x01000000 | 0x4);
            }
            let file = match options.open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(_) => return Err(error("file_transaction_identity_unavailable")),
            };
            let metadata = file
                .metadata()
                .map_err(|_| error("file_transaction_identity_unavailable"))?;
            if !metadata.is_file() {
                return Err(error("file_transaction_target_unsupported"));
            }
            #[cfg(windows)]
            {
                let (attributes, links) =
                    crate::files::windows_runtime_metadata::descriptor_attributes_and_links(&file)
                        .map_err(|_| error("file_transaction_identity_unavailable"))?;
                if attributes & 0x400 != 0 || links != 1 {
                    return Err(error("file_transaction_path_alias"));
                }
                let id = crate::files::windows_runtime_metadata::descriptor_identity(&file)
                    .map_err(|_| error("file_transaction_identity_unavailable"))?;
                Ok(Some(format!(
                    "windows:{:08x}:{:08x}:{:08x}:{:08x}:{:08x}",
                    id[0], id[1], id[2], id[3], id[4]
                )))
            }
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() != 1 {
                    return Err(error("file_transaction_path_alias"));
                }
                Ok(Some(format!("macos:{}:{}", metadata.dev(), metadata.ino())))
            }
        }
    }

    fn check_resources(&self, paths: &[String]) -> Result<(), WorkError> {
        let mut size = 0_u64;
        for path in paths {
            let target = self.path(path)?;
            let length = match fs::metadata(&target) {
                Ok(metadata) => metadata.len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                Err(_) => return Err(error("file_transaction_file_read")),
            };
            #[cfg(target_os = "macos")]
            if target.is_file() {
                let file =
                    fs::File::open(&target).map_err(|_| error("file_transaction_metadata_read"))?;
                for (name, value) in
                    crate::execution::project_file_metadata::macos::read_xattrs(&file)?
                {
                    size = size
                        .checked_add(name.len() as u64)
                        .and_then(|size| size.checked_add(value.len() as u64))
                        .ok_or_else(|| error("file_transaction_resource_limit"))?;
                }
            }
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
        let (unix_uid, unix_gid) = crate::execution::project_file_metadata::macos::read(&file)?;
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
            windows_security: Some(crate::execution::project_file_metadata::windows::read(
                &target,
            )?),
            #[cfg(not(windows))]
            windows_security: None,
            #[cfg(target_os = "macos")]
            macos_xattrs: Some(crate::execution::project_file_metadata::macos::read_xattrs(
                &file,
            )?),
            #[cfg(not(target_os = "macos"))]
            macos_xattrs: None,
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
            if crate::execution::project_file_metadata::macos::read(&file)? != (unix_uid, unix_gid)
                || Some(crate::execution::project_file_metadata::macos::read_xattrs(
                    &file,
                )?) != permissions.macos_xattrs
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
                .publish_with_rollback(context, preview, evidence, |_| Ok(()))
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
        let observed_identities: BTreeMap<_, _> = manifest
            .targets
            .iter()
            .map(|t| Ok((t.path.clone(), self.path_identity(&t.path)?)))
            .collect::<Result<_, WorkError>>()?;
        let observed = self.observed(&manifest)?;
        for (p, s) in &observed {
            if *s != before[p] && *s != after[p] {
                return Err(error("file_transaction_external_change"));
            }
        }
        for (path, expected) in &observed_identities {
            if self.path_identity(path)? != *expected {
                return Err(error("file_transaction_external_change"));
            }
        }
        let approval_sha256 = derive::recovery_approval(&manifest, &observed, &observed_identities);
        Ok(FileRecoveryPreview {
            schema: PublicSchema::WorkFileRecoveryPreview,
            manifest,
            observed,
            observed_identities,
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
    let mut names = Vec::new();
    let mut budget = 0_u64;
    for entry in fs::read_dir(&path).map_err(|_| error("file_transaction_inventory_read"))? {
        let name = entry
            .map_err(|_| error("file_transaction_inventory_read"))?
            .file_name();
        let name = name
            .to_str()
            .ok_or_else(|| error("file_transaction_identity"))?
            .to_owned();
        let directory = store.directory(requirement, &name)?;
        let manifest = store.path(
            &directory
                .join("transaction.json")
                .strip_prefix(&store.root)
                .map_err(|_| error("file_transaction_root"))?
                .to_string_lossy()
                .replace('\\', "/"),
        )?;
        let length = fs::metadata(manifest)
            .map_err(|_| error("file_transaction_inventory_read"))?
            .len();
        if length > resources::MAX_CONTROL_BYTES {
            return Err(error("file_transaction_resource_limit"));
        }
        budget = budget
            .checked_add(length)
            .and_then(|n| n.checked_add(name.len() as u64 + 4096))
            .ok_or_else(|| error("file_transaction_memory_insufficient"))?;
        resources::history_memory(budget)?;
        names.push(name);
    }
    let mut result = Vec::new();
    for name in &names {
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

fn attempt_lineage(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<std::collections::BTreeSet<String>, WorkError> {
    let store = LocalFileTransactions::new(context.writer().canonical_project_root.clone());
    let mut lineage = std::collections::BTreeSet::new();
    let mut id = attempt_id.to_owned();
    let mut child: Option<serde_json::Value> = None;
    loop {
        if !lineage.insert(id.clone()) {
            return Err(error("file_transaction_continuation_invalid"));
        }
        let path = format!(
            "{}/{}/{id}/attempt.json",
            context.target().execution_dir,
            context.target().task_id
        );
        let raw = store.read_source(&path)?;
        let attempt = work_operations::canonical::parse_json_contract(&raw)
            .map_err(|_| error("file_transaction_continuation_invalid"))?;
        work_operations::execution::attempt::validate_attempt_bytes(&attempt, &raw)
            .map_err(|e| error(e.reason_code))?;
        if attempt["task_id"] != context.target().task_id || attempt["attempt_id"] != id {
            return Err(error("file_transaction_continuation_invalid"));
        }
        if let Some(child) = &child {
            if attempt["status"] != "stopped"
                || work_operations::execution::closed_task_status(
                    "stopped",
                    attempt["final_type"].as_str(),
                ) != "pending_retry"
                || [
                    "task_item_sha256",
                    "task_collection_sha256",
                    "task_index_sha256",
                    "task_instructions_sha256",
                ]
                .iter()
                .any(|key| attempt[*key] != child[*key])
            {
                return Err(error("file_transaction_continuation_invalid"));
            }
        }
        let Some(previous) = attempt["continued_from"].as_str() else {
            break;
        };
        if previous >= id.as_str() {
            return Err(error("file_transaction_continuation_invalid"));
        }
        id = previous.to_owned();
        child = Some(attempt);
    }
    Ok(lineage)
}

/// Follow only complete, authenticated publications, in predecessor order.
/// Each move in a publication maps from that publication's initial tree.
pub(crate) fn resolved_input_paths(
    context: &ExecutionWriterContext,
    task: &serde_json::Value,
    attempt_id: &str,
) -> Result<Vec<String>, WorkError> {
    let mut paths = work_operations::execution::file_transaction::input_paths(
        &context.task_context().contract,
        task,
    )
    .map_err(|e| error(e.reason_code))?;
    if paths.is_empty() {
        return Ok(paths);
    }
    let current = retained_history(context, attempt_id)?;
    let lineage = attempt_lineage(context, attempt_id)?;
    let mut transactions = BTreeMap::new();
    for manifest in check_retained(
        &context.writer().canonical_project_root,
        context.writer().requirement_id.as_str(),
        context.target().execution_dir,
    )? {
        let binding = LocalFileTransactions::binding(&manifest)?;
        if binding.task_id == context.target().task_id && lineage.contains(&binding.attempt_id) {
            transactions.insert(
                manifest.transaction_identity.clone(),
                (manifest.phase, binding),
            );
        }
    }
    let mut chain = Vec::new();
    let mut head = (!current.head.is_empty()).then_some(current.head.as_str());
    while let Some(id) = head {
        let row = transactions
            .get(id)
            .ok_or_else(|| error("file_transaction_history_disconnected"))?;
        chain.push(row);
        head = row.1.previous_transaction.as_deref();
    }
    for (phase, binding) in chain.into_iter().rev() {
        if *phase != RuntimePhase::PublishedVerified {
            continue;
        }
        for path in &mut paths {
            if let Some(destination) = binding.changes.iter().find_map(|change| match change {
                FileChange::Move {
                    source,
                    destination,
                    ..
                } if source == path => Some(destination),
                _ => None,
            }) {
                *path = destination.clone();
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(crate) fn continuation_baseline(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<BTreeMap<String, bool>, WorkError> {
    current_version(context, attempt_id)?;
    Ok(retained_history(context, attempt_id)?
        .initial
        .into_iter()
        .map(|(path, state)| (path, state.is_some()))
        .collect())
}

fn retained_history(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<work_operations::execution::file_transaction::history::Current, WorkError> {
    use work_operations::execution::file_transaction::history::{self, Entry};
    let store = LocalFileTransactions::new(context.writer().canonical_project_root.clone());
    let lineage = attempt_lineage(context, attempt_id)?;
    let mut frozen = Vec::new();
    for manifest in check_retained(
        &store.root,
        context.writer().requirement_id.as_str(),
        context.target().execution_dir,
    )? {
        let binding = LocalFileTransactions::binding(&manifest)?;
        if binding.task_id == context.target().task_id && lineage.contains(&binding.attempt_id) {
            let before = LocalFileTransactions::expected(&manifest, &binding, false)?;
            let after = LocalFileTransactions::expected(&manifest, &binding, true)?;
            let effective_identities = store.terminal_identities(&manifest)?;
            frozen.push((manifest, binding, before, after, effective_identities));
        }
    }
    let entries: Vec<_> = frozen
        .iter()
        .map(
            |(manifest, binding, before, after, effective_identities)| Entry {
                identity: &manifest.transaction_identity,
                previous: binding.previous_transaction.as_deref(),
                confirmation: binding.baseline_confirmation.as_ref(),
                path_identities: &binding.path_identities,
                effective_identities,
                phase: manifest.phase,
                before,
                after,
            },
        )
        .collect();
    history::current(&entries).map_err(error)
}

/// Resolve the exact version owned by a historical receipt without claiming it validates current files.
pub(in crate::execution) fn version_snapshot(
    context: &ExecutionWriterContext,
    attempt_id: &str,
    wanted: &str,
) -> Result<BTreeMap<String, Option<FileState>>, WorkError> {
    use work_operations::execution::file_transaction::history::{self, Entry};
    let current = retained_history(context, attempt_id)?;
    let store = LocalFileTransactions::new(context.writer().canonical_project_root.clone());
    let lineage = attempt_lineage(context, attempt_id)?;
    let mut transactions = BTreeMap::new();
    for manifest in check_retained(
        &store.root,
        context.writer().requirement_id.as_str(),
        context.target().execution_dir,
    )? {
        let binding = LocalFileTransactions::binding(&manifest)?;
        if binding.task_id == context.target().task_id && lineage.contains(&binding.attempt_id) {
            let before = LocalFileTransactions::expected(&manifest, &binding, false)?;
            let after = LocalFileTransactions::expected(&manifest, &binding, true)?;
            transactions.insert(manifest.transaction_identity.clone(), {
                let effective_identities = store.terminal_identities(&manifest)?;
                (manifest, binding, before, after, effective_identities)
            });
        }
    }
    let mut chain = Vec::new();
    let mut head = (!current.head.is_empty()).then_some(current.head.as_str());
    while let Some(id) = head {
        let row = transactions
            .get(id)
            .ok_or_else(|| error("file_transaction_history_disconnected"))?;
        chain.push(row);
        head = row.1.previous_transaction.as_deref();
    }
    chain.reverse();
    for count in 0..=chain.len() {
        let entries: Vec<_> = chain
            .iter()
            .take(count)
            .map(
                |(manifest, binding, before, after, effective_identities)| Entry {
                    identity: &manifest.transaction_identity,
                    previous: binding.previous_transaction.as_deref(),
                    confirmation: binding.baseline_confirmation.as_ref(),
                    path_identities: &binding.path_identities,
                    effective_identities,
                    phase: manifest.phase,
                    before,
                    after,
                },
            )
            .collect();
        let past = history::current(&entries).map_err(error)?;
        if history::version(&past) == wanted {
            let mut snapshot = current.initial;
            snapshot.extend(past.expected);
            return Ok(snapshot);
        }
    }
    Err(error("file_transaction_evidence_version_unknown"))
}

pub(in crate::execution) fn current_version(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<String, WorkError> {
    let store = LocalFileTransactions::new(context.writer().canonical_project_root.clone());
    let current = retained_history(context, attempt_id)?;
    for (path, expected) in &current.expected {
        if store.inspect(path)? != *expected
            || store.path_identity(path)? != current.path_identities[path]
        {
            return Err(error("file_transaction_target_drift"));
        }
    }
    Ok(work_operations::execution::file_transaction::history::version(&current))
}

/// Ground an AI diff reference in a complete retained transaction of this exact owner.
pub(in crate::execution) fn verify_change_reference(
    context: &ExecutionWriterContext,
    attempt_id: &str,
    transaction: &str,
    path: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<(), WorkError> {
    retained_history(context, attempt_id)?;
    let store = LocalFileTransactions::new(context.writer().canonical_project_root.clone());
    let manifest = store.load(context.writer().requirement_id.as_str(), transaction)?;
    let binding = LocalFileTransactions::binding(&manifest)?;
    if binding.task_id != context.target().task_id
        || binding.attempt_id != attempt_id
        || manifest.phase != RuntimePhase::PublishedVerified
    {
        return Err(error("validation_review_reference_unverified"));
    }
    let target = manifest
        .targets
        .iter()
        .find(|target| target.path == path)
        .ok_or_else(|| error("validation_review_reference_unverified"))?;
    if target.before.as_ref().map(|b| b.sha256.as_str()) != before
        || target.after.as_ref().map(|b| b.sha256.as_str()) != after
    {
        return Err(error("validation_review_reference_unverified"));
    }
    Ok(())
}

pub fn require_published(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<(), WorkError> {
    published_before(context, attempt_id).map(|_| ())
}

/// Original input bytes require complete retained publication from the verified
/// continuation lineage and current after bytes, metadata and path identities.
pub(crate) fn published_before(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<BTreeMap<String, Option<FileState>>, WorkError> {
    let current = retained_history(context, attempt_id)?;
    if current.published_paths.is_empty() {
        return Err(error("file_transaction_publication_required"));
    }
    let task = context.task_context().contract["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|task| task["id"] == context.target().task_id)
        .ok_or_else(|| error("file_transaction_task_missing"))?;
    for file in task["files"].as_array().into_iter().flatten() {
        for path in ["path", "source", "destination"]
            .iter()
            .filter_map(|key| file[*key].as_str())
        {
            if !current.published_paths.contains(path) {
                return Err(error("file_transaction_incomplete_set"));
            }
        }
    }
    current_version(context, attempt_id)?;
    Ok(current.initial)
}

/// Old approved record transactions retain forward recovery. A retained project-file
/// transaction for this Attempt must still prove its complete outcome during closure recovery.
pub fn require_retained_publication(
    context: &ExecutionWriterContext,
    attempt_id: &str,
) -> Result<(), WorkError> {
    let lineage = attempt_lineage(context, attempt_id)?;
    for manifest in check_retained(
        &context.writer().canonical_project_root,
        context.writer().requirement_id.as_str(),
        context.target().execution_dir,
    )? {
        let binding = LocalFileTransactions::binding(&manifest)?;
        if binding.task_id == context.target().task_id && lineage.contains(&binding.attempt_id) {
            return require_published(context, attempt_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // This helper loads synthetic fixture bytes; production requests contain changes directly.
    fn prepare(
        store: &LocalFileTransactions,
        context: &ExecutionWriterContext,
        attempt: &str,
        fixtures: &BTreeMap<String, String>,
    ) -> Result<FileTransactionPreview, WorkError> {
        let read = |path: &str| -> Result<Vec<u8>, WorkError> {
            store
                .inspect(&fixtures[path])?
                .map(|s| s.bytes)
                .ok_or_else(|| error("file_transaction_fixture_missing"))
        };
        let changes = vec![
            FileChange::Move {
                source: "a-old.txt".into(),
                destination: "b-moved.txt".into(),
                bytes: Some(read("b-moved.txt")?),
            },
            FileChange::Write {
                path: "c-existing.txt".into(),
                bytes: read("c-existing.txt")?,
            },
            FileChange::Write {
                path: "d-new.txt".into(),
                bytes: read("d-new.txt")?,
            },
        ];
        work_feature::execution::file_transaction::prepare(store, context, attempt, &changes, None)
    }

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
    fn assert_inherited_acl(path: &Path) {
        let output = std::process::Command::new("/bin/ls")
            .args(["-lde"])
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success());
        let evidence = String::from_utf8(output.stdout).unwrap();
        assert!(
            evidence
                .lines()
                .any(|line| line.contains("inherited allow read")
                    && (line.contains("everyone")
                        || line.contains("ABCDEFAB-CDEF-ABCD-EFAB-CDEF0000000C"))),
            "{evidence}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_ownership_survives_publication_and_restoration() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        for (before_mode, after_mode) in [(0o640, 0o644), (0o644, 0o640)] {
            let (store, context, staged) = setup();
            let target = store.root.join("c-existing.txt");
            fs::set_permissions(&target, fs::Permissions::from_mode(before_mode)).unwrap();
            fs::set_permissions(
                store.root.join(&staged["c-existing.txt"]),
                fs::Permissions::from_mode(after_mode),
            )
            .unwrap();
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
            let expected_after =
                LocalFileTransactions::expected(&preview.manifest, &binding, true).unwrap();
            assert_eq!(Some(&published), expected_after["c-existing.txt"].as_ref());
            assert_eq!(published.metadata.unix_mode.unwrap() & 0o777, before_mode);
            assert_eq!(
                binding.after_metadata["c-existing.txt"],
                binding.before_metadata["c-existing.txt"]
            );
            assert_eq!(published.metadata.unix_uid, metadata.unix_uid);
            assert_eq!(published.metadata.unix_gid, metadata.unix_gid);
            let recovery = store
                .recovery_preview(&context, &preview.manifest.transaction_identity)
                .unwrap();
            store
                .restore(&context, &recovery, "approve complete restoration")
                .unwrap();
            assert_eq!(store.inspect("c-existing.txt").unwrap(), original);
            assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, before_mode);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unsupported_metadata_rejects_before_any_target_write() {
        for (program, args) in [
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
    fn macos_xattrs_are_bound_preserved_and_restored_with_bytes() {
        let (store, context, staged) = setup();
        let target = store.root.join("c-existing.txt");
        mac_command(
            "/usr/bin/xattr",
            &["-wx", "com.work.before", "000aff"],
            &target,
        );
        mac_command(
            "/usr/bin/xattr",
            &["-wx", "com.work.after", "ff0a00"],
            &store.root.join(&staged["c-existing.txt"]),
        );
        let original = store.inspect("c-existing.txt").unwrap();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let binding = LocalFileTransactions::binding(&preview.manifest).unwrap();
        assert_eq!(
            binding.before_metadata["c-existing.txt"]
                .as_ref()
                .unwrap()
                .macos_xattrs
                .as_ref()
                .unwrap()["com.work.before"],
            [0, 10, 255]
        );
        assert_eq!(
            store
                .fixture_publish(&context, &preview, preview.manifest.targets.len())
                .unwrap_err()
                .reason_code,
            "fixture_interruption"
        );
        let expected = LocalFileTransactions::expected(&preview.manifest, &binding, true).unwrap();
        assert_eq!(
            store.inspect("c-existing.txt").unwrap(),
            expected["c-existing.txt"]
        );
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        store
            .restore(&context, &recovery, "approve complete xattr restoration")
            .unwrap();
        assert_eq!(store.inspect("c-existing.txt").unwrap(), original);

        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        mac_command(
            "/usr/bin/xattr",
            &["-w", "com.work.drift", "changed"],
            &store.root.join("c-existing.txt"),
        );
        let changed = store.inspect("c-existing.txt").unwrap();
        assert!(
            store
                .apply(&context, &preview, "approve stale metadata")
                .is_err()
        );
        assert_eq!(store.inspect("c-existing.txt").unwrap(), changed);
        assert_original(&store);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_inherited_acl_blocks_full_publish_and_restore_preflight() {
        let (store, context, staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        let before_publish = store.observed(&preview.manifest).unwrap();
        let dir = runtime_manifest_directory(&store.root, &preview.manifest).unwrap();
        let staging = dir.parent().unwrap();
        fs::create_dir_all(staging).unwrap();
        mac_command(
            "/bin/chmod",
            &["+a", "everyone allow read,file_inherit,directory_inherit"],
            staging,
        );
        let probe = staging.join("acl-inheritance-probe");
        fs::write(&probe, b"ACL fixture proof").unwrap();
        assert_inherited_acl(&probe);
        // Retain proof outside the runtime namespace: every entry there must be a real transaction.
        fs::rename(&probe, store.root.join("acl-inheritance-proof")).unwrap();
        let rejection = store.apply(&context, &preview, "approve").unwrap_err();
        assert_eq!(rejection.reason_code, "file_transaction_acl_unsupported");
        let (index, target) = preview
            .manifest
            .targets
            .iter()
            .enumerate()
            .find(|(_, target)| target.after.is_some() || target.before.is_some())
            .unwrap();
        let direction = if target.after.is_some() {
            "publish"
        } else {
            "restore"
        };
        assert_inherited_acl(&dir.join(format!("{direction}-{index}.tmp")));
        assert_original(&store);
        assert_eq!(store.observed(&preview.manifest).unwrap(), before_publish);

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
        let index = preview
            .manifest
            .targets
            .iter()
            .position(|target| target.before.is_some())
            .unwrap();
        let proof = dir.join(format!("restore-metadata-{index}.tmp"));
        fs::write(
            &proof,
            &preview.manifest.targets[index]
                .before
                .as_ref()
                .unwrap()
                .bytes,
        )
        .unwrap();
        assert_inherited_acl(&proof);
        let rejection = store
            .restore(&context, &recovery, "approve restore")
            .unwrap_err();
        assert_eq!(rejection.reason_code, "file_transaction_acl_unsupported");
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
        setup_scoped(None)
    }
    fn setup_scoped(
        rollback: Option<bool>,
    ) -> (
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
        let context = match rollback {
            Some(allowed) => {
                crate::fixture_support::file_transaction_scope_fixture(&root, &skills, allowed)
            }
            None => crate::fixture_support::file_transaction_fixture(&root, &skills),
        }
        .unwrap();
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
    fn current_path_identity_distinguishes_replacements_and_rejects_aliases() {
        let (store, _context, _staged) = setup();
        let path = "c-existing.txt";
        let old = store.path_identity(path).unwrap().unwrap();
        let bytes = store.read_source(path).unwrap();
        let temporary = store.root.join("replacement-version.txt");
        LocalFiles.create_new(&temporary, &bytes).unwrap();
        LocalFiles
            .replace(&temporary, &store.root.join(path))
            .unwrap();
        let current = store.path_identity(path).unwrap().unwrap();
        assert_ne!(
            old, current,
            "Identical bytes are not the same physical file"
        );
        assert_eq!(store.read_source(path).unwrap(), bytes);
        assert_eq!(store.path_identity("new-absent.txt").unwrap(), None);
        assert_eq!(
            store
                .check_complete_resources(resources::MAX_PROJECT_BYTES + 1)
                .unwrap_err()
                .reason_code,
            "file_transaction_resource_limit"
        );
        fs::hard_link(store.root.join(path), store.root.join("identity-alias.txt")).unwrap();
        assert!(store.path_identity(path).is_err());
        assert_eq!(fs::read(store.root.join(path)).unwrap(), bytes);
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
    fn normal_output_bytes_are_preserved_and_duplicate_destinations_are_refused() {
        let (store, context, staged) = setup();
        fs::create_dir(store.root.join("tool-output")).unwrap();
        let normal: BTreeMap<_, _> = staged
            .iter()
            .map(|(target, source)| {
                let source_path = format!("tool-output/{target}");
                fs::write(
                    store.root.join(&source_path),
                    store.read_source(source).unwrap(),
                )
                .unwrap();
                (target.clone(), source_path)
            })
            .collect();
        let preview = prepare(&store, &context, "ATTEMPT-001", &normal).unwrap();
        assert_original(&store);
        assert!(
            !store
                .root
                .join("outputs/work/runtime/staging/example/project-files")
                .exists()
        );
        let mut ambiguous = LocalFileTransactions::binding(&preview.manifest)
            .unwrap()
            .changes;
        ambiguous.push(FileChange::Write {
            path: "c-existing.txt".into(),
            bytes: b"ambiguous second destination".into(),
        });
        assert_eq!(
            work_feature::execution::file_transaction::prepare(
                &store,
                &context,
                "ATTEMPT-001",
                &ambiguous,
                None
            )
            .unwrap_err()
            .reason_code,
            "file_transaction_destination_duplicate"
        );
        store
            .apply(
                &context,
                &preview,
                "Approve complete candidate from normal output",
            )
            .unwrap();
        for (target, source) in normal {
            assert_eq!(
                store.read_source(&target).unwrap(),
                store.read_source(&source).unwrap()
            );
        }
    }
    fn every_publication_boundary_restores_and_every_restore_boundary_reenters(shard: usize) {
        for publish_fault in 0..=4 {
            if publish_fault != shard {
                continue;
            }
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
    fn publication_and_restore_boundaries_0() {
        every_publication_boundary_restores_and_every_restore_boundary_reenters(0);
    }

    #[test]
    fn publication_and_restore_boundaries_1() {
        every_publication_boundary_restores_and_every_restore_boundary_reenters(1);
    }

    #[test]
    fn publication_and_restore_boundaries_2() {
        every_publication_boundary_restores_and_every_restore_boundary_reenters(2);
    }

    #[test]
    fn publication_and_restore_boundaries_3() {
        every_publication_boundary_restores_and_every_restore_boundary_reenters(3);
    }

    #[test]
    fn publication_and_restore_boundaries_4() {
        every_publication_boundary_restores_and_every_restore_boundary_reenters(4);
    }
    #[test]
    fn stale_approval_foreign_inventory_and_external_changes_never_overwrite_targets() {
        let (store, context, mut staged) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &staged).unwrap();
        staged.remove("d-new.txt");
        assert_eq!(
            work_feature::execution::file_transaction::prepare(
                &store,
                &context,
                "ATTEMPT-001",
                &[],
                None
            )
            .unwrap_err()
            .reason_code,
            "file_transaction_empty_change"
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
    fn direct(
        store: &LocalFileTransactions,
        context: &ExecutionWriterContext,
        changes: &[FileChange],
        confirmation: Option<&FileBaselineConfirmation>,
    ) -> Result<FileTransactionPreview, WorkError> {
        work_feature::execution::file_transaction::prepare(
            store,
            context,
            "ATTEMPT-001",
            changes,
            confirmation,
        )
    }
    #[test]
    fn accepted_scopes_are_reloaded_from_retained_transactions() {
        let (store, context, fixtures) = setup_scoped(Some(true));
        assert!(store.accepted_file_scopes(&context).unwrap().is_empty());
        let preview = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
        store
            .apply(&context, &preview, "Original scoped authorization")
            .unwrap();
        let reopened = LocalFileTransactions::new(store.root.clone());
        let scopes = reopened.accepted_file_scopes(&context).unwrap();
        assert_eq!(scopes["TASK-001"].len(), 4);
        for path in ["a-old.txt", "b-moved.txt", "c-existing.txt", "d-new.txt"] {
            assert!(scopes["TASK-001"].contains(&path.to_owned()));
        }
    }
    fn write(path: &str, bytes: &[u8]) -> FileChange {
        FileChange::Write {
            path: path.into(),
            bytes: bytes.into(),
        }
    }
    #[test]
    fn known_second_write_failure_restores_only_this_change_and_next_correction_can_complete() {
        for fault in 0..=2 {
            let (store, context, fixtures) = setup_scoped(Some(true));
            let first = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
            store
                .apply(&context, &first, "Original scoped authorization")
                .unwrap();
            let first_state = store.observed(&first.manifest).unwrap();
            let first_version = current_version(&context, "ATTEMPT-001").unwrap();
            let changes = [
                write("c-existing.txt", b"failed second c"),
                write("d-new.txt", b"failed second d"),
            ];
            let second = direct(&store, &context, &changes, None).unwrap();
            let owned = LocalFileTransactions {
                root: store.root.clone(),
                owned: true,
                instruction_root: store.instruction_root.clone(),
            };
            let failure = with_runtime_writer(
                &LocalWriterLock,
                context.writer(),
                LockClass::Execution,
                |_| {
                    owned.publish_with_rollback(
                        &context,
                        &second,
                        "Original scoped authorization",
                        |count| {
                            if count == fault {
                                Err(error("injected_second_write_failure"))
                            } else {
                                Ok(())
                            }
                        },
                    )
                },
            )
            .unwrap_err();
            assert_eq!(
                failure.reason_code,
                "file_transaction_write_failed_restored"
            );
            assert_eq!(failure.details["recovery_required"], false);
            assert_eq!(store.observed(&first.manifest).unwrap(), first_state);
            assert_ne!(
                first_version,
                current_version(&context, "ATTEMPT-001").unwrap()
            );
            assert_eq!(
                store
                    .load("example", &first.manifest.transaction_identity)
                    .unwrap()
                    .phase,
                RuntimePhase::PublishedVerified
            );
            assert_eq!(
                store
                    .load("example", &second.manifest.transaction_identity)
                    .unwrap()
                    .phase,
                RuntimePhase::Restored
            );
            let third = direct(
                &store,
                &context,
                &[write("c-existing.txt", b"corrected third c")],
                None,
            )
            .unwrap();
            store
                .apply(&context, &third, "Original scoped authorization")
                .unwrap();
            require_published(&context, "ATTEMPT-001").unwrap();
            assert_eq!(
                store.read_source("c-existing.txt").unwrap(),
                b"corrected third c"
            );
            assert_eq!(
                store.read_source("d-new.txt").unwrap(),
                b"new complete output\n"
            );
            assert_eq!(
                published_before(&context, "ATTEMPT-001").unwrap()["c-existing.txt"]
                    .as_ref()
                    .unwrap()
                    .bytes,
                b"original modify\0bytes\n"
            );
        }
    }
    #[test]
    fn ungranted_rollback_and_external_replacement_preserve_pending_evidence() {
        for external in [false, true] {
            let (store, context, fixtures) = setup_scoped(Some(external));
            let preview = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
            let owned = LocalFileTransactions {
                root: store.root.clone(),
                owned: true,
                instruction_root: store.instruction_root.clone(),
            };
            let failure = with_runtime_writer(
                &LocalWriterLock,
                context.writer(),
                LockClass::Execution,
                |_| {
                    owned.publish_with_rollback(
                        &context,
                        &preview,
                        "Original scoped authorization",
                        |count| {
                            if count == 2 {
                                if external {
                                    let replacement = store.root.join("user-replacement");
                                    fs::write(&replacement, b"user replacement must survive")
                                        .unwrap();
                                    fs::rename(replacement, store.root.join("b-moved.txt"))
                                        .unwrap();
                                }
                                Err(error("injected_write_failure"))
                            } else {
                                Ok(())
                            }
                        },
                    )
                },
            )
            .unwrap_err();
            assert_eq!(
                failure.reason_code,
                if external {
                    "file_transaction_rollback_blocked"
                } else {
                    "injected_write_failure"
                }
            );
            assert!(
                check_retained(&store.root, "example", context.target().execution_dir).is_err()
            );
            assert!(current_version(&context, "ATTEMPT-001").is_err());
            assert!(
                runtime_manifest_directory(&store.root, &preview.manifest)
                    .unwrap()
                    .exists()
            );
            if external {
                assert_eq!(
                    store.read_source("b-moved.txt").unwrap(),
                    b"user replacement must survive"
                );
            }
        }
    }
    #[test]
    fn confirmed_user_baseline_is_persisted_and_current_failure_restores_user_contents() {
        let (store, context, fixtures) = setup_scoped(Some(true));
        let first = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
        store
            .apply(&context, &first, "Original authorization")
            .unwrap();
        fs::write(
            store.root.join("c-existing.txt"),
            b"user edit after first success",
        )
        .unwrap();
        let changes = [write("c-existing.txt", b"proposed current change")];
        let required = direct(&store, &context, &changes, None).unwrap_err();
        assert_eq!(
            required.reason_code,
            "file_transaction_baseline_confirmation_required"
        );
        let confirmation = FileBaselineConfirmation {
            previous_transaction: required.details["previous_transaction"]
                .as_str()
                .unwrap()
                .into(),
            before_sha256: required.details["before_sha256"].as_str().unwrap().into(),
            evidence: "User explicitly confirmed the new inspected file baseline".into(),
        };
        let preview = direct(&store, &context, &changes, Some(&confirmation)).unwrap();
        assert_eq!(
            LocalFileTransactions::binding(&preview.manifest)
                .unwrap()
                .baseline_confirmation,
            Some(confirmation.clone())
        );
        let owned = LocalFileTransactions {
            root: store.root.clone(),
            owned: true,
            instruction_root: store.instruction_root.clone(),
        };
        let failure = with_runtime_writer(
            &LocalWriterLock,
            context.writer(),
            LockClass::Execution,
            |_| {
                owned.publish_with_rollback(&context, &preview, "Original authorization", |count| {
                    if count == 1 {
                        Err(error("injected"))
                    } else {
                        Ok(())
                    }
                })
            },
        )
        .unwrap_err();
        assert_eq!(
            failure.reason_code,
            "file_transaction_write_failed_restored"
        );
        assert_eq!(
            store.read_source("c-existing.txt").unwrap(),
            b"user edit after first success"
        );
        require_published(&context, "ATTEMPT-001").unwrap();
        assert_eq!(
            store
                .load("example", &first.manifest.transaction_identity)
                .unwrap()
                .phase,
            RuntimePhase::PublishedVerified
        );
        assert!(
            direct(&store, &context, &changes, Some(&confirmation)).is_err(),
            "old confirmation cannot bind a new history head"
        );
        let third = direct(&store, &context, &changes, None).unwrap();
        store
            .apply(&context, &third, "Original authorization")
            .unwrap();
        require_published(&context, "ATTEMPT-001").unwrap();
    }
    #[test]
    fn equal_bytes_replacement_requires_a_new_baseline_and_stale_recovery_identity_is_refused() {
        let (store, context, fixtures) = setup_scoped(Some(true));
        let first = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
        store
            .apply(&context, &first, "Original authorization")
            .unwrap();
        let before = store.inspect("c-existing.txt").unwrap().unwrap();
        let replace = || {
            let temporary = store.root.join("same-bytes-user-replacement");
            fs::write(&temporary, &before.bytes).unwrap();
            store.set_metadata(&temporary, &before.metadata).unwrap();
            fs::rename(&temporary, store.root.join("c-existing.txt")).unwrap();
        };
        replace();
        assert_eq!(
            store.inspect("c-existing.txt").unwrap(),
            Some(before.clone())
        );
        assert_eq!(
            current_version(&context, "ATTEMPT-001")
                .unwrap_err()
                .reason_code,
            "file_transaction_target_drift"
        );
        let changes = [write("c-existing.txt", b"new proposed change")];
        let required = direct(&store, &context, &changes, None).unwrap_err();
        assert_eq!(
            required.reason_code,
            "file_transaction_baseline_confirmation_required"
        );
        let confirmation = FileBaselineConfirmation {
            previous_transaction: required.details["previous_transaction"]
                .as_str()
                .unwrap()
                .into(),
            before_sha256: required.details["before_sha256"].as_str().unwrap().into(),
            evidence: "User confirmed exact current file identity and contents".into(),
        };
        direct(&store, &context, &changes, Some(&confirmation)).unwrap();
        replace();
        assert_eq!(
            direct(&store, &context, &changes, Some(&confirmation))
                .unwrap_err()
                .reason_code,
            "file_transaction_baseline_confirmation_invalid"
        );

        let (store, context, fixtures) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
        store.fixture_publish(&context, &preview, 2).unwrap_err();
        let recovery = store
            .recovery_preview(&context, &preview.manifest.transaction_identity)
            .unwrap();
        let current = store.inspect("b-moved.txt").unwrap().unwrap();
        let temporary = store.root.join("recovery-user-replacement");
        fs::write(&temporary, &current.bytes).unwrap();
        store.set_metadata(&temporary, &current.metadata).unwrap();
        fs::rename(temporary, store.root.join("b-moved.txt")).unwrap();
        assert_eq!(
            store
                .restore(&context, &recovery, "Old recovery approval")
                .unwrap_err()
                .reason_code,
            "file_transaction_recovery_approval_stale"
        );
        assert_eq!(store.inspect("b-moved.txt").unwrap(), Some(current));
    }
    #[test]
    fn terminal_identity_evidence_is_required_and_foreign_records_are_not_discarded() {
        let (store, context, fixtures) = setup();
        let preview = prepare(&store, &context, "ATTEMPT-001", &fixtures).unwrap();
        store
            .apply(&context, &preview, "Original authorization")
            .unwrap();
        let dir = runtime_manifest_directory(&store.root, &preview.manifest).unwrap();
        let path = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("publication-")
                    && p.extension().is_some_and(|e| e == "json")
            })
            .unwrap();
        let raw = fs::read(&path).unwrap();
        fs::rename(&path, store.root.join("retained-terminal-evidence")).unwrap();
        assert_eq!(
            require_published(&context, "ATTEMPT-001")
                .unwrap_err()
                .reason_code,
            "file_transaction_terminal_evidence_missing"
        );
        fs::write(&path, &raw).unwrap();
        require_published(&context, "ATTEMPT-001").unwrap();
        fs::write(
            dir.join(format!("publication-{}.json", "0".repeat(64))),
            &raw,
        )
        .unwrap();
        assert!(require_published(&context, "ATTEMPT-001").is_err());
        assert_eq!(
            fs::read(store.root.join("retained-terminal-evidence")).unwrap(),
            raw
        );
    }
    #[test]
    fn native_move_with_original_path_replacement_and_failed_swap_keep_one_initial_tree() {
        let (store, context, fixtures) = setup_scoped(Some(true));
        let mut changes = crate::fixture_support::file_changes_fixture(&store.root).unwrap();
        changes.push(write("a-old.txt", b"replacement at original path"));
        let first = direct(&store, &context, &changes, None).unwrap();
        store
            .apply(&context, &first, "Original scoped authorization")
            .unwrap();
        assert_eq!(
            store.read_source("a-old.txt").unwrap(),
            b"replacement at original path"
        );
        assert_eq!(store.read_source("b-moved.txt").unwrap(), b"final move\n");
        let swap = [
            FileChange::Move {
                source: "a-old.txt".into(),
                destination: "b-moved.txt".into(),
                bytes: None,
            },
            FileChange::Move {
                source: "b-moved.txt".into(),
                destination: "a-old.txt".into(),
                bytes: None,
            },
        ];
        let second = direct(&store, &context, &swap, None).unwrap();
        let owned = LocalFileTransactions {
            root: store.root.clone(),
            owned: true,
            instruction_root: store.instruction_root.clone(),
        };
        let failure = with_runtime_writer(
            &LocalWriterLock,
            context.writer(),
            LockClass::Execution,
            |_| {
                owned.publish_with_rollback(
                    &context,
                    &second,
                    "Original scoped authorization",
                    |n| {
                        if n == 1 {
                            Err(error("injected_swap"))
                        } else {
                            Ok(())
                        }
                    },
                )
            },
        )
        .unwrap_err();
        assert_eq!(
            failure.reason_code,
            "file_transaction_write_failed_restored"
        );
        assert_eq!(
            store.read_source("a-old.txt").unwrap(),
            b"replacement at original path"
        );
        assert_eq!(store.read_source("b-moved.txt").unwrap(), b"final move\n");
        let third = direct(&store, &context, &swap, None).unwrap();
        store
            .apply(&context, &third, "Original scoped authorization")
            .unwrap();
        assert_eq!(store.read_source("a-old.txt").unwrap(), b"final move\n");
        assert_eq!(
            store.read_source("b-moved.txt").unwrap(),
            b"replacement at original path"
        );
        require_published(&context, "ATTEMPT-001").unwrap();
        assert_eq!(
            published_before(&context, "ATTEMPT-001").unwrap()["a-old.txt"]
                .as_ref()
                .unwrap()
                .bytes,
            b"original move\r\n"
        );
        for path in fixtures.values() {
            assert!(
                store.root.join(path).is_file(),
                "fixture-owned generated output remains intact"
            );
        }
    }
}
