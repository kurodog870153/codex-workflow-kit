//! Specification transaction journal and safe local publication.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
#[cfg(test)]
use work_operations::canonical::sha256_hex;
use work_operations::derivation::fingerprint;
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

pub fn require_no_spec_update(
    root: &Path,
    execution_dir: &str,
    ignored_record: Option<&str>,
) -> Result<(), WorkError> {
    let directory = storage_path(root, execution_dir)?;
    let mut records = Vec::new();
    if directory.is_dir() {
        for entry in fs::read_dir(directory).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "The transaction directory could not be read.",
                json!({}),
            )
        })? {
            let entry = entry.map_err(|_| {
                error(
                    ExitCode::IoFailure,
                    "file_read_failed",
                    "The transaction directory could not be read.",
                    json!({}),
                )
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json")
                && [
                    ".work-spec-update-",
                    ".work-spec-migration-",
                    ".work-source-refresh-",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                records.push(format!("{execution_dir}/{name}"));
            }
        }
    }
    records.sort();
    for relative in records {
        if ignored_record == Some(relative.as_str()) {
            continue;
        }
        let record = storage_path(root, &relative)?;
        let marker = storage_path(
            root,
            &work_operations::derivation::publication::completion_marker_path(&relative),
        )?;
        let raw = LocalFiles.read_raw(&record)?;
        if !marker.is_file() || LocalFiles.read_raw(&marker)? != completion_marker(&raw) {
            return Err(error(
                ExitCode::LockConflict,
                "spec_update_pending",
                "An incomplete specification update requires separately authorized recovery.",
                json!({"recovery_required":true,"record":relative}),
            ));
        }
    }
    Ok(())
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
    let directory = storage_path(root, execution)?;
    let mut result = BTreeMap::new();
    if !directory.is_dir() {
        return Ok(result);
    }
    let entries = fs::read_dir(&directory).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "file_read_failed",
            "The execution history could not be read.",
            json!({}),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "The execution history could not be read.",
                json!({}),
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(TASK_ID_PREFIX) {
            let relative = format!("{execution}/{name}");
            let path = storage_path(root, &relative)?;
            if !path.is_dir() {
                return Err(error(
                    ExitCode::ArtifactIntegrity,
                    "spec_update_history_layout",
                    "An execution TASK entry must be a directory.",
                    json!({}),
                ));
            }
            collect_history_bytes(root, &relative, &mut result)?;
        }
    }
    Ok(result)
}

fn collect_history_bytes(
    root: &Path,
    relative: &str,
    result: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), WorkError> {
    let directory = storage_path(root, relative)?;
    for entry in fs::read_dir(directory).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "file_read_failed",
            "The execution history could not be read.",
            json!({}),
        )
    })? {
        let entry = entry.map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "The execution history could not be read.",
                json!({}),
            )
        })?;
        let child = format!("{relative}/{}", entry.file_name().to_string_lossy());
        let path = storage_path(root, &child)?;
        if path.is_dir() {
            collect_history_bytes(root, &child, result)?;
        } else if path.is_file() {
            result.insert(child, LocalFiles.read_raw(&path)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        for prefix in ["spec-update", "spec-migration"] {
            let relative = format!("execution/.work-{prefix}-example.json");
            let record = root.join(&relative);
            fs::write(&record, b"{}\n").unwrap();
            let marker = root.join(format!("{relative}.done"));
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
                "spec_update_pending"
            );
            fs::write(&marker, completion_marker(b"{}\n")).unwrap();
            require_no_spec_update(&root, "execution", None).unwrap();
        }
    }

    #[test]
    fn guard_preserves_underlying_record_read_failure() {
        let root = policy_test_root("read-failure");
        fs::create_dir_all(root.join("execution/.work-spec-update-example.json")).unwrap();
        let failure = require_no_spec_update(&root, "execution", None).unwrap_err();
        assert_eq!(failure.exit_code, ExitCode::ArtifactIntegrity);
        assert_eq!(failure.reason_code, "file_not_found");
    }

    #[test]
    fn journal_publication_and_resume_match_python_progress() {
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
        let value = json!({"schema":"work-spec-transaction/v1","transaction_id":derived_transaction_id("UPDATE", &approval).unwrap(),"approval_sha256":approval,"state":"prepared","published_count":0,"metadata":metadata,"files":files});
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
