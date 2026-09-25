//! Pure selection rules for a reviewed Attempt-start request.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::canonical::canonical_json_sha256;
use crate::execution::deviation::formalize_semantic_action;
use crate::execution::requests::validate_attempt_start_request;
use crate::execution::{ExecutionIssue, validate_authorization_scope};

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

fn selected(
    choice: &Value,
    field: &str,
    available: &[Value],
) -> Result<Vec<Value>, ExecutionIssue> {
    let positions = choice[field].as_array().ok_or_else(|| {
        issue(
            "attempt_start_prepare_invalid_scope",
            "Selected positions must be unique and present in the current TASK.",
            json!({"field":field}),
        )
    })?;
    let mut seen = HashSet::new();
    for position in positions {
        let number = position
            .as_u64()
            .filter(|number| *number > 0)
            .ok_or_else(|| {
                issue(
                    "attempt_start_prepare_invalid_scope",
                    "Selected positions must be unique and present in the current TASK.",
                    json!({"field":field}),
                )
            })?;
        if number > available.len() as u64 || !seen.insert(number) {
            return Err(issue(
                "attempt_start_prepare_invalid_scope",
                "Selected positions must be unique and present in the current TASK.",
                json!({"field":field}),
            ));
        }
    }
    Ok(available
        .iter()
        .enumerate()
        .filter(|(index, _)| seen.contains(&((*index + 1) as u64)))
        .map(|(_, row)| row.clone())
        .collect())
}

fn strict_choice_fields(
    value: &Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), ExecutionIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            if location == "contract" {
                "json_contract_not_object"
            } else {
                "invalid_contract_value"
            },
            if location == "contract" {
                "The JSON contract root must be an object."
            } else {
                "The JSON contract contains an invalid value."
            },
            if location == "contract" {
                json!({})
            } else {
                json!({"location":location,
            "validation_type":"model_type","issues":[{"location":location,
                "validation_type":"model_type"}]})
            },
        )
    })?;
    let mut missing = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .map(|field| (*field).to_owned())
        .collect::<Vec<_>>();
    let mut unknown = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    unknown.sort();
    if !missing.is_empty() || !unknown.is_empty() {
        let mut issues = missing
            .iter()
            .map(|field| {
                json!({"location":if location == "contract" { field.clone() }
                else { format!("{location}.{field}") },"validation_type":"missing"})
            })
            .collect::<Vec<_>>();
        issues.extend(unknown.iter().map(|field| {
            json!({
            "location":if location == "contract" { field.clone() }
                else { format!("{location}.{field}") },"validation_type":"extra_forbidden"})
        }));
        issues.sort_by(|left, right| {
            left["location"]
                .as_str()
                .cmp(&right["location"].as_str())
                .then(
                    left["validation_type"]
                        .as_str()
                        .cmp(&right["validation_type"].as_str()),
                )
        });
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":location,"missing":missing,"unknown":unknown,"issues":issues}),
        ));
    }
    Ok(())
}

fn invalid_choice_value(location: &str, validation_type: &'static str) -> ExecutionIssue {
    issue(
        "invalid_contract_value",
        "The JSON contract contains an invalid value.",
        json!({"location":location,"validation_type":validation_type,
            "issues":[{"location":location,"validation_type":validation_type}]}),
    )
}

fn positive_position(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    if !value.is_i64() && !value.is_u64() {
        return Err(invalid_choice_value(location, "int_type"));
    }
    if value.as_u64().is_none_or(|number| number == 0) {
        return Err(invalid_choice_value(location, "greater_than"));
    }
    Ok(())
}

fn nonempty_text(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_choice_value(location, "string_type"))?;
    if text.is_empty() {
        return Err(invalid_choice_value(location, "string_too_short"));
    }
    if text.trim().is_empty() {
        return Err(invalid_choice_value(location, "string_pattern_mismatch"));
    }
    Ok(())
}

