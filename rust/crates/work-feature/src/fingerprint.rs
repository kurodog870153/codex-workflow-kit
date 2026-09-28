//! Text fingerprint response over canonical source bytes.

use std::path::Path;

use serde_json::{Value, json};
use work_operations::canonical::{canonical_sha256, sha256_hex};

use crate::error::{ExitCode, WorkError};

pub fn text(raw: &[u8], relative: &str, source: &Path) -> Result<Value, WorkError> {
    let canonical = canonical_sha256(raw).map_err(|error| {
        WorkError::new(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":source,"byte_offset":error.valid_up_to()}),
        )
    })?;
    Ok(json!({"schema":"work-fingerprint/v1","path":relative,
        "canonical_sha256":canonical,"raw_sha256":sha256_hex(raw)}))
}
