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

/// Strict candidate validation shared by the new runtime families, not yet a producer switch.
pub fn resolve_runtime_path(root: &Path, relative: &str) -> Result<PathBuf, WorkError> {
    if !work_operations::derivation::identity::runtime_relative_path(relative) {
        return Err(error(
            ExitCode::Contract,
            "runtime_path_invalid",
            "Runtime paths must use canonical relative segments.",
            root,
        ));
    }
    let root = root.canonicalize().map_err(|_| {
        error(
            ExitCode::IoFailure,
            "runtime_root_unavailable",
            "The canonical project root is unavailable.",
            root,
        )
    })?;
    let mut candidate = root;
    let parts: Vec<_> = relative.split('/').collect();
    let mut missing = false;
    for (index, part) in parts.iter().enumerate() {
        let parent = candidate.clone();
        candidate.push(part);
        if missing {
            continue;
        }
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || runtime_reparse(&metadata)
                    || runtime_hardlink(&candidate, &metadata)?
                {
                    return Err(error(
                        ExitCode::ArtifactIntegrity,
                        "runtime_path_alias",
                        "Runtime paths cannot traverse links, reparse points or file aliases.",
                        &candidate,
                    ));
                }
                if index + 1 < parts.len() && !metadata.is_dir() {
                    return Err(error(
                        ExitCode::ArtifactIntegrity,
                        "runtime_path_not_directory",
                        "A runtime path ancestor is not a directory.",
                        &candidate,
                    ));
                }
                // Case-insensitive filesystems may accept a different spelling for the same entry.
                let exact = fs::read_dir(&parent)
                    .map_err(|_| {
                        error(
                            ExitCode::IoFailure,
                            "runtime_path_unreadable",
                            "A runtime ancestor could not be inspected.",
                            &parent,
                        )
                    })?
                    .try_fold(false, |found, entry| {
                        entry.map(|entry| found || entry.file_name() == *part)
                    })
                    .map_err(|_| {
                        error(
                            ExitCode::IoFailure,
                            "runtime_path_unreadable",
                            "A runtime ancestor could not be inspected.",
                            &parent,
                        )
                    })?;
                require_runtime_exact_entry(&candidate, exact)?;
            }
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(_) => {
                return Err(error(
                    ExitCode::IoFailure,
                    "runtime_path_unreadable",
                    "Runtime metadata could not be inspected.",
                    &candidate,
                ));
            }
        }
    }
    Ok(candidate)
}

fn require_runtime_exact_entry(path: &Path, exact: bool) -> Result<(), WorkError> {
    if exact {
        return Ok(());
    }
    if fs::symlink_metadata(path).is_err_and(|issue| issue.kind() == std::io::ErrorKind::NotFound) {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "runtime_path_changed",
            "The runtime entry disappeared during physical identity inspection.",
            path,
        ));
    }
    Err(error(
        ExitCode::ArtifactIntegrity,
        "runtime_path_alias",
        "Runtime entry spelling does not match its physical name.",
        path,
    ))
}

#[cfg(windows)]
fn runtime_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn runtime_reparse(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn runtime_hardlink(_: &Path, metadata: &fs::Metadata) -> Result<bool, WorkError> {
    use std::os::unix::fs::MetadataExt;
    Ok(metadata.is_file() && metadata.nlink() > 1)
}

#[cfg(windows)]
fn runtime_hardlink(path: &Path, metadata: &fs::Metadata) -> Result<bool, WorkError> {
    if !metadata.is_file() {
        return Ok(false);
    }
    windows_runtime_metadata::multiple_links(path).map_err(|issue| {
        error(
            ExitCode::IoFailure,
            if issue.kind() == std::io::ErrorKind::NotFound {
                "runtime_path_changed"
            } else {
                "runtime_file_identity_unavailable"
            },
            "The physical file identity could not be inspected.",
            path,
        )
    })
}

fn existing_runtime_ancestor(path: &Path) -> Result<PathBuf, WorkError> {
    let mut ancestor = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() || runtime_reparse(&metadata) => {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "runtime_path_alias",
                    "The filesystem ancestor is a link or reparse point.",
                    &ancestor,
                ));
            }
            Ok(_) => return Ok(ancestor),
            Err(issue) if issue.kind() == std::io::ErrorKind::NotFound && ancestor.pop() => {}
            Err(_) => {
                return Err(error(
                    ExitCode::IoFailure,
                    "runtime_filesystem_unavailable",
                    "The filesystem ancestor could not be inspected.",
                    &ancestor,
                ));
            }
        }
    }
}

