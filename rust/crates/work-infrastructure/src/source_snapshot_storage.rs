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
use work_model::source::snapshot::{
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
use crate::writer_lock::LocalWriterLock;

const MANIFEST: &str = "manifest.json";
const MARKER: &str = "manifest.json.done";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureStage {
    Prepared,
    Allocated,
    ManifestWritten,
    ContentPartial,
    BeforeMarker,
    AfterMarker,
    CleanupMarkerRemoved,
    CleanupManifestRemoved,
    CleanupDirectoryRemoved,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedCapture {
    schema: String,
    canonical_root: String,
    source_root: String,
    transaction_identity: String,
    approval_sha256: String,
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
    fn capture_evidence(
        &self,
        source_root: &str,
        snapshot: &SnapshotBytes,
    ) -> Result<PreparedCapture, WorkError> {
        source_snapshot::validate(&snapshot.manifest, Some(&snapshot.bytes))
            .map_err(|issue| error(issue.reason_code, issue.message, &self.project_root))?;
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        let canonical_root = root.to_str().ok_or_else(|| {
            error(
                "source_project_root",
                "The project root must have a portable representation.",
                &root,
            )
        })?;
        let requirement: RequirementId =
            snapshot.manifest.requirement_id.parse().map_err(|_| {
                error(
                    "source_identity_mismatch",
                    "The Source requirement is invalid.",
                    &root,
                )
            })?;
        let target = format!("{source_root}/{}", snapshot.manifest.source_id.as_str());
        let targets = vec![
            format!("{target}/{MANIFEST}"),
            format!("{target}/{}", snapshot.manifest.content.path.as_str()),
            format!("{target}/{MARKER}"),
        ];
        let business = json!({"source_id":snapshot.manifest.source_id,"source_root":source_root});
        let approval =
            fingerprint::structured(&json!({"manifest":snapshot.manifest,"bytes":snapshot.bytes}))
                .map_err(|_| {
                    error(
                        "source_encode_failed",
                        "Capture evidence could not be encoded.",
                        &root,
                    )
                })?;
        let identity = work_operations::derivation::identity::runtime_transaction_identity(
            canonical_root,
            &requirement,
            "source-capture",
            &approval,
            &business,
            &targets,
        )
        .map_err(|issue| error(issue.reason_code, issue.message, &root))?;
        Ok(PreparedCapture {
            schema: "work-runtime-source-capture".into(),
            canonical_root: canonical_root.into(),
            source_root: source_root.into(),
            transaction_identity: identity,
            approval_sha256: approval,
            manifest: snapshot.manifest.clone(),
            bytes: snapshot.bytes.clone(),
        })
    }

    fn staging_path(
        &self,
        requirement: &RequirementId,
        source: &SourceId,
    ) -> Result<PathBuf, WorkError> {
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        crate::files::resolve_runtime_path(
            &root,
            &work_operations::derivation::publication::source_capture_path(requirement, source),
        )
    }

    fn require_capture_namespace(&self, source_root: &str) -> Result<PathBuf, WorkError> {
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        let directory = crate::files::resolve_runtime_path(&root, source_root)?;
        if directory.symlink_metadata().is_ok() {
            require_directory(&directory)?;
            for entry in fs::read_dir(&directory).map_err(|_| {
                error(
                    "source_directory_read_failed",
                    "Source versions could not be inspected.",
                    &directory,
                )
            })? {
                let entry = entry.map_err(|_| {
                    error(
                        "source_directory_read_failed",
                        "Source versions could not be inspected.",
                        &directory,
                    )
                })?;
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(".capture-"))
                {
                    return Err(error(
                        "legacy_source_capture_present",
                        "Legacy capture evidence requires reviewed offline migration.",
                        &entry.path(),
                    ));
                }
            }
        }
        Ok(directory)
    }

    fn next_staging_source_id(
        &self,
        requirement: &RequirementId,
        source_root: &str,
    ) -> Result<SourceId, WorkError> {
        let formal = self.require_capture_namespace(source_root)?;
        let first: SourceId = "SRC-001".parse().expect("initial Source ID");
        let staging = self
            .staging_path(requirement, &first)?
            .parent()
            .expect("capture family parent")
            .to_path_buf();
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        let mut maximum = 0u64;
        for directory in [formal, staging] {
            if directory.symlink_metadata().is_err() {
                continue;
            }
            require_directory(&directory)?;
            for entry in fs::read_dir(&directory).map_err(|_| {
                error(
                    "source_directory_read_failed",
                    "Source versions could not be inspected.",
                    &directory,
                )
            })? {
                let entry = entry.map_err(|_| {
                    error(
                        "source_directory_read_failed",
                        "Source versions could not be inspected.",
                        &directory,
                    )
                })?;
                let name = entry.file_name();
                let id: SourceId = name
                    .to_str()
                    .ok_or_else(|| {
                        error(
                            "invalid_source_namespace",
                            "A Source name is not portable.",
                            &entry.path(),
                        )
                    })?
                    .parse()
                    .map_err(|_| {
                        error(
                            "invalid_source_namespace",
                            "A Source ID is invalid.",
                            &entry.path(),
                        )
                    })?;
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .map_err(|_| {
                        error(
                            "source_identity_mismatch",
                            "Source namespace escaped its project.",
                            &entry.path(),
                        )
                    })?
                    .to_string_lossy()
                    .replace('\\', "/");
                let checked = crate::files::resolve_runtime_path(&root, &relative)?;
                require_directory(&checked)?;
                maximum = maximum.max(id.as_str()[4..].parse().expect("validated Source ID"));
            }
        }
        let number = maximum.checked_add(1).ok_or_else(|| {
            error(
                "source_sequence_exhausted",
                "Source sequence numbers are exhausted.",
                &root,
            )
        })?;
        format!("SRC-{number:03}").parse().map_err(|_| {
            error(
                "invalid_source_id",
                "The generated Source ID is invalid.",
                &root,
            )
        })
    }

    fn require_capture_entries(&self, directory: &Path) -> Result<(), WorkError> {
        require_directory(directory)?;
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        for entry in fs::read_dir(directory).map_err(|_| {
            error(
                "source_recovery_evidence_incomplete",
                "Capture inventory could not be read.",
                directory,
            )
        })? {
            let entry = entry.map_err(|_| {
                error(
                    "source_recovery_evidence_incomplete",
                    "Capture inventory could not be read.",
                    directory,
                )
            })?;
            let name = entry.file_name();
            if !matches!(name.to_str(), Some("capture.json" | "prepared.sha256")) {
                return Err(error(
                    "source_capture_foreign_entry",
                    "Capture inventory contains foreign evidence.",
                    &entry.path(),
                ));
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| {
                    error(
                        "source_identity_mismatch",
                        "Capture inventory escaped its project.",
                        &entry.path(),
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let checked = crate::files::resolve_runtime_path(&root, &relative)?;
            regular_bytes(&checked)?;
        }
        Ok(())
    }

    fn publish_staging_with_hook(
        &self,
        source_root: &str,
        snapshot: &SnapshotBytes,
        mut after_stage: impl FnMut(CaptureStage) -> Result<(), WorkError>,
    ) -> Result<(), WorkError> {
        let evidence = self.capture_evidence(source_root, snapshot)?;
        let requirement: RequirementId = snapshot
            .manifest
            .requirement_id
            .parse()
            .expect("validated requirement");
        let prepared = self.staging_path(&requirement, &snapshot.manifest.source_id)?;
        let formal = self.require_capture_namespace(source_root)?;
        let target = formal.join(snapshot.manifest.source_id.as_str());
        crate::files::require_runtime_same_filesystem(&prepared, &target)?;
        LocalFiles.create_directories(prepared.parent().expect("capture parent"))?;
        fs::create_dir(&prepared).map_err(|_| {
            error(
                "source_capture_exists",
                "Capture evidence must be allocated exclusively.",
                &prepared,
            )
        })?;
        let raw = serde_json::to_vec(&evidence).map_err(|_| {
            error(
                "source_encode_failed",
                "Capture evidence could not be encoded.",
                &prepared,
            )
        })?;
        LocalFiles.create_new(&prepared.join("capture.json"), &raw)?;
        LocalFiles.create_new(
            &prepared.join("prepared.sha256"),
            &work_operations::derivation::publication::completion_marker(&raw),
        )?;
        self.require_capture_entries(&prepared)?;
        after_stage(CaptureStage::Prepared)?;
        LocalFiles.create_directories(&formal)?;
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
        after_stage(CaptureStage::BeforeMarker)?;
        write_completion_marker(&target.join(MANIFEST), &target.join(MARKER))?;
        after_stage(CaptureStage::AfterMarker)?;
        self.verify_and_clean_capture(
            &requirement,
            &snapshot.manifest.source_id,
            source_root,
            &mut after_stage,
        )?;
        Ok(())
    }

    fn read_staging_formal(
        &self,
        requirement: &RequirementId,
        source: &SourceId,
        source_root: &str,
    ) -> Result<SnapshotBytes, WorkError> {
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        let prefix = format!("{source_root}/{}", source.as_str());
        let manifest = crate::files::resolve_runtime_path(&root, &format!("{prefix}/{MANIFEST}"))?;
        crate::files::resolve_runtime_path(&root, &format!("{prefix}/{MARKER}"))?;
        let parsed: SourceSnapshot =
            serde_json::from_slice(&regular_bytes(&manifest)?).map_err(|_| {
                error(
                    "invalid_source_manifest",
                    "Source metadata does not match its contract.",
                    &manifest,
                )
            })?;
        crate::files::resolve_runtime_path(
            &root,
            &format!("{prefix}/{}", parsed.content.path.as_str()),
        )?;
        self.read_snapshot_at(requirement, source, source_root)
    }

    fn verify_and_clean_capture(
        &self,
        requirement: &RequirementId,
        source: &SourceId,
        source_root: &str,
        after_stage: &mut impl FnMut(CaptureStage) -> Result<(), WorkError>,
    ) -> Result<SnapshotBytes, WorkError> {
        let saved = self.read_staging_formal(requirement, source, source_root)?;
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        for name in [MANIFEST, saved.manifest.content.path.as_str(), MARKER] {
            crate::files::resolve_runtime_path(
                &root,
                &format!("{source_root}/{}/{name}", source.as_str()),
            )?;
        }
        let directory = self.staging_path(requirement, source)?;
        if directory.symlink_metadata().is_err() {
            return Ok(saved);
        }
        self.require_capture_entries(&directory)?;
        let mut expected_manifest = serde_json::to_vec_pretty(&saved.manifest).map_err(|_| {
            error(
                "source_encode_failed",
                "Source metadata could not be encoded.",
                &directory,
            )
        })?;
        expected_manifest.push(b'\n');
        let formal_prefix = format!("{source_root}/{}", source.as_str());
        if regular_bytes(&root.join(format!("{formal_prefix}/{MANIFEST}")))? != expected_manifest
            || regular_bytes(&root.join(format!("{formal_prefix}/{MARKER}")))?
                != work_operations::derivation::publication::completion_marker(&expected_manifest)
        {
            return Err(error(
                "source_capture_formal_changed",
                "Formal Source bytes differ from the frozen publication.",
                &directory,
            ));
        }
        let capture = directory.join("capture.json");
        let marker = directory.join("prepared.sha256");
        if capture.symlink_metadata().is_ok() {
            let expected = self.capture_evidence(source_root, &saved)?;
            let raw = regular_bytes(&capture)?;
            if raw
                != serde_json::to_vec(&expected).map_err(|_| {
                    error(
                        "source_encode_failed",
                        "Capture evidence could not be encoded.",
                        &capture,
                    )
                })?
            {
                return Err(error(
                    "source_capture_identity_changed",
                    "Capture evidence differs from the verified formal Source.",
                    &capture,
                ));
            }
            if marker.symlink_metadata().is_ok() {
                let digest = regular_bytes(&marker)?;
                if completion_state(&raw, Some(&digest)) != CompletionState::Completed {
                    return Err(error(
                        "source_recovery_evidence_incomplete",
                        "Capture marker does not match its evidence.",
                        &marker,
                    ));
                }
                fs::remove_file(&marker).map_err(|_| {
                    error(
                        "source_capture_cleanup_required",
                        "Capture marker cleanup failed.",
                        &marker,
                    )
                })?;
                after_stage(CaptureStage::CleanupMarkerRemoved)?;
            }
            self.require_capture_entries(&directory)?;
            if regular_bytes(&capture)? != raw
                || self.read_staging_formal(requirement, source, source_root)? != saved
            {
                return Err(error(
                    "source_capture_identity_changed",
                    "Capture or formal evidence changed before final manifest cleanup.",
                    &capture,
                ));
            }
            fs::remove_file(&capture).map_err(|_| {
                error(
                    "source_capture_cleanup_required",
                    "Capture manifest cleanup failed.",
                    &capture,
                )
            })?;
            after_stage(CaptureStage::CleanupManifestRemoved)?;
        } else if marker.symlink_metadata().is_ok() {
            return Err(error(
                "source_recovery_evidence_incomplete",
                "Capture marker has no matching manifest.",
                &marker,
            ));
        }
        fs::remove_dir(&directory).map_err(|_| {
            error(
                "source_capture_cleanup_required",
                "Capture directory cleanup failed.",
                &directory,
            )
        })?;
        after_stage(CaptureStage::CleanupDirectoryRemoved)?;
        Ok(saved)
    }

    /// Read-only inventory used by the prepared workflow scanner. `true` means
    /// formal publication is verified and only owned capture cleanup remains.
    pub fn staging_captures_at(
        &self,
        requirement: &RequirementId,
        source_root: &str,
    ) -> Result<Vec<(SourceId, bool)>, WorkError> {
        self.require_capture_namespace(source_root)?;
        let first: SourceId = "SRC-001".parse().expect("initial Source ID");
        let family = self
            .staging_path(requirement, &first)?
            .parent()
            .expect("capture family parent")
            .to_path_buf();
        if family.symlink_metadata().is_err() {
            return Ok(Vec::new());
        }
        require_directory(&family)?;
        let mut states = Vec::new();
        for entry in fs::read_dir(&family).map_err(|_| {
            error(
                "source_recovery_evidence_incomplete",
                "Capture inventory could not be read.",
                &family,
            )
        })? {
            let entry = entry.map_err(|_| {
                error(
                    "source_recovery_evidence_incomplete",
                    "Capture inventory could not be read.",
                    &family,
                )
            })?;
            let name = entry.file_name();
            let source: SourceId = name
                .to_str()
                .ok_or_else(|| {
                    error(
                        "invalid_source_namespace",
                        "A capture name is not portable.",
                        &entry.path(),
                    )
                })?
                .parse()
                .map_err(|_| {
                    error(
                        "invalid_source_namespace",
                        "A capture Source ID is invalid.",
                        &entry.path(),
                    )
                })?;
            let directory = self.staging_path(requirement, &source)?;
            self.require_capture_entries(&directory)?;
            let saved = match self.read_staging_formal(requirement, &source, source_root) {
                Ok(saved) => Some(saved),
                Err(issue) if issue.reason_code.starts_with("runtime_path_") => return Err(issue),
                Err(_) => None,
            };
            let capture = directory.join("capture.json");
            let marker = directory.join("prepared.sha256");
            if capture.symlink_metadata().is_ok() {
                let raw = regular_bytes(&capture)?;
                let evidence: PreparedCapture = serde_json::from_slice(&raw).map_err(|_| {
                    error(
                        "source_recovery_evidence_incomplete",
                        "Capture evidence is incomplete or invalid.",
                        &capture,
                    )
                })?;
                let snapshot = SnapshotBytes {
                    manifest: evidence.manifest,
                    bytes: evidence.bytes,
                };
                let expected = self.capture_evidence(source_root, &snapshot)?;
                if snapshot.manifest.requirement_id != requirement.as_str()
                    || snapshot.manifest.source_id != source
                    || raw
                        != serde_json::to_vec(&expected).map_err(|_| {
                            error(
                                "source_encode_failed",
                                "Capture evidence could not be encoded.",
                                &capture,
                            )
                        })?
                {
                    return Err(error(
                        "source_capture_identity_changed",
                        "Capture evidence does not match the requested namespace.",
                        &capture,
                    ));
                }
                if let Some(saved) = &saved {
                    source_snapshot::validate_immutable(
                        &saved.manifest,
                        &saved.bytes,
                        &snapshot.manifest,
                        &snapshot.bytes,
                    )
                    .map_err(|issue| error(issue.reason_code, issue.message, &capture))?;
                }
                if marker.symlink_metadata().is_ok() {
                    let digest = regular_bytes(&marker)?;
                    if completion_state(&raw, Some(&digest)) != CompletionState::Completed {
                        return Err(error(
                            "source_recovery_evidence_incomplete",
                            "Capture marker is incomplete or invalid.",
                            &marker,
                        ));
                    }
                } else if saved.is_none() {
                    return Err(error(
                        "source_recovery_evidence_incomplete",
                        "An incomplete Source requires complete capture proof.",
                        &capture,
                    ));
                }
            } else if saved.is_none() || marker.symlink_metadata().is_ok() {
                return Err(error(
                    "source_recovery_evidence_incomplete",
                    "Capture proof is missing or incomplete.",
                    &directory,
                ));
            }
            states.push((source, saved.is_some()));
        }
        states.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
        Ok(states)
    }

    #[cfg(test)]
    pub(crate) fn interrupt_staging_capture_for_test(
        &self,
        requirement: &RequirementId,
        source_root: &str,
        published: bool,
    ) -> Result<SnapshotBytes, WorkError> {
        let bytes = b"captured immutable evidence".to_vec();
        let snapshot = SnapshotBytes {
            manifest: SourceSnapshot {
                schema: PublicSchema::WorkSourceSnapshot,
                requirement_id: requirement.as_str().into(),
                source_id: self.next_staging_source_id(requirement, source_root)?,
                captured_at: "2026-10-06T02:00:00Z".into(),
                source: SnapshotSource::UserText {},
                content: SnapshotContent {
                    path: "source.txt"
                        .to_owned()
                        .try_into()
                        .expect("Source content path"),
                    sha256: fingerprint::raw(&bytes),
                    size: bytes.len() as u64,
                },
            },
            bytes,
        };
        let boundary = if published {
            CaptureStage::AfterMarker
        } else {
            CaptureStage::ContentPartial
        };
        let result = self.with_runtime_source_writer(requirement, |_| {
            self.publish_staging_with_hook(source_root, &snapshot, |stage| {
                if stage == boundary {
                    Err(error(
                        "injected_capture_fixture",
                        "Injected capture fixture.",
                        &self.project_root,
                    ))
                } else {
                    Ok(())
                }
            })
        });
        match result {
            Err(issue) if issue.reason_code == "injected_capture_fixture" => Ok(snapshot),
            Err(issue) => Err(issue),
            Ok(()) => Err(error(
                "capture_fixture_boundary_missing",
                "The intended capture boundary was not reached.",
                &self.project_root,
            )),
        }
    }

    pub fn capture_with_staging(
        &self,
        requirement: &RequirementId,
        source: &SnapshotSource,
        content_path: &SourceContentPath,
        bytes: &[u8],
        captured_at: &str,
    ) -> Result<SnapshotBytes, WorkError> {
        self.with_runtime_source_writer(requirement, |_| {
            let source_root = default_artifact_paths(requirement).source;
            let mut snapshot = SnapshotBytes {
                manifest: SourceSnapshot {
                    schema: PublicSchema::WorkSourceSnapshot,
                    requirement_id: requirement.as_str().into(),
                    source_id: "SRC-001".parse().expect("initial Source ID"),
                    captured_at: captured_at.into(),
                    source: source.clone(),
                    content: SnapshotContent {
                        path: content_path.clone(),
                        sha256: fingerprint::raw(bytes),
                        size: bytes.len() as u64,
                    },
                },
                bytes: bytes.to_vec(),
            };
            self.capture_evidence(&source_root, &snapshot)?;
            snapshot.manifest.source_id = self.next_staging_source_id(requirement, &source_root)?;
            self.publish_staging_with_hook(&source_root, &snapshot, |_| Ok(()))?;
            Ok(snapshot)
        })
    }

    pub fn recover_with_staging(
        &self,
        requirement: &RequirementId,
        source: &SourceId,
    ) -> Result<SnapshotBytes, WorkError> {
        self.with_runtime_source_writer(requirement, |_| {
            let source_root = default_artifact_paths(requirement).source;
            let formal = self.require_capture_namespace(&source_root)?;
            let directory = self.staging_path(requirement, source)?;
            let root = self.project_root.canonicalize().map_err(|_| {
                error(
                    "source_project_root",
                    "The project root could not be resolved.",
                    &self.project_root,
                )
            })?;
            let formal_manifest = crate::files::resolve_runtime_path(
                &root,
                &format!("{source_root}/{}/{MANIFEST}", source.as_str()),
            )?;
            let formal_marker = crate::files::resolve_runtime_path(
                &root,
                &format!("{source_root}/{}/{MARKER}", source.as_str()),
            )?;
            let committed = if formal_marker.symlink_metadata().is_ok() {
                let manifest_bytes = regular_bytes(&formal_manifest)?;
                let marker_bytes = regular_bytes(&formal_marker)?;
                completion_state(&manifest_bytes, Some(&marker_bytes)) == CompletionState::Completed
            } else {
                false
            };
            if committed {
                return self.verify_and_clean_capture(
                    requirement,
                    source,
                    &source_root,
                    &mut |_| Ok(()),
                );
            }
            if !directory.exists() {
                return Err(error(
                    "source_recovery_evidence_missing",
                    "An incomplete Source requires complete capture evidence.",
                    &directory,
                ));
            }
            self.require_capture_entries(&directory)?;
            let raw = regular_bytes(&directory.join("capture.json"))?;
            let marker = regular_bytes(&directory.join("prepared.sha256"))?;
            if completion_state(&raw, Some(&marker)) != CompletionState::Completed {
                return Err(error(
                    "source_recovery_evidence_incomplete",
                    "Capture evidence is incomplete or changed.",
                    &directory,
                ));
            }
            let evidence: PreparedCapture = serde_json::from_slice(&raw).map_err(|_| {
                error(
                    "invalid_source_recovery_evidence",
                    "Capture evidence does not match its contract.",
                    &directory,
                )
            })?;
            let snapshot = SnapshotBytes {
                manifest: evidence.manifest.clone(),
                bytes: evidence.bytes.clone(),
            };
            let expected = self.capture_evidence(&source_root, &snapshot)?;
            if snapshot.manifest.requirement_id != requirement.as_str()
                || &snapshot.manifest.source_id != source
                || raw
                    != serde_json::to_vec(&expected).map_err(|_| {
                        error(
                            "source_encode_failed",
                            "Capture evidence could not be encoded.",
                            &directory,
                        )
                    })?
            {
                return Err(error(
                    "source_capture_identity_changed",
                    "Capture evidence does not match the requested Source identity.",
                    &directory,
                ));
            }
            let target = formal.join(source.as_str());
            crate::files::require_runtime_same_filesystem(&directory, &target)?;
            let root = self.project_root.canonicalize().map_err(|_| {
                error(
                    "source_project_root",
                    "The project root could not be resolved.",
                    &self.project_root,
                )
            })?;
            let mut manifest = serde_json::to_vec_pretty(&snapshot.manifest).map_err(|_| {
                error(
                    "source_encode_failed",
                    "Source metadata could not be encoded.",
                    &target,
                )
            })?;
            manifest.push(b'\n');
            LocalFiles.create_directories(&target)?;
            for (name, bytes) in [
                (MANIFEST, manifest.as_slice()),
                (
                    snapshot.manifest.content.path.as_str(),
                    snapshot.bytes.as_slice(),
                ),
            ] {
                let relative = format!("{source_root}/{}/{name}", source.as_str());
                let path = crate::files::resolve_runtime_path(&root, &relative)?;
                complete_write(&path, bytes)?;
            }
            let marker_path = crate::files::resolve_runtime_path(
                &root,
                &format!("{source_root}/{}/{MARKER}", source.as_str()),
            )?;
            complete_write(
                &marker_path,
                &work_operations::derivation::publication::completion_marker(&manifest),
            )?;
            self.verify_and_clean_capture(requirement, source, &source_root, &mut |_| Ok(()))
        })
    }

    pub fn capture_with_runtime(
        &self,
        requirement_id: &RequirementId,
        source: &SnapshotSource,
        content_path: &SourceContentPath,
        bytes: &[u8],
        captured_at: &str,
    ) -> Result<SnapshotBytes, WorkError> {
        self.capture_with_staging(requirement_id, source, content_path, bytes, captured_at)
    }

    /// Candidate critical section shared by capture and recovery after family activation.
    pub fn with_runtime_source_writer<T>(
        &self,
        requirement_id: &RequirementId,
        write: impl FnOnce(&work_model::runtime::RuntimeOwner) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        let root = self.project_root.canonicalize().map_err(|_| {
            error(
                "source_project_root",
                "The project root could not be resolved.",
                &self.project_root,
            )
        })?;
        work_feature::ports::with_runtime_writer(
            &LocalWriterLock,
            &work_feature::ports::RequirementWriterContext {
                canonical_project_root: root,
                requirement_id: requirement_id.clone(),
            },
            work_model::runtime::LockClass::Source,
            write,
        )
    }

    #[cfg(test)]
    fn source_root(&self, id: &RequirementId) -> Result<PathBuf, WorkError> {
        LocalArtifactPaths {
            project_root: self.project_root.clone(),
        }
        .resolve(&default_artifact_paths(id).source)
    }

    #[cfg(test)]
    fn publish_new(&self, root: &Path, snapshot: &SnapshotBytes) -> Result<(), WorkError> {
        self.publish_with_hook(root, snapshot, |_| Ok(()))
    }

    #[cfg(test)]
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
        self.recover_with_runtime(requirement_id, source_id)
    }

    pub fn recover_with_runtime(
        &self,
        requirement_id: &RequirementId,
        source_id: &SourceId,
    ) -> Result<SnapshotBytes, WorkError> {
        self.recover_with_staging(requirement_id, source_id)
    }

    #[cfg(test)]
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
        self.capture_with_runtime(requirement_id, source, content_path, bytes, captured_at)
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
    #[test]
    fn staging_fixture_builder_stops_at_the_real_prepared_or_published_boundary() {
        for published in [false, true] {
            let store = storage();
            let requirement: RequirementId = "example".parse().unwrap();
            let source_root = default_artifact_paths(&requirement).source;
            let snapshot = store
                .interrupt_staging_capture_for_test(&requirement, &source_root, published)
                .unwrap();
            assert_eq!(
                store
                    .staging_captures_at(&requirement, &source_root)
                    .unwrap(),
                vec![(snapshot.manifest.source_id.clone(), published)]
            );
            assert_eq!(
                store
                    .recover_with_staging(&requirement, &snapshot.manifest.source_id)
                    .unwrap(),
                snapshot
            );
            assert!(
                store
                    .staging_captures_at(&requirement, &source_root)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn staging_capture_recovers_every_publication_and_cleanup_boundary_without_changing_formal_bytes()
     {
        for stage in [
            CaptureStage::Prepared,
            CaptureStage::Allocated,
            CaptureStage::ManifestWritten,
            CaptureStage::ContentPartial,
            CaptureStage::BeforeMarker,
            CaptureStage::AfterMarker,
            CaptureStage::CleanupMarkerRemoved,
            CaptureStage::CleanupManifestRemoved,
            CaptureStage::CleanupDirectoryRemoved,
        ] {
            let store = storage();
            let requirement: RequirementId = "example".parse().unwrap();
            let source_root = default_artifact_paths(&requirement).source;
            let first = store
                .capture_with_staging(
                    &requirement,
                    &SnapshotSource::File {
                        path: "original.bin".into(),
                        media_type: "application/octet-stream".into(),
                    },
                    &"source.bin".to_owned().try_into().unwrap(),
                    b"old\0\xff\r\n",
                    "2026-10-06T02:00:00Z",
                )
                .unwrap();
            assert!(
                !store
                    .staging_path(&requirement, &first.manifest.source_id)
                    .unwrap()
                    .exists()
            );
            let formal = store.project_root.join(&source_root);
            let before: Vec<_> = [MANIFEST, MARKER, "source.bin"]
                .iter()
                .map(|name| fs::read(formal.join("SRC-001").join(name)).unwrap())
                .collect();
            let mut next = first.clone();
            next.manifest.source_id = "SRC-002".parse().unwrap();
            next.bytes = b"next\0\xfe\r\n".to_vec();
            next.manifest.content.sha256 = fingerprint::raw(&next.bytes);
            next.manifest.content.size = next.bytes.len() as u64;
            let failure = store
                .with_runtime_source_writer(&requirement, |_| {
                    store.publish_staging_with_hook(&source_root, &next, |current| {
                        if current == stage {
                            Err(error(
                                "injected_capture_boundary",
                                "Injected capture boundary.",
                                &formal,
                            ))
                        } else {
                            Ok(())
                        }
                    })
                })
                .unwrap_err();
            assert_eq!(
                failure.reason_code, "injected_capture_boundary",
                "{stage:?}"
            );
            let states = store
                .staging_captures_at(&requirement, &source_root)
                .unwrap();
            if stage == CaptureStage::CleanupDirectoryRemoved {
                assert!(states.is_empty());
            } else {
                assert_eq!(
                    states,
                    vec![(
                        next.manifest.source_id.clone(),
                        matches!(
                            stage,
                            CaptureStage::AfterMarker
                                | CaptureStage::CleanupMarkerRemoved
                                | CaptureStage::CleanupManifestRemoved
                        )
                    )],
                    "{stage:?}"
                );
            }
            assert_eq!(
                store
                    .recover_with_staging(&requirement, &next.manifest.source_id)
                    .unwrap(),
                next,
                "{stage:?}"
            );
            assert_eq!(
                store
                    .recover_with_staging(&requirement, &next.manifest.source_id)
                    .unwrap(),
                next
            );
            assert!(
                !store
                    .staging_path(&requirement, &next.manifest.source_id)
                    .unwrap()
                    .exists()
            );
            assert_eq!(
                store
                    .next_staging_source_id(&requirement, &source_root)
                    .unwrap()
                    .as_str(),
                "SRC-003"
            );
            for (name, raw) in [MANIFEST, MARKER, "source.bin"].iter().zip(before) {
                assert_eq!(fs::read(formal.join("SRC-001").join(name)).unwrap(), raw);
            }
            assert!(
                !store
                    .project_root
                    .join("outputs/work/runtime/locks/example/source.lock")
                    .exists()
            );
        }
    }

    #[test]
    fn staging_capture_rejects_foreign_cleanup_and_completed_content_drift() {
        let store = storage();
        let requirement: RequirementId = "example".parse().unwrap();
        let source_root = default_artifact_paths(&requirement).source;
        let snapshot = store
            .capture_with_staging(
                &requirement,
                &SnapshotSource::UserText {},
                &"source.txt".to_owned().try_into().unwrap(),
                b"immutable source",
                "2026-10-06T02:00:00Z",
            )
            .unwrap();
        let directory = store
            .staging_path(&requirement, &snapshot.manifest.source_id)
            .unwrap();
        fs::create_dir(&directory).unwrap();
        let evidence = store.capture_evidence(&source_root, &snapshot).unwrap();
        let raw = serde_json::to_vec(&evidence).unwrap();
        fs::write(directory.join("capture.json"), &raw).unwrap();
        fs::write(
            directory.join("prepared.sha256"),
            work_operations::derivation::publication::completion_marker(&raw),
        )
        .unwrap();
        fs::write(directory.join("foreign"), b"foreign evidence").unwrap();
        assert_eq!(
            store
                .recover_with_staging(&requirement, &snapshot.manifest.source_id)
                .unwrap_err()
                .reason_code,
            "source_capture_foreign_entry"
        );
        assert_eq!(fs::read(directory.join("capture.json")).unwrap(), raw);
        fs::remove_file(directory.join("foreign")).unwrap();
        let content = store
            .project_root
            .join(&source_root)
            .join("SRC-001/source.txt");
        fs::write(&content, b"immutable").unwrap();
        assert!(
            store
                .recover_with_staging(&requirement, &snapshot.manifest.source_id)
                .is_err()
        );
        assert_eq!(fs::read(&content).unwrap(), b"immutable");
        assert_eq!(fs::read(directory.join("capture.json")).unwrap(), raw);
        fs::write(&content, &snapshot.bytes).unwrap();
        store
            .recover_with_staging(&requirement, &snapshot.manifest.source_id)
            .unwrap();
        assert!(!directory.exists());
        let legacy = store
            .project_root
            .join(&source_root)
            .join(".capture-SRC-002");
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("journal.json"), b"legacy evidence").unwrap();
        assert_eq!(
            store
                .next_staging_source_id(&requirement, &source_root)
                .unwrap_err()
                .reason_code,
            "legacy_source_capture_present"
        );
        assert_eq!(
            fs::read(legacy.join("journal.json")).unwrap(),
            b"legacy evidence"
        );
    }

    #[test]
    fn staging_allocation_reserves_unknown_evidence_and_missing_proof_never_reconstructs_source() {
        let store = storage();
        let requirement: RequirementId = "example".parse().unwrap();
        let reserved: SourceId = "SRC-005".parse().unwrap();
        let path = store.staging_path(&requirement, &reserved).unwrap();
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("capture.json"), b"partial unknown evidence").unwrap();
        assert_eq!(
            store
                .next_staging_source_id(&requirement, &default_artifact_paths(&requirement).source)
                .unwrap()
                .as_str(),
            "SRC-006"
        );
        assert!(store.recover_with_staging(&requirement, &reserved).is_err());
        assert_eq!(
            fs::read(path.join("capture.json")).unwrap(),
            b"partial unknown evidence"
        );
        assert!(
            !store
                .project_root
                .join(default_artifact_paths(&requirement).source)
                .exists()
        );
    }
    use std::sync::{Arc, Barrier};

    #[test]
    fn runtime_capture_and_recovery_share_exclusive_source_scope() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        let store = storage();
        let id: RequirementId = "example".parse().unwrap();
        let context = work_feature::ports::RequirementWriterContext {
            canonical_project_root: store.project_root.canonicalize().unwrap(),
            requirement_id: id.clone(),
        };
        let content: SourceContentPath = "source.bin".to_owned().try_into().unwrap();
        let held = LocalWriterLock
            .acquire_runtime(&context, work_model::runtime::LockClass::Source)
            .unwrap();
        assert!(
            store
                .capture_with_runtime(
                    &id,
                    &SnapshotSource::File {
                        path: "original.bin".into(),
                        media_type: "application/octet-stream".into()
                    },
                    &content,
                    b"binary\0\xff",
                    "2026-10-06T02:00:00Z"
                )
                .is_err()
        );
        held.release().unwrap();
        let first = store
            .capture_with_runtime(
                &id,
                &SnapshotSource::File {
                    path: "original.bin".into(),
                    media_type: "application/octet-stream".into(),
                },
                &content,
                b"binary\0\xff",
                "2026-10-06T02:00:00Z",
            )
            .unwrap();
        let root = store.source_root(&id).unwrap();
        let before = fs::read(root.join("SRC-001/manifest.json")).unwrap();
        let second = store
            .capture_with_runtime(
                &id,
                &SnapshotSource::File {
                    path: "original.bin".into(),
                    media_type: "application/octet-stream".into(),
                },
                &content,
                b"second",
                "2026-10-06T02:01:00Z",
            )
            .unwrap();
        assert_eq!(second.manifest.source_id.as_str(), "SRC-002");
        assert_eq!(
            store
                .recover_with_runtime(&id, &first.manifest.source_id)
                .unwrap()
                .bytes,
            first.bytes
        );
        assert_eq!(
            fs::read(root.join("SRC-001/manifest.json")).unwrap(),
            before
        );
        assert!(
            store
                .recover_with_runtime(&id, &"SRC-999".parse().unwrap())
                .is_err()
        );
        assert!(!root.join(".work-source-writer.lock").exists());
        LocalWriterLock
            .require_runtime_idle(&context, work_model::runtime::LockClass::Source)
            .unwrap();
    }

    #[test]
    fn runtime_source_section_releases_success_and_early_error() {
        let store = storage();
        let id: RequirementId = "example".parse().unwrap();
        for fail in [false, true] {
            let result = store.with_runtime_source_writer(&id, |owner| {
                assert_eq!(owner.requirement_id, "example");
                assert_eq!(owner.class, work_model::runtime::LockClass::Source);
                assert!(store.with_runtime_source_writer(&id, |_| Ok(())).is_err());
                if fail {
                    Err(error(
                        "injected_source_failure",
                        "Injected failure.",
                        &store.project_root,
                    ))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_err(), fail);
            assert!(
                !store
                    .project_root
                    .join("outputs/work/runtime/locks/example/source.lock")
                    .exists()
            );
            store.with_runtime_source_writer(&id, |_| Ok(())).unwrap();
        }
    }

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
            use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};

            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: store.project_root.canonicalize().unwrap(),
                requirement_id: id.clone(),
            };
            let runtime_guard = {
                LocalWriterLock
                    .acquire_runtime(&context, work_model::runtime::LockClass::Source)
                    .unwrap()
            };

            let hook = |current| {
                if current == stage {
                    Err(error(
                        "injected_interruption",
                        "Injected capture interruption.",
                        &root,
                    ))
                } else {
                    Ok(())
                }
            };
            let result = {
                store.publish_staging_with_hook(&default_artifact_paths(&id).source, &next, hook)
            };
            assert_eq!(result.unwrap_err().reason_code, "injected_interruption");
            {
                let guard = runtime_guard;
                guard.release().unwrap();
            }

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
            { "source_recovery_evidence_missing" }
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
        for capture_evidence in [false, true] {
            let store = storage();
            let id = "example".parse().unwrap();
            let snapshot = {
                store
                    .interrupt_staging_capture_for_test(
                        &id,
                        &default_artifact_paths(&id).source,
                        true,
                    )
                    .unwrap()
            };
            let root = store.source_root(&id).unwrap();
            let path = if capture_evidence {
                {
                    store
                        .staging_path(&id, &snapshot.manifest.source_id)
                        .unwrap()
                }
            } else {
                root.join("SRC-001")
            };
            let moved = store.project_root.join("moved-evidence");
            fs::rename(&path, &moved).unwrap();
            std::os::unix::fs::symlink(&moved, &path).unwrap();
            assert_eq!(
                store
                    .recover(&id, &snapshot.manifest.source_id)
                    .unwrap_err()
                    .reason_code,
                { "runtime_path_alias" }
            );
            fs::remove_dir_all(store.project_root).unwrap();
        }
    }
}
