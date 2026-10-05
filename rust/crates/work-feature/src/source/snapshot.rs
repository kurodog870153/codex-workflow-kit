//! Source capture over host-provided metadata and original payload bytes.

use serde::{Deserialize, Serialize};
use serde_json::json;
use work_model::identifiers::{RequirementId, SourceId};
use work_model::schema::PublicSchema;
use work_model::source::snapshot::{
    SnapshotContent, SnapshotSource, SourceContentPath, SourceRead, SourceSnapshot,
    SourceValidation,
};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::source_snapshot as rules;

use crate::error::{ExitCode, WorkError};
use crate::ports::{SnapshotBytes, SourceSnapshotReader, SourceSnapshotWriter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureMetadata {
    pub requirement_id: String,
    pub captured_at: String,
    pub source: SnapshotSource,
    pub content_path: SourceContentPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_comment_count: Option<u64>,
}

pub fn capture(
    repository: &impl SourceSnapshotWriter,
    metadata: &CaptureMetadata,
    bytes: &[u8],
) -> Result<SnapshotBytes, WorkError> {
    let id: RequirementId = metadata.requirement_id.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_requirement_id",
            "A portable requirement ID is required.",
            json!({}),
        )
    })?;
    if matches!(metadata.source, SnapshotSource::GithubIssue { .. }) {
        let issue = parse_json_contract(bytes).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_issue_content",
                "Issue content must be a complete host-provided JSON snapshot.",
                json!({}),
            )
        })?;
        let comments = issue["comments"].as_array();
        if issue["title"].as_str().is_none()
            || issue["body"].as_str().is_none()
            || comments.is_none()
            || metadata.issue_comment_count != comments.map(|comments| comments.len() as u64)
            || comments.is_some_and(|comments| {
                comments
                    .iter()
                    .any(|comment| comment["body"].as_str().is_none())
            })
        {
            return Err(WorkError::new(
                ExitCode::Contract,
                "incomplete_issue_content",
                "Issue title, body and every comment must be supplied with the total comment count.",
                json!({}),
            ));
        }
    } else if metadata.issue_comment_count.is_some() {
        return Err(WorkError::new(
            ExitCode::Contract,
            "unexpected_issue_comment_count",
            "Comment counts apply only to Issue sources.",
            json!({}),
        ));
    }
    if matches!(metadata.source, SnapshotSource::UserText {}) && std::str::from_utf8(bytes).is_err()
    {
        return Err(WorkError::new(
            ExitCode::InputFormat,
            "invalid_source_text",
            "User Text must be UTF-8.",
            json!({}),
        ));
    }
    let provisional = SourceSnapshot {
        schema: PublicSchema::WorkSourceSnapshot,
        requirement_id: metadata.requirement_id.clone(),
        source_id: "SRC-001".parse().expect("initial Source ID"),
        captured_at: metadata.captured_at.clone(),
        source: metadata.source.clone(),
        content: SnapshotContent {
            path: metadata.content_path.clone(),
            sha256: fingerprint::raw(bytes),
            size: bytes.len() as u64,
        },
    };
    rules::validate(&provisional, Some(bytes)).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            json!({}),
        )
    })?;
    let saved = repository.capture(
        &id,
        &metadata.source,
        &metadata.content_path,
        bytes,
        &metadata.captured_at,
    )?;
    rules::validate(&saved.manifest, Some(&saved.bytes)).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            json!({}),
        )
    })?;
    if saved.bytes != bytes
        || saved.manifest.requirement_id != metadata.requirement_id
        || saved.manifest.source != metadata.source
        || saved.manifest.captured_at != metadata.captured_at
        || saved.manifest.content.path != metadata.content_path
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_capture_mismatch",
            "The captured Snapshot must preserve the host-provided metadata and bytes.",
            json!({}),
        ));
    }
    Ok(saved)
}

pub fn read_snapshot(
    repository: &impl SourceSnapshotReader,
    raw_id: &str,
    raw_source_id: &str,
) -> Result<SourceRead, WorkError> {
    let id: RequirementId = raw_id.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_requirement_id",
            "A portable requirement ID is required.",
            json!({}),
        )
    })?;
    let source_id: SourceId = raw_source_id.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_source_id",
            "A valid Source ID is required.",
            json!({}),
        )
    })?;
    let saved = repository.read_snapshot(&id, &source_id)?;
    if saved.manifest.requirement_id != id.as_str() || saved.manifest.source_id != source_id {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_identity_mismatch",
            "The reader returned a different Source Snapshot.",
            json!({}),
        ));
    }
    rules::validate(&saved.manifest, Some(&saved.bytes)).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            json!({}),
        )
    })?;
    Ok(SourceRead {
        schema: PublicSchema::WorkSourceRead,
        manifest: saved.manifest,
        bytes: saved.bytes,
    })
}

