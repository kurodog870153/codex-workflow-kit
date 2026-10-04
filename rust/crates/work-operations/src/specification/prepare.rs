//! Pure validation for caller-authored Specification edits.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct SpecificationIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
    pub artifact_integrity: bool,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> SpecificationIssue {
    SpecificationIssue {
        reason_code,
        message,
        details,
        artifact_integrity: false,
    }
}

fn artifact(reason_code: &'static str, message: &'static str) -> SpecificationIssue {
    SpecificationIssue {
        reason_code,
        message,
        details: json!({}),
        artifact_integrity: true,
    }
}

fn fields<'a>(
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
    location: &str,
) -> Result<&'a Map<String, Value>, SpecificationIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location":location}),
        )
    })?;
    let missing = required
        .iter()
        .filter(|key| !object.contains_key(**key))
        .map(|key| (*key).to_owned())
        .collect::<BTreeSet<_>>();
    let unknown = object
        .keys()
        .filter(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":location,"missing":missing,"unknown":unknown}),
        ));
    }
    Ok(object)
}

fn formal_id(value: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "TASK",
        "STEP",
        "CMD",
        "VAL",
        "OP",
        "FILE",
        "INPUT",
        "RISK",
        "GOAL",
        "SCOPE",
        "CONSTRAINT",
        "DEPENDENCY",
        "MILESTONE",
        "DELIVERABLE",
        "ACCEPTANCE",
        "PLAN-DECISION",
        "TASK-DECISION",
        "PLAN-CHANGE",
        "TASK-CHANGE",
        "TASK-SPEC",
    ];
    PREFIXES.iter().any(|prefix| {
        value
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|digits| {
                digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
    })
}

