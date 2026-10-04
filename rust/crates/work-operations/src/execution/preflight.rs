//! Pure preflight checks after formal TASK and execution index validation.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::execution::ExecutionIssue;
use crate::protocol::TASK_ID_PREFIX;

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn check_task_eligibility(
    collection: &Value,
    index: &Value,
    task_id: &str,
    allowed_lock: Option<&Value>,
    eligible_statuses: &[&str],
) -> Result<Value, ExecutionIssue> {
    let tasks = collection["tasks"].as_array().ok_or_else(|| {
        issue(
            "invalid_task_collection",
            "TASK collection tasks must be an array.",
            json!({}),
        )
    })?;
    let task = tasks
        .iter()
        .find(|task| task["id"] == task_id)
        .ok_or_else(|| {
            issue(
                "execute_preflight_task_not_found",
                "The requested TASK ID does not exist in the formal TASK document.",
                json!({"task_id":task_id}),
            )
        })?;
    if let Some(lock) = index.get("lock") {
        if Some(lock) != allowed_lock {
            return Err(issue(
                "execute_preflight_lock_present",
                "The execution index already contains a lock.",
                json!({"lock":lock}),
            ));
        }
    }
    let rows = index["tasks"].as_array().ok_or_else(|| {
        issue(
            "invalid_execution_index_tasks",
            "Execution index tasks must be an array.",
            json!({}),
        )
    })?;
    let by_id: HashMap<_, _> = rows
        .iter()
        .filter_map(|row| row["id"].as_str().map(|id| (id, row)))
        .collect();
    let row = by_id.get(task_id).ok_or_else(|| {
        issue(
            "execute_preflight_task_not_found",
            "The requested TASK ID does not exist in the formal TASK document.",
            json!({"task_id":task_id}),
        )
    })?;
    let status = row["status"].as_str().unwrap_or("");
    if !eligible_statuses.contains(&status) {
        return Err(issue(
            "execute_preflight_task_not_eligible",
            "The requested TASK status is not eligible for a new Attempt.",
            json!({"task_id":task_id,"status":status}),
        ));
    }
    let incomplete: Vec<_> = task["dependencies"].as_array().into_iter().flatten().filter_map(|dependency| dependency.as_str())
        .filter(|dependency| by_id.get(*dependency).is_none_or(|row| row["status"] != "completed"))
        .map(|dependency| json!({"task_id":dependency,"status":by_id.get(dependency).map_or(&Value::Null, |row| &row["status"])})).collect();
    if !incomplete.is_empty() {
        return Err(issue(
            "execute_preflight_dependency_incomplete",
            "A direct TASK dependency is not completed.",
            json!({"task_id":task_id,"dependencies":incomplete}),
        ));
    }
    Ok(task.clone())
}

pub fn check_confirmed_inputs(task: &Value, confirmed: &[String]) -> Result<Value, ExecutionIssue> {
    let mut seen = HashSet::new();
    if confirmed.iter().any(|id| !seen.insert(id)) {
        return Err(issue(
            "execute_preflight_duplicate_confirmed_input",
            "Confirmed input IDs must be unique.",
            json!({}),
        ));
    }
    if confirmed.iter().any(|id| {
        let Some((task_id, input_id)) = id.split_once('/') else {
            return true;
        };
        !valid_id(task_id, TASK_ID_PREFIX) || !valid_id(input_id, "INPUT-")
    }) {
        return Err(issue(
            "execute_preflight_invalid_confirmed_input",
            "A confirmed input must use TASK-nnn/INPUT-nnn format.",
            json!({"confirmed_inputs":confirmed}),
        ));
    }
    let inputs = task["inputs"].as_array();
    let manual: HashSet<_> = inputs
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == "user_provided" || row["kind"] == "external")
        .filter_map(|row| {
            row["id"]
                .as_str()
                .map(|id| format!("{}/{}", task["id"].as_str().unwrap_or(""), id))
        })
        .collect();
    let mut unknown: Vec<_> = confirmed
        .iter()
        .filter(|id| !manual.contains(*id))
        .cloned()
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(issue(
            "execute_preflight_unknown_confirmed_input",
            "A confirmed input is not a manual input of the requested TASK.",
            json!({"confirmed_inputs":unknown}),
        ));
    }
    let mut missing: Vec<_> = manual
        .iter()
        .filter(|id| !seen.contains(id))
        .cloned()
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(issue(
            "execute_preflight_input_confirmation_required",
            "A user-provided or external input requires explicit confirmation.",
            json!({"required_confirmations":missing}),
        ));
    }
    Ok(json!(
        inputs
            .into_iter()
            .flatten()
            .map(|row| json!({"id":row["id"],"kind":row["kind"],"status":"ready"}))
            .collect::<Vec<_>>()
    ))
}

