//! Pure skill identity, bundle, and selection fingerprints.

use serde_json::Value;

use crate::canonical::{canonical_json_sha256, sha256_hex};

#[derive(Clone, Copy)]
pub struct BundleEntry<'a> {
    pub path: &'a str,
    pub normalization: &'a str,
    pub content_sha256: &'a str,
}

pub fn identity_sha256(name: &str, scope: &str, root: &str, source: &str) -> String {
    sha256_hex(format!("WORK-SKILL-IDENTITY-V1\n{name}\n{scope}\n{root}\n{source}\n").as_bytes())
}

pub fn bundle_sha256(entries: &[BundleEntry<'_>]) -> String {
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
    sha256_hex(&framed)
}

pub fn selection_sha256(decision: &str, skills: &[Value]) -> String {
    canonical_json_sha256(&serde_json::json!({"decision": decision, "skills": skills}))
        .expect("JSON values serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_bundle_are_order_sensitive() {
        assert_ne!(
            identity_sha256("ui", "repo", "a", "ui/SKILL.md"),
            identity_sha256("ui", "user", "a", "ui/SKILL.md")
        );
        let a = BundleEntry {
            path: "SKILL.md",
            normalization: "canonical-text",
            content_sha256: "a",
        };
        let b = BundleEntry {
            path: "references/guide.md",
            normalization: "canonical-text",
            content_sha256: "b",
        };
        assert_ne!(bundle_sha256(&[a, b]), bundle_sha256(&[b, a]));
        assert_eq!(
            selection_sha256("base_only", &[]),
            canonical_json_sha256(&serde_json::json!({"decision": "base_only", "skills": []}))
                .unwrap()
        );
    }
}
