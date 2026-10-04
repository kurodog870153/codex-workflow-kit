//! Pure validation of immutable Source Snapshot metadata and original bytes.

use work_model::identifiers::RequirementId;
use work_model::schema::PublicSchema;
use work_model::source_snapshot::{SnapshotSource, SourceSnapshot};

use crate::derivation::fingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
}

fn issue(reason_code: &'static str, message: &'static str) -> SnapshotIssue {
    SnapshotIssue {
        reason_code,
        message,
    }
}

pub fn validate_metadata(manifest: &SourceSnapshot) -> Result<(), SnapshotIssue> {
    if manifest.schema != PublicSchema::WorkSourceSnapshotV1 {
        return Err(issue(
            "invalid_source_schema",
            "The Source Snapshot schema is required.",
        ));
    }
    if manifest.requirement_id.parse::<RequirementId>().is_err() {
        return Err(issue(
            "invalid_requirement_id",
            "A portable requirement ID is required.",
        ));
    }
    if !valid_timestamp(&manifest.captured_at) {
        return Err(issue(
            "invalid_source_timestamp",
            "A valid UTC capture timestamp is required.",
        ));
    }
    if manifest.content.sha256.len() != 64
        || !manifest
            .content
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(issue(
            "invalid_source_hash",
            "A lowercase SHA-256 digest is required.",
        ));
    }
    match &manifest.source {
        SnapshotSource::GithubIssue {
            repository,
            number,
            url,
        } => {
            let parts: Vec<_> = repository.split('/').collect();
            if parts.len() != 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || matches!(*part, "." | "..")
                        || !part
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                })
                || *number == 0
                || !url.starts_with("https://")
                || !url.ends_with(&format!("/{repository}/issues/{number}"))
            {
                return Err(issue(
                    "invalid_issue_source",
                    "Issue identity and URL must agree.",
                ));
            }
        }
        SnapshotSource::File { path, media_type } => {
            if path.trim().is_empty() || path.contains('\0') {
                return Err(issue(
                    "invalid_file_source",
                    "The original file path is required.",
                ));
            }
            if !valid_media_type(media_type) {
                return Err(issue(
                    "invalid_source_media_type",
                    "File media type must be a host-provided type/subtype name without parameters.",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

// Validate the RFC 6838 restricted names without guessing from payload or filename.
fn valid_media_type(value: &str) -> bool {
    fn restricted_name(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 127
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&byte))
    }
    value
        .split_once('/')
        .is_some_and(|(kind, subtype)| restricted_name(kind) && restricted_name(subtype))
}

pub fn validate(manifest: &SourceSnapshot, bytes: Option<&[u8]>) -> Result<(), SnapshotIssue> {
    validate_metadata(manifest)?;
    let bytes =
        bytes.ok_or_else(|| issue("source_content_missing", "Source content is missing."))?;
    if bytes.len() as u64 != manifest.content.size {
        return Err(issue(
            "source_size_mismatch",
            "Source content size has changed.",
        ));
    }
    if !fingerprint::verify_raw(bytes, &manifest.content.sha256) {
        return Err(issue(
            "source_hash_mismatch",
            "Source content bytes have changed.",
        ));
    }
    if matches!(manifest.source, SnapshotSource::UserText {}) && std::str::from_utf8(bytes).is_err()
    {
        return Err(issue(
            "invalid_source_text",
            "User Text must be UTF-8 without normalization.",
        ));
    }
    Ok(())
}

pub fn validate_immutable(
    saved: &SourceSnapshot,
    saved_bytes: &[u8],
    proposed: &SourceSnapshot,
    proposed_bytes: &[u8],
) -> Result<(), SnapshotIssue> {
    validate(saved, Some(saved_bytes))?;
    validate(proposed, Some(proposed_bytes))?;
    if saved != proposed || saved_bytes != proposed_bytes {
        return Err(issue(
            "source_snapshot_immutable",
            "An existing Snapshot cannot be changed.",
        ));
    }
    Ok(())
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || [0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
            .iter()
            .any(|range| !bytes[range.clone()].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| {
        bytes[range]
            .iter()
            .fold(0u32, |n, b| n * 10 + u32::from(b - b'0'))
    };
    let year = number(0..4);
    let month = number(5..7);
    let day = number(8..10);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0
        && day > 0
        && day <= days
        && number(11..13) < 24
        && number(14..16) < 60
        && number(17..19) < 60
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(bytes: &[u8]) -> SourceSnapshot {
        let value = work_model::contract_data::registry_value()["items"]["work-source-snapshot/v1"]
            ["description"]["example"]
            .clone();
        let mut manifest: SourceSnapshot = serde_json::from_value(value).unwrap();
        manifest.content.sha256 = fingerprint::raw(bytes);
        manifest.content.size = bytes.len() as u64;
        manifest
    }

    #[test]
    fn missing_size_and_same_size_byte_drift_have_distinct_errors() {
        let saved = manifest(b"abc");
        assert_eq!(
            validate(&saved, None).unwrap_err().reason_code,
            "source_content_missing"
        );
        assert_eq!(
            validate(&saved, Some(b"ab")).unwrap_err().reason_code,
            "source_size_mismatch"
        );
        assert_eq!(
            validate(&saved, Some(b"abd")).unwrap_err().reason_code,
            "source_hash_mismatch"
        );
        assert!(validate(&saved, Some(b"abc")).is_ok());
    }

    #[test]
    fn raw_file_bytes_and_text_whitespace_are_preserved() {
        let bytes = b"\xff\x00\xef\xbb\xbf\r\n";
        let mut saved = manifest(bytes);
        assert_eq!(
            validate(&saved, Some(bytes)).unwrap_err().reason_code,
            "invalid_source_text"
        );
        saved.source = SnapshotSource::File {
            path: "input.pdf".into(),
            media_type: "application/pdf".into(),
        };
        assert!(validate(&saved, Some(bytes)).is_ok());
        let text = "\u{feff}  原文\r\n ".as_bytes();
        assert!(validate(&manifest(text), Some(text)).is_ok());
    }

    #[test]
    fn file_media_type_rejects_invalid_names_and_is_immutable_metadata() {
        let mut saved = manifest(b"original bytes");
        for media_type in [
            "application/pdf",
            "Application/PDF",
            "application/octet-stream",
            "application/vnd.example+json",
            "text/plain",
            "x-custom/x-custom",
        ] {
            saved.source = SnapshotSource::File {
                path: "input.bin".into(),
                media_type: media_type.into(),
            };
            assert!(
                validate(&saved, Some(b"original bytes")).is_ok(),
                "{media_type}"
            );
        }
        for media_type in [
            "",
            " ",
            "pdf",
            "/pdf",
            "application/",
            "application/pdf/extra",
            "application/pdf ",
            " application/pdf",
            "application/*",
            "*/pdf",
            "application/pdf; charset=utf-8",
            "application/需",
            "application/pdf\r\n",
            "-app/pdf",
        ] {
            saved.source = SnapshotSource::File {
                path: "input.bin".into(),
                media_type: media_type.into(),
            };
            assert_eq!(
                validate_metadata(&saved).unwrap_err().reason_code,
                "invalid_source_media_type",
                "{media_type}"
            );
        }
        saved.source = SnapshotSource::File {
            path: "input.pdf".into(),
            media_type: "application/pdf".into(),
        };
        let mut changed = saved.clone();
        changed.source = SnapshotSource::File {
            path: "input.pdf".into(),
            media_type: "application/octet-stream".into(),
        };
        assert_eq!(
            validate_immutable(&saved, b"original bytes", &changed, b"original bytes")
                .unwrap_err()
                .reason_code,
            "source_snapshot_immutable"
        );
        saved.source = SnapshotSource::File {
            path: "input.bin".into(),
            media_type: format!("application/{}", "a".repeat(128)),
        };
        assert_eq!(
            validate_metadata(&saved).unwrap_err().reason_code,
            "invalid_source_media_type"
        );
    }

    #[test]
    fn immutable_validation_rejects_valid_replacements_and_metadata_changes() {
        let saved = manifest(b"abc");
        assert!(validate_immutable(&saved, b"abc", &saved, b"abc").is_ok());
        assert_eq!(
            validate_immutable(&saved, b"abc", &manifest(b"abd"), b"abd")
                .unwrap_err()
                .reason_code,
            "source_snapshot_immutable"
        );
        let mut changed = saved.clone();
        changed.source_id = "SRC-002".parse().unwrap();
        assert_eq!(
            validate_immutable(&saved, b"abc", &changed, b"abc")
                .unwrap_err()
                .reason_code,
            "source_snapshot_immutable"
        );
    }

    #[test]
    fn metadata_rejects_invalid_calendar_dates_schema_hash_and_identity() {
        for timestamp in [
            "",
            "2026-02-29T00:00:00Z",
            "2026-10-03T24:00:00Z",
            "2026-10-03T00:00:00+00:00",
        ] {
            let mut saved = manifest(b"");
            saved.captured_at = timestamp.into();
            assert_eq!(
                validate_metadata(&saved).unwrap_err().reason_code,
                "invalid_source_timestamp"
            );
        }
        assert!(valid_timestamp("2024-02-29T23:59:59Z"));
        let mut saved = manifest(b"");
        saved.schema = PublicSchema::WorkTaskIndexV1;
        assert_eq!(
            validate_metadata(&saved).unwrap_err().reason_code,
            "invalid_source_schema"
        );
        saved.schema = PublicSchema::WorkSourceSnapshotV1;
        saved.content.sha256 = "X".repeat(64);
        assert_eq!(
            validate_metadata(&saved).unwrap_err().reason_code,
            "invalid_source_hash"
        );
        saved.content.sha256 = fingerprint::raw(b"");
        saved.source = SnapshotSource::GithubIssue {
            repository: "owner/repo".into(),
            number: 68,
            url: "https://github.com/owner/other/issues/68".into(),
        };
        assert_eq!(
            validate_metadata(&saved).unwrap_err().reason_code,
            "invalid_issue_source"
        );
    }

    #[test]
    fn unsafe_manifest_paths_are_rejected_before_validation() {
        for path in [
            "../source.txt",
            "C:/source.txt",
            "source./file",
            "source.txt:stream",
        ] {
            let mut value = serde_json::to_value(manifest(b"")).unwrap();
            value["content"]["path"] = serde_json::json!(path);
            assert!(serde_json::from_value::<SourceSnapshot>(value).is_err());
        }
    }
}