pub fn check_input_sources(
    collection: &Value,
    task: &Value,
    present_paths: &HashSet<String>,
) -> Result<Value, ExecutionIssue> {
    let tasks: HashMap<_, _> = collection["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|task| task["id"].as_str().map(|id| (id, task)))
        .collect();
    let mut readiness = Vec::new();
    for input in task["inputs"].as_array().into_iter().flatten() {
        let mut row = json!({"id":input["id"],"kind":input["kind"],"status":"ready"});
        let kind = input["kind"].as_str().unwrap_or("");
        let source = if kind == "project_state" {
            input["source"].as_str()
        } else if kind == "task_output" {
            let value = input["source"].as_str().unwrap_or("");
            let (task_id, file_id) = value.split_once('/').unwrap_or(("", ""));
            tasks
                .get(task_id)
                .and_then(|source_task| source_task["files"].as_array())
                .and_then(|files| files.iter().find(|file| file["id"] == file_id))
                .and_then(|file| {
                    if file["action"] == "move" {
                        file["destination"].as_str()
                    } else {
                        file["path"].as_str()
                    }
                })
        } else {
            None
        };
        if let Some(path) = source {
            if !present_paths.contains(path) {
                let (code, message) = if kind == "project_state" {
                    (
                        "execute_preflight_project_state_missing",
                        "A project_state input does not exist.",
                    )
                } else {
                    (
                        "execute_preflight_task_output_missing",
                        "A completed dependency TASK output does not exist.",
                    )
                };
                return Err(issue(
                    code,
                    message,
                    json!({"input_id":input["id"],"path":path}),
                ));
            }
            row["resolved_source"] = json!(path);
        }
        readiness.push(row);
    }
    Ok(json!(readiness))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub identity: String,
    pub exists: bool,
}

pub fn check_file_lifecycle(
    task: &Value,
    state: &HashMap<String, FileState>,
) -> Result<Value, ExecutionIssue> {
    let mut existence: HashMap<&str, bool> = HashMap::new();
    for entry in state.values() {
        *existence.entry(entry.identity.as_str()).or_default() |= entry.exists;
    }
    let mut readiness = Vec::new();
    for file in task["files"].as_array().into_iter().flatten() {
        let action = file["action"].as_str().unwrap_or("");
        let mut row = json!({"id":file["id"],"action":action,"status":"ready"});
        if matches!(action, "create" | "modify") {
            let path = file["path"].as_str().unwrap_or("");
            let identity = state
                .get(path)
                .map(|entry| entry.identity.as_str())
                .unwrap_or("");
            let present = *existence.get(identity).unwrap_or(&false);
            if action == "create" && present {
                return Err(issue(
                    "execute_preflight_create_target_exists",
                    "A create target already exists.",
                    json!({"file_id":file["id"],"path":path}),
                ));
            }
            if action == "modify" && !present {
                return Err(issue(
                    "execute_preflight_modify_target_missing",
                    "A modify target does not exist.",
                    json!({"file_id":file["id"],"path":path}),
                ));
            }
            if action == "create" {
                existence.insert(identity, true);
            }
            row["path"] = json!(path);
        } else if action == "move" {
            let source = file["source"].as_str().unwrap_or("");
            let destination = file["destination"].as_str().unwrap_or("");
            let source_id = state
                .get(source)
                .map(|entry| entry.identity.as_str())
                .unwrap_or("");
            let destination_id = state
                .get(destination)
                .map(|entry| entry.identity.as_str())
                .unwrap_or("");
            if !existence.get(source_id).copied().unwrap_or(false) {
                return Err(issue(
                    "execute_preflight_move_source_missing",
                    "A move source does not exist.",
                    json!({"file_id":file["id"],"path":source}),
                ));
            }
            if existence.get(destination_id).copied().unwrap_or(false) {
                return Err(issue(
                    "execute_preflight_move_destination_exists",
                    "A move destination already exists.",
                    json!({"file_id":file["id"],"path":destination}),
                ));
            }
            existence.insert(source_id, false);
            existence.insert(destination_id, true);
            row["source"] = json!(source);
            row["destination"] = json!(destination);
        }
        readiness.push(row);
    }
    Ok(json!(readiness))
}

fn valid_id(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_and_confirmations_follow_current_contract_order() {
        let collection = json!({"tasks":[{"id":"TASK-001","dependencies":[],"inputs":[]},
            {"id":"TASK-002","dependencies":["TASK-001"],"inputs":[{"id":"INPUT-001","kind":"external"}]}]});
        let index = json!({"tasks":[{"id":"TASK-001","status":"pending"},{"id":"TASK-002","status":"pending"}]});
        assert_eq!(
            check_task_eligibility(
                &collection,
                &index,
                "TASK-002",
                None,
                &["pending", "pending_retry"]
            )
            .unwrap_err()
            .reason_code,
            "execute_preflight_dependency_incomplete"
        );
        let mut completed = index.clone();
        completed["tasks"][0]["status"] = json!("completed");
        let task = check_task_eligibility(
            &collection,
            &completed,
            "TASK-002",
            None,
            &["pending", "pending_retry"],
        )
        .unwrap();
        assert_eq!(
            check_confirmed_inputs(&task, &[]).unwrap_err().reason_code,
            "execute_preflight_input_confirmation_required"
        );
        assert_eq!(
            check_confirmed_inputs(&task, &["TASK-002/INPUT-001".into()]).unwrap(),
            json!([{"id":"INPUT-001","kind":"external","status":"ready"}])
        );
    }

    #[test]
    fn input_and_file_lifecycle_follow_formal_order() {
        let collection = json!({"tasks":[{"id":"TASK-001","files":[{"id":"FILE-001","action":"create","path":"out.txt"}]},
            {"id":"TASK-002","inputs":[{"id":"INPUT-001","kind":"task_output","source":"TASK-001/FILE-001"}],
            "files":[{"id":"FILE-001","action":"create","path":"new.txt"},{"id":"FILE-002","action":"move","source":"new.txt","destination":"done.txt"}]}]});
        let task = &collection["tasks"][1];
        assert_eq!(
            check_input_sources(&collection, task, &HashSet::new())
                .unwrap_err()
                .reason_code,
            "execute_preflight_task_output_missing"
        );
        assert_eq!(
            check_input_sources(&collection, task, &HashSet::from(["out.txt".into()])).unwrap(),
            json!([{"id":"INPUT-001","kind":"task_output","status":"ready","resolved_source":"out.txt"}])
        );
        let states = HashMap::from([
            (
                "new.txt".into(),
                FileState {
                    identity: "new.txt".into(),
                    exists: false,
                },
            ),
            (
                "done.txt".into(),
                FileState {
                    identity: "done.txt".into(),
                    exists: false,
                },
            ),
        ]);
        assert_eq!(
            check_file_lifecycle(task, &states).unwrap()[1]["destination"],
            "done.txt"
        );
    }
}
