//! Pure formal TASK item validation.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::task::TaskIssue;
use crate::task::ordering::{TaskDocumentKind, render_task};

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details,
    }
}

fn strict<'a>(
    value: &'a Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, TaskIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location": location}),
        ));
    };
    let missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location": location, "missing": missing, "unknown": unknown}),
        ));
    }
    Ok(object)
}

fn text<'a>(value: &'a Value, location: &str) -> Result<&'a str, TaskIssue> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            issue(
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location": location}),
            )
        })
}

fn string_array(
    value: &Value,
    location: &str,
    allow_empty: bool,
) -> Result<Vec<String>, TaskIssue> {
    let Some(values) = value
        .as_array()
        .filter(|values| allow_empty || !values.is_empty())
    else {
        return Err(issue(
            "invalid_string_array",
            "A string array with the required cardinality is required.",
            json!({"location": location}),
        ));
    };
    let result: Vec<_> = values
        .iter()
        .map(|value| text(value, &format!("{location}[]")).map(str::to_owned))
        .collect::<Result<_, _>>()?;
    if result.iter().collect::<BTreeSet<_>>().len() != result.len() {
        return Err(issue(
            "duplicate_array_value",
            "Array values must be unique.",
            json!({"location": location}),
        ));
    }
    Ok(result)
}

fn numbered(value: &str, prefix: &str) -> Option<usize> {
    let suffix = value.strip_prefix(prefix)?.strip_prefix('-')?;
    (suffix.len() == 3 && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

fn items<'a>(
    task: &'a serde_json::Map<String, Value>,
    group: &str,
    prefix: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<Vec<&'a serde_json::Map<String, Value>>, TaskIssue> {
    let values = task[group]
        .as_array()
        .filter(|values| !values.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_item_array",
                "A present TASK item array must be non-empty.",
                json!({"location": group}),
            )
        })?;
    let mut previous = 0;
    let mut result = Vec::new();
    for (index, item) in values.iter().enumerate() {
        let mut fields = vec!["id"];
        fields.extend_from_slice(required);
        let item = strict(item, &format!("{group}[{index}]"), &fields, optional)?;
        let id = text(&item["id"], &format!("{group}[{index}].id"))?;
        let Some(number) = numbered(id, prefix).filter(|number| *number > previous) else {
            return Err(issue(
                "invalid_or_unsorted_id",
                "TASK item IDs must use the expected prefix and ascending order.",
                json!({}),
            ));
        };
        previous = number;
        result.push(item);
    }
    Ok(result)
}

