//! Exclusive, immutable Source Snapshot publication.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use work_feature::artifact_paths::{ArtifactPathRepository, default_artifact_paths};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{
    ArtifactStore, SnapshotBytes, SourceSnapshotReader, SourceSnapshotWriter,
};
use work_model::identifiers::{RequirementId, SourceId};
use work_model::schema::PublicSchema;
use work_model::source_snapshot::{
    SnapshotContent, SnapshotSource, SourceContentPath, SourceSnapshot,
};
use work_operations::derivation::fingerprint;
use work_operations::source_snapshot;

use crate::artifact_paths::LocalArtifactPaths;
use crate::files::LocalFiles;
use crate::transaction_storage::{
    CompletionState, complete_write, completion_state, prepare_transaction_directory,
    write_completion_marker,
};
use crate::writer_lock::{LocalWriterLock, WriterLock};

const MANIFEST: &str = "manifest.json";
const MARKER: &str = "manifest.json.done";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureStage {
    Allocated,
    ManifestWritten,
    ContentPartial,
    BeforeMarker,
    AfterMarker,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedSource {
    manifest: SourceSnapshot,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LocalSourceSnapshotStorage {
    pub project_root: PathBuf,
}

fn error(reason: &str, message: &str, path: &Path) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        reason,
        message,
        json!({"path": path.to_string_lossy()}),
    )
}

impl LocalSourceSnapshotStorage {
    fn source_root(&self, id: &RequirementId) -> Result<PathBuf, WorkError> {
        LocalArtifactPaths {
            project_root: self.project_root.clone(),
        }
        .resolve(&default_artifact_paths(id).source)
    }