fn semantic_only(value: &Value) -> Result<(), SpecificationIssue> {
    const FORMAL_KEYS: &[&str] = &[
        "id",
        "command_ids",
        "command_id",
        "validation_id",
        "goal_ids",
        "deliverable_ids",
        "milestone_ids",
        "task_ids",
        "canonical_sha256",
        "raw_sha256",
        "instructions_sha256",
        "fingerprint",
        "revision",
        "status",
        "transaction_id",
        "spec_id",
        "change_id",
    ];
    match value {
        Value::Object(object) => {
            if object.keys().any(|key| FORMAL_KEYS.contains(&key.as_str())) {
                return Err(issue(
                    "invalid_contract_value",
                    "Semantic input cannot contain formal fields.",
                    json!({}),
                ));
            }
            for (key, child) in object {
                if key == "acceptance_ids" || key == "acceptance_criteria" {
                    let input = json!({key:child});
                    crate::task::candidate::validate_semantic_candidate(&input, false)
                        .map_err(|error| issue(error.reason_code, error.message, error.details))?;
                } else {
                    semantic_only(child)?;
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                semantic_only(child)?;
            }
        }
        Value::String(text) if formal_id(text) => {
            return Err(issue(
                "invalid_contract_value",
                "Semantic input cannot contain formal IDs.",
                json!({}),
            ));
        }
        _ => {}
    }
    Ok(())
}

pub fn validate_prepare_request(value: &Value) -> Result<(), SpecificationIssue> {
    fields(
        value,
        &["schema", "requirement_id", "reason", "edits"],
        &["source_update"],
        "spec_prepare",
    )?;
    if value["schema"] != "work-spec-prepare-request/v1" {
        return Err(artifact(
            "spec_prepare_schema",
            "Use work-spec-prepare-request/v1.",
        ));
    }
    if value["requirement_id"]
        .as_str()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(issue(
            "empty_text_value",
            "A non-empty string is required.",
            json!({"location":"requirement_id"}),
        ));
    }
    if !value["reason"].is_string() {
        return Err(issue(
            "invalid_contract_value",
            "A specification reason is required.",
            json!({}),
        ));
    }
    if let Some(update) = value.get("source_update") {
        serde_json::from_value::<work_model::specification::SpecificationSourceUpdate>(
            update.clone(),
        )
        .map_err(|_| {
            issue(
                "source_confirmation_required",
                "A complete Source context, selections and impact confirmation are required.",
                json!({}),
            )
        })?;
        crate::task::source::validate_planning_source(
            &update["source"],
            value["requirement_id"].as_str().unwrap(),
        )
        .map_err(|e| issue(e.reason_code, e.message, e.details))?;
    }
    let edits = value["edits"]
        .as_array()
        .filter(|edits| !edits.is_empty() || value.get("source_update").is_some())
        .ok_or_else(|| artifact("spec_prepare_edits", "Supply non-empty collection edits."))?;
    for (position, edit) in edits.iter().enumerate() {
        let location = format!("edits[{position}]");
        let object = fields(
            edit,
            &[],
            &[
                "target",
                "field",
                "after",
                "semantic_after",
                "operation",
                "task",
                "task_position",
            ],
            &location,
        )?;
        if let Some(operation) = object.get("operation") {
            match operation.as_str() {
                Some("add_task") => {
                    if object.len() != 2 || !object.contains_key("task") {
                        return Err(issue(
                            "invalid_contract_value",
                            "add_task requires only a semantic task.",
                            json!({}),
                        ));
                    }
                    let task = &edit["task"];
                    fields(
                        task,
                        &[
                            "title",
                            "goal",
                            "skill_id",
                            "selected_paths",
                            "references",
                            "candidate",
                        ],
                        &["dependency_positions"],
                        &format!("{location}.task"),
                    )?;
                    for key in ["title", "goal"] {
                        if task[key].as_str().is_none_or(str::is_empty) {
                            return Err(issue(
                                "invalid_contract_value",
                                "A semantic TASK needs title and goal.",
                                json!({}),
                            ));
                        }
                    }
                    if !task["skill_id"].is_null() && !task["skill_id"].is_string()
                        || !task["selected_paths"].is_array()
                        || !task["references"].is_array()
                        || !task["candidate"].is_object()
                    {
                        return Err(issue(
                            "invalid_contract_value",
                            "The semantic TASK is invalid.",
                            json!({}),
                        ));
                    }
                    semantic_only(&task["candidate"])?;
                }
                Some("remove_task") => {
                    if object.len() != 2 || edit["task_position"].as_u64().is_none_or(|v| v == 0) {
                        return Err(issue(
                            "invalid_contract_value",
                            "remove_task requires one-based task_position.",
                            json!({}),
                        ));
                    }
                }
                _ => {
                    return Err(issue(
                        "invalid_contract_value",
                        "Invalid specification operation.",
                        json!({}),
                    ));
                }
            }
            continue;
        }
        if object.contains_key("task") || object.contains_key("task_position") {
            return Err(issue(
                "invalid_contract_value",
                "A field edit requires a target and field.",
                json!({}),
            ));
        }
        let target = edit.get("target").ok_or_else(|| {
            issue(
                "invalid_contract_value",
                "A field edit requires a target and field.",
                json!({}),
            )
        })?;
        fields(
            target,
            &["artifact"],
            &["task_id"],
            &format!("{location}.target"),
        )?;
        let artifact_name = target["artifact"]
            .as_str()
            .filter(|name| ["task_index", "task_item"].contains(name))
            .ok_or_else(|| artifact("spec_prepare_artifact", "Use task_index or task_item."))?;
        if (artifact_name == "task_item") != target.get("task_id").is_some_and(Value::is_string) {
            return Err(issue(
                "invalid_contract_value",
                "Only a task_item field edit requires task_id.",
                json!({}),
            ));
        }
        let field = edit["field"].as_str().ok_or_else(|| {
            issue(
                "invalid_contract_value",
                "A field edit requires a target and field.",
                json!({}),
            )
        })?;
        let simple = match artifact_name {
            "task_index" => ["title", "summary"].contains(&field),
            _ => ["title", "goal"].contains(&field),
        };
        let semantic = match artifact_name {
            "task_index" => ["decisions", "execution_defaults"].contains(&field),
            _ => [
                "traceability",
                "dependencies",
                "inputs",
                "decisions",
                "files",
                "risks",
                "steps",
                "commands",
                "operations",
                "validations",
            ]
            .contains(&field),
        };
        if simple {
            if !object.contains_key("after")
                || object.contains_key("semantic_after")
                || !(edit["after"].is_null() || edit["after"].is_string())
            {
                return Err(issue(
                    "invalid_contract_value",
                    "Simple fields require after.",
                    json!({}),
                ));
            }
        } else if semantic {
            if !object.contains_key("semantic_after") || object.contains_key("after") {
                return Err(issue(
                    "invalid_contract_value",
                    "Machine-bearing fields require semantic_after.",
                    json!({}),
                ));
            }
            semantic_only(&edit["semantic_after"])?;
        } else {
            return Err(issue(
                "invalid_contract_value",
                "This specification field has no semantic operation.",
                json!({}),
            ));
        }
    }
    let _: work_model::specification::SpecPrepareRequest = serde_json::from_value(value.clone())
        .expect("validated Specification edit request matches its model");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        json!({"schema":"work-spec-prepare-request/v1","requirement_id":"example",
            "reason":"Confirmed goal revision","edits":[{"target":{"artifact":"task_item",
                "task_id":"TASK-001"},"field":"goal","after":"Reviewed goal"}]})
    }

    #[test]
    fn request_preserves_null_and_rejects_missing_unknown_and_invalid_source_fields() {
        let mut value = example();
        value["edits"][0]["after"] = Value::Null;
        validate_prepare_request(&value).unwrap();
        let raw = crate::canonical::canonical_json(&value).unwrap();
        validate_prepare_request(&serde_json::from_slice::<Value>(&raw).unwrap()).unwrap();
        value["edits"][0].as_object_mut().unwrap().remove("after");
        assert_eq!(
            validate_prepare_request(&value).unwrap_err().reason_code,
            "invalid_contract_value"
        );
        for nested in [false, true] {
            let mut value = example();
            let target = if nested {
                &mut value["edits"][0]
            } else {
                &mut value
            };
            target["unexpected"] = json!(true);
            let error = validate_prepare_request(&value).unwrap_err();
            assert_eq!(error.reason_code, "invalid_object_fields");
            assert_eq!(
                error.details["location"],
                if nested { "edits[0]" } else { "spec_prepare" }
            );
        }
        for edits in [json!([]), json!({})] {
            let mut value = example();
            value["edits"] = edits;
            assert_eq!(
                validate_prepare_request(&value).unwrap_err().reason_code,
                "spec_prepare_edits"
            );
        }
        let mut value = example();
        value["schema"] = json!("unsupported");
        assert_eq!(
            validate_prepare_request(&value).unwrap_err().reason_code,
            "spec_prepare_schema"
        );
        let mut value = example();
        value["edits"][0]["target"]["artifact"] = json!("unknown");
        assert_eq!(
            validate_prepare_request(&value).unwrap_err().reason_code,
            "spec_prepare_artifact"
        );
    }

    #[test]
    fn semantic_edits_reject_formal_fields_and_accept_identity_free_addition() {
        assert!(formal_id("PLAN-DECISION-001"));
        assert!(formal_id("TASK-SPEC-001"));
        for field in ["before", "path", "operation"] {
            let mut value = example();
            value["edits"][0][field] = json!("caller value");
            assert_eq!(
                validate_prepare_request(&value).unwrap_err().reason_code,
                if field == "operation" {
                    "invalid_contract_value"
                } else {
                    "invalid_object_fields"
                }
            );
        }
        for edit in [
            json!({"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"/","after":{"schema":"work-task-item/v1"}}),
            json!({"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"validations","after":[{"id":"VAL-001"}]}),
            json!({"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"validations","semantic_after":[{"key":"verify","id":"VAL-001"}]}),
            json!({"target":{"artifact":"plan"},"field":"constraints","semantic_after":[{"key":"boundary","applies_to":["GOAL-001"]}]}),
        ] {
            let mut value = example();
            value["edits"] = json!([edit]);
            assert!(validate_prepare_request(&value).is_err());
        }
        let mut value = example();
        value["edits"] = json!([{"operation":"add_task","task":{
            "title":"Implement","goal":"Deliver","skill_id":null,"selected_paths":[],
            "references":[],"dependency_positions":[],"candidate":{
                "steps":[{"key":"check","action":"Check.","references":[{"kind":"validations","key":"verify"}]}],
                "validations":[{"key":"verify","kind":"manual","confirmer":"user","criteria":"Approved."}]}}}]);
        validate_prepare_request(&value).unwrap();
    }
}
