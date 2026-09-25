//! Pure formal TASK index validation and item references.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::protocol::{INVALID_SHA256_ERROR_CODE, valid_sha256};
use crate::task::TaskIssue;
use crate::task::changes::validate_index_changes;
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
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            issue(
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location": location}),
            )
        })
}

fn numbered(value: &str, prefix: &str) -> Option<usize> {
    let suffix = value.strip_prefix(prefix)?.strip_prefix('-')?;
    (suffix.len() == 3 && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

fn sha(value: &Value, location: &str) -> Result<(), TaskIssue> {
    if !value.as_str().is_some_and(valid_sha256) {
        return Err(issue(
            INVALID_SHA256_ERROR_CODE,
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            json!({"location": location}),
        ));
    }
    Ok(())
}

pub fn validate_task_index(
    value: &Value,
    raw: &[u8],
    actual_path: &str,
) -> Result<Value, TaskIssue> {
    let required = [
        "schema",
        "requirement_id",
        "spec_id",
        "status",
        "title",
        "summary",
        "artifacts",
        "source_plan",
        "instruction_selection",
        "tasks",
        "readiness",
    ];
    let optional = ["execution_defaults", "decisions", "changes"];
    let index = strict(value, "task_index", &required, &optional)?;
    if index["schema"] != "work-task-index/v1" {
        return Err(issue(
            "invalid_task_index_schema",
            "The TASK index schema is invalid.",
            json!({}),
        ));
    }
    let requirement = text(&index["requirement_id"], "requirement_id")?;
    let spec = text(&index["spec_id"], "spec_id")?;
    let spec_number = numbered(spec, "TASK-SPEC")
        .ok_or_else(|| issue("invalid_task_spec_id", "Invalid TASK spec ID.", json!({})))?;
    if index["status"] != "confirmed" {
        return Err(issue(
            "invalid_task_status",
            "A formal TASK status must be confirmed.",
            json!({}),
        ));
    }
    for field in ["title", "summary"] {
        text(&index[field], field)?;
    }
    let artifacts = strict(
        &index["artifacts"],
        "artifacts",
        &["plan", "task", "execution"],
        &[],
    )?;
    let task_path = text(&artifacts["task"], "artifacts.task")?;
    let expected_tail = format!("/{requirement}/index.json");
    if !task_path.ends_with(&expected_tail) {
        return Err(issue(
            "task_index_path_requirement_mismatch",
            "The TASK index path must end with <requirement-id>/index.json.",
            json!({"path": task_path, "requirement_id": requirement}),
        ));
    }
    if task_path != actual_path {
        return Err(issue(
            "task_artifact_path_mismatch",
            "The TASK index path does not match artifacts.task.",
            json!({}),
        ));
    }
    let plan_path = text(&artifacts["plan"], "artifacts.plan")?;
    if !plan_path.ends_with(&format!("/{requirement}.json")) {
        return Err(issue(
            "plan_path_requirement_mismatch",
            "The Plan path must end with the requirement ID and .json.",
            json!({}),
        ));
    }
    let execution = text(&artifacts["execution"], "artifacts.execution")?;
    if !execution.ends_with(&format!("/{requirement}")) {
        return Err(issue(
            "execution_path_requirement_mismatch",
            "The execution path must end with the requirement ID.",
            json!({}),
        ));
    }
    let source_plan = strict(
        &index["source_plan"],
        "source_plan",
        &["canonical_sha256", "hierarchy_selection_sha256"],
        &[],
    )?;
    for field in ["canonical_sha256", "hierarchy_selection_sha256"] {
        sha(&source_plan[field], &format!("source_plan.{field}"))?;
    }
    if let Some(execution) = index.get("execution_defaults") {
        let execution = strict(
            execution,
            "execution_defaults",
            &["working_directory", "os", "shell"],
            &[],
        )?;
        for field in ["working_directory", "os", "shell"] {
            text(&execution[field], &format!("execution_defaults.{field}"))?;
        }
    }
    let tasks = index["tasks"]
        .as_array()
        .filter(|tasks| !tasks.is_empty())
        .ok_or_else(|| issue("invalid_item_array", "tasks must be non-empty.", json!({})))?;
    let mut ids = Vec::new();
    let mut paths = serde_json::Map::new();
    let mut hashes = serde_json::Map::new();
    let mut previous = 0;
    for (position, reference) in tasks.iter().enumerate() {
        let reference = strict(
            reference,
            &format!("tasks[{position}]"),
            &["id", "path", "canonical_sha256"],
            &[],
        )?;
        let id = text(&reference["id"], &format!("tasks[{position}].id"))?;
        let number = numbered(id, "TASK")
            .filter(|number| *number > previous)
            .ok_or_else(|| {
                issue(
                    "invalid_or_unsorted_id",
                    "Invalid or unsorted TASK ID.",
                    json!({}),
                )
            })?;
        previous = number;
        let path = text(&reference["path"], &format!("tasks[{position}].path"))?;
        if path != format!("tasks/{id}.json") {
            return Err(issue(
                "task_item_path_mismatch",
                "A TASK item path must exactly match tasks/<TASK-ID>.json.",
                json!({"task_id": id, "expected": format!("tasks/{id}.json"), "actual": path}),
            ));
        }
        sha(
            &reference["canonical_sha256"],
            &format!("tasks[{position}].canonical_sha256"),
        )?;
        ids.push(id.to_owned());
        paths.insert(id.into(), json!(path));
        hashes.insert(id.into(), reference["canonical_sha256"].clone());
    }
    if let Some(decisions) = index.get("decisions") {
        let decisions = decisions
            .as_array()
            .filter(|decisions| !decisions.is_empty())
            .ok_or_else(|| {
                issue(
                    "invalid_item_array",
                    "decisions must be non-empty.",
                    json!({}),
                )
            })?;
        for (position, decision) in decisions.iter().enumerate() {
            let decision = strict(
                decision,
                &format!("decisions[{position}]"),
                &["id", "statement", "rationale", "task_ids"],
                &[],
            )?;
            for field in ["id", "statement", "rationale"] {
                text(&decision[field], &format!("decisions[{position}].{field}"))?;
            }
            let applies = decision["task_ids"].as_array().ok_or_else(|| {
                issue(
                    "invalid_string_array",
                    "A non-empty string array is required.",
                    json!({}),
                )
            })?;
            if applies.len() < 2
                || applies
                    .iter()
                    .any(|id| !id.as_str().is_some_and(|id| ids.contains(&id.into())))
            {
                return Err(issue(
                    "invalid_shared_decision_scope",
                    "A shared decision must apply to at least two TASKs.",
                    json!({}),
                ));
            }
        }
    }
    if spec_number == 1 && index.contains_key("changes") {
        return Err(issue(
            "initial_task_has_changes",
            "Initial TASK index must omit changes.",
            json!({}),
        ));
    }
    if spec_number > 1 && !index.contains_key("changes") {
        return Err(issue(
            "task_changes_required",
            "A revised TASK index must contain changes.",
            json!({}),
        ));
    }
    if let Some(changes) = index.get("changes") {
        validate_index_changes(changes, spec, &ids)?;
    }
    let readiness = strict(
        &index["readiness"],
        "readiness",
        &["status", "spec_id"],
        &[],
    )?;
    if readiness["status"] != "passed" || readiness["spec_id"] != spec {
        return Err(issue(
            "invalid_readiness",
            "Formal TASK readiness must pass for the current spec.",
            json!({}),
        ));
    }
    let rendered = render_task(value, TaskDocumentKind::Index).expect("JSON values serialize");
    if raw != rendered {
        return Err(issue(
            "noncanonical_json_contract",
            "The JSON contract does not match the required canonical rendering.",
            json!({}),
        ));
    }
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(issue(
            "invalid_or_unsorted_id",
            "Invalid or unsorted TASK ID.",
            json!({}),
        ));
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskIndexValidation,
    >(
        json!({"schema": "work-task-index-validation/v1", "requirement_id": requirement, "spec_id": spec, "task_ids": ids, "task_paths": paths, "task_item_sha256": hashes, "task_index_sha256": sha256_hex(raw)}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_index_matches_python_fingerprint() {
        let index = json!({"schema": "work-task-index/v1", "requirement_id": "example", "spec_id": "TASK-SPEC-001", "status": "confirmed", "title": "Example", "summary": "Example tasks.",
            "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
            "source_plan": {"canonical_sha256": "c".repeat(64), "hierarchy_selection_sha256": "d".repeat(64)},
            "instruction_selection": {"sources": [{"kind": "instruction", "logical_name": "task.general", "canonical_sha256": "a".repeat(64)}], "references": [], "instructions_sha256": "b".repeat(64)},
            "tasks": [{"id": "TASK-001", "path": "tasks/TASK-001.json", "canonical_sha256": "e".repeat(64)}],
            "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"}});
        let raw = render_task(&index, TaskDocumentKind::Index).unwrap();
        let preserved = index.clone();
        assert_eq!(
            validate_task_index(&index, &raw, "outputs/work/tasks/example/index.json").unwrap()["task_index_sha256"],
            sha256_hex(&raw)
        );
        let typed: work_model::task::index::TaskIndex =
            serde_json::from_value(index.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), index);
        assert_eq!(index, preserved);
        let mut retired_schema = index.clone();
        retired_schema["schema"] = json!("work-task-index/v2");
        let retired_raw = render_task(&retired_schema, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_task_index(
                &retired_schema,
                &retired_raw,
                "outputs/work/tasks/example/index.json"
            )
            .unwrap_err()
            .reason_code,
            "invalid_task_index_schema"
        );
        let validated =
            validate_task_index(&index, &raw, "outputs/work/tasks/example/index.json").unwrap();
        assert_eq!(validated["task_ids"], json!(["TASK-001"]));
        assert_eq!(validated["task_item_sha256"]["TASK-001"], "e".repeat(64));
        let mut legacy = index.clone();
        legacy["rule_selection"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("instruction_selection")
            .unwrap();
        let legacy_raw = render_task(&legacy, TaskDocumentKind::Index).unwrap();
        let error = validate_task_index(
            &legacy,
            &legacy_raw,
            "outputs/work/tasks/example/index.json",
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["missing"], json!(["instruction_selection"]));
        assert_eq!(error.details["unknown"], json!(["rule_selection"]));
        assert!(raw.starts_with(b"{\n  \"schema\": \"work-task-index/v1\""));
        assert!(raw.windows(b"\"id\": \"TASK-001\",\n      \"path\": \"tasks/TASK-001.json\",\n      \"canonical_sha256\"".len()).any(|window| window == b"\"id\": \"TASK-001\",\n      \"path\": \"tasks/TASK-001.json\",\n      \"canonical_sha256\""));
        let mut noncanonical = b" ".to_vec();
        noncanonical.extend_from_slice(&raw);
        assert_eq!(
            validate_task_index(
                &index,
                &noncanonical,
                "outputs/work/tasks/example/index.json"
            )
            .unwrap_err()
            .reason_code,
            "noncanonical_json_contract"
        );
        let mut mismatched = index.clone();
        mismatched["tasks"][0]["path"] = json!("tasks/TASK-002.json");
        let mismatched_raw = render_task(&mismatched, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_task_index(
                &mismatched,
                &mismatched_raw,
                "outputs/work/tasks/example/index.json"
            )
            .unwrap_err()
            .reason_code,
            "task_item_path_mismatch"
        );
        for wrong_path in [
            r"tasks\TASK-001.json",
            "tasks/task-001.json",
            "TASK-001.json",
            "../TASK-001.json",
        ] {
            let mut wrong = index.clone();
            wrong["tasks"][0]["path"] = json!(wrong_path);
            let wrong_raw = render_task(&wrong, TaskDocumentKind::Index).unwrap();
            assert_eq!(
                validate_task_index(&wrong, &wrong_raw, "outputs/work/tasks/example/index.json")
                    .unwrap_err()
                    .reason_code,
                "task_item_path_mismatch"
            );
        }
        let mut invalid_id = index.clone();
        invalid_id["tasks"][0]["id"] = json!("task-001");
        let invalid_raw = render_task(&invalid_id, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_task_index(
                &invalid_id,
                &invalid_raw,
                "outputs/work/tasks/example/index.json"
            )
            .unwrap_err()
            .reason_code,
            "invalid_or_unsorted_id"
        );
        let mut unsorted = index.clone();
        let mut second = unsorted["tasks"][0].clone();
        second["id"] = json!("TASK-002");
        second["path"] = json!("tasks/TASK-002.json");
        unsorted["tasks"] = json!([second, unsorted["tasks"][0]]);
        let unsorted_raw = render_task(&unsorted, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_task_index(
                &unsorted,
                &unsorted_raw,
                "outputs/work/tasks/example/index.json"
            )
            .unwrap_err()
            .reason_code,
            "invalid_or_unsorted_id"
        );
        let mut custom = index.clone();
        custom["artifacts"]["task"] = json!("custom/example/index.json");
        let custom_raw = render_task(&custom, TaskDocumentKind::Index).unwrap();
        validate_task_index(&custom, &custom_raw, "custom/example/index.json").unwrap();
        for invalid_path in [
            "outputs/work/tasks/example/task.json",
            "outputs/work/tasks/other/index.json",
            "outputs/work/tasks/example/tasks/index.json",
        ] {
            let mut invalid = index.clone();
            invalid["artifacts"]["task"] = json!(invalid_path);
            let raw = render_task(&invalid, TaskDocumentKind::Index).unwrap();
            assert_eq!(
                validate_task_index(&invalid, &raw, invalid_path)
                    .unwrap_err()
                    .reason_code,
                "task_index_path_requirement_mismatch",
                "{invalid_path}"
            );
        }
        let mut revised = index;
        revised["spec_id"] = json!("TASK-SPEC-003");
        revised["readiness"]["spec_id"] = json!("TASK-SPEC-003");
        revised["changes"] = json!([
            {"id":"TASK-CHANGE-001","spec_id":"TASK-SPEC-002","date":"2026-09-15",
                "reason":"First revision","affected_ids":["TASK-001"],"edits":[
                    {"artifact":"task_index","operation":"replace","path":"/summary",
                        "before":"Example tasks.","after":"First summary."}]},
            {"id":"TASK-CHANGE-002","spec_id":"TASK-SPEC-003","date":"2026-09-16",
                "reason":"Second revision","affected_ids":["TASK-001"],"edits":[
                    {"artifact":"task_index","operation":"replace","path":"/summary",
                        "before":"First summary.","after":"Second summary."}]}
        ]);
        let revised_raw = render_task(&revised, TaskDocumentKind::Index).unwrap();
        validate_task_index(
            &revised,
            &revised_raw,
            "outputs/work/tasks/example/index.json",
        )
        .unwrap();
        revised["changes"][1]["spec_id"] = json!("TASK-SPEC-002");
        let invalid_raw = render_task(&revised, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_task_index(
                &revised,
                &invalid_raw,
                "outputs/work/tasks/example/index.json"
            )
            .unwrap_err()
            .reason_code,
            "change_spec_mismatch"
        );
    }
}