    fn next_id(&self, root: &Path) -> Result<SourceId, WorkError> {
        let mut maximum = 0_u64;
        for entry in fs::read_dir(root).map_err(|_| {
            error(
                "source_directory_read_failed",
                "Source versions could not be inspected.",
                root,
            )
        })? {
            let entry = entry.map_err(|_| {
                error(
                    "source_directory_read_failed",
                    "Source versions could not be inspected.",
                    root,
                )
            })?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                error(
                    "invalid_source_namespace",
                    "The Source namespace contains an invalid name.",
                    root,
                )
            })?;
            let candidate = name.strip_prefix(".capture-").unwrap_or(name);
            if candidate.starts_with("SRC-") {
                let id: SourceId = candidate.parse().map_err(|_| {
                    error(
                        "invalid_source_namespace",
                        "The Source namespace contains an invalid ID.",
                        root,
                    )
                })?;
                let sequence: u64 = id.as_str()[4..].parse().expect("validated Source ID");
                maximum = maximum.max(sequence);
            }
        }
        let sequence = maximum.checked_add(1).ok_or_else(|| {
            error(
                "source_sequence_exhausted",
                "Source sequence numbers are exhausted.",
                root,
            )
        })?;
        format!("SRC-{sequence:03}").parse().map_err(|_| {
            error(
                "invalid_source_id",
                "The generated Source ID is invalid.",
                root,
            )
        })
    }

    fn publish_new(&self, root: &Path, snapshot: &SnapshotBytes) -> Result<(), WorkError> {
        self.publish_with_hook(root, snapshot, |_| Ok(()))
    }

    fn publish_with_hook(
        &self,
        root: &Path,
        snapshot: &SnapshotBytes,
        mut after_stage: impl FnMut(CaptureStage) -> Result<(), WorkError>,
    ) -> Result<(), WorkError> {
        let id = snapshot.manifest.source_id.as_str();
        let prepared = root.join(format!(".capture-{id}"));
        prepare_transaction_directory(&prepared)?;
        let journal =
            serde_json::to_vec(&json!({"manifest": snapshot.manifest, "bytes": snapshot.bytes}))
                .map_err(|_| {
                    error(
                        "source_encode_failed",
                        "Source recovery evidence could not be encoded.",
                        &prepared,
                    )
                })?;
        LocalFiles.create_new(&prepared.join("journal.json"), &journal)?;
        write_completion_marker(
            &prepared.join("journal.json"),
            &prepared.join("journal.json.done"),
        )?;
        let target = root.join(id);
        prepare_transaction_directory(&target)?;
        after_stage(CaptureStage::Allocated)?;
        let mut manifest = serde_json::to_vec_pretty(&snapshot.manifest).map_err(|_| {
            error(
                "source_encode_failed",
                "Source metadata could not be encoded.",
                &target,
            )
        })?;
        manifest.push(b'\n');
        LocalFiles.create_new(&target.join(MANIFEST), &manifest)?;
        after_stage(CaptureStage::ManifestWritten)?;
        let content = target.join(snapshot.manifest.content.path.as_str());
        LocalFiles.create_new(&content, &snapshot.bytes[..snapshot.bytes.len() / 2])?;
        after_stage(CaptureStage::ContentPartial)?;
        complete_write(&content, &snapshot.bytes)?;
        source_snapshot::validate(&snapshot.manifest, Some(&LocalFiles.read_raw(&content)?))
            .map_err(|issue| error(issue.reason_code, issue.message, &content))?;
        after_stage(CaptureStage::BeforeMarker)?;
        write_completion_marker(&target.join(MANIFEST), &target.join(MARKER))?;
        after_stage(CaptureStage::AfterMarker)?;
        Ok(())
    }

    pub fn recover(
        &self,
        requirement_id: &RequirementId,
        source_id: &SourceId,
    ) -> Result<SnapshotBytes, WorkError> {
        let root = self.source_root(requirement_id)?;
        let _guard = LocalWriterLock.acquire(&root.join(".work-source-writer.lock"))?;
        let prepared = root.join(format!(".capture-{}", source_id.as_str()));
        require_directory(&prepared)?;
        let journal = regular_bytes(&prepared.join("journal.json"))?;
        let journal_marker = regular_bytes(&prepared.join("journal.json.done"))?;
        if completion_state(&journal, Some(&journal_marker)) != CompletionState::Completed {
            return Err(error(
                "source_recovery_evidence_incomplete",
                "Source recovery evidence is incomplete or changed.",
                &prepared,
            ));
        }
        let prepared_source: PreparedSource = serde_json::from_slice(&journal).map_err(|_| {
            error(
                "invalid_source_recovery_evidence",
                "Source recovery evidence does not match its contract.",
                &prepared,
            )
        })?;
        let manifest = prepared_source.manifest;
        let bytes = prepared_source.bytes;
        if manifest.requirement_id != requirement_id.as_str() || &manifest.source_id != source_id {
            return Err(error(
                "source_identity_mismatch",
                "Recovery evidence must identify the requested Snapshot.",
                &prepared,
            ));
        }
        source_snapshot::validate(&manifest, Some(&bytes))
            .map_err(|issue| error(issue.reason_code, issue.message, &prepared))?;
        let target = root.join(source_id.as_str());
        if target.symlink_metadata().is_ok() {
            require_directory(&target)?;
        }
        if target.join(MARKER).exists() && self.is_complete(&target)? {
            let saved = self.read_snapshot(requirement_id, source_id)?;
            source_snapshot::validate_immutable(&saved.manifest, &saved.bytes, &manifest, &bytes)
                .map_err(|issue| error(issue.reason_code, issue.message, &target))?;
            return Ok(saved);
        }
        if !target.exists() {
            prepare_transaction_directory(&target)?;
        }
        if !fs::symlink_metadata(&target)
            .map_err(|_| {
                error(
                    "invalid_source_directory",
                    "Source directory could not be inspected.",
                    &target,
                )
            })?
            .is_dir()
        {
            return Err(error(
                "invalid_source_directory",
                "Source recovery requires a real directory.",
                &target,
            ));
        }
        let mut manifest_raw = serde_json::to_vec_pretty(&manifest).map_err(|_| {
            error(
                "source_encode_failed",
                "Source metadata could not be encoded.",
                &target,
            )
        })?;
        manifest_raw.push(b'\n');
        for (path, expected) in [
            (target.join(MANIFEST), manifest_raw.as_slice()),
            (
                target.join(manifest.content.path.as_str()),
                bytes.as_slice(),
            ),
        ] {
            if path.symlink_metadata().is_ok() {
                regular_bytes(&path)?;
            }
            complete_write(&path, expected)?;
        }
        source_snapshot::validate(
            &manifest,
            Some(&regular_bytes(
                &target.join(manifest.content.path.as_str()),
            )?),
        )
        .map_err(|issue| error(issue.reason_code, issue.message, &target))?;
        let marker = work_operations::derivation::publication::completion_marker(&manifest_raw);
        if target.join(MARKER).symlink_metadata().is_ok() {
            regular_bytes(&target.join(MARKER))?;
        }
        complete_write(&target.join(MARKER), &marker)?;
        self.read_snapshot(requirement_id, source_id)
    }

    fn is_complete(&self, directory: &Path) -> Result<bool, WorkError> {
        let manifest = LocalFiles.read_raw(&directory.join(MANIFEST))?;
        let marker = if directory.join(MARKER).exists() {
            Some(LocalFiles.read_raw(&directory.join(MARKER))?)
        } else {
            None
        };
        Ok(completion_state(&manifest, marker.as_deref()) == CompletionState::Completed)
    }
}

