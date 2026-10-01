//! Text fingerprint response over canonical source bytes.

use std::path::Path;

use serde_json::{Value, json};
use work_operations::derivation::fingerprint;

use crate::error::{ExitCode, WorkError};

pub fn text(raw: &[u8], relative: &str, source: &Path) -> Result<Value, WorkError> {
    let canonical = fingerprint::canonical(raw).map_err(|error| {
        WorkError::new(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":source,"byte_offset":error.valid_up_to()}),
        )
    })?;
    Ok(json!({"schema":"work-fingerprint/v1","path":relative,
        "canonical_sha256":canonical,"raw_sha256":fingerprint::raw(raw)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_operations::derivation::fingerprint::{
        canonical as canonical_sha256, raw as sha256_hex,
    };

    #[test]
    fn text_fingerprint_preserves_raw_canonical_and_utf8_error() {
        let raw = b"first\r\nsecond\n";
        let result = text(raw, "source.md", Path::new("source.md")).unwrap();
        assert_eq!(result["schema"], "work-fingerprint/v1");
        assert_eq!(result["path"], "source.md");
        assert_eq!(result["raw_sha256"], sha256_hex(raw));
        assert_eq!(result["canonical_sha256"], canonical_sha256(raw).unwrap());

        let error = text(b"ok\xff", "source.md", Path::new("source.md")).unwrap_err();
        assert_eq!(error.reason_code, "invalid_utf8");
        assert_eq!(error.details["byte_offset"], 2);
    }
}
