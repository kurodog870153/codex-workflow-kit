//! Compare one installed artifact with its current public contract.

use serde_json::{Value, json};
use work_model::specification::{ArtifactMigrationItem, ArtifactMigrationItemStatus};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::task::index::validate_task_index;
use work_operations::task::item::validate_task_item;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

fn current_schema(kind: &str) -> Option<&'static str> {
    match kind {
        "source" => Some("work-source-snapshot/v1"),
        "task_index" => Some("work-task-index/v1"),
        "task_item" => Some("work-task-item/v1"),
        "execution_index" => Some("work-execution-index/v1"),
        _ => None,
    }
}

/// Verify an exact reviewed analysis without consulting or interpreting legacy schemas.
pub fn verify_raw_analysis(
    value: &Value,
) -> Result<work_model::specification::ArtifactMigrationAnalysis, &'static str> {
    use std::collections::BTreeSet;
    let analysis: work_model::specification::ArtifactMigrationAnalysis =
        serde_json::from_value(value.clone()).map_err(|_| "migration_analysis")?;
    if analysis.schema != work_model::schema::PublicSchema::WorkArtifactMigrationAnalysisV1
        || analysis
            .requirement_id
            .parse::<work_model::identifiers::RequirementId>()
            .is_err()
    {
        return Err("migration_analysis");
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for item in &analysis.items {
        if item.source_size != item.raw.len() as u64
            || fingerprint::raw(&item.raw) != item.source_sha256
        {
            return Err("migration_raw_evidence_invalid");
        }
        if !work_operations::task::source::valid_relative_path(&item.path)
            || item.id != format!("MIGRATION-{}", item.path.replace(['/', '.'], "-"))
            || !ids.insert(&item.id)
            || !paths.insert(work_operations::canonical::portable_path_identity(
                &item.path,
            ))
            || item.issue.trim().is_empty()
            || (item.kind != "raw_evidence" && current_schema(&item.kind).is_none())
            || item.target_schema != current_schema(&item.kind).unwrap_or_default()
            || (item.resolution_status == ArtifactMigrationItemStatus::Proposed)
                != item.proposed_content.is_some()
        {
            return Err("migration_analysis");
        }
    }
    let mut evidence = json!({"requirement_id":analysis.requirement_id,"items":analysis.items});
    if !analysis.diagnostics.is_empty() {
        evidence["diagnostics"] = json!(analysis.diagnostics);
    }
    if fingerprint::structured(&evidence).map_err(|_| "migration_analysis")? != analysis.fingerprint
    {
        return Err("migration_analysis_fingerprint_mismatch");
    }
    Ok(analysis)
}

pub fn candidate_bytes(kind: &str, content: &Value) -> Result<Vec<u8>, String> {
    match kind {
        "task_index" => render_task(content, TaskDocumentKind::Index),
        "task_item" => render_task(content, TaskDocumentKind::Item),
        "execution_index" => render_execution_index(content),
        _ => return Err("Unknown artifact kind".into()),
    }
    .map_err(|error| error.to_string())
}

pub fn validate_candidate(kind: &str, path: &str, content: &Value) -> Result<Vec<u8>, String> {
    if content["schema"].as_str() != current_schema(kind) {
        return Err("Artifact schema differs from the current contract".into());
    }
    let raw = candidate_bytes(kind, content)?;
    match kind {
        "task_index" => {
            validate_task_index(content, &raw, path).map_err(|error| error.message.to_owned())?;
        }
        "task_item" => {
            let id = path
                .rsplit('/')
                .next()
                .and_then(|name| name.strip_suffix(".json"))
                .ok_or("Invalid TASK item path")?;
            validate_task_item(content, &raw, id).map_err(|error| error.message.to_owned())?;
        }
        "execution_index" => {
            validate_execution_index(content, &raw).map_err(|error| error.message.to_owned())?;
        }
        _ => return Err("Unknown artifact kind".into()),
    }
    Ok(raw)
}

pub fn analyze_artifact(kind: &str, path: &str, raw: &[u8]) -> ArtifactMigrationItem {
    let id = format!("MIGRATION-{}", path.replace(['/', '.'], "-"));
    let mut result = ArtifactMigrationItem {
        id,
        path: path.into(),
        kind: kind.into(),
        target_schema: current_schema(kind).unwrap_or_default().into(),
        required: true,
        source_sha256: fingerprint::raw(raw),
        source_size: raw.len() as u64,
        raw: raw.to_vec(),
        issue: String::new(),
        resolution_status: ArtifactMigrationItemStatus::NeedsReview,
        proposed_content: None,
    };
    let parsed = parse_json_contract(raw);
    let Ok(content) = parsed else {
        result.issue = "Source is not valid JSON; provide reviewed content with Modify".into();
        return result;
    };
    if !content.is_object() {
        result.issue = "Source must be a JSON object; provide reviewed content with Modify".into();
        return result;
    }
    if current_schema(kind).is_none() || content["schema"].as_str() != current_schema(kind) {
        result.issue = "Raw evidence requires AI comparison against current contracts and reviewed replacement content; no legacy schema is parsed or upgraded".into();
        return result;
    }
    match validate_candidate(kind, path, &content) {
        Ok(candidate) => {
            if candidate == raw {
                result.issue.clear();
            } else {
                result.issue = "Current contract document is not canonically rendered".into();
                result.proposed_content = Some(content);
                result.resolution_status = ArtifactMigrationItemStatus::Proposed;
            }
        }
        Err(problem) => {
            result.issue = format!("Current contract validation failed: {problem}");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn damaged_and_binary_sources_remain_exact_raw_evidence() {
        for raw in [
            b"broken".as_slice(),
            b"%PDF-1.7\n\xff\x00".as_slice(),
            b"{\"schema\":\"work-plan/v0\",\"unknown\":true}".as_slice(),
        ] {
            let item = analyze_artifact("task_index", "legacy/source.pdf", raw);
            assert_eq!(item.raw, raw);
            assert_eq!(item.source_size, raw.len() as u64);
            assert_eq!(item.source_sha256, fingerprint::raw(raw));
            assert!(item.required);
            assert!(item.proposed_content.is_none());
            assert_eq!(
                item.resolution_status,
                ArtifactMigrationItemStatus::NeedsReview
            );
            assert!(!item.issue.is_empty());
        }
        assert!(
            validate_candidate("plan", "legacy.json", &json!({"schema":"work-plan/v1"})).is_err()
        );
    }
    #[test]
    fn only_current_contract_content_can_receive_a_canonical_proposal() {
        let path = "outputs/work/tasks/example/index.json";
        let raw = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../work-infrastructure/fixtures/task-diagnostics/outputs/work/tasks/example/index.json"
        ));
        let current = analyze_artifact("task_index", path, raw);
        assert!(current.issue.is_empty());
        let mut legacy: Value = serde_json::from_slice(raw).unwrap();
        legacy["schema"] = json!("legacy/v0");
        let item = analyze_artifact("task_index", path, &serde_json::to_vec(&legacy).unwrap());
        assert!(item.proposed_content.is_none());
        assert_eq!(
            item.resolution_status,
            ArtifactMigrationItemStatus::NeedsReview
        );
        let compact = serde_json::to_vec(&serde_json::from_slice::<Value>(raw).unwrap()).unwrap();
        let item = analyze_artifact("task_index", path, &compact);
        assert!(item.proposed_content.is_some());
        assert_eq!(
            item.resolution_status,
            ArtifactMigrationItemStatus::Proposed
        );
    }
}