impl SourceSnapshotWriter for LocalSourceSnapshotStorage {
    fn capture(
        &self,
        requirement_id: &RequirementId,
        source: &SnapshotSource,
        content_path: &SourceContentPath,
        bytes: &[u8],
        captured_at: &str,
    ) -> Result<SnapshotBytes, WorkError> {
        let mut manifest = SourceSnapshot {
            schema: PublicSchema::WorkSourceSnapshotV1,
            requirement_id: requirement_id.as_str().into(),
            source_id: "SRC-001".parse().expect("initial Source ID"),
            captured_at: captured_at.into(),
            source: source.clone(),
            content: SnapshotContent {
                path: content_path.clone(),
                sha256: fingerprint::raw(bytes),
                size: bytes.len() as u64,
            },
        };
        source_snapshot::validate(&manifest, Some(bytes)).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                json!({}),
            )
        })?;
        let root = self.source_root(requirement_id)?;
        LocalFiles.create_directories(&root)?;
        let _guard = LocalWriterLock.acquire(&root.join(".work-source-writer.lock"))?;
        manifest.source_id = self.next_id(&root)?;
        let snapshot = SnapshotBytes {
            manifest,
            bytes: bytes.to_vec(),
        };
        self.publish_new(&root, &snapshot)?;
        if !self.is_complete(&root.join(snapshot.manifest.source_id.as_str()))? {
            return Err(error(
                "source_snapshot_incomplete",
                "Source publication has not completed.",
                &root,
            ));
        }
        Ok(snapshot)
    }
}

fn regular_bytes(path: &Path) -> Result<Vec<u8>, WorkError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        error(
            "source_content_missing",
            "A Snapshot file is missing.",
            path,
        )
    })?;
    if !metadata.is_file() {
        return Err(error(
            "invalid_source_file",
            "Snapshot files must be regular files, without symlinks.",
            path,
        ));
    }
    LocalFiles.read_raw(path)
}

fn require_directory(path: &Path) -> Result<(), WorkError> {
    if !fs::symlink_metadata(path)
        .map_err(|_| {
            error(
                "invalid_source_directory",
                "Source directory could not be inspected.",
                path,
            )
        })?
        .is_dir()
    {
        return Err(error(
            "invalid_source_directory",
            "Source recovery requires a real directory.",
            path,
        ));
    }
    Ok(())
}

