//! Pure skill identity, bundle, and selection fingerprints.

#[derive(Clone, Copy)]
pub struct BundleEntry<'a> {
    pub path: &'a str,
    pub normalization: &'a str,
    pub content_sha256: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_bundle_are_order_sensitive() {
        assert_ne!(
            crate::derivation::fingerprint::skill_identity("ui", "repo", "a", "ui/SKILL.md"),
            crate::derivation::fingerprint::skill_identity("ui", "user", "a", "ui/SKILL.md")
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
        assert_ne!(
            crate::derivation::fingerprint::skill_bundle(&[a, b]),
            crate::derivation::fingerprint::skill_bundle(&[b, a])
        );
        assert_eq!(
            crate::derivation::fingerprint::skill_selection("base_only", &[]),
            crate::derivation::fingerprint::structured(
                &serde_json::json!({"decision": "base_only", "skills": []})
            )
            .unwrap()
        );
    }
}
