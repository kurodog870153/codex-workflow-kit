//! Artifact fingerprint entry points. Byte primitives stay in `canonical`.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_model::task::index::TaskItemReference;

use crate::canonical::{
    InstructionSource, canonical_json_sha256, canonical_sha256, instructions_sha256, sha256_hex,
};
use crate::skill::BundleEntry;
use crate::task::collection::collection_fingerprint_sha256;

pub fn raw(raw: &[u8]) -> String {
    sha256_hex(raw)
}

pub fn verify_raw(raw: &[u8], expected: &str) -> bool {
    self::raw(raw) == expected
}

pub fn ledger(raw: &[u8]) -> String {
    sha256_hex(raw)
}

pub fn journal(raw: &[u8]) -> String {
    sha256_hex(raw)
}

pub fn history(bytes: &[u8]) -> String {
    raw(bytes)
}

pub fn skill_bundle(entries: &[BundleEntry<'_>]) -> String {
    let mut framed = b"WORK-SKILL-BUNDLE-SHA-256-V1\n".to_vec();
    for entry in entries {
        framed.push(b'F');
        for field in [entry.path, entry.normalization, entry.content_sha256] {
            framed.extend_from_slice(field.len().to_string().as_bytes());
            framed.push(b':');
            framed.extend_from_slice(field.as_bytes());
        }
        framed.push(b'\n');
    }
    framed.extend_from_slice(b"END\n");
    raw(&framed)
}

pub fn skill_identity(name: &str, scope: &str, root: &str, source: &str) -> String {
    raw(format!("WORK-SKILL-IDENTITY-V1\n{name}\n{scope}\n{root}\n{source}\n").as_bytes())
}

pub fn skill_selection(decision: &str, skills: &[Value]) -> String {
    structured(&json!({"decision": decision, "skills": skills})).expect("JSON values serialize")
}

/// Fingerprints of the complete installed Specification baseline.
pub fn specification_baseline(
    source_sha256: &str,
    index_raw: &[u8],
    execution_raw: &[u8],
    items: &BTreeMap<String, Vec<u8>>,
) -> Value {
    json!({"source_sha256": source_sha256,
        "task_index_sha256": raw(index_raw),
        "execution_index_sha256": raw(execution_raw),
        "task_item_sha256": items.iter().map(|(id, bytes)|
            (id.clone(), raw(bytes))).collect::<BTreeMap<_, _>>()})
}

pub fn verify_ledger(raw: &[u8], expected: &str) -> bool {
    ledger(raw) == expected
}

pub fn verify_journal(raw: &[u8], expected: &str) -> bool {
    journal(raw) == expected
}

pub fn canonical(raw: &[u8]) -> Result<String, std::str::Utf8Error> {
    canonical_sha256(raw)
}

pub fn verify_canonical(raw: &[u8], expected: &str) -> Result<bool, std::str::Utf8Error> {
    Ok(canonical(raw)? == expected)
}

pub fn structured(value: &Value) -> Result<String, serde_json::Error> {
    canonical_json_sha256(value)
}

pub fn verify_structured(value: &Value, expected: &str) -> Result<bool, serde_json::Error> {
    Ok(structured(value)? == expected)
}

pub fn instruction_selection(scope: &str, sources: &[InstructionSource<'_>]) -> String {
    instructions_sha256(scope, sources)
}

pub fn hierarchy_selection(
    decision: &str,
    selected_paths: &[String],
    entries: &[Value],
    catalog_sha256: &str,
) -> String {
    crate::hierarchy::selection_sha256(decision, selected_paths, entries, catalog_sha256)
}

pub fn task_collection(index_sha256: &str, references: &[TaskItemReference]) -> String {
    collection_fingerprint_sha256(index_sha256, references)
}

pub fn task_draft_approval(index_raw: &[u8], approval_bytes: &[u8]) -> String {
    let mut review = b"WORK-TASK-DRAFT-APPROVAL-V1\n".to_vec();
    review.extend_from_slice(index_raw);
    review.extend_from_slice(approval_bytes);
    raw(&review)
}

pub fn task_provenance(source: &work_model::task::source::TaskProvenance) -> String {
    structured(&serde_json::to_value(source).expect("provenance serializes"))
        .expect("JSON serializes")
}
pub fn migration_source_approval(
    requirement: &str,
    sources: &[work_model::task::source::MigrationSourceEvidence],
) -> String {
    let material =
        serde_json::to_vec(&serde_json::json!({"requirement_id":requirement,"sources":sources}))
            .expect("evidence serializes");
    let mut bytes = b"WORK-MIGRATION-SOURCE-APPROVAL-V1\n".to_vec();
    bytes.extend(material);
    raw(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_preserves_raw_canonical_and_structured_hashes() {
        let bytes = b"first\r\nsecond\r\n";
        let raw_digest = sha256_hex(bytes);
        let canonical_digest = canonical_sha256(bytes).unwrap();
        let value = json!({"z": [2, 1], "a": "value"});
        let structured_digest = canonical_json_sha256(&value).unwrap();

        assert_eq!(raw(bytes), raw_digest);
        assert!(verify_raw(bytes, &raw_digest));
        assert!(!verify_raw(b"changed", &raw_digest));
        assert_eq!(canonical(bytes).unwrap(), canonical_digest);
        assert!(verify_canonical(bytes, &canonical_digest).unwrap());
        assert!(!verify_canonical(b"changed", &canonical_digest).unwrap());
        assert_eq!(structured(&value).unwrap(), structured_digest);
        assert!(verify_structured(&value, &structured_digest).unwrap());
        assert!(!verify_structured(&json!({"a": "changed"}), &structured_digest).unwrap());
        assert!(canonical(b"\xff").is_err());
        assert!(verify_canonical(b"\xff", &canonical_digest).is_err());
    }

    #[test]
    fn task_collection_preserves_framing_and_reference_order() {
        let first = TaskItemReference {
            id: "TASK-001".into(),
            path: "tasks/TASK-001.json".into(),
            canonical_sha256: "a".repeat(64),
        };
        let second = TaskItemReference {
            id: "TASK-002".into(),
            path: "tasks/TASK-002.json".into(),
            canonical_sha256: "b".repeat(64),
        };
        let index = "c".repeat(64);
        assert_eq!(
            task_collection(&index, &[first.clone(), second.clone()]),
            "36f7a8d7b942cd29739fa1950700271e32f7bc1edba9cf8adf33d2f39afa96f7"
        );
        assert_ne!(
            task_collection(&index, &[first.clone(), second.clone()]),
            task_collection(&index, &[second, first])
        );
    }

    #[test]
    fn task_draft_approval_preserves_review_byte_framing() {
        let expected = b"WORK-TASK-DRAFT-APPROVAL-V1\nindex\ncollection\n";
        assert_eq!(
            task_draft_approval(b"index\n", b"collection\n"),
            sha256_hex(expected)
        );
        assert_ne!(
            task_draft_approval(b"index\n", b"collection\n"),
            task_draft_approval(b"collection\n", b"index\n")
        );
    }

    #[test]
    fn hierarchy_selection_preserves_catalog_and_entry_order() {
        let paths = vec!["web".to_owned(), "web/backend".to_owned()];
        let entries = vec![json!({"path": "web"}), json!({"path": "web/backend"})];
        let catalog = "a".repeat(64);
        let expected = crate::hierarchy::selection_sha256("guided", &paths, &entries, &catalog);
        assert_eq!(
            hierarchy_selection("guided", &paths, &entries, &catalog),
            expected
        );
        assert_ne!(
            hierarchy_selection("guided", &paths, &entries, &catalog),
            hierarchy_selection("guided", &paths, &entries, &"b".repeat(64))
        );
        let mut reversed = entries;
        reversed.reverse();
        assert_ne!(
            hierarchy_selection("guided", &paths, &reversed, &catalog),
            expected
        );
    }
}
