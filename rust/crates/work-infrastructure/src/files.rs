//! Filesystem and document adapters.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{ArtifactStore, DocumentRepository};
use work_operations::identifiers::path_segment_issue;

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFiles;

fn error(code: ExitCode, reason: &str, message: &str, path: &Path) -> WorkError {
    WorkError::new(
        code,
        reason,
        message,
        json!({"path": path.to_string_lossy()}),
    )
}

impl ArtifactStore for LocalFiles {
    fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
        let mut file = File::open(path).map_err(|open_error| {
            if matches!(
                open_error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory
            ) || path.is_dir()
            {
                error(
                    ExitCode::ArtifactIntegrity,
                    "file_not_found",
                    "The required file does not exist or is not a regular file.",
                    path,
                )
            } else {
                error(
                    ExitCode::IoFailure,
                    "file_read_failed",
                    "The file could not be read.",
                    path,
                )
            }
        })?;
        if !file.metadata().map(|meta| meta.is_file()).unwrap_or(false) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "file_not_found",
                "The required file does not exist or is not a regular file.",
                path,
            ));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "The file could not be read.",
                path,
            )
        })?;
        Ok(bytes)
    }

    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| {
                error(
                    ExitCode::IoFailure,
                    "file_write_failed",
                    "The file could not be created.",
                    path,
                )
            })?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| {
                error(
                    ExitCode::IoFailure,
                    "file_write_failed",
                    "The file could not be written.",
                    path,
                )
            })
    }

    fn replace(&self, temporary: &Path, target: &Path) -> Result<(), WorkError> {
        fs::rename(temporary, target).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_replace_failed",
                "The prepared file could not be installed.",
                temporary,
            )
        })
    }

    fn remove(&self, path: &Path) -> Result<(), WorkError> {
        fs::remove_file(path).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_remove_failed",
                "The file could not be removed.",
                path,
            )
        })
    }

    fn create_directories(&self, path: &Path) -> Result<(), WorkError> {
        fs::create_dir_all(path).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "directory_create_failed",
                "The directory could not be created.",
                path,
            )
        })
    }
}

impl DocumentRepository for LocalFiles {
    fn read_json(&self, path: &Path) -> Result<Value, WorkError> {
        let bytes = self.read_raw(path)?;
        let text = decode_document(&bytes, path)?;
        serde_json::from_str(text).map_err(|_| {
            error(
                ExitCode::InputFormat,
                "invalid_json",
                "The file is not valid JSON.",
                path,
            )
        })
    }

    fn write_json_new(&self, path: &Path, value: &Value) -> Result<(), WorkError> {
        let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| {
            error(
                ExitCode::InternalError,
                "json_encode_failed",
                "The JSON file could not be encoded.",
                path,
            )
        })?;
        bytes.push(b'\n');
        self.create_new(path, &bytes)
    }

    fn read_yaml(&self, path: &Path) -> Result<Value, WorkError> {
        let bytes = self.read_raw(path)?;
        let text = decode_document(&bytes, path)?;
        serde_yaml_ng::from_str(text).map_err(|_| {
            error(
                ExitCode::InputFormat,
                "invalid_yaml",
                "The file is not valid YAML.",
                path,
            )
        })
    }
}

pub fn decode_document<'a>(bytes: &'a [u8], path: &Path) -> Result<&'a str, WorkError> {
    let raw = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let text = std::str::from_utf8(raw).map_err(|utf8_error| {
        WorkError::new(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source": path.to_string_lossy(), "byte_offset": utf8_error.valid_up_to()}),
        )
    })?;
    if text.starts_with('\u{feff}') {
        return Err(WorkError::new(
            ExitCode::InputFormat,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            json!({"source": path.to_string_lossy()}),
        ));
    }
    Ok(text)
}

pub fn normalize_relative_path(raw: &str) -> Result<String, WorkError> {
    if raw.is_empty() {
        return Err(WorkError::new(
            ExitCode::Contract,
            "empty_relative_path",
            "The project-relative path cannot be empty.",
            json!({"field": "path"}),
        ));
    }
    if raw.starts_with(['/', '\\']) || raw.as_bytes().get(1) == Some(&b':') {
        return Err(WorkError::new(
            ExitCode::Contract,
            "absolute_path_rejected",
            "The path must be project-relative.",
            json!({"field": "path", "path": raw}),
        ));
    }
    let raw = raw
        .strip_prefix("./")
        .or_else(|| raw.strip_prefix(".\\"))
        .unwrap_or(raw);
    let normalized = raw.replace('\\', "/");
    for segment in normalized.split('/') {
        if let Some(issue) = path_segment_issue(segment) {
            return Err(WorkError::new(
                ExitCode::Contract,
                issue.reason_code(),
                "The path contains an unsafe segment.",
                json!({"field": "path", "segment": segment}),
            ));
        }
    }
    Ok(normalized)
}

