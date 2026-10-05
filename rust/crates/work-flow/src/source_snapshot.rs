//! Source flows never fetch or parse external documents.

use work_feature::error::WorkError;
use work_feature::ports::{SnapshotBytes, SourceSnapshotReader, SourceSnapshotWriter};
pub use work_feature::source::snapshot::CaptureMetadata;
use work_model::source::snapshot::{SourceRead, SourceValidation};

pub fn capture(
    repository: &impl SourceSnapshotWriter,
    metadata: &CaptureMetadata,
    bytes: &[u8],
) -> Result<SnapshotBytes, WorkError> {
    work_feature::source::snapshot::capture(repository, metadata, bytes)
}

pub fn read_snapshot(
    repository: &impl SourceSnapshotReader,
    requirement_id: &str,
    source_id: &str,
) -> Result<SourceRead, WorkError> {
    work_feature::source::snapshot::read_snapshot(repository, requirement_id, source_id)
}

pub fn validate_snapshot(
    repository: &impl SourceSnapshotReader,
    requirement_id: &str,
    source_id: &str,
) -> Result<SourceValidation, WorkError> {
    work_feature::source::snapshot::validate_snapshot(repository, requirement_id, source_id)
}
