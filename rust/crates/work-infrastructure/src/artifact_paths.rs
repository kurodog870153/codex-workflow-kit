//! Project-scoped artifact paths and exclusive raw-file operations.

use std::path::{Path, PathBuf};

use serde_json::json;
use work_feature::artifact_paths::{ArtifactPathRepository, ArtifactPaths};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_model::identifiers::RequirementId;

use crate::files::{LocalFiles, resolve_project_path};

#[derive(Debug, Clone)]
pub struct LocalArtifactPaths {
    pub project_root: PathBuf,
}

impl ArtifactPathRepository for LocalArtifactPaths {
    fn resolve(&self, relative: &str) -> Result<PathBuf, WorkError> {
        let candidate = resolve_project_path(&self.project_root, relative)?.1;
        let mut ancestor = candidate.as_path();
        while !ancestor.exists() {
            ancestor = ancestor.parent().expect("resolved project ancestor");
        }
        let canonical = ancestor.canonicalize().map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "path_resolution_failed",
                "The artifact path could not be resolved.",
                json!({"path": relative}),
            )
        })?;
        let suffix = candidate
            .strip_prefix(ancestor)
            .expect("existing path ancestor");
        if suffix.as_os_str().is_empty() {
            Ok(canonical)
        } else {
            Ok(canonical.join(suffix))
        }
    }

    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        self.resolve(relative)?.try_exists().map_err(|error| {
            WorkError::new(
                ExitCode::IoFailure,
                "artifact_stat_failed",
                "The artifact could not be inspected.",
                json!({"path": relative, "error": error.to_string()}),
            )
        })
    }

    fn read_raw(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&self.resolve(relative)?)
    }

    fn create_new(&self, relative: &str, bytes: &[u8]) -> Result<(), WorkError> {
        LocalFiles.create_new(&self.resolve(relative)?, bytes)
    }

    fn validate_paths(
        &self,
        requirement_id: &RequirementId,
        paths: &ArtifactPaths,
    ) -> Result<(), WorkError> {
        let id = requirement_id.as_str();
        let source = self.resolve(&paths.source)?;
        let task = self.resolve(&paths.task)?;
        let execution = self.resolve(&paths.execution)?;
        if source.file_name().and_then(|name| name.to_str()) != Some(id)
            || execution.file_name().and_then(|name| name.to_str()) != Some(id)
            || task.file_name().and_then(|name| name.to_str()) != Some("index.json")
            || task
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                != Some(id)
        {
            return Err(WorkError::new(
                ExitCode::Contract,
                "artifact_path_requirement_mismatch",
                "Artifact paths must identify the same requirement.",
                json!({"requirement_id": id}),
            ));
        }
        let roots = [
            source,
            task.parent()
                .expect("validated task directory")
                .to_path_buf(),
            execution,
        ];
        let identities: Vec<_> = roots
            .iter()
            .map(|root| work_operations::canonical::portable_path_identity(&root.to_string_lossy()))
            .collect();
        for first in 0..identities.len() {
            for second in first + 1..identities.len() {
                let a = &identities[first];
                let b = &identities[second];
                if a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/")) {
                    return Err(WorkError::new(
                        ExitCode::Contract,
                        "artifact_path_alias",
                        "Artifact directories must have distinct portable identities.",
                        json!({}),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl work_feature::ports::SourceSnapshotReader for LocalArtifactPaths {
    fn read_snapshot(
        &self,
        id: &work_model::identifiers::RequirementId,
        source_id: &work_model::identifiers::SourceId,
    ) -> Result<work_feature::ports::SnapshotBytes, WorkError> {
        work_feature::ports::SourceSnapshotReader::read_snapshot(
            &crate::source_snapshot_storage::LocalSourceSnapshotStorage {
                project_root: self.project_root.clone(),
            },
            id,
            source_id,
        )
    }
    fn read_snapshot_at(
        &self,
        id: &work_model::identifiers::RequirementId,
        source_id: &work_model::identifiers::SourceId,
        root: &str,
    ) -> Result<work_feature::ports::SnapshotBytes, WorkError> {
        work_feature::ports::SourceSnapshotReader::read_snapshot_at(
            &crate::source_snapshot_storage::LocalSourceSnapshotStorage {
                project_root: self.project_root.clone(),
            },
            id,
            source_id,
            root,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> PathBuf {
        static NEXT_ROOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "work-artifact-paths-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ROOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn paths_and_exclusive_raw_io_work_without_a_planning_repository() {
        let root = root();
        let adapter = LocalArtifactPaths {
            project_root: root.clone(),
        };
        let paths = adapter.default_paths(&"example".parse().unwrap()).unwrap();
        assert!(
            adapter
                .resolve(&paths.source)
                .unwrap()
                .starts_with(root.canonicalize().unwrap())
        );
        assert!(!adapter.exists("source.bin").unwrap());
        adapter.create_new("source.bin", b"\xff\x00\r\n").unwrap();
        assert!(adapter.create_new("source.bin", b"replacement").is_err());
        assert_eq!(adapter.read_raw("source.bin").unwrap(), b"\xff\x00\r\n");
        assert!(adapter.resolve("../outside").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_locations_keep_identity_and_reject_aliases() {
        let root = root();
        let adapter = LocalArtifactPaths {
            project_root: root.clone(),
        };
        let id = "example".parse().unwrap();
        let mut paths = ArtifactPaths {
            source: "custom/sources/example".into(),
            task: "custom/tasks/example/index.json".into(),
            execution: "custom/executions/example".into(),
        };
        assert!(adapter.validate_paths(&id, &paths).is_ok());
        paths.execution = paths.source.clone();
        assert_eq!(
            adapter.validate_paths(&id, &paths).unwrap_err().reason_code,
            "artifact_path_alias"
        );
        paths.execution = "custom/executions/other".into();
        assert_eq!(
            adapter.validate_paths(&id, &paths).unwrap_err().reason_code,
            "artifact_path_requirement_mismatch"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_directory_aliases_are_rejected() {
        let root = root();
        fs::create_dir(root.join("sources")).unwrap();
        std::os::unix::fs::symlink(root.join("sources"), root.join("executions")).unwrap();
        let adapter = LocalArtifactPaths {
            project_root: root.clone(),
        };
        let paths = ArtifactPaths {
            source: "sources/example".into(),
            task: "tasks/example/index.json".into(),
            execution: "executions/example".into(),
        };
        assert_eq!(
            adapter
                .validate_paths(&"example".parse().unwrap(), &paths)
                .unwrap_err()
                .reason_code,
            "artifact_path_alias"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