/// Check the filesystem of actual existing ancestors before any prepare/publication write.
pub fn require_runtime_same_filesystem(staging: &Path, target: &Path) -> Result<(), WorkError> {
    let staging = existing_runtime_ancestor(staging)?;
    let target = existing_runtime_ancestor(target)?;
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&staging)
            .map_err(|_| {
                error(
                    ExitCode::IoFailure,
                    "runtime_filesystem_unavailable",
                    "The staging filesystem is unavailable.",
                    &staging,
                )
            })?
            .dev()
            == fs::metadata(&target)
                .map_err(|_| {
                    error(
                        ExitCode::IoFailure,
                        "runtime_filesystem_unavailable",
                        "The target filesystem is unavailable.",
                        &target,
                    )
                })?
                .dev()
    };
    #[cfg(windows)]
    let same = windows_runtime_metadata::volume(&staging)
        .and_then(|left| windows_runtime_metadata::volume(&target).map(|right| left == right))
        .map_err(|_| {
            error(
                ExitCode::IoFailure,
                "runtime_filesystem_unavailable",
                "The volume identity is unavailable.",
                &target,
            )
        })?;
    if !same {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "runtime_cross_filesystem",
            "Runtime staging and its formal target must share a filesystem.",
            &target,
        ));
    }
    Ok(())
}