fn semantic_command(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    strict_choice_fields(value, location, &["mode"], &["argv", "script", "execution"])?;
    if let Some(execution) = value.get("execution").filter(|value| !value.is_null()) {
        let execution_location = format!("{location}.execution");
        strict_choice_fields(
            execution,
            &execution_location,
            &["working_directory", "os", "shell"],
            &[],
        )?;
        for field in ["working_directory", "os", "shell"] {
            if !execution[field].is_string() {
                return Err(invalid_choice_value(
                    &format!("{execution_location}.{field}"),
                    "string_type",
                ));
            }
        }
    }
    match value["mode"].as_str() {
        Some("argv") => {
            if value.get("argv").is_none_or(Value::is_null) {
                return Err(invalid_choice_value(location, "value_error"));
            }
            let argv = value["argv"]
                .as_array()
                .ok_or_else(|| invalid_choice_value(&format!("{location}.argv"), "list_type"))?;
            for (index, argument) in argv.iter().enumerate() {
                nonempty_text(argument, &format!("{location}.argv[{index}]"))?;
            }
            if argv.is_empty() || value.get("script").is_some_and(|script| !script.is_null()) {
                return Err(invalid_choice_value(location, "value_error"));
            }
        }
        Some("shell") => {
            if value.get("script").is_none_or(Value::is_null) {
                return Err(invalid_choice_value(location, "value_error"));
            }
            nonempty_text(&value["script"], &format!("{location}.script"))?;
            if value.get("argv").is_some_and(|argv| !argv.is_null()) {
                return Err(invalid_choice_value(location, "value_error"));
            }
        }
        _ => {
            return Err(invalid_choice_value(
                &format!("{location}.mode"),
                "literal_error",
            ));
        }
    }
    Ok(())
}

pub fn validate_prepare_choice(choice: &Value) -> Result<(), ExecutionIssue> {
    strict_choice_fields(
        choice,
        "contract",
        &[
            "command_positions",
            "validation_positions",
            "modifiable_files",
            "external_operation_positions",
            "allowed_deviations",
            "authorization_evidence",
        ],
        &["carried_records"],
    )?;
    for field in [
        "command_positions",
        "validation_positions",
        "external_operation_positions",
    ] {
        let positions = choice[field]
            .as_array()
            .ok_or_else(|| invalid_choice_value(field, "list_type"))?;
        for (index, value) in positions.iter().enumerate() {
            positive_position(value, &format!("{field}[{index}]"))?;
        }
    }
    let files = choice["modifiable_files"]
        .as_array()
        .ok_or_else(|| invalid_choice_value("modifiable_files", "list_type"))?;
    for (index, value) in files.iter().enumerate() {
        if !value.is_string() {
            return Err(invalid_choice_value(
                &format!("modifiable_files[{index}]"),
                "string_type",
            ));
        }
    }
    if !choice["authorization_evidence"].is_string() {
        return Err(invalid_choice_value(
            "authorization_evidence",
            "string_type",
        ));
    }
    let deviations = choice["allowed_deviations"]
        .as_array()
        .ok_or_else(|| invalid_choice_value("allowed_deviations", "list_type"))?;
    for (index, selection) in deviations.iter().enumerate() {
        let location = format!("allowed_deviations[{index}]");
        strict_choice_fields(
            selection,
            &location,
            &["anchor_kind", "anchor_position", "action"],
            &[],
        )?;
        if !matches!(
            selection["anchor_kind"].as_str(),
            Some("command" | "validation" | "operation")
        ) {
            return Err(invalid_choice_value(
                &format!("{location}.anchor_kind"),
                "literal_error",
            ));
        }
        positive_position(
            &selection["anchor_position"],
            &format!("{location}.anchor_position"),
        )?;
        let action = &selection["action"];
        let kind = action["kind"].as_str().ok_or_else(|| {
            invalid_choice_value(&format!("{location}.action"), "union_tag_not_found")
        })?;
        let field = match kind {
            "replace_command" => "replacement",
            "add_command" => "command",
            "add_validation" => "validation",
            "skip_record" => "reason",
            "adjust_operation" => "operation",
            _ => {
                return Err(invalid_choice_value(
                    &format!("{location}.action"),
                    "union_tag_invalid",
                ));
            }
        };
        strict_choice_fields(
            action,
            &format!("{location}.action.{kind}"),
            &["kind", field],
            &[],
        )?;
        let field_location = format!("{location}.action.{kind}.{field}");
        match kind {
            "replace_command" | "add_command" => semantic_command(&action[field], &field_location)?,
            "skip_record" => nonempty_text(&action[field], &field_location)?,
            "add_validation" => {
                let validation = &action[field];
                strict_choice_fields(
                    validation,
                    &field_location,
                    &["kind"],
                    &[
                        "command_positions",
                        "pass_condition",
                        "confirmer",
                        "criteria",
                        "acceptance_positions",
                    ],
                )?;
                if !matches!(validation["kind"].as_str(), Some("automated" | "manual")) {
                    return Err(invalid_choice_value(
                        &format!("{field_location}.kind"),
                        "literal_error",
                    ));
                }
                for key in ["command_positions", "acceptance_positions"] {
                    if let Some(value) = validation.get(key).filter(|value| !value.is_null()) {
                        let rows = value.as_array().ok_or_else(|| {
                            invalid_choice_value(&format!("{field_location}.{key}"), "list_type")
                        })?;
                        for (index, row) in rows.iter().enumerate() {
                            positive_position(row, &format!("{field_location}.{key}[{index}]"))?;
                        }
                    }
                }
                for key in ["pass_condition", "confirmer", "criteria"] {
                    if let Some(value) = validation.get(key).filter(|value| !value.is_null()) {
                        nonempty_text(value, &format!("{field_location}.{key}"))?;
                    }
                }
            }
            "adjust_operation" => {
                let operation = &action[field];
                strict_choice_fields(
                    operation,
                    &field_location,
                    &["kind", "action", "target", "validation_position"],
                    &["command_position"],
                )?;
                for key in ["kind", "action", "target"] {
                    nonempty_text(&operation[key], &format!("{field_location}.{key}"))?;
                }
                positive_position(
                    &operation["validation_position"],
                    &format!("{field_location}.validation_position"),
                )?;
                if let Some(position) = operation
                    .get("command_position")
                    .filter(|value| !value.is_null())
                {
                    positive_position(position, &format!("{field_location}.command_position"))?;
                }
            }
            _ => unreachable!("validated action kind"),
        }
    }
    if let Some(value) = choice.get("carried_records") {
        let carried = value
            .as_array()
            .ok_or_else(|| invalid_choice_value("carried_records", "list_type"))?;
        for (index, selection) in carried.iter().enumerate() {
            let location = format!("carried_records[{index}]");
            strict_choice_fields(selection, &location, &["position", "evidence"], &[])?;
            positive_position(&selection["position"], &format!("{location}.position"))?;
            if !selection["evidence"].is_string() {
                return Err(invalid_choice_value(
                    &format!("{location}.evidence"),
                    "string_type",
                ));
            }
            if selection["evidence"]
                .as_str()
                .is_some_and(|text| text.trim().is_empty())
            {
                return Err(invalid_choice_value(
                    &format!("{location}.evidence"),
                    "value_error",
                ));
            }
        }
    }
    work_model::execution::request::verified::<
        work_model::execution::request::AttemptStartPrepareRequest,
    >(choice);
    Ok(())
}