pub fn validate_task_item(
    value: &Value,
    raw: &[u8],
    expected_task_id: &str,
) -> Result<Value, TaskIssue> {
    let required = [
        "schema",
        "id",
        "title",
        "skill_id",
        "instruction_selection",
        "traceability",
        "goal",
        "steps",
        "validations",
    ];
    let optional = [
        "dependencies",
        "inputs",
        "decisions",
        "files",
        "risks",
        "commands",
        "operations",
    ];
    let task = strict(value, "task_item", &required, &optional)?;
    if task["schema"] != "work-task-item/v1" {
        return Err(issue(
            "invalid_task_item_schema",
            "The TASK item schema is invalid.",
            json!({}),
        ));
    }
    let id = text(&task["id"], "id")?;
    if numbered(id, "TASK").is_none() || id != expected_task_id {
        return Err(issue(
            "task_item_identity_mismatch",
            "The TASK item ID does not match its reference.",
            json!({}),
        ));
    }
    for field in ["title", "goal"] {
        text(&task[field], field)?;
    }
    if !task["skill_id"].is_null() {
        text(&task["skill_id"], "skill_id")?;
    }
    let trace = strict(
        &task["traceability"],
        "traceability",
        &["goal_ids", "deliverable_ids", "acceptance_ids"],
        &["milestone_ids"],
    )?;
    for (field, prefix) in [
        ("goal_ids", "GOAL"),
        ("deliverable_ids", "DELIVERABLE"),
        ("acceptance_ids", "ACCEPTANCE"),
        ("milestone_ids", "MILESTONE"),
    ] {
        if let Some(raw) = trace.get(field) {
            let ids = string_array(raw, &format!("traceability.{field}"), false)?;
            if ids.iter().any(|id| numbered(id, prefix).is_none()) {
                return Err(issue(
                    "invalid_reference",
                    "A traceability ID has the wrong type.",
                    json!({}),
                ));
            }
        }
    }
    let dependencies = string_array(
        task.get("dependencies")
            .unwrap_or(&Value::Array(Vec::new())),
        "dependencies",
        true,
    )?;
    if dependencies
        .iter()
        .any(|dependency| numbered(dependency, "TASK").is_none() || dependency == id)
    {
        return Err(issue(
            "invalid_task_dependency",
            "A TASK dependency is invalid.",
            json!({}),
        ));
    }
    let mut local = BTreeSet::new();
    let mut tracked = BTreeSet::new();
    let mut commands = BTreeSet::new();
    let mut validations = BTreeSet::new();
    if task.contains_key("inputs") {
        for input in items(
            task,
            "inputs",
            "INPUT",
            &["kind", "source", "precondition"],
            &[],
        )? {
            if !matches!(
                input["kind"].as_str(),
                Some("task_output" | "project_state" | "user_provided" | "external")
            ) {
                return Err(issue(
                    "invalid_input_kind",
                    "Invalid TASK input kind.",
                    json!({}),
                ));
            }
            for field in ["source", "precondition"] {
                text(
                    &input[field],
                    &format!("{}.{field}", input["id"].as_str().unwrap()),
                )?;
            }
            local.insert(input["id"].as_str().unwrap().to_owned());
        }
    }
    if task.contains_key("decisions") {
        for decision in items(
            task,
            "decisions",
            "TASK-DECISION",
            &["statement", "rationale"],
            &[],
        )? {
            for field in ["statement", "rationale"] {
                text(
                    &decision[field],
                    &format!("{}.{field}", decision["id"].as_str().unwrap()),
                )?;
            }
            local.insert(decision["id"].as_str().unwrap().to_owned());
        }
    }
    if task.contains_key("files") {
        for file in items(
            task,
            "files",
            "FILE",
            &["action"],
            &["path", "source", "destination"],
        )? {
            let expected: BTreeSet<_> = match file["action"].as_str() {
                Some("create" | "modify") => ["id", "action", "path"].into_iter().collect(),
                Some("move") => ["id", "action", "source", "destination"]
                    .into_iter()
                    .collect(),
                _ => BTreeSet::new(),
            };
            if expected.is_empty()
                || file.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
            {
                return Err(issue(
                    "invalid_file_action",
                    "Invalid fields for TASK file action.",
                    json!({}),
                ));
            }
            for field in ["path", "source", "destination"] {
                if let Some(value) = file.get(field) {
                    text(value, &format!("{}.{field}", file["id"].as_str().unwrap()))?;
                }
            }
            let id = file["id"].as_str().unwrap().to_owned();
            tracked.insert(id.clone());
            local.insert(id);
        }
    }
    if task.contains_key("risks") {
        for risk in items(
            task,
            "risks",
            "RISK",
            &["condition", "impact", "mitigation"],
            &[],
        )? {
            for field in ["condition", "impact", "mitigation"] {
                text(
                    &risk[field],
                    &format!("{}.{field}", risk["id"].as_str().unwrap()),
                )?;
            }
            local.insert(risk["id"].as_str().unwrap().to_owned());
        }
    }
    if task.contains_key("commands") {
        for command in items(
            task,
            "commands",
            "CMD",
            &["mode"],
            &["argv", "script", "execution"],
        )? {
            let mode = command["mode"].as_str();
            let mut expected: BTreeSet<_> = match mode {
                Some("argv") => ["id", "mode", "argv"].into_iter().collect(),
                Some("shell") => ["id", "mode", "script"].into_iter().collect(),
                _ => BTreeSet::new(),
            };
            if command.contains_key("execution") {
                expected.insert("execution");
            }
            if expected.is_empty()
                || command.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected
            {
                return Err(issue(
                    "invalid_command_mode",
                    "Invalid command mode or fields.",
                    json!({}),
                ));
            }
            if mode == Some("argv") {
                string_array(&command["argv"], "command.argv", false)?;
            } else {
                text(&command["script"], "command.script")?;
            }
            if let Some(execution) = command.get("execution") {
                let execution = strict(
                    execution,
                    "command.execution",
                    &["working_directory", "os", "shell"],
                    &[],
                )?;
                for field in ["working_directory", "os", "shell"] {
                    text(&execution[field], &format!("command.execution.{field}"))?;
                }
            }
            let id = command["id"].as_str().unwrap().to_owned();
            tracked.insert(id.clone());
            commands.insert(id.clone());
            local.insert(id);
        }
    }
    for validation in items(
        task,
        "validations",
        "VAL",
        &["kind"],
        &[
            "command_ids",
            "pass_condition",
            "confirmer",
            "criteria",
            "acceptance_ids",
        ],
    )? {
        let mut common: BTreeSet<&str> = ["id", "kind"].into_iter().collect();
        if validation.contains_key("acceptance_ids") {
            common.insert("acceptance_ids");
        }
        let expected: BTreeSet<_> = match validation["kind"].as_str() {
            Some("automated") => common
                .union(&["command_ids", "pass_condition"].into_iter().collect())
                .copied()
                .collect(),
            Some("manual") => common
                .union(&["confirmer", "criteria"].into_iter().collect())
                .copied()
                .collect(),
            _ => BTreeSet::new(),
        };
        if expected.is_empty()
            || validation
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != expected
        {
            return Err(issue(
                "invalid_validation_kind",
                "Invalid validation kind or fields.",
                json!({}),
            ));
        }
        if validation["kind"] == "automated" {
            let ids = string_array(&validation["command_ids"], "validation.command_ids", false)?;
            if ids.iter().any(|id| !commands.contains(id)) {
                return Err(issue(
                    "invalid_reference",
                    "Automated validation references an unknown command.",
                    json!({}),
                ));
            }
            text(&validation["pass_condition"], "validation.pass_condition")?;
        } else {
            text(&validation["confirmer"], "validation.confirmer")?;
            text(&validation["criteria"], "validation.criteria")?;
        }
        if let Some(ids) = validation.get("acceptance_ids") {
            string_array(ids, "validation.acceptance_ids", false)?;
        }
        let id = validation["id"].as_str().unwrap().to_owned();
        tracked.insert(id.clone());
        validations.insert(id.clone());
        local.insert(id);
    }
    if task.contains_key("operations") {
        for operation in items(
            task,
            "operations",
            "OP",
            &["kind", "action", "target", "validation_id"],
            &["command_id"],
        )? {
            if !matches!(
                operation["kind"].as_str(),
                Some("local_state" | "external_state")
            ) || !operation["validation_id"]
                .as_str()
                .is_some_and(|id| validations.contains(id))
            {
                return Err(issue(
                    "invalid_reference",
                    "Invalid TASK operation kind or validation reference.",
                    json!({}),
                ));
            }
            if operation
                .get("command_id")
                .is_some_and(|id| !id.as_str().is_some_and(|id| commands.contains(id)))
            {
                return Err(issue(
                    "invalid_reference",
                    "TASK operation references an unknown command.",
                    json!({}),
                ));
            }
            text(&operation["action"], "operation.action")?;
            text(&operation["target"], "operation.target")?;
            let id = operation["id"].as_str().unwrap().to_owned();
            tracked.insert(id.clone());
            local.insert(id);
        }
    }
    let mut referenced = BTreeSet::new();
    for step in items(task, "steps", "STEP", &["action", "references"], &[])? {
        text(&step["action"], "step.action")?;
        for reference in string_array(&step["references"], "step.references", false)? {
            if !local.contains(&reference) && numbered(&reference, "DECISION").is_none() {
                return Err(issue(
                    "invalid_reference",
                    "STEP references an unknown TASK-local ID.",
                    json!({}),
                ));
            }
            referenced.insert(reference);
        }
    }
    if !tracked.is_subset(&referenced) {
        return Err(issue(
            "unreferenced_task_item",
            "Every FILE, CMD, OP, and VAL must be referenced by a STEP.",
            json!({}),
        ));
    }
    let rendered = render_task(value, TaskDocumentKind::Item).expect("JSON values serialize");
    if raw != rendered {
        return Err(issue(
            "noncanonical_json_contract",
            "The JSON contract does not match the required canonical rendering.",
            json!({}),
        ));
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskItemValidation,
    >(
        json!({"schema": "work-task-item-validation/v1", "task_id": id, "dependencies": dependencies, "task_item_sha256": sha256_hex(raw)}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_object_and_nonempty_text_keep_contract_field_boundaries() {
        let value = json!({"id":"ITEM-001","note":"Optional."});
        let object = strict(&value, "item", &["id"], &["note"]).unwrap();
        assert!(std::ptr::eq(object, value.as_object().unwrap()));
        let error = strict(&json!([]), "item", &["id"], &[]).unwrap_err();
        assert_eq!(error.reason_code, "expected_object");
        assert_eq!(error.details["location"], "item");
        let error = strict(&json!({"extra":true}), "item", &["id"], &[]).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["missing"], json!(["id"]));
        assert_eq!(error.details["unknown"], json!(["extra"]));
        let error = text(&json!(" "), "item.name").unwrap_err();
        assert_eq!(error.reason_code, "empty_text_value");
        assert_eq!(error.details["location"], "item.name");
    }

    #[test]
    fn example_task_item_round_trip_and_reference_error() {
        let item = json!({"schema": "work-task-item/v1", "id": "TASK-001", "title": "Example", "skill_id": null,
            "instruction_selection": {"selected_paths": [], "resolved_paths": ["general"], "sources": [{"kind": "instruction", "logical_name": "task.general", "canonical_sha256": "a".repeat(64)}], "references": [], "instructions_sha256": "b".repeat(64)},
            "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
            "goal": "Produce the result.", "steps": [{"id": "STEP-001", "action": "Validate.", "references": ["VAL-001"]}],
            "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "user", "criteria": "Approved."}]});
        let raw = render_task(&item, TaskDocumentKind::Item).unwrap();
        let preserved = item.clone();
        assert_eq!(
            validate_task_item(&item, &raw, "TASK-001").unwrap()["task_item_sha256"],
            sha256_hex(&raw)
        );
        let typed: work_model::task::item::TaskItem = serde_json::from_value(item.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), item);
        assert_eq!(item, preserved);
        let mut legacy = item.clone();
        legacy["rule_selection"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("instruction_selection")
            .unwrap();
        let legacy_raw = render_task(&legacy, TaskDocumentKind::Item).unwrap();
        let error = validate_task_item(&legacy, &legacy_raw, "TASK-001").unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["missing"], json!(["instruction_selection"]));
        assert_eq!(error.details["unknown"], json!(["rule_selection"]));
        assert!(
            raw.starts_with(b"{\n  \"schema\": \"work-task-item/v1\",\n  \"id\": \"TASK-001\"")
        );
        let mut noncanonical = b" ".to_vec();
        noncanonical.extend_from_slice(&raw);
        assert_eq!(
            validate_task_item(&item, &noncanonical, "TASK-001")
                .unwrap_err()
                .reason_code,
            "noncanonical_json_contract"
        );
        let mut wrong_id = item.clone();
        wrong_id["id"] = json!("TASK-002");
        let wrong_raw = render_task(&wrong_id, TaskDocumentKind::Item).unwrap();
        assert_eq!(
            validate_task_item(&wrong_id, &wrong_raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "task_item_identity_mismatch"
        );
        let mut unknown = item.clone();
        unknown["unknown"] = json!(true);
        let unknown_raw = render_task(&unknown, TaskDocumentKind::Item).unwrap();
        assert_eq!(
            validate_task_item(&unknown, &unknown_raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let mut nested_unknown = item.clone();
        nested_unknown["traceability"]["unknown"] = json!(true);
        let nested_raw = render_task(&nested_unknown, TaskDocumentKind::Item).unwrap();
        assert_eq!(
            validate_task_item(&nested_unknown, &nested_raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let mut retired_schema = item.clone();
        retired_schema["schema"] = json!("work-task-item/v2");
        let retired_raw = render_task(&retired_schema, TaskDocumentKind::Item).unwrap();
        assert_eq!(
            validate_task_item(&retired_schema, &retired_raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "invalid_task_item_schema"
        );
        let mut coerced_title = item.clone();
        coerced_title["title"] = json!(1);
        let coerced_raw = render_task(&coerced_title, TaskDocumentKind::Item).unwrap();
        assert_eq!(
            validate_task_item(&coerced_title, &coerced_raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "empty_text_value"
        );
        let mut invalid = item;
        invalid["steps"][0]["references"] = json!(["VAL-999"]);
        let raw = render_task(&invalid, TaskDocumentKind::Item).unwrap();
        assert_eq!(
            validate_task_item(&invalid, &raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "invalid_reference"
        );
    }
}
