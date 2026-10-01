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
    plan_raw: &[u8],
    index_raw: &[u8],
    execution_raw: &[u8],
    items: &BTreeMap<String, Vec<u8>>,
) -> Value {
    json!({"plan_sha256": raw(plan_raw),
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

pub fn structured(value: &Value) -> Result<String, serde_json::Error> {
    canonical_json_sha256(value)
}

pub fn instruction_selection(scope: &str, sources: &[InstructionSource<'_>]) -> String {
    instructions_sha256(scope, sources)
}

pub fn task_collection(index_sha256: &str, references: &[TaskItemReference]) -> String {
    collection_fingerprint_sha256(index_sha256, references)
}
