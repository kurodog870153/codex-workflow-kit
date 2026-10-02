//! Pure Task-owned acceptance, dependency and execution boundary rules.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::canonical::portable_path_identity;
use crate::task::{TaskIssue, resolve_dependencies};

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details,
    }
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect()
}

fn defined_ids(contract: &Value, group: &str) -> BTreeSet<String> {
    contract[group]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect()
}

pub fn validate_task_file_state(
    contract: &Value,
    task_order: &[String],
    initial_existence: &BTreeMap<String, bool>,
) -> Result<(), TaskIssue> {
    let mut existence = initial_existence.clone();
    let tasks = contract["tasks"].as_array().expect("validated TASKs");
    for task_id in task_order {
        let task = tasks
            .iter()
            .find(|task| task["id"] == task_id.as_str())
            .expect("ordered TASK");
        for file in task["files"].as_array().into_iter().flatten() {
            match file["action"].as_str() {
                Some("create") => {
                    let path = file["path"].as_str().expect("validated path");
                    let identity = portable_path_identity(path);
                    if existence.get(&identity).copied().unwrap_or(false) {
                        return Err(issue(
                            "file_create_target_exists",
                            "A create target already exists.",
                            json!({"path":path}),
                        ));
                    }
                    existence.insert(identity, true);
                }
                Some("modify") => {
                    let path = file["path"].as_str().expect("validated path");
                    if !existence
                        .get(&portable_path_identity(path))
                        .copied()
                        .unwrap_or(false)
                    {
                        return Err(issue(
                            "file_modify_target_missing",
                            "A modify target does not exist.",
                            json!({"path":path}),
                        ));
                    }
                }
                Some("move") => {
                    let source =
                        portable_path_identity(file["source"].as_str().expect("validated source"));
                    let destination = portable_path_identity(
                        file["destination"].as_str().expect("validated destination"),
                    );
                    if !existence.get(&source).copied().unwrap_or(false)
                        || existence.get(&destination).copied().unwrap_or(false)
                    {
                        return Err(issue(
                            "invalid_file_move_state",
                            "A file move source or destination state is invalid.",
                            json!({}),
                        ));
                    }
                    existence.insert(source, false);
                    existence.insert(destination, true);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Validate a structurally checked Task projection and its owned responsibilities.
/// Source bytes, filesystem paths and current instruction contents belong to Application.
pub fn validate_task_semantics(contract: &Value) -> Result<Value, TaskIssue> {
    let tasks = contract["tasks"]
        .as_array()
        .filter(|tasks| !tasks.is_empty())
        .ok_or_else(|| issue("invalid_item_array", "tasks must be non-empty.", json!({})))?;
    let ids: Vec<String> = tasks
        .iter()
        .filter_map(|task| task["id"].as_str().map(str::to_owned))
        .collect();
    if ids.len() != tasks.len() || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(issue(
            "invalid_or_unsorted_id",
            "Invalid or duplicate TASK ID.",
            json!({}),
        ));
    }
    let mut dependencies = BTreeMap::new();
    let main_acceptance = defined_ids(contract, "acceptance_criteria");
    let mut coverage = BTreeSet::new();
    let selected_skills: BTreeMap<&str, &Value> = contract["skill_selection"]["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|skill| Some((skill["id"].as_str()?, skill)))
        .collect();
    let mut task_skill_ids = serde_json::Map::new();
    let mut task_instruction_hashes = serde_json::Map::new();
    let mut has_commands = false;
    for task in tasks {
        let task_id = task["id"].as_str().expect("checked TASK id");
        if let Some(skill_id) = task["skill_id"].as_str() {
            let selected = selected_skills.get(skill_id).ok_or_else(|| {
                issue(
                    "task_skill_not_selected",
                    "A TASK skill must be independently selected in Task planning.",
                    json!({"task_id": task_id, "skill_id": skill_id}),
                )
            })?;
            if selected["mode_support"]["task"] == "unsupported" {
                return Err(issue(
                    "task_skill_mode_unsupported",
                    "A TASK skill must support Task mode.",
                    json!({"task_id": task_id, "skill_id": skill_id}),
                ));
            }
        }
        task_skill_ids.insert(task_id.into(), task["skill_id"].clone());
        let references = string_set(&task["instruction_selection"]["references"]);
        if !references.contains("task.general.task-records") {
            return Err(issue(
                "task_records_reference_required",
                "Formal TASKs require task.general.task-records.",
                json!({}),
            ));
        }
        task_instruction_hashes.insert(
            task_id.into(),
            task["instruction_selection"]["instructions_sha256"].clone(),
        );
        let direct: Vec<String> = task
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|dependency| dependency.as_str().map(str::to_owned))
            .collect();
        if direct
            .iter()
            .any(|dependency| dependency == task_id || !ids.contains(dependency))
        {
            return Err(issue(
                "invalid_task_dependency",
                "A TASK dependency is invalid.",
                json!({}),
            ));
        }
        dependencies.insert(task_id.into(), direct);
        let responsible = string_set(&task["traceability"]["acceptance_ids"]);
        if !responsible.is_subset(&main_acceptance) {
            return Err(issue(
                "invalid_reference",
                "Main acceptance responsibilities must exist in the Task collection.",
                json!({"task_id":task_id}),
            ));
        }
        coverage.extend(responsible.iter().cloned());
        let technical = defined_ids(task, "acceptance_criteria");
        if technical.is_empty() {
            return Err(issue(
                "missing_task_acceptance",
                "Every Task needs technical acceptance definitions.",
                json!({"task_id":task_id}),
            ));
        }
        let allowed = responsible
            .union(&technical)
            .cloned()
            .collect::<BTreeSet<_>>();
        let covered = task["validations"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|row| string_set(&row["acceptance_ids"]))
            .collect::<BTreeSet<_>>();
        if !covered.is_subset(&allowed) {
            return Err(issue(
                "invalid_reference",
                "Validation references must belong to the responsible Task's main or technical acceptance.",
                json!({"task_id":task_id}),
            ));
        }
        if !allowed.is_subset(&covered) {
            return Err(issue(
                "acceptance_without_validation",
                "Every responsible main and technical acceptance needs a final VAL in that Task.",
                json!({"task_id":task_id}),
            ));
        }
        if task.get("commands").is_some() {
            has_commands = true;
        }
        for operation in task["operations"].as_array().into_iter().flatten() {
            if operation["kind"] == "external_state"
                && !references.contains("task.general.external-operations")
            {
                return Err(issue(
                    "external_operations_reference_required",
                    "External operations require the external-operations reference.",
                    json!({}),
                ));
            }
        }
    }
    let order = resolve_dependencies(&ids, &dependencies)?;
    if contract["spec_id"] == "TASK-SPEC-001" {
        let positions: BTreeMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(position, id)| (id, position))
            .collect();
        if dependencies.iter().any(|(task, direct)| {
            direct
                .iter()
                .any(|dependency| positions[dependency] >= positions[task])
        }) {
            return Err(issue(
                "initial_task_order_mismatch",
                "Initial TASK IDs must follow dependency order.",
                json!({}),
            ));
        }
    }
    if coverage != main_acceptance {
        return Err(issue(
            "incomplete_task_acceptance_coverage",
            "Every main acceptance must have responsible Tasks.",
            json!({"missing_ids":main_acceptance.difference(&coverage).collect::<Vec<_>>() }),
        ));
    }
    if has_commands != contract.get("execution_defaults").is_some() {
        return Err(issue(
            "execution_defaults_mismatch",
            "execution_defaults must exist exactly when commands exist.",
            json!({}),
        ));
    }
    Ok(
        json!({"task_ids": ids, "task_order": order.order, "task_skill_ids": task_skill_ids, "task_instruction_hashes": task_instruction_hashes}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_acceptance_requires_validation_coverage_and_instruction_reference() {
        let mut task = json!({"requirement_id":"example", "spec_id":"TASK-SPEC-001",
            "skill_selection":{"skills":[]},"acceptance_criteria":[{"id":"ACCEPTANCE-001","criterion":"Result verified."}],
            "tasks":[{"id":"TASK-001","skill_id":null,"instruction_selection":{"references":["task.general.task-records"],"instructions_sha256":"a".repeat(64)},
                "traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001","criterion":"Technical result verified."}],
                "validations":[{"acceptance_ids":["ACCEPTANCE-001","TASK-001-ACCEPTANCE-001"]}]}]});
        assert_eq!(
            validate_task_semantics(&task).unwrap()["task_order"],
            json!(["TASK-001"])
        );
        let mut uncovered = task.clone();
        uncovered["tasks"][0]["traceability"]["acceptance_ids"] = json!([]);
        uncovered["tasks"][0]["validations"][0]["acceptance_ids"] =
            json!(["TASK-001-ACCEPTANCE-001"]);
        assert_eq!(
            validate_task_semantics(&uncovered).unwrap_err().reason_code,
            "incomplete_task_acceptance_coverage"
        );
        for missing in ["ACCEPTANCE-001", "TASK-001-ACCEPTANCE-001"] {
            let mut no_val = task.clone();
            no_val["tasks"][0]["validations"][0]["acceptance_ids"]
                .as_array_mut()
                .unwrap()
                .retain(|id| id != missing);
            assert_eq!(
                validate_task_semantics(&no_val).unwrap_err().reason_code,
                "acceptance_without_validation",
                "{missing}"
            );
        }
        let mut unknown_skill = task.clone();
        unknown_skill["tasks"][0]["skill_id"] = json!("missing");
        let error = validate_task_semantics(&unknown_skill).unwrap_err();
        assert_eq!(error.reason_code, "task_skill_not_selected");
        assert_eq!(error.details["task_id"], "TASK-001");
        assert_eq!(error.details["skill_id"], "missing");
        task["tasks"][0]["validations"][0]["acceptance_ids"] = json!([]);
        assert_eq!(
            validate_task_semantics(&task).unwrap_err().reason_code,
            "acceptance_without_validation"
        );
    }

    #[test]
    fn file_state_follows_dependency_order_and_portable_aliases() {
        let contract = json!({"tasks":[
            {"id":"TASK-001","files":[{"action":"create","path":"Café.txt"}]},
            {"id":"TASK-002","files":[{"action":"modify","path":"cafe\u{301}.txt"},
                {"action":"move","source":"Café.txt","destination":"done.txt"}]}
        ]});
        let order = ["TASK-001".to_owned(), "TASK-002".to_owned()];
        assert!(validate_task_file_state(&contract, &order, &BTreeMap::new()).is_ok());
        let reverse = ["TASK-002".to_owned(), "TASK-001".to_owned()];
        assert_eq!(
            validate_task_file_state(&contract, &reverse, &BTreeMap::new())
                .unwrap_err()
                .reason_code,
            "file_modify_target_missing"
        );
    }
}