pub fn validate_snapshot(
    repository: &impl SourceSnapshotReader,
    raw_id: &str,
    raw_source_id: &str,
) -> Result<SourceValidation, WorkError> {
    let read = read_snapshot(repository, raw_id, raw_source_id)?;
    Ok(SourceValidation {
        schema: PublicSchema::WorkSourceValidation,
        status: "valid".into(),
        requirement_id: read.manifest.requirement_id,
        source_id: read.manifest.source_id,
        content_sha256: read.manifest.content.sha256,
        content_size: read.manifest.content.size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use work_model::schema::PublicSchema;
    use work_model::source::snapshot::{SnapshotContent, SourceSnapshot};
    use work_operations::derivation::fingerprint;

    struct MemoryWriter(Cell<u64>);

    impl SourceSnapshotWriter for MemoryWriter {
        fn capture(
            &self,
            id: &RequirementId,
            source: &SnapshotSource,
            path: &SourceContentPath,
            bytes: &[u8],
            captured_at: &str,
        ) -> Result<SnapshotBytes, WorkError> {
            self.0.set(self.0.get() + 1);
            Ok(SnapshotBytes {
                manifest: SourceSnapshot {
                    schema: PublicSchema::WorkSourceSnapshot,
                    requirement_id: id.as_str().into(),
                    source_id: format!("SRC-{:03}", self.0.get()).parse().unwrap(),
                    captured_at: captured_at.into(),
                    source: source.clone(),
                    content: SnapshotContent {
                        path: path.clone(),
                        sha256: fingerprint::raw(bytes),
                        size: bytes.len() as u64,
                    },
                },
                bytes: bytes.to_vec(),
            })
        }
    }

    fn metadata(source: SnapshotSource) -> CaptureMetadata {
        CaptureMetadata {
            requirement_id: "example".into(),
            captured_at: "2026-10-03T00:00:00Z".into(),
            source,
            content_path: "source.bin".to_owned().try_into().unwrap(),
            issue_comment_count: None,
        }
    }

    struct MemoryReader(Result<SnapshotBytes, WorkError>);
    impl SourceSnapshotReader for MemoryReader {
        fn read_snapshot_at(
            &self,
            id: &RequirementId,
            source_id: &SourceId,
            _source_root: &str,
        ) -> Result<SnapshotBytes, WorkError> {
            self.read_snapshot(id, source_id)
        }
        fn read_snapshot(
            &self,
            _id: &RequirementId,
            _source_id: &SourceId,
        ) -> Result<SnapshotBytes, WorkError> {
            self.0.clone()
        }
    }

    #[test]
    fn read_and_validate_share_integrity_errors_and_keep_binary_bytes() {
        let writer = MemoryWriter(Cell::new(0));
        let bytes = b"\xff\x00\xef\xbb\xbf\r\n";
        let saved = capture(
            &writer,
            &metadata(SnapshotSource::File {
                path: "input.pdf".into(),
                media_type: "application/pdf".into(),
            }),
            bytes,
        )
        .unwrap();
        let reader = MemoryReader(Ok(saved.clone()));
        assert_eq!(
            read_snapshot(&reader, "example", "SRC-001").unwrap().bytes,
            bytes
        );
        let validation = validate_snapshot(&reader, "example", "SRC-001").unwrap();
        assert_eq!(validation.schema, PublicSchema::WorkSourceValidation);
        assert_eq!(validation.content_sha256, fingerprint::raw(bytes));
        for (changed, expected) in [
            (
                [vec![1], saved.bytes[1..].to_vec()].concat(),
                "source_hash_mismatch",
            ),
            (vec![1], "source_size_mismatch"),
        ] {
            let reader = MemoryReader(Ok(SnapshotBytes {
                manifest: saved.manifest.clone(),
                bytes: changed,
            }));
            assert_eq!(
                read_snapshot(&reader, "example", "SRC-001")
                    .unwrap_err()
                    .reason_code,
                expected
            );
            assert_eq!(
                validate_snapshot(&reader, "example", "SRC-001")
                    .unwrap_err()
                    .reason_code,
                expected
            );
        }
        for expected in ["source_content_missing", "source_snapshot_incomplete"] {
            let reader = MemoryReader(Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                expected,
                "Injected reader failure.",
                json!({}),
            )));
            assert_eq!(
                read_snapshot(&reader, "example", "SRC-001")
                    .unwrap_err()
                    .reason_code,
                expected
            );
            assert_eq!(
                validate_snapshot(&reader, "example", "SRC-001")
                    .unwrap_err()
                    .reason_code,
                expected
            );
        }
        assert_eq!(
            read_snapshot(&reader, "example", "SRC-002")
                .unwrap_err()
                .reason_code,
            "source_identity_mismatch"
        );
    }

    #[test]
    fn all_host_source_kinds_keep_original_payloads_including_links_and_comments() {
        let repository = MemoryWriter(Cell::new(0));
        let mut issue = metadata(SnapshotSource::GithubIssue {
            repository: "owner/repo".into(),
            number: 68,
            url: "https://github.com/owner/repo/issues/68".into(),
        });
        issue.issue_comment_count = Some(2);
        let issue_bytes = br#"{ "title":"Issue", "body":"[attachment](https://example.com/raw.pdf)", "comments":[{"id":1,"body":"First link https://example.com/a"},{"id":2,"body":"Second comment"}], "extra":"preserved" }"#;
        assert_eq!(
            capture(&repository, &issue, issue_bytes).unwrap().bytes,
            issue_bytes
        );
        let file_bytes = b"%PDF\r\n\xff\x00\xef\xbb\xbf";
        assert_eq!(
            capture(
                &repository,
                &metadata(SnapshotSource::File {
                    path: "原始需求.pdf".into(),
                    media_type: "application/pdf".into(),
                }),
                file_bytes
            )
            .unwrap()
            .bytes,
            file_bytes
        );
        let text_bytes = "\u{feff}  原樣文字\r\n  ".as_bytes();
        assert_eq!(
            capture(
                &repository,
                &metadata(SnapshotSource::UserText {}),
                text_bytes
            )
            .unwrap()
            .bytes,
            text_bytes
        );
        assert_eq!(repository.0.get(), 3);
    }

    #[test]
    fn invalid_file_media_type_fails_before_storage_and_metadata_is_preserved() {
        let repository = MemoryWriter(Cell::new(0));
        let mut input = metadata(SnapshotSource::File {
            path: "原始需求.pdf".into(),
            media_type: "Application/PDF".into(),
        });
        let saved = capture(&repository, &input, b"\xff\x00\r\n").unwrap();
        assert_eq!(saved.manifest.source, input.source);
        for media_type in ["", "pdf", "application/pdf\r\n"] {
            input.source = SnapshotSource::File {
                path: "原始需求.pdf".into(),
                media_type: media_type.into(),
            };
            assert_eq!(
                capture(&repository, &input, b"\xff\x00\r\n")
                    .unwrap_err()
                    .reason_code,
                "invalid_source_media_type"
            );
        }
        assert_eq!(repository.0.get(), 1);
    }

    #[test]
    fn truncated_issue_comments_and_invalid_text_fail_before_storage() {
        let repository = MemoryWriter(Cell::new(0));
        let mut issue = metadata(SnapshotSource::GithubIssue {
            repository: "owner/repo".into(),
            number: 68,
            url: "https://github.com/owner/repo/issues/68".into(),
        });
        issue.issue_comment_count = Some(2);
        assert_eq!(
            capture(
                &repository,
                &issue,
                br#"{"title":"Issue","body":"body","comments":[]}"#
            )
            .unwrap_err()
            .reason_code,
            "incomplete_issue_content"
        );
        assert_eq!(
            capture(&repository, &metadata(SnapshotSource::UserText {}), b"\xff")
                .unwrap_err()
                .reason_code,
            "invalid_source_text"
        );
        assert_eq!(repository.0.get(), 0);
        let mut invalid_metadata = metadata(SnapshotSource::UserText {});
        invalid_metadata.captured_at = "2026-02-29T00:00:00Z".into();
        assert_eq!(
            capture(&repository, &invalid_metadata, b"valid text")
                .unwrap_err()
                .reason_code,
            "invalid_source_timestamp"
        );
        assert_eq!(repository.0.get(), 0);
    }
}
