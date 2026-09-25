//! Hierarchy use cases over a catalog repository.

use std::path::Path;

use serde_json::{Value, json};
use work_operations::hierarchy::{
    CrossModeCatalog, HierarchyIssue, authorize_task_paths, build_hierarchy, selection_sha256,
};
use work_operations::protocol::{INVALID_SHA256_ERROR_CODE, valid_sha256 as valid_sha256_text};

use crate::error::{ExitCode, WorkError};

pub trait HierarchyCatalogRepository {
    fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError>;
    fn mode_paths(&self, mode: &str) -> Result<Vec<String>, WorkError>;
}

fn domain_error(error: HierarchyIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        error.reason_code,
        error.message,
        error.details,
    )
}

pub fn resolve(
    mode: &str,
    selected_paths: &[String],
    project_root: &Path,
) -> Result<Value, WorkError> {
    let hierarchy = build_hierarchy(mode, selected_paths).map_err(domain_error)?;
    let mut value = serde_json::to_value(hierarchy).expect("hierarchy serializes");
    value["project_root"] = json!(project_root);
    Ok(value)
}

fn contract(reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(ExitCode::Contract, reason, message, details)
}

fn strict_object<'a>(
    value: &'a Value,
    location: &str,
    fields: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, WorkError> {
    let object = value.as_object().ok_or_else(|| {
        contract(
            "expected_object",
            "A JSON object is required.",
            json!({"location": location}),
        )
    })?;
    let mut missing: Vec<_> = fields
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    let mut unknown: Vec<_> = object
        .keys()
        .filter(|field| !fields.contains(&field.as_str()))
        .cloned()
        .collect();
    missing.sort_unstable();
    unknown.sort();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(contract(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location": location, "missing": missing, "unknown": unknown}),
        ));
    }
    Ok(object)
}

fn text<'a>(value: &'a Value, location: &str) -> Result<&'a str, WorkError> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            contract(
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location": location}),
            )
        })
}

fn valid_sha256<'a>(value: &'a Value, location: &str) -> Result<&'a str, WorkError> {
    value
        .as_str()
        .filter(|hash| valid_sha256_text(hash))
        .ok_or_else(|| {
            contract(
                INVALID_SHA256_ERROR_CODE,
                "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
                json!({"location": location}),
            )
        })
}

pub fn build_selection(
    repository: &impl HierarchyCatalogRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    let request = strict_object(
        request,
        "hierarchy_selection_request",
        &["decision", "selections"],
    )?;
    let selections = request["selections"].as_array().ok_or_else(|| {
        contract(
            "invalid_hierarchy_selections",
            "Hierarchy selections must be an array.",
            json!({}),
        )
    })?;
    let decision = request["decision"].as_str().unwrap_or("");
    if !matches!(decision, "instruction_paths" | "general_only") {
        return Err(contract(
            "invalid_hierarchy_selection_decision",
            "The hierarchy selection decision is invalid.",
            json!({"decision": request["decision"]}),
        ));
    }
    if (decision == "instruction_paths") == selections.is_empty() {
        return Err(contract(
            "hierarchy_selection_decision_mismatch",
            "The hierarchy selection decision does not match its selected paths.",
            json!({}),
        ));
    }
    let mut paths = Vec::new();
    let mut reasons = Vec::new();
    for (index, selection) in selections.iter().enumerate() {
        let location = format!("hierarchy_selection_request.selections[{index}]");
        let object = strict_object(selection, &location, &["path", "recommendation_reason"])?;
        paths.push(text(&object["path"], &format!("{location}.path"))?.to_owned());
        reasons.push(
            text(
                &object["recommendation_reason"],
                &format!("{location}.recommendation_reason"),
            )?
            .to_owned(),
        );
    }
    let _: work_model::hierarchy::HierarchySelectionRequest =
        serde_json::from_value(Value::Object(request.clone()))
            .expect("validated hierarchy request matches its model");
    build_hierarchy("plan", &paths).map_err(domain_error)?;
    let catalog = repository.cross_mode_catalog()?;
    let mut entries = Vec::new();
    for (path, reason) in paths.iter().zip(reasons) {
        if !catalog.paths.contains(path) {
            let parent = path
                .rsplit_once('/')
                .map_or("general", |(parent, _)| parent);
            return Err(contract(
                "hierarchy_selection_path_missing",
                "A selected hierarchy path does not exist in the cross-mode catalog.",
                json!({"path": path, "parent": parent, "valid_choices": catalog.children.get(parent).cloned().unwrap_or_default()}),
            ));
        }
        let metadata = &catalog.metadata[path];
        entries.push(json!({
            "path": path,
            "mode_support": metadata["mode_support"],
            "mode_metadata": metadata["modes"],
            "recommendation_reason": reason,
        }));
    }
    let hash = selection_sha256(decision, &paths, &entries, &catalog.catalog_sha256);
    let result = json!({
        "schema": "work-hierarchy-selection/v1",
        "decision": decision,
        "selected_paths": paths,
        "entries": entries,
        "catalog_sha256": catalog.catalog_sha256,
        "selection_sha256": hash,
    });
    let _: work_model::hierarchy::HierarchySelection = serde_json::from_value(result.clone())
        .expect("built hierarchy selection matches its model");
    Ok(result)
}

