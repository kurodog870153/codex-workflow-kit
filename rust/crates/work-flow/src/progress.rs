//! Progress command flows.

use serde_json::Value;
use work_feature::error::WorkError;
use work_feature::progress::ProgressRepository;

pub fn read(
    repository: &impl ProgressRepository,
    requirement_id: &str,
    mode: &str,
) -> Result<Value, WorkError> {
    work_feature::progress::read_progress(repository, requirement_id, mode)
}

pub fn prepare(
    repository: &impl ProgressRepository,
    raw: &[u8],
    requirement_id: &str,
    mode: &str,
    expected_revision: u64,
) -> Result<Value, WorkError> {
    work_feature::progress::prepare_progress_raw(
        repository,
        raw,
        requirement_id,
        mode,
        expected_revision,
    )
}

pub fn validate(
    repository: &impl ProgressRepository,
    raw: &[u8],
    expected_revision: u64,
) -> Result<Value, WorkError> {
    work_feature::progress::preview_progress_raw(repository, raw, expected_revision)
}

pub fn preview_value(
    repository: &impl ProgressRepository,
    value: &Value,
    expected_revision: u64,
) -> Result<Value, WorkError> {
    work_feature::progress::preview_progress(repository, value, expected_revision)
}

pub fn save(
    repository: &impl ProgressRepository,
    raw: &[u8],
    expected_revision: u64,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    work_feature::progress::save_progress_raw(repository, raw, expected_revision, approved_sha256)
}
