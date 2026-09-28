//! Final migration preview readiness and fingerprint decisions.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_operations::canonical::{canonical_json_sha256, sha256_hex};

use crate::error::{ExitCode, WorkError};

pub struct MigrationPreviewInput<'a> {
    pub request: &'a Value,
    pub sources: &'a BTreeMap<String, Vec<u8>>,
    pub candidates: &'a BTreeMap<String, Vec<u8>>,
    pub validators: Vec<Value>,
    pub relationships: Vec<Value>,
    pub all_paths: Vec<String>,
    pub diffs: Vec<Value>,
}

pub fn finish_preview(input: MigrationPreviewInput<'_>) -> Result<Value, WorkError> {
    let MigrationPreviewInput {
        request,
        sources,
        candidates,
        validators,
        relationships,
        all_paths,
        diffs,
    } = input;
    let mut unresolved = request["semantic_decisions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["resolution"].is_null())
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    unresolved.sort();
    let ready = unresolved.is_empty()
        && validators
            .iter()
            .chain(&relationships)
            .all(|row| row["status"] == "passed");
    let source_hashes = sources
        .iter()
        .map(|(path, raw)| (path.clone(), sha256_hex(raw)))
        .collect::<BTreeMap<_, _>>();
    let candidate_hashes = candidates
        .iter()
        .map(|(path, raw)| (path.clone(), sha256_hex(raw)))
        .collect::<BTreeMap<_, _>>();
    let evidence = json!({"request":request,"source_sha256":source_hashes,
        "candidate_sha256":candidate_hashes,"validator_results":validators,
        "relationship_results":relationships,"unresolved_items":unresolved});
    let fingerprint = canonical_json_sha256(&evidence).map_err(|_| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "invalid_contract_value",
            "Migration evidence cannot be fingerprinted.",
            json!({}),
        )
    })?;
    Ok(work_model::specification::verified::<
        work_model::specification::SpecMigrationPreview,
    >(json!({"schema":"work-spec-migration-preview/v1",
        "status":if ready {"ready"} else {"blocked"},"documents":all_paths,
        "diffs":diffs,"validator_results":validators,"relationship_results":relationships,
        "unresolved_items":unresolved,"fingerprint":fingerprint,"writable_ready":ready})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_decision_blocks_publication() {
        let request = json!({"semantic_decisions":[{"id":"D-1","resolution":null}]});
        let result = finish_preview(MigrationPreviewInput {
            request: &request,
            sources: &BTreeMap::new(),
            candidates: &BTreeMap::new(),
            validators: vec![json!({"status":"passed"})],
            relationships: vec![json!({"status":"passed"})],
            all_paths: vec![],
            diffs: vec![],
        })
        .unwrap();
        assert_eq!(result["status"], "blocked");
        assert_eq!(result["unresolved_items"], json!(["D-1"]));
    }
}