pub fn validate_selection(
    repository: &impl HierarchyCatalogRepository,
    value: &Value,
) -> Result<Value, WorkError> {
    let selection = strict_object(
        value,
        "hierarchy_selection",
        &[
            "schema",
            "decision",
            "selected_paths",
            "entries",
            "catalog_sha256",
            "selection_sha256",
        ],
    )?;
    if selection["schema"] != "work-hierarchy-selection/v1" {
        return Err(contract(
            "invalid_hierarchy_selection_schema",
            "The hierarchy selection schema is invalid.",
            json!({}),
        ));
    }
    let paths = selection["selected_paths"]
        .as_array()
        .filter(|paths| paths.iter().all(Value::is_string))
        .ok_or_else(|| {
            contract(
                "invalid_hierarchy_selected_paths",
                "Hierarchy selected_paths must be an array of strings.",
                json!({}),
            )
        })?;
    let entries = selection["entries"]
        .as_array()
        .filter(|entries| entries.len() == paths.len())
        .ok_or_else(|| {
            contract(
                "invalid_hierarchy_selection_entries",
                "Hierarchy selection entries must align with selected_paths.",
                json!({}),
            )
        })?;
    let decision = selection["decision"].as_str().unwrap_or("");
    if !matches!(decision, "instruction_paths" | "general_only") {
        return Err(contract(
            "invalid_hierarchy_selection_decision",
            "The hierarchy selection decision is invalid.",
            json!({"decision": selection["decision"]}),
        ));
    }
    if (decision == "instruction_paths") == paths.is_empty() {
        return Err(contract(
            "hierarchy_selection_decision_mismatch",
            "The hierarchy selection decision does not match its selected paths.",
            json!({}),
        ));
    }
    let catalog = repository.cross_mode_catalog()?;
    if valid_sha256(
        &selection["catalog_sha256"],
        "hierarchy_selection.catalog_sha256",
    )? != catalog.catalog_sha256
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "instruction_catalog_snapshot_mismatch",
            "The instruction catalog no longer matches its confirmed snapshot; return to Plan.",
            json!({}),
        ));
    }
    let mut validated_entries = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let location = format!("hierarchy_selection.entries[{index}]");
        let object = strict_object(
            entry,
            &location,
            &[
                "path",
                "mode_support",
                "mode_metadata",
                "recommendation_reason",
            ],
        )?;
        let path = text(&object["path"], &format!("{location}.path"))?;
        let reason = text(
            &object["recommendation_reason"],
            &format!("{location}.recommendation_reason"),
        )?;
        if path != paths[index].as_str().unwrap() {
            return Err(contract(
                "hierarchy_selection_entry_order_mismatch",
                "Hierarchy selection entries must match selected_paths order.",
                json!({"location": location}),
            ));
        }
        let expected = catalog.metadata.get(path).map(|metadata| {
            json!({
                "path": path,
                "mode_support": metadata["mode_support"],
                "mode_metadata": metadata["modes"],
                "recommendation_reason": reason,
            })
        });
        if expected.as_ref() != Some(entry) {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "hierarchy_selection_metadata_mismatch",
                "A selected hierarchy path no longer matches its confirmed metadata; return to Plan.",
                json!({"path": path}),
            ));
        }
        validated_entries.push(entry.clone());
    }
    let selected_paths: Vec<String> = paths
        .iter()
        .map(|path| path.as_str().unwrap().to_owned())
        .collect();
    build_hierarchy("plan", &selected_paths).map_err(domain_error)?;
    let stored = valid_sha256(
        &selection["selection_sha256"],
        "hierarchy_selection.selection_sha256",
    )?;
    if stored
        != selection_sha256(
            decision,
            &selected_paths,
            &validated_entries,
            &catalog.catalog_sha256,
        )
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "hierarchy_selection_fingerprint_mismatch",
            "The hierarchy selection fingerprint does not match its contents.",
            json!({}),
        ));
    }
    let result = json!({"schema": "work-hierarchy-selection-validation/v1", "status": "valid", "hierarchy_selection": value});
    let _: work_model::hierarchy::HierarchySelectionValidation =
        serde_json::from_value(result.clone())
            .expect("validated hierarchy selection matches its model");
    Ok(result)
}

pub fn validate_task_paths(
    repository: &impl HierarchyCatalogRepository,
    paths: &[String],
    confirmed: &Value,
    location: &str,
) -> Result<(), WorkError> {
    build_hierarchy("task", paths).map_err(domain_error)?;
    let confirmed_paths = confirmed
        .get("selected_paths")
        .and_then(Value::as_array)
        .filter(|paths| paths.iter().all(Value::is_string))
        .ok_or_else(|| {
            contract(
                "invalid_confirmed_hierarchy_selection",
                "The source Plan hierarchy selection is invalid.",
                json!({}),
            )
        })?;
    let confirmed_paths: Vec<String> = confirmed_paths
        .iter()
        .map(|path| path.as_str().unwrap().to_owned())
        .collect();
    authorize_task_paths(paths, &confirmed_paths).map_err(|error| {
        let mut details = error.details;
        details["location"] = json!(location);
        contract(error.reason_code, error.message, details)
    })?;
    for mode in ["task", "execute"] {
        let available = repository.mode_paths(mode)?;
        let hierarchy = build_hierarchy(mode, paths).map_err(domain_error)?;
        for path in hierarchy.resolved_paths {
            if !available.contains(&path) {
                let parent = path
                    .rsplit_once('/')
                    .map_or("general", |(parent, _)| parent);
                let choices: Vec<_> = available
                    .iter()
                    .filter_map(|candidate| candidate.strip_prefix(&format!("{parent}/")))
                    .filter(|rest| !rest.contains('/'))
                    .collect();
                return Err(contract(
                    "instruction_hierarchy_path_missing",
                    "A selected instruction hierarchy path does not exist in the catalog.",
                    json!({"mode": mode, "path": path, "parent": parent, "valid_choices": choices}),
                ));
            }
        }
    }
    Ok(())
}
