//! Compare one installed artifact with its current public contract.

use serde_json::{Value, json};
use work_model::specification::{ArtifactMigrationItem, ArtifactMigrationItemStatus};
use work_operations::canonical::{parse_json_contract, sha256_hex};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::plan::{render_plan_value, validation::validate_plan_structure};
use work_operations::task::index::validate_task_index;
use work_operations::task::item::validate_task_item;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

fn current_schema(kind: &str) -> Option<&'static str> {
    match kind {
        "plan" => Some("work-plan/v1"),
        "task_index" => Some("work-task-index/v1"),
        "task_item" => Some("work-task-item/v1"),
        "execution_index" => Some("work-execution-index/v1"),
        _ => None,
    }
}

pub fn candidate_bytes(kind: &str, content: &Value) -> Result<Vec<u8>, String> {
    match kind {
        "plan" => render_plan_value(content),
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
        "plan" => {
            serde_json::from_value::<work_model::plan::PlanArtifact>(content.clone())
                .map_err(|error| error.to_string())?;
            validate_plan_structure(content).map_err(|error| error.message.to_owned())?;
            if content["artifacts"]["plan"] != path {
                return Err("Plan artifact path differs from its installed path".into());
            }
        }
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
        source_sha256: sha256_hex(raw),
        issue: String::new(),
        resolution_status: ArtifactMigrationItemStatus::NeedsReview,
        proposed_content: None,
    };
    let parsed = parse_json_contract(raw);
    let Ok(mut content) = parsed else {
        result.issue = "Source is not valid JSON; provide reviewed content with Modify".into();
        return result;
    };
    if !content.is_object() {
        result.issue = "Source must be a JSON object; provide reviewed content with Modify".into();
        return result;
    }
    let original_schema = content["schema"].as_str().unwrap_or("missing").to_owned();
    if let Some(schema) = current_schema(kind) {
        content["schema"] = json!(schema);
    }
    match validate_candidate(kind, path, &content) {
        Ok(candidate) => {
            if original_schema == current_schema(kind).unwrap_or("") && candidate == raw {
                result.issue.clear();
            } else {
                result.issue = if original_schema != current_schema(kind).unwrap_or("") {
                    format!("Source schema {original_schema} differs from current specification")
                } else {
                    "Source is not canonically rendered".into()
                };
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
    fn damaged_source_remains_reviewable() {
        let item = analyze_artifact("plan", "outputs/work/plans/example.json", b"broken");
        assert_eq!(item.source_sha256, sha256_hex(b"broken"));
        assert!(item.required);
        assert!(item.proposed_content.is_none());
        assert_eq!(
            item.resolution_status,
            ArtifactMigrationItemStatus::NeedsReview
        );
        assert!(item.issue.contains("not valid JSON"));
    }

    #[test]
    fn current_source_has_no_item_and_semantic_gap_needs_review() {
        let raw = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../work-infrastructure/fixtures/specification-migration/outputs/work/plans/example.json"
        ));
        let current = analyze_artifact("plan", "outputs/work/plans/example.json", raw);
        assert!(current.issue.is_empty());
        assert!(current.proposed_content.is_none());
        let mut incomplete: Value = serde_json::from_slice(raw).unwrap();
        incomplete["schema"] = json!("work-plan/v0");
        incomplete.as_object_mut().unwrap().remove("title");
        let broken = serde_json::to_vec(&incomplete).unwrap();
        let item = analyze_artifact("plan", "outputs/work/plans/example.json", &broken);
        assert_eq!(item.source_sha256, sha256_hex(&broken));
        assert_eq!(item.target_schema, "work-plan/v1");
        assert_eq!(
            item.resolution_status,
            ArtifactMigrationItemStatus::NeedsReview
        );
        assert!(item.proposed_content.is_none());
        assert!(!item.issue.is_empty());
    }
}