pub fn build_prepare_request(
    choice: &Value,
    task: &Value,
    defaults: &Value,
    worktree: &Value,
    index: &Value,
    source_attempt: Option<&Value>,
) -> Result<Value, ExecutionIssue> {
    validate_prepare_choice(choice)?;
    let commands = selected(
        choice,
        "command_positions",
        task["commands"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    )?;
    let validations = selected(
        choice,
        "validation_positions",
        task["validations"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    )?;
    let external = task["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == "external_state")
        .cloned()
        .collect::<Vec<_>>();
    let operations = selected(choice, "external_operation_positions", &external)?;
    let mut deviations = Vec::new();
    let mut allocated = json!({"execution_deviations":[]});
    for selection in choice["allowed_deviations"].as_array().ok_or_else(|| {
        issue(
            "attempt_start_prepare_invalid_choice",
            "Allowed deviations must be an array.",
            json!({}),
        )
    })? {
        let group = match selection["anchor_kind"].as_str() {
            Some("command") => "commands",
            Some("validation") => "validations",
            Some("operation") => "operations",
            _ => {
                return Err(issue(
                    "attempt_start_prepare_invalid_anchor",
                    "The deviation anchor position is outside the current TASK.",
                    json!({}),
                ));
            }
        };
        let position = selection["anchor_position"]
            .as_u64()
            .filter(|position| *position > 0)
            .ok_or_else(|| {
                issue(
                    "attempt_start_prepare_invalid_anchor",
                    "The deviation anchor position is outside the current TASK.",
                    json!({}),
                )
            })?;
        let anchor = task[group]
            .as_array()
            .and_then(|rows| rows.get((position - 1) as usize))
            .and_then(|row| row["id"].as_str())
            .ok_or_else(|| {
                issue(
                    "attempt_start_prepare_invalid_anchor",
                    "The deviation anchor position is outside the current TASK.",
                    json!({}),
                )
            })?;
        let formal = formalize_semantic_action(&selection["action"], task, &allocated, anchor)?;
        if deviations.contains(&formal) {
            return Err(issue(
                "attempt_start_prepare_duplicate_deviation",
                "The same deviation cannot be authorized twice.",
                json!({}),
            ));
        }
        allocated["execution_deviations"]
            .as_array_mut()
            .expect("array")
            .push(json!({"proposal":{"action":formal}}));
        deviations.push(formal);
    }
    let available_files = task["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| {
            ["path", "source", "destination"]
                .into_iter()
                .filter_map(move |field| row[field].as_str())
        })
        .collect::<HashSet<_>>();
    let files = choice["modifiable_files"].as_array().ok_or_else(|| {
        issue(
            "attempt_start_prepare_invalid_scope",
            "Selected files must be unique and declared by the current TASK.",
            json!({"field":"modifiable_files"}),
        )
    })?;
    let mut seen_files = HashSet::new();
    if files.iter().any(|file| {
        file.as_str()
            .is_none_or(|file| !available_files.contains(file) || !seen_files.insert(file))
    }) {
        return Err(issue(
            "attempt_start_prepare_invalid_scope",
            "Selected files must be unique and declared by the current TASK.",
            json!({"field":"modifiable_files"}),
        ));
    }
    let mut directories = Vec::new();
    for command in &commands {
        let execution = command
            .get("execution")
            .filter(|value| !value.is_null())
            .unwrap_or(defaults);
        let directory = execution["working_directory"].as_str().ok_or_else(|| {
            issue(
                "attempt_start_prepare_execution_required",
                "A selected command needs execution defaults.",
                json!({}),
            )
        })?;
        if !directories.contains(&directory) {
            directories.push(directory);
        }
    }
    let authorization = json!({"schema":"work-attempt-authorization/v1",
        "task_id":task["id"],"commands":commands,"validations":validations,
        "modifiable_files":files,"working_directories":directories,
        "external_operations":operations,"allowed_deviations":deviations,
        "reapproval_conditions":["scope_expansion","source_or_worktree_drift",
            "failure_divergence","retry","recovery","unknown_result"],
        "authorization_evidence":choice["authorization_evidence"]});
    validate_authorization_scope(&authorization, task, defaults)?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task["id"])
        .ok_or_else(|| {
            issue(
                "attempt_start_prepare_ineligible",
                "The TASK is not eligible for a new Attempt.",
                json!({}),
            )
        })?;
    let status = row["status"].as_str().unwrap_or("");
    if !matches!(status, "pending" | "pending_retry") || row["status"] != worktree["task_status"] {
        return Err(issue(
            "attempt_start_prepare_ineligible",
            "The TASK is not eligible for a new Attempt.",
            json!({}),
        ));
    }
    let mut request = json!({"schema":"work-attempt-start-request/v1",
        "worktree_snapshot_sha256":worktree["snapshot_sha256"],"authorization":authorization});
    let carried_choices = match choice.get("carried_records") {
        None => Vec::new(),
        Some(Value::Array(rows)) => rows.clone(),
        _ => {
            return Err(issue(
                "attempt_start_prepare_invalid_choice",
                "Carried records must be an array.",
                json!({}),
            ));
        }
    };
    if status == "pending_retry" {
        let latest = row["latest_attempt"].as_str().ok_or_else(|| {
            issue(
                "attempt_start_latest_attempt_required",
                "A retry needs its latest Attempt.",
                json!({}),
            )
        })?;
        let source = source_attempt.ok_or_else(|| {
            issue(
                "attempt_start_latest_attempt_required",
                "A retry needs its latest Attempt.",
                json!({}),
            )
        })?;
        let available = source["records"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(source["carried_records"].as_array().into_iter().flatten())
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let mut carried = Vec::new();
        for selection in carried_choices {
            let position = selection["position"].as_u64().filter(|position| *position > 0)
                .ok_or_else(|| issue("attempt_start_prepare_invalid_carried_position",
                    "Carried record positions must be unique and present in the source Attempt.", json!({})))?;
            if !seen.insert(position) || position > available.len() as u64 {
                return Err(issue(
                    "attempt_start_prepare_invalid_carried_position",
                    "Carried record positions must be unique and present in the source Attempt.",
                    json!({}),
                ));
            }
            let row = available[(position - 1) as usize];
            carried.push(
                json!({"record_id":row.get("id").unwrap_or(&row["record_id"]),
                "evidence":selection["evidence"]}),
            );
        }
        request["continuation"] = json!({"source_attempt_id":latest,"carried_records":carried});
    } else if !carried_choices.is_empty() {
        return Err(issue(
            "attempt_start_unexpected_continuation",
            "An initial Attempt cannot carry records.",
            json!({}),
        ));
    }
    validate_attempt_start_request(&request)?;
    let authorization_sha256 = canonical_json_sha256(&authorization).expect("JSON authorization");
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::AttemptStartPrepareResponse,
    >(
        json!({"schema":"work-attempt-start-prepare/v1","status":"prepared",
        "request":request,"authorization_sha256":authorization_sha256}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_derive_exact_initial_and_retry_scope() {
        let task = json!({"id":"TASK-001","commands":[{"id":"CMD-001","mode":"argv","argv":["tool"]}],
            "validations":[],"operations":[],"files":[{"path":"src.txt"}]});
        let defaults = json!({"working_directory":".","os":"windows","shell":"powershell"});
        let choice = json!({"command_positions":[1],"validation_positions":[],
            "modifiable_files":["src.txt"],"external_operation_positions":[],
            "allowed_deviations":[],"authorization_evidence":"Approved","carried_records":[]});
        let worktree = json!({"task_status":"pending","snapshot_sha256":"a".repeat(64)});
        let index = json!({"tasks":[{"id":"TASK-001","status":"pending"}]});
        let old_request = build_prepare_request(
            &json!({"authorization":{}}),
            &task,
            &defaults,
            &worktree,
            &index,
            None,
        )
        .unwrap_err();
        assert_eq!(old_request.reason_code, "invalid_object_fields");
        assert_eq!(old_request.details["unknown"], json!(["authorization"]));
        let mut formal_scope = choice.clone();
        formal_scope["command_ids"] = json!(["CMD-001"]);
        let error = validate_prepare_choice(&formal_scope).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["unknown"], json!(["command_ids"]));
        for (field, value, location, kind) in [
            (
                "command_positions",
                json!("x"),
                "command_positions",
                "list_type",
            ),
            (
                "command_positions",
                json!([0]),
                "command_positions[0]",
                "greater_than",
            ),
            (
                "command_positions",
                json!(["1"]),
                "command_positions[0]",
                "int_type",
            ),
            (
                "modifiable_files",
                json!([3]),
                "modifiable_files[0]",
                "string_type",
            ),
            (
                "authorization_evidence",
                json!(3),
                "authorization_evidence",
                "string_type",
            ),
            (
                "carried_records",
                json!("x"),
                "carried_records",
                "list_type",
            ),
        ] {
            let mut invalid = choice.clone();
            invalid[field] = value;
            let error = validate_prepare_choice(&invalid).unwrap_err();
            assert_eq!(error.reason_code, "invalid_contract_value");
            assert_eq!(error.details["location"], location);
            assert_eq!(error.details["validation_type"], kind);
        }
        let mut invalid_deviation = choice.clone();
        invalid_deviation["allowed_deviations"] = json!([{"anchor_kind":"command",
            "anchor_position":1,"action":{"kind":"skip_record",
                "record_id":"CMD-001","reason":"skip"}}]);
        let invalid = validate_prepare_choice(&invalid_deviation).unwrap_err();
        assert_eq!(invalid.reason_code, "invalid_object_fields");
        assert_eq!(
            invalid.details["location"],
            "allowed_deviations[0].action.skip_record"
        );
        let mut invalid_carried = choice.clone();
        invalid_carried["carried_records"] = json!([{"record_id":"CMD-001","evidence":"valid"}]);
        let invalid = validate_prepare_choice(&invalid_carried).unwrap_err();
        assert_eq!(invalid.reason_code, "invalid_object_fields");
        assert_eq!(invalid.details["location"], "carried_records[0]");
        let result =
            build_prepare_request(&choice, &task, &defaults, &worktree, &index, None).unwrap();
        assert_eq!(
            result["request"]["authorization"]["commands"],
            task["commands"]
        );
        assert_eq!(
            result["request"]["authorization"]["working_directories"],
            json!(["."])
        );
        assert!(result["request"].get("continuation").is_none());
        assert_eq!(
            result["authorization_sha256"],
            "3b4ade41af600f4f4f83d9c59d0b48a267d1424f3738a3e5650fd5ed1247323c"
        );
        let mut deviation_choice = choice.clone();
        deviation_choice["allowed_deviations"] = json!([{
            "anchor_kind":"command","anchor_position":1,
            "action":{"kind":"replace_command","replacement":{"mode":"argv","argv":["tool","--reviewed"]}}
        }]);
        let deviation =
            build_prepare_request(&deviation_choice, &task, &defaults, &worktree, &index, None)
                .unwrap();
        assert_eq!(
            deviation["request"]["authorization"]["allowed_deviations"],
            json!([{"kind":"replace_command","record_id":"CMD-001",
                "replacement":{"mode":"argv","argv":["tool","--reviewed"]}}])
        );
        let mut invalid_command = deviation_choice.clone();
        invalid_command["allowed_deviations"][0]["action"]["replacement"]["argv"] = json!([]);
        let invalid = validate_prepare_choice(&invalid_command).unwrap_err();
        assert_eq!(invalid.reason_code, "invalid_contract_value");
        assert_eq!(
            invalid.details["location"],
            "allowed_deviations[0].action.replace_command.replacement"
        );
        assert_eq!(invalid.details["validation_type"], "value_error");
        let duplicate = deviation_choice["allowed_deviations"][0].clone();
        deviation_choice["allowed_deviations"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(
            build_prepare_request(&deviation_choice, &task, &defaults, &worktree, &index, None)
                .unwrap_err()
                .reason_code,
            "attempt_start_prepare_duplicate_deviation"
        );
        let mut retry_index = index;
        retry_index["tasks"][0]["status"] = json!("pending_retry");
        retry_index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        let mut retry_worktree = worktree;
        retry_worktree["task_status"] = json!("pending_retry");
        let retry = build_prepare_request(
            &choice,
            &task,
            &defaults,
            &retry_worktree,
            &retry_index,
            Some(&json!({"records":[{"id":"CMD-001"}]})),
        )
        .unwrap();
        assert_eq!(
            retry["request"]["continuation"]["source_attempt_id"],
            "ATTEMPT-001"
        );
        let mut omitted_carried = choice.clone();
        omitted_carried
            .as_object_mut()
            .unwrap()
            .remove("carried_records");
        let retry_without_carried = build_prepare_request(
            &omitted_carried,
            &task,
            &defaults,
            &retry_worktree,
            &retry_index,
            Some(&json!({"records":[{"id":"CMD-001"}]})),
        )
        .unwrap();
        assert_eq!(
            retry_without_carried["request"]["continuation"]["carried_records"],
            json!([])
        );
        let mut carried_choice = choice.clone();
        carried_choice["carried_records"] = json!([{"position":1,"evidence":"Reviewed"}]);
        let carried = build_prepare_request(
            &carried_choice,
            &task,
            &defaults,
            &retry_worktree,
            &retry_index,
            Some(&json!({"records":[{"id":"CMD-001"}]})),
        )
        .unwrap();
        assert_eq!(
            carried["request"]["continuation"]["carried_records"],
            json!([{"record_id":"CMD-001","evidence":"Reviewed"}])
        );
        carried_choice["carried_records"] = json!([{"position":2,"evidence":"Reviewed"}]);
        assert_eq!(
            build_prepare_request(
                &carried_choice,
                &task,
                &defaults,
                &retry_worktree,
                &retry_index,
                Some(&json!({"records":[{"id":"CMD-001"}]}))
            )
            .unwrap_err()
            .reason_code,
            "attempt_start_prepare_invalid_carried_position"
        );
    }
}
