//! Fingerprint command flow.

use std::path::{Path, PathBuf};

use serde_json::Value;
use work_feature::error::WorkError;

pub fn text(
    raw_path: &str,
    resolve: impl FnOnce(&str) -> Result<(String, PathBuf), WorkError>,
    read: impl FnOnce(&Path) -> Result<Vec<u8>, WorkError>,
) -> Result<Value, WorkError> {
    let (relative, path) = resolve(raw_path)?;
    let raw = read(&path)?;
    work_feature::fingerprint::text(&raw, &relative, &path)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use super::text;

    #[test]
    fn resolve_precedes_read_and_feature_fingerprint() {
        let order = RefCell::new(Vec::new());
        let result = text(
            "notes.txt",
            |path| {
                order.borrow_mut().push("resolve");
                Ok((path.to_owned(), PathBuf::from(path)))
            },
            |_| {
                order.borrow_mut().push("read");
                Ok(b"hello\r\n".to_vec())
            },
        )
        .unwrap();
        assert_eq!(*order.borrow(), ["resolve", "read"]);
        assert_eq!(result["path"], "notes.txt");
        assert_ne!(result["canonical_sha256"], result["raw_sha256"]);
    }
}