pub fn resolve_project_path(root: &Path, raw: &str) -> Result<(String, PathBuf), WorkError> {
    let normalized = normalize_relative_path(raw)?;
    let root = root.canonicalize().map_err(|_| {
        error(
            ExitCode::IoFailure,
            "path_resolution_failed",
            "The project root could not be resolved.",
            root,
        )
    })?;
    let candidate = normalized
        .split('/')
        .fold(root.clone(), |path, part| path.join(part));
    let mut ancestor = candidate.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| {
            error(
                ExitCode::IoFailure,
                "path_resolution_failed",
                "The path could not be resolved.",
                &candidate,
            )
        })?;
    }
    if ancestor != candidate && !ancestor.is_dir() {
        return Err(error(
            ExitCode::IoFailure,
            "path_resolution_failed",
            "The path could not be resolved.",
            &candidate,
        ));
    }
    let resolved = ancestor.canonicalize().map_err(|_| {
        error(
            ExitCode::IoFailure,
            "path_resolution_failed",
            "The path could not be resolved.",
            &candidate,
        )
    })?;
    if !resolved.starts_with(&root) {
        return Err(error(
            ExitCode::Contract,
            "path_escapes_project_root",
            "The resolved path escapes the project root.",
            &candidate,
        ));
    }
    Ok((normalized, candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use work_operations::identifiers::RequirementId;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-rust-file-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn create_replace_and_document_bytes() {
        let root = temp_root();
        let store = LocalFiles;
        let target = root.join("plan.json");
        let staged = root.join("plan.tmp");
        store.create_new(&target, b"old\n").unwrap();
        assert!(store.create_new(&target, b"new\n").is_err());
        assert_eq!(store.read_raw(&target).unwrap(), b"old\n");
        store.create_new(&staged, b"new\n").unwrap();
        store.replace(&staged, &target).unwrap();
        assert_eq!(store.read_raw(&target).unwrap(), b"new\n");
        assert!(!staged.exists());
        let json = root.join("data.json");
        store
            .write_json_new(&json, &json!({"name": "中文"}))
            .unwrap();
        assert_eq!(
            store.read_raw(&json).unwrap(),
            "{\n  \"name\": \"中文\"\n}\n".as_bytes()
        );
        assert_eq!(store.read_json(&json).unwrap()["name"], "中文");
        let yaml = root.join("data.yaml");
        store
            .create_new(&yaml, b"name: \xe4\xb8\xad\xe6\x96\x87\n")
            .unwrap();
        assert_eq!(store.read_yaml(&yaml).unwrap()["name"], "中文");
    }

    #[test]
    fn invalid_sources_and_paths_are_rejected() {
        let root = temp_root();
        let store = LocalFiles;
        assert_eq!(
            store
                .read_raw(&root.join("missing.txt"))
                .unwrap_err()
                .reason_code,
            "file_not_found"
        );
        assert_eq!(
            store.read_raw(&root).unwrap_err().reason_code,
            "file_not_found"
        );
        assert_eq!(
            normalize_relative_path("../outside")
                .unwrap_err()
                .reason_code,
            "unsafe_path_segment"
        );
        assert_eq!(
            normalize_relative_path("C:\\outside")
                .unwrap_err()
                .reason_code,
            "absolute_path_rejected"
        );
        let file = root.join("multiple-bom.json");
        store
            .create_new(&file, b"\xef\xbb\xbf\xef\xbb\xbf{}")
            .unwrap();
        assert_eq!(
            store.read_json(&file).unwrap_err().reason_code,
            "input_file_multiple_bom"
        );
        let id: RequirementId = "example".parse().unwrap();
        assert_eq!(
            work_feature::plan::default_artifact_paths(&id)[0].1,
            "outputs/work/plans/example.json"
        );
    }

    #[test]
    fn normalized_and_resolved_project_paths_match_python_cases() {
        assert_eq!(
            normalize_relative_path(".\\outputs\\work\\task.json").unwrap(),
            "outputs/work/task.json"
        );
        for (raw, expected) in [
            ("../task.json", "unsafe_path_segment"),
            ("C:/task.json", "absolute_path_rejected"),
            ("outputs/NUL.txt", "windows_device_name"),
        ] {
            assert_eq!(
                normalize_relative_path(raw).unwrap_err().reason_code,
                expected
            );
        }
        let root = temp_root();
        let (normalized, resolved) = resolve_project_path(&root, "outputs/work/task.json").unwrap();
        assert_eq!(normalized, "outputs/work/task.json");
        assert_eq!(
            resolved,
            root.canonicalize().unwrap().join("outputs/work/task.json")
        );
        assert!(!resolved.exists());
    }

    #[test]
    fn file_read_preserves_crlf_while_canonical_fingerprint_normalizes_it() {
        let root = temp_root();
        let path = root.join("source.txt");
        fs::write(&path, b"source\r\n").unwrap();
        let raw = LocalFiles.read_raw(&path).unwrap();
        assert_eq!(raw, b"source\r\n");
        let canonical = work_operations::canonical::canonical_sha256(&raw).unwrap();
        let original = work_operations::canonical::sha256_hex(&raw);
        assert_eq!(canonical.len(), 64);
        assert_eq!(original.len(), 64);
        assert_ne!(canonical, original);
        let document_path = root.join("contract.json");
        fs::write(&document_path, b"{\"value\": 1}\r\n").unwrap();
        let document_raw = LocalFiles.read_raw(&document_path).unwrap();
        assert_eq!(document_raw, b"{\"value\": 1}\r\n");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&document_raw).unwrap(),
            json!({"value": 1})
        );
    }
}