#[cfg(windows)]
mod windows_runtime_metadata {
    use super::*;
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    #[repr(C)]
    struct FileInformation {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumePathNameW(path: *const u16, output: *mut u16, length: u32) -> i32;
        fn GetVolumeNameForVolumeMountPointW(
            path: *const u16,
            output: *mut u16,
            length: u32,
        ) -> i32;
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            information: *mut FileInformation,
        ) -> i32;
    }
    pub fn volume(path: &Path) -> std::io::Result<Vec<u16>> {
        let path = path.canonicalize()?;
        let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut mount = vec![0u16; 32768];
        let mut guid = vec![0u16; 32768];
        // SAFETY: NUL-terminated input and writable buffers remain valid for both synchronous calls.
        unsafe {
            if GetVolumePathNameW(input.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) == 0
                || GetVolumeNameForVolumeMountPointW(
                    mount.as_ptr(),
                    guid.as_mut_ptr(),
                    guid.len() as u32,
                ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        let end = guid
            .iter()
            .position(|c| *c == 0)
            .ok_or_else(|| std::io::Error::other("unterminated volume identity"))?;
        if end == 0 {
            return Err(std::io::Error::other("empty volume identity"));
        }
        guid.truncate(end);
        Ok(guid)
    }
    pub fn multiple_links(path: &Path) -> std::io::Result<bool> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x00200000)
            .open(path)?;
        let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
        // SAFETY: The handle remains open, and success initializes the complete documented C structure.
        unsafe {
            if GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(information.assume_init().links > 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-rust-file-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after the Unix epoch")
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn runtime_disappeared_entry_is_distinct_from_a_physical_alias() {
        let root = temp_root();
        let path = root.join("owner.lock");
        fs::write(&path, b"owner").unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(metadata.is_file());
        assert_eq!(
            require_runtime_exact_entry(&path, false)
                .unwrap_err()
                .reason_code,
            "runtime_path_alias"
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(
            require_runtime_exact_entry(&path, false)
                .unwrap_err()
                .reason_code,
            "runtime_path_changed"
        );
    }

    #[test]
    fn runtime_paths_preserve_unicode_and_reject_aliases_before_writes() {
        let root = temp_root();
        let directory = root.join("中文 空白");
        fs::create_dir(&directory).unwrap();
        let original = directory.join("original.json");
        fs::write(&original, b"original").unwrap();
        assert_eq!(
            resolve_runtime_path(&root, "中文 空白/original.json").unwrap(),
            original.canonicalize().unwrap()
        );
        let target = resolve_runtime_path(
            &root,
            "outputs/work/runtime/staging/example/missing/index.json.tmp",
        )
        .unwrap();
        assert!(!target.exists());
        require_runtime_same_filesystem(&target, &original).unwrap();
        for path in ["../outside", "/outside", "a\\b", "a//b"] {
            assert!(resolve_runtime_path(&root, path).is_err(), "{path}");
        }
        let alias = directory.join("alias.json");
        fs::hard_link(&original, &alias).unwrap();
        for path in ["中文 空白/original.json", "中文 空白/alias.json"] {
            assert_eq!(
                resolve_runtime_path(&root, path).unwrap_err().reason_code,
                "runtime_path_alias"
            );
        }
        assert_eq!(fs::read(original).unwrap(), b"original");
        assert!(!root.join("outputs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_gate_rejects_dangling_links_and_actual_other_filesystem() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let root = temp_root();
        symlink(root.join("missing"), root.join("dangling")).unwrap();
        symlink(&root, root.join("alias")).unwrap();
        for path in ["dangling", "alias/new.json"] {
            assert_eq!(
                resolve_runtime_path(&root, path).unwrap_err().reason_code,
                "runtime_path_alias"
            );
        }
        // /dev is an actual devfs/tmpfs mount on the supported Unix runners.
        assert_ne!(
            fs::metadata(&root).unwrap().dev(),
            fs::metadata("/dev").unwrap().dev()
        );
        assert_eq!(
            require_runtime_same_filesystem(&root, Path::new("/dev"))
                .unwrap_err()
                .reason_code,
            "runtime_cross_filesystem"
        );
        assert!(!root.join("alias/new.json").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_gate_rejects_real_junction_reparse_aliases() {
        let root = temp_root();
        fs::create_dir(root.join("original")).unwrap();
        fs::write(root.join("original/evidence.json"), b"immutable evidence").unwrap();
        // Fixed relative operands avoid interpolating user paths into cmd.exe syntax.
        let output = std::process::Command::new("cmd.exe")
            .current_dir(&root)
            .args(["/D", "/C", "mklink", "/J", "alias", "original"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction creation failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(runtime_reparse(
            &fs::symlink_metadata(root.join("alias")).unwrap()
        ));
        for path in ["alias/evidence.json", "alias/new.json"] {
            assert_eq!(
                resolve_runtime_path(&root, path).unwrap_err().reason_code,
                "runtime_path_alias"
            );
        }
        assert_eq!(
            fs::read(root.join("original/evidence.json")).unwrap(),
            b"immutable evidence"
        );
        assert!(!root.join("original/new.json").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_gate_rejects_an_actual_different_volume() {
        let root = temp_root();
        let volume = windows_runtime_metadata::volume(&root).unwrap();
        let other = ('C'..='Z')
            .map(|drive| PathBuf::from(format!("{drive}:\\")))
            .find(|path| {
                windows_runtime_metadata::volume(path).is_ok_and(|candidate| candidate != volume)
            });
        let Some(other) = other else {
            assert_ne!(
                std::env::var("GITHUB_ACTIONS").ok().as_deref(),
                Some("true"),
                "Windows CI must supply an actual second volume; this required case cannot be skipped."
            );
            eprintln!(
                "Local Windows host has no second volume; cross-volume acceptance remains unverified."
            );
            return;
        };
        assert_eq!(
            require_runtime_same_filesystem(&root, &other)
                .unwrap_err()
                .reason_code,
            "runtime_cross_filesystem"
        );
        assert!(!root.join("new.json").exists());
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
    }

    #[test]
    fn normalized_and_resolved_project_paths_match_current_contract_cases() {
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
        let canonical = work_operations::derivation::fingerprint::canonical(&raw).unwrap();
        let original = work_operations::derivation::fingerprint::raw(&raw);
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