impl SourceSnapshotReader for LocalSourceSnapshotStorage {
    fn read_snapshot(
        &self,
        requirement_id: &RequirementId,
        source_id: &SourceId,
    ) -> Result<SnapshotBytes, WorkError> {
        self.read_snapshot_at(
            requirement_id,
            source_id,
            &default_artifact_paths(requirement_id).source,
        )
    }
    fn read_snapshot_at(
        &self,
        requirement_id: &RequirementId,
        source_id: &SourceId,
        source_root: &str,
    ) -> Result<SnapshotBytes, WorkError> {
        if !source_root.ends_with(&format!("/{}", requirement_id.as_str())) {
            return Err(error(
                "source_requirement_mismatch",
                "Source root must identify the same requirement.",
                &self.project_root,
            ));
        }
        let directory = LocalArtifactPaths {
            project_root: self.project_root.clone(),
        }
        .resolve(source_root)?
        .join(source_id.as_str());
        let metadata = fs::symlink_metadata(&directory).map_err(|_| {
            error(
                "source_snapshot_missing",
                "The Source Snapshot does not exist.",
                &directory,
            )
        })?;
        if !metadata.is_dir() {
            return Err(error(
                "invalid_source_directory",
                "The Snapshot must be a real directory.",
                &directory,
            ));
        }
        if !directory.join(MARKER).exists() {
            return Err(error(
                "source_snapshot_incomplete",
                "Source publication has not completed.",
                &directory,
            ));
        }
        let manifest_raw = regular_bytes(&directory.join(MANIFEST))?;
        let marker = regular_bytes(&directory.join(MARKER))?;
        if completion_state(&manifest_raw, Some(&marker)) != CompletionState::Completed {
            return Err(error(
                "source_marker_mismatch",
                "The Source completion marker does not match its manifest.",
                &directory,
            ));
        }
        let manifest: SourceSnapshot = serde_json::from_slice(&manifest_raw).map_err(|_| {
            error(
                "invalid_source_manifest",
                "Source metadata does not match its contract.",
                &directory,
            )
        })?;
        if manifest.requirement_id != requirement_id.as_str() || &manifest.source_id != source_id {
            return Err(error(
                "source_identity_mismatch",
                "Source metadata must identify its containing Snapshot.",
                &directory,
            ));
        }
        let bytes = regular_bytes(&directory.join(manifest.content.path.as_str()))?;
        source_snapshot::validate(&manifest, Some(&bytes))
            .map_err(|issue| error(issue.reason_code, issue.message, &directory))?;
        Ok(SnapshotBytes { manifest, bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn storage() -> LocalSourceSnapshotStorage {
        static NEXT_ROOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "work-source-snapshot-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ROOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        LocalSourceSnapshotStorage { project_root: root }
    }

    fn capture(
        store: &LocalSourceSnapshotStorage,
        bytes: &[u8],
    ) -> Result<SnapshotBytes, WorkError> {
        store.capture(
            &"example".parse().unwrap(),
            &SnapshotSource::UserText {},
            &"source.txt".to_owned().try_into().unwrap(),
            bytes,
            "2026-10-03T00:00:00Z",
        )
    }

    #[test]
    fn captures_only_add_versions_and_preserve_every_old_byte() {
        let store = storage();
        let first = capture(&store, b"first\r\n").unwrap();
        let root = store.source_root(&"example".parse().unwrap()).unwrap();
        let manifest = fs::read(root.join("SRC-001/manifest.json")).unwrap();
        let second = capture(&store, b"second").unwrap();
        assert_eq!(first.manifest.source_id.as_str(), "SRC-001");
        assert_eq!(second.manifest.source_id.as_str(), "SRC-002");
        assert_eq!(
            fs::read(root.join("SRC-001/manifest.json")).unwrap(),
            manifest
        );
        assert_eq!(
            fs::read(root.join("SRC-001/source.txt")).unwrap(),
            b"first\r\n"
        );
        assert!(store.publish_new(&root, &first).is_err());
        assert_eq!(
            fs::read(root.join("SRC-001/manifest.json")).unwrap(),
            manifest
        );
        fs::remove_dir_all(store.project_root).unwrap();
    }

    #[test]
    fn parallel_captures_never_share_a_completed_sequence() {
        let store = storage();
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    loop {
                        match capture(&store, b"parallel") {
                            Err(error) if error.reason_code == "work_state_writer_busy" => {
                                std::thread::yield_now()
                            }
                            result => return result.unwrap().manifest.source_id,
                        }
                    }
                })
            })
            .collect();
        let ids: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_ne!(ids[0], ids[1]);
        fs::remove_dir_all(store.project_root).unwrap();
    }

    #[test]
    fn reserved_versions_are_not_overwritten_and_missing_marker_is_incomplete() {
        let store = storage();
        let root = store.source_root(&"example".parse().unwrap()).unwrap();
        fs::create_dir_all(root.join("SRC-001")).unwrap();
        fs::write(root.join("SRC-001/manifest.json"), b"preserved").unwrap();
        assert!(!store.is_complete(&root.join("SRC-001")).unwrap());
        let result = capture(&store, b"new").unwrap();
        assert_eq!(result.manifest.source_id.as_str(), "SRC-002");
        assert_eq!(
            fs::read(root.join("SRC-001/manifest.json")).unwrap(),
            b"preserved"
        );
        fs::remove_dir_all(store.project_root).unwrap();
    }

    #[test]
    fn reader_preserves_binary_payload_and_rejects_missing_or_changed_bytes() {
        let store = storage();
        let id = "example".parse().unwrap();
        let snapshot = store
            .capture(
                &id,
                &SnapshotSource::File {
                    path: "input.pdf".into(),
                    media_type: "application/pdf".into(),
                },
                &"source.pdf".to_owned().try_into().unwrap(),
                b"\xff\x00\r\n",
                "2026-10-03T00:00:00Z",
            )
            .unwrap();
        assert_eq!(
            snapshot.manifest.source,
            SnapshotSource::File {
                path: "input.pdf".into(),
                media_type: "application/pdf".into(),
            }
        );
        assert_eq!(
            store
                .read_snapshot(&id, &snapshot.manifest.source_id)
                .unwrap()
                .manifest
                .source,
            snapshot.manifest.source
        );
        assert_eq!(
            store
                .read_snapshot(&id, &snapshot.manifest.source_id)
                .unwrap(),
            snapshot
        );
        let content = store.source_root(&id).unwrap().join("SRC-001/source.pdf");
        fs::write(&content, b"\xff\x00\rX").unwrap();
        assert_eq!(
            store
                .read_snapshot(&id, &snapshot.manifest.source_id)
                .unwrap_err()
                .reason_code,
            "source_hash_mismatch"
        );
        fs::write(&content, b"short").unwrap();
        assert_eq!(
            store
                .read_snapshot(&id, &snapshot.manifest.source_id)
                .unwrap_err()
                .reason_code,
            "source_size_mismatch"
        );
        fs::remove_file(&content).unwrap();
        assert_eq!(
            store
                .read_snapshot(&id, &snapshot.manifest.source_id)
                .unwrap_err()
                .reason_code,
            "source_content_missing"
        );
        fs::remove_dir_all(store.project_root).unwrap();
    }

    #[test]
    fn reader_rejects_incomplete_corrupt_marker_identity_and_unsafe_manifest_paths() {
        for mutation in [
            "missing_marker",
            "bad_marker",
            "bad_identity",
            "unsafe_path",
        ] {
            let store = storage();
            let snapshot = capture(&store, b"original").unwrap();
            let directory = store
                .source_root(&"example".parse().unwrap())
                .unwrap()
                .join("SRC-001");
            let expected = match mutation {
                "missing_marker" => {
                    fs::remove_file(directory.join(MARKER)).unwrap();
                    "source_snapshot_incomplete"
                }
                "bad_marker" => {
                    fs::write(directory.join(MARKER), b"changed").unwrap();
                    "source_marker_mismatch"
                }
                _ => {
                    let mut value = serde_json::to_value(&snapshot.manifest).unwrap();
                    if mutation == "bad_identity" {
                        value["source_id"] = json!("SRC-002");
                    } else {
                        value["content"]["path"] = json!("../outside");
                    }
                    let raw = serde_json::to_vec(&value).unwrap();
                    fs::write(directory.join(MANIFEST), &raw).unwrap();
                    fs::write(
                        directory.join(MARKER),
                        work_operations::derivation::publication::completion_marker(&raw),
                    )
                    .unwrap();
                    if mutation == "bad_identity" {
                        "source_identity_mismatch"
                    } else {
                        "invalid_source_manifest"
                    }
                }
            };
            assert_eq!(
                store
                    .read_snapshot(&"example".parse().unwrap(), &snapshot.manifest.source_id)
                    .unwrap_err()
                    .reason_code,
                expected
            );
            fs::remove_dir_all(store.project_root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn reader_rejects_symlink_content_even_when_it_has_matching_bytes() {
        let store = storage();
        let snapshot = capture(&store, b"original").unwrap();
        let directory = store
            .source_root(&"example".parse().unwrap())
            .unwrap()
            .join("SRC-001");
        let outside = store.project_root.join("outside.txt");
        fs::write(&outside, b"original").unwrap();
        fs::remove_file(directory.join("source.txt")).unwrap();
        std::os::unix::fs::symlink(outside, directory.join("source.txt")).unwrap();
        assert_eq!(
            store
                .read_snapshot(&"example".parse().unwrap(), &snapshot.manifest.source_id)
                .unwrap_err()
                .reason_code,
            "invalid_source_file"
        );
        fs::remove_dir_all(store.project_root).unwrap();
    }

    #[test]
    fn every_capture_interruption_recovers_repeatably_without_touching_old_snapshots() {
        for stage in [
            CaptureStage::Allocated,
            CaptureStage::ManifestWritten,
            CaptureStage::ContentPartial,
            CaptureStage::BeforeMarker,
            CaptureStage::AfterMarker,
        ] {
            let store = storage();
            let old = capture(&store, b"unchanged old source").unwrap();
            let id = "example".parse().unwrap();
            let root = store.source_root(&id).unwrap();
            let old_manifest = fs::read(root.join("SRC-001/manifest.json")).unwrap();
            let mut next = old.clone();
            next.manifest.source_id = "SRC-002".parse().unwrap();
            next.bytes = b"recover original bytes\r\n".to_vec();
            next.manifest.content.sha256 = fingerprint::raw(&next.bytes);
            next.manifest.content.size = next.bytes.len() as u64;
            let _guard = LocalWriterLock
                .acquire(&root.join(".work-source-writer.lock"))
                .unwrap();
            let result = store.publish_with_hook(&root, &next, |current| {
                if current == stage {
                    Err(error(
                        "injected_interruption",
                        "Injected capture interruption.",
                        &root,
                    ))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.unwrap_err().reason_code, "injected_interruption");
            drop(_guard);
            if stage != CaptureStage::AfterMarker {
                assert!(store.read_snapshot(&id, &next.manifest.source_id).is_err());
            }
            assert_eq!(store.recover(&id, &next.manifest.source_id).unwrap(), next);
            assert_eq!(store.recover(&id, &next.manifest.source_id).unwrap(), next);
            assert_eq!(
                store.read_snapshot(&id, &old.manifest.source_id).unwrap(),
                old
            );
            assert_eq!(
                fs::read(root.join("SRC-001/manifest.json")).unwrap(),
                old_manifest
            );
            fs::remove_dir_all(store.project_root).unwrap();
        }
    }

    #[test]
    fn recovery_rejects_conflicting_partial_bytes_and_never_repairs_completed_drift() {
        let store = storage();
        let saved = capture(&store, b"original bytes").unwrap();
        let id = "example".parse().unwrap();
        let target = store.source_root(&id).unwrap().join("SRC-001");
        fs::write(target.join("source.txt"), b"changed source").unwrap();
        assert!(store.recover(&id, &saved.manifest.source_id).is_err());
        assert_eq!(
            fs::read(target.join("source.txt")).unwrap(),
            b"changed source"
        );
        fs::remove_file(target.join(MARKER)).unwrap();
        assert_eq!(
            store
                .recover(&id, &saved.manifest.source_id)
                .unwrap_err()
                .reason_code,
            "spec_update_partial_conflict"
        );
        assert_eq!(
            fs::read(target.join("source.txt")).unwrap(),
            b"changed source"
        );
        fs::remove_dir_all(store.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn recovery_rejects_symlink_snapshot_and_evidence_directories_before_reading_them() {
        for name in ["SRC-001", ".capture-SRC-001"] {
            let store = storage();
            let snapshot = capture(&store, b"unchanged").unwrap();
            let id = "example".parse().unwrap();
            let root = store.source_root(&id).unwrap();
            let moved = store.project_root.join("moved-evidence");
            fs::rename(root.join(name), &moved).unwrap();
            std::os::unix::fs::symlink(&moved, root.join(name)).unwrap();
            assert_eq!(
                store
                    .recover(&id, &snapshot.manifest.source_id)
                    .unwrap_err()
                    .reason_code,
                "invalid_source_directory"
            );
            fs::remove_dir_all(store.project_root).unwrap();
        }
    }
}
