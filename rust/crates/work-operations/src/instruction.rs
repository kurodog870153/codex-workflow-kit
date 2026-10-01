//! Pure instruction source and selection contracts.

use crate::canonical::InstructionSource as FingerprintSource;
use crate::derivation::fingerprint;
use crate::hierarchy::Hierarchy;

pub use work_model::instruction::{
    InstructionSelection, LoadedSource, ModeCatalog, SourceSet, SourceSummary,
};

pub fn compatibility_revision(content: &[u8]) -> Result<u64, &'static str> {
    let marker = b"<!-- work-compatibility-revision: ";
    let mut revision = None;
    for line in content.split(|byte| *byte == b'\n') {
        let Some(number) = line
            .strip_prefix(marker)
            .and_then(|line| line.strip_suffix(b" -->"))
        else {
            continue;
        };
        if number.is_empty() || number[0] == b'0' || !number.iter().all(u8::is_ascii_digit) {
            continue;
        }
        let Ok(number) = std::str::from_utf8(number)
            .expect("ASCII digits")
            .parse::<u64>()
        else {
            continue;
        };
        if revision.replace(number).is_some() {
            return Err("duplicate_instruction_compatibility_revision");
        }
    }
    Ok(revision.unwrap_or(1))
}

pub fn source_summary(
    kind: &str,
    logical_name: &str,
    content: &[u8],
) -> Result<SourceSummary, &'static str> {
    Ok(SourceSummary {
        kind: kind.into(),
        logical_name: logical_name.into(),
        canonical_sha256: fingerprint::raw(content),
        compatibility_revision: compatibility_revision(content)?,
    })
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
    fn revision_marker_and_source_hash() {
        assert_eq!(compatibility_revision(b"text\n").unwrap(), 1);
        assert_eq!(
            compatibility_revision(b"<!-- work-compatibility-revision: 2 -->\n").unwrap(),
            2
        );
        assert_eq!(compatibility_revision(b"<!-- work-compatibility-revision: 2 -->\n<!-- work-compatibility-revision: 3 -->\n").unwrap_err(), "duplicate_instruction_compatibility_revision");
        assert_eq!(
            source_summary("workflow", "work.test", b"text\n")
                .unwrap()
                .canonical_sha256,
            sha256_hex(b"text\n")
        );
    }
}
