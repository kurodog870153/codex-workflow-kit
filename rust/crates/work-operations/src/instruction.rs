//! Pure instruction source and selection contracts.

use crate::canonical::InstructionSource as FingerprintSource;
use crate::derivation::fingerprint;
use crate::hierarchy::Hierarchy;

pub use work_model::instruction::{
    InstructionSelection, LoadedSource, ModeCatalog, SourceSet, SourceSummary,
};

pub fn source_summary(kind: &str, logical_name: &str, content: &[u8]) -> SourceSummary {
    SourceSummary {
        kind: kind.into(),
        logical_name: logical_name.into(),
        canonical_sha256: fingerprint::raw(content),
    }
}

pub fn from_sources(mode: &str, hierarchy: Hierarchy, sources: Vec<LoadedSource>) -> SourceSet {
    let references = sources
        .iter()
        .filter(|source| source.summary.kind == "reference")
        .map(|source| source.summary.logical_name.clone())
        .collect();
    let fingerprint_sources: Vec<_> = sources
        .iter()
        .map(|source| FingerprintSource {
            kind: &source.summary.kind,
            logical_name: &source.summary.logical_name,
            content: &source.canonical_content,
        })
        .collect();
    let digest = fingerprint::instruction_selection(mode, &fingerprint_sources);
    SourceSet {
        mode: mode.into(),
        hierarchy,
        sources,
        references,
        instructions_sha256: digest,
    }
}

pub fn selection(source: &SourceSet) -> InstructionSelection {
    InstructionSelection {
        selected_paths: source.hierarchy.selected_paths.clone(),
        resolved_paths: source.hierarchy.resolved_paths.clone(),
        sources: source
            .sources
            .iter()
            .map(|source| source.summary.clone())
            .collect(),
        references: source.references.clone(),
        instructions_sha256: source.instructions_sha256.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;

    #[test]
    fn source_identity_and_hash_treat_markers_as_ordinary_content() {
        for raw in [
            b"text\n".as_slice(),
            b"<!-- work-compatibility-revision: 2 -->\n<!-- work-compatibility-revision: 3 -->\n"
                .as_slice(),
        ] {
            let source = source_summary("workflow", "work.test", raw);
            assert_eq!(source.kind, "workflow");
            assert_eq!(source.logical_name, "work.test");
            assert_eq!(source.canonical_sha256, sha256_hex(raw));
            assert!(
                serde_json::to_value(source)
                    .unwrap()
                    .get("compatibility_revision")
                    .is_none()
            );
        }
    }
    #[test]
    fn source_summary_rejects_retired_revision_instead_of_defaulting() {
        let current = serde_json::json!({"kind":"workflow","logical_name":"work.test","canonical_sha256":"a".repeat(64)});
        assert!(serde_json::from_value::<SourceSummary>(current.clone()).is_ok());
        for revision in [
            serde_json::json!(1),
            serde_json::json!(0),
            serde_json::Value::Null,
        ] {
            let mut stale = current.clone();
            stale["compatibility_revision"] = revision;
            assert!(serde_json::from_value::<SourceSummary>(stale.clone()).is_err());
            assert!(
                serde_json::from_value::<work_model::task::index::TaskInstructionSource>(stale)
                    .is_err()
            );
        }
    }
}
