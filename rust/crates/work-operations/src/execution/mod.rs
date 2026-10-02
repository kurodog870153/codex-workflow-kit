//! Pure execution identity, sequencing, and state transitions.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

#[cfg(test)]
use crate::derivation::identity::next_attempt_id;
use crate::derivation::identity::next_record_id;
use crate::protocol::BLOCKING_STOPPED_TYPES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn derive_overall_status(statuses: &[&str]) -> &'static str {
    let active: Vec<_> = statuses
        .iter()
        .copied()
        .filter(|status| *status != "cancelled")
        .collect();
    if active.is_empty() {
        "cancelled"
    } else if active.iter().all(|status| *status == "completed") {
        "completed"
    } else if active.contains(&"in_progress") {
        "in_progress"
    } else if active
        .iter()
        .filter(|status| **status != "completed")
        .all(|status| *status == "blocked")
    {
        "blocked"
    } else {
        "pending"
    }
}

pub fn closed_task_status(status: &str, final_type: Option<&str>) -> &'static str {
    if status == "completed" {
        "completed"
    } else if status == "blocked"
        || final_type.is_some_and(|value| BLOCKING_STOPPED_TYPES.contains(&value))
    {
        "blocked"
    } else {
        "pending_retry"
    }
}

pub fn build_execution_lock(task_id: &str, attempt_id: &str, instructions_sha256: &str) -> Value {
    json!({"kind":"execution","task_id":task_id,"attempt_id":attempt_id,
        "execute_instructions_sha256":instructions_sha256})
}

pub fn start_index(
    index: &Value,
    task_id: &str,
    attempt_id: &str,
) -> Result<Value, ExecutionIssue> {
    let mut result = index.clone();
    let rows = result["tasks"].as_array_mut().ok_or_else(|| {
        issue(
            "invalid_execution_index_tasks",
            "Execution index tasks must be an array.",
            json!({}),
        )
    })?;
    let row = rows
        .iter_mut()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            issue(
                "execution_task_not_found",
                "The TASK is not in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    row["status"] = json!("in_progress");
    row["latest_attempt"] = json!(attempt_id);
    if let Some(object) = row.as_object_mut() {
        object.remove("status_reason");
    }
    let statuses: Vec<_> = rows
        .iter()
        .filter_map(|row| row["status"].as_str())
        .collect();
    result["overall_status"] = json!(derive_overall_status(&statuses));
    acceptance::invalidate(&result, &[task_id.to_owned()])
}

pub fn close_index(
    index: &Value,
    task_id: &str,
    attempt_id: &str,
    task_status: &str,
) -> Result<Value, ExecutionIssue> {
    let mut result = index.clone();
    let rows = result["tasks"].as_array_mut().ok_or_else(|| {
        issue(
            "invalid_execution_index_tasks",
            "Execution index tasks must be an array.",
            json!({}),
        )
    })?;
    let row = rows
        .iter_mut()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            issue(
                "execution_task_not_found",
                "The TASK is not in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    row["status"] = json!(task_status);
    if let Some(object) = row.as_object_mut() {
        if task_status == "completed" {
            object.remove("status_reason");
        } else {
            object.insert(
                "status_reason".into(),
                json!({"kind":"attempt","ref":attempt_id}),
            );
        }
    }
    let statuses: Vec<_> = rows
        .iter()
        .filter_map(|row| row["status"].as_str())
        .collect();
    result["overall_status"] = json!(derive_overall_status(&statuses));
    if let Some(object) = result.as_object_mut() {
        object.remove("lock");
    }
    Ok(result)
}

pub fn formal_record_kind(
    task: &Value,
    base_record_id: &str,
) -> Result<&'static str, ExecutionIssue> {
    let (field, kind) = if base_record_id.starts_with("CMD-") {
        ("commands", "command")
    } else if base_record_id.starts_with("OP-") {
        ("operations", "operation")
    } else if base_record_id.starts_with("VAL-") {
        ("validations", "validation")
    } else {
        return Err(issue(
            "record_begin_invalid_base_record_id",
            "record_id must be a base CMD-, OP-, or VAL- identifier.",
            json!({"record_id":base_record_id}),
        ));
    };
    if !task[field]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["id"] == base_record_id)
    {
        return Err(issue(
            "record_begin_record_not_found",
            "The requested record ID is not defined by the target TASK.",
            json!({"record_id":base_record_id}),
        ));
    }
    Ok(kind)
}

pub fn record_begin_candidate(
    task: &Value,
    attempt: &Value,
    index: &Value,
    task_id: &str,
    base_record_id: &str,
    authorization_evidence: Option<&str>,
) -> Result<(Value, String, &'static str), ExecutionIssue> {
    let effective = crate::execution::authorization::effective_task(task, attempt)?;
    let kind = formal_record_kind(&effective, base_record_id)?;
    let expected_lock = build_execution_lock(
        task_id,
        attempt["attempt_id"].as_str().unwrap_or(""),
        attempt["execute_instructions_sha256"]
            .as_str()
            .unwrap_or(""),
    );
    let lock = index.get("lock");
    if lock.is_none_or(|lock| {
        expected_lock
            .as_object()
            .is_none_or(|fields| fields.iter().any(|(field, value)| lock[field] != *value))
    }) {
        return Err(issue(
            "record_begin_lock_mismatch",
            "The execution lock does not match the active Attempt.",
            json!({"expected":expected_lock,"actual":lock}),
        ));
    }
    if lock.is_some_and(|lock| lock.get("record_id").is_some()) {
        return Err(issue(
            "record_begin_record_already_reserved",
            "The execution lock already reserves a record.",
            json!({"record_id":index["lock"]["record_id"]}),
        ));
    }
    let record_id = next_record_id(base_record_id, attempt)?;
    crate::execution::authorization::require_record_scope(attempt, base_record_id)?;
    crate::execution::authorization::require_retry_evidence(
        attempt,
        &record_id,
        authorization_evidence,
    )?;
    let mut candidate = index.clone();
    candidate["lock"]["record_id"] = json!(record_id);
    if let Some(evidence) = authorization_evidence {
        candidate["lock"]["retry_authorization_evidence"] = json!(evidence);
    }
    Ok((candidate, record_id, kind))
}

pub fn finished_record_index(index: &Value) -> Result<Value, ExecutionIssue> {
    let mut result = index.clone();
    let lock = result
        .get_mut("lock")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            issue(
                "invalid_execution_lock",
                "An execution lock is required to finish a record.",
                json!({}),
            )
        })?;
    for field in [
        "record_id",
        "command_correction",
        "retry_authorization_evidence",
    ] {
        lock.remove(field);
    }
    Ok(result)
}

pub fn attempt_close_request(attempt: &Value) -> Value {
    let mut request = json!({"schema":"work-attempt-close-request/v1","status":attempt["status"]});
    if attempt["status"] != "completed" {
        request["final_type"] = attempt["final_type"].clone();
        request["reason"] = attempt["reason"].clone();
        request["authorization_evidence"] = attempt["closing_authorization_evidence"].clone();
    }
    request
}

pub fn affected_task_ids(
    index: &Value,
    collection: &Value,
    task_id: &str,
    invalidates_completion: bool,
) -> Result<Vec<String>, ExecutionIssue> {
    if !invalidates_completion {
        return Ok(vec![]);
    }
    let rows = index["tasks"].as_array().ok_or_else(|| {
        issue(
            "invalid_execution_index_tasks",
            "Execution index tasks must be an array.",
            json!({}),
        )
    })?;
    if !rows
        .iter()
        .any(|row| row["id"] == task_id && row["status"] == "completed")
    {
        return Err(issue(
            "correction_create_target_not_completed",
            "Completion invalidation requires a completed target TASK.",
            json!({}),
        ));
    }
    let mut affected = HashSet::from([task_id.to_owned()]);
    let tasks = collection["tasks"].as_array().ok_or_else(|| {
        issue(
            "invalid_task_collection",
            "TASK collection tasks must be an array.",
            json!({}),
        )
    })?;
    loop {
        let mut changed = false;
        for task in tasks {
            let Some(id) = task["id"].as_str() else {
                continue;
            };
            if !affected.contains(id)
                && task["dependencies"].as_array().is_some_and(|deps| {
                    deps.iter()
                        .any(|dep| dep.as_str().is_some_and(|dep| affected.contains(dep)))
                })
            {
                affected.insert(id.to_owned());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(rows
        .iter()
        .filter(|row| row["status"] == "completed")
        .filter_map(|row| row["id"].as_str())
        .filter(|id| affected.contains(*id))
        .map(str::to_owned)
        .collect())
}

pub fn corrected_index(
    index: &Value,
    task_id: &str,
    correction_id: &str,
    affected_ids: &[String],
) -> Result<Value, ExecutionIssue> {
    let mut result = index.clone();
    let rows = result["tasks"].as_array_mut().ok_or_else(|| {
        issue(
            "invalid_execution_index_tasks",
            "Execution index tasks must be an array.",
            json!({}),
        )
    })?;
    let target = rows
        .iter_mut()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            issue(
                "execution_task_not_found",
                "The TASK is not in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    target["latest_correction"] = json!(correction_id);
    let affected: HashSet<_> = affected_ids.iter().map(String::as_str).collect();
    for row in rows.iter_mut() {
        if row["id"].as_str().is_some_and(|id| affected.contains(id)) {
            row["status"] = json!("pending_retry");
            row["status_reason"] = json!({"kind":"correction","ref":correction_id});
        }
    }
    let statuses: Vec<_> = rows
        .iter()
        .filter_map(|row| row["status"].as_str())
        .collect();
    result["overall_status"] = json!(derive_overall_status(&statuses));
    if let Some(object) = result.as_object_mut() {
        object.remove("lock");
    }
    acceptance::invalidate(&result, affected_ids)
}

pub fn validate_authorization_scope(
    manifest: &Value,
    task: &Value,
    defaults: &Value,
) -> Result<(), ExecutionIssue> {
    if manifest["task_id"] != task["id"] {
        return Err(issue(
            "attempt_authorization_task_mismatch",
            "The authorization must target the selected TASK.",
            json!({}),
        ));
    }
    for (field, formal_field, external_only) in [
        ("commands", "commands", false),
        ("validations", "validations", false),
        ("external_operations", "operations", true),
    ] {
        let formal: HashMap<_, _> = task[formal_field]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| !external_only || row["kind"] == "external_state")
            .filter_map(|row| row["id"].as_str().map(|id| (id, row)))
            .collect();
        for entry in manifest[field].as_array().into_iter().flatten() {
            let id = entry["id"].as_str().unwrap_or("");
            if formal.get(id).is_none_or(|row| **row != *entry) {
                return Err(issue(
                    "attempt_authorization_scope_expansion",
                    "Authorization entries must exactly match the current TASK.",
                    json!({"field":field,"id":id}),
                ));
            }
        }
    }
    let files: HashSet<_> = task["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| ["path", "source", "destination"].map(|field| &row[field]))
        .filter_map(Value::as_str)
        .collect();
    if manifest["modifiable_files"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|path| path.as_str().is_none_or(|path| !files.contains(path)))
    {
        return Err(issue(
            "attempt_authorization_scope_expansion",
            "Modifiable files must be declared by the current TASK.",
            json!({}),
        ));
    }
    let mut directories = Vec::new();
    for command in manifest["commands"].as_array().into_iter().flatten() {
        let execution = command
            .get("execution")
            .filter(|value| !value.is_null())
            .unwrap_or(defaults);
        let directory = &execution["working_directory"];
        if !directories.contains(directory) {
            directories.push(directory.clone());
        }
    }
    if manifest["working_directories"] != json!(directories) {
        return Err(issue(
            "attempt_authorization_working_directories",
            "Working directories must exactly match the authorized commands.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn validate_preflight_identity(
    index: &Value,
    task: &Value,
    validation: &Value,
) -> Result<(), ExecutionIssue> {
    let identity = [
        ("requirement_id", "requirement_id"),
        ("task_spec_id", "spec_id"),
        ("task_collection_sha256", "task_collection_sha256"),
        ("task_index_sha256", "task_index_sha256"),
        ("task_instructions_sha256", "instructions_sha256"),
        ("hierarchy_selection_sha256", "hierarchy_selection_sha256"),
    ];
    let mut expected = serde_json::Map::new();
    let mut observed = serde_json::Map::new();
    for (field, source) in identity {
        let value = if field == "requirement_id" || field == "task_spec_id" {
            &task[source]
        } else {
            &validation[source]
        };
        expected.insert(field.into(), value.clone());
        observed.insert(field.into(), index[field].clone());
    }
    if expected != observed {
        return Err(issue(
            "execute_preflight_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            json!({"expected":expected,"observed":observed}),
        ));
    }
    let expected_ids = &validation["task_ids"];
    let observed_ids: Vec<_> = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| row["id"].clone())
        .collect();
    if *expected_ids != json!(observed_ids) {
        return Err(issue(
            "execute_preflight_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK document.",
            json!({"expected":expected_ids,"observed":observed_ids}),
        ));
    }
    acceptance::require_collection(index, task)?;
    let mut mismatches = serde_json::Map::new();
    for row in index["tasks"].as_array().into_iter().flatten() {
        let Some(id) = row["id"].as_str() else {
            continue;
        };
        if row["instructions_sha256"] != validation["task_instructions_sha256"][id] {
            mismatches.insert(id.into(), json!({"expected":validation["task_instructions_sha256"][id],"observed":row["instructions_sha256"]}));
        }
    }
    if !mismatches.is_empty() {
        return Err(issue(
            "execute_preflight_task_instructions_mismatch",
            "The execution index per-TASK instruction fingerprints are stale.",
            json!({"tasks":mismatches}),
        ));
    }
    Ok(())
}

pub fn validate_execution_identity(
    collection: &Value,
    validation: &Value,
    index: &Value,
    attempt: &Value,
    task_id: &str,
) -> Result<Value, ExecutionIssue> {
    let expected_index = json!({"requirement_id":collection["requirement_id"],
        "task_spec_id":collection["spec_id"],
        "task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_instructions_sha256":validation["instructions_sha256"],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"]});
    let mut actual_index = serde_json::Map::new();
    for field in expected_index.as_object().expect("fixed fields").keys() {
        actual_index.insert(field.clone(), index[field].clone());
    }
    if json!(actual_index) != expected_index {
        return Err(issue(
            "record_begin_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            json!({"expected":expected_index,"actual":actual_index}),
        ));
    }
    let observed_ids: Vec<_> = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| row["id"].clone())
        .collect();
    if json!(observed_ids) != validation["task_ids"] {
        return Err(issue(
            "record_begin_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK.",
            json!({}),
        ));
    }
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            issue(
                "record_begin_task_not_found",
                "The requested TASK is not present in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    if row["instructions_sha256"] != validation["task_instructions_sha256"][task_id] {
        return Err(issue(
            "record_begin_task_instructions_mismatch",
            "The target TASK instruction fingerprint is stale.",
            json!({}),
        ));
    }
    let expected_attempt = json!({"task_spec_id":collection["spec_id"],"task_id":task_id,
        "task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_item_sha256":validation["task_item_sha256"][task_id],
        "task_instructions_sha256":validation["task_instructions_sha256"][task_id],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"]});
    let mut actual_attempt = serde_json::Map::new();
    for field in expected_attempt.as_object().expect("fixed fields").keys() {
        actual_attempt.insert(field.clone(), attempt[field].clone());
    }
    if json!(actual_attempt) != expected_attempt {
        return Err(issue(
            "record_begin_attempt_identity_mismatch",
            "The active Attempt does not match the formal TASK identity.",
            json!({"expected":expected_attempt,"actual":actual_attempt}),
        ));
    }
    acceptance::require_collection(index, collection)?;
    acceptance::require_ids(
        &attempt["acceptance_results"],
        &row["acceptance_results"]
            .as_array()
            .expect("validated results")
            .iter()
            .map(|result| result["id"].as_str().expect("validated ID").to_owned())
            .collect(),
    )?;
    Ok(row.clone())
}

pub fn overall_operation_result(records: &[Value]) -> Option<Value> {
    let operations: Vec<_> = records
        .iter()
        .filter(|record| record["kind"] == "operation" && record["status"] != "skipped")
        .collect();
    if operations.is_empty() {
        return None;
    }
    let collect = |outcome| -> Vec<Value> {
        operations
            .iter()
            .filter(|record| record["outcome"] == outcome)
            .map(|record| record["id"].clone())
            .collect()
    };
    let effective = collect("success");
    let not_effective = collect("failure");
    let unknown = collect("unknown");
    let status = if !unknown.is_empty() {
        "uncertain_result"
    } else if !effective.is_empty() && !not_effective.is_empty() {
        "partial_success"
    } else if !not_effective.is_empty() {
        "failure"
    } else {
        "complete_success"
    };
    let mut result = json!({"status":status});
    for (field, values) in [
        ("effective", effective),
        ("not_effective", not_effective),
        ("unknown", unknown),
    ] {
        if !values.is_empty() {
            result[field] = json!(values);
        }
    }
    Some(result)
}

pub fn finish_attempt_candidate(
    attempt: &Value,
    request: &Value,
    record_id: &str,
    kind: &str,
    command_correction: Option<&Value>,
) -> Result<Value, ExecutionIssue> {
    let mut record = request["record"].clone();
    let object = record.as_object_mut().ok_or_else(|| {
        issue(
            "record_finish_invalid_record",
            "record must be a JSON object.",
            json!({}),
        )
    })?;
    let repeated: Vec<_> = ["correction", "id", "kind"]
        .iter()
        .filter(|field| object.contains_key(**field))
        .copied()
        .collect();
    if !repeated.is_empty() {
        return Err(issue(
            "record_finish_machine_fields",
            "Record identity and command correction are derived from the active lock.",
            json!({"fields":repeated}),
        ));
    }
    object.insert("id".into(), json!(record_id));
    object.insert("kind".into(), json!(kind));
    if let Some(correction) = command_correction {
        if kind != "command" {
            return Err(issue(
                "record_finish_invalid_command_correction_lock",
                "Only a reserved command can carry command correction data.",
                json!({}),
            ));
        }
        object.insert("correction".into(), correction.clone());
    }
    let mut candidate = attempt.clone();
    let records = candidate["records"].as_array_mut().ok_or_else(|| {
        issue(
            "attempt_invalid_records",
            "Attempt records must be an array.",
            json!({}),
        )
    })?;
    records.push(record);
    let overall = overall_operation_result(records);
    if let Some(modified_files) = request.get("modified_files").and_then(Value::as_array) {
        let files = candidate
            .as_object_mut()
            .expect("Attempt has records")
            .entry("modified_files")
            .or_insert_with(|| json!([]));
        let files = files.as_array_mut().ok_or_else(|| {
            issue(
                "record_finish_invalid_modified_files",
                "modified_files must be an array.",
                json!({}),
            )
        })?;
        for path in modified_files {
            if !files.contains(path) {
                files.push(path.clone());
            }
        }
    }
    if let Some(overall) = overall {
        candidate["overall_result"] = overall;
    } else {
        candidate
            .as_object_mut()
            .expect("Attempt has records")
            .remove("overall_result");
    }
    Ok(candidate)
}

pub fn validate_completed_coverage(task: &Value, attempt: &Value) -> Result<(), ExecutionIssue> {
    let mut outcomes: HashMap<&str, &str> = HashMap::new();
    for carried in attempt["carried_records"].as_array().into_iter().flatten() {
        if let Some(base) = carried["record_id"]
            .as_str()
            .map(|id| id.split('#').next().unwrap_or(id))
            .filter(|id| id.starts_with("VAL-"))
        {
            outcomes.insert(base, "passed");
        }
    }
    for record in attempt["records"].as_array().into_iter().flatten() {
        if record["kind"] == "validation" {
            if let Some(base) = record["id"]
                .as_str()
                .map(|id| id.split('#').next().unwrap_or(id))
            {
                outcomes.insert(
                    base,
                    if record["status"] == "skipped" {
                        "skipped"
                    } else {
                        record["outcome"].as_str().unwrap_or("")
                    },
                );
            }
        }
    }
    let required: Vec<_> = task["validations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str())
        .collect();
    let missing: Vec<_> = required
        .iter()
        .filter(|id| !outcomes.contains_key(**id))
        .copied()
        .collect();
    let failed: Vec<_> = required
        .iter()
        .filter(|id| {
            outcomes
                .get(**id)
                .is_some_and(|outcome| !matches!(*outcome, "passed" | "skipped"))
        })
        .copied()
        .collect();
    if !missing.is_empty() || !failed.is_empty() {
        return Err(issue(
            "attempt_close_incomplete_validations",
            "A completed Attempt requires every formal validation to pass or have approved skipped evidence.",
            json!({"missing":missing,"failed":failed}),
        ));
    }
    Ok(())
}

pub fn deviation_reconciliation_target(proposal: &Value) -> &'static str {
    let impact = &proposal["impact"];
    if [
        "requirement_changed",
        "scope_changed",
        "deliverables_changed",
        "acceptance_criteria_changed",
        "safety_boundary_changed",
        "external_side_effect_boundary_changed",
    ]
    .iter()
    .any(|field| impact[*field] == true)
    {
        "task_and_execution"
    } else {
        "task_only"
    }
}

pub fn validate_deviation_action(
    proposal: &Value,
    task: &Value,
    record_kind: &str,
) -> Result<(), ExecutionIssue> {
    let formal_ids: HashSet<_> = [
        "steps",
        "commands",
        "operations",
        "validations",
        "decisions",
    ]
    .into_iter()
    .flat_map(|field| task[field].as_array().into_iter().flatten())
    .filter_map(|row| row["id"].as_str())
    .collect();
    let action = &proposal["action"];
    let anchor = proposal["anchor_record_id"].as_str().unwrap_or("");
    let base_id = anchor.split('#').next().unwrap_or(anchor);
    let basis = proposal["task_basis"].as_array();
    if basis.is_none_or(|basis| {
        !basis.iter().any(|id| id == base_id)
            || basis
                .iter()
                .any(|id| id.as_str().is_none_or(|id| !formal_ids.contains(id)))
    }) {
        return Err(issue(
            "deviation_task_basis",
            "task_basis must contain only formal target TASK IDs and include the anchor base ID.",
            json!({}),
        ));
    }
    match action["kind"].as_str().unwrap_or("") {
        "replace_command" if record_kind != "command" || action["record_id"] != anchor => {
            Err(issue(
                "deviation_action_anchor",
                "replace_command must target the reserved command record.",
                json!({}),
            ))
        }
        "skip_record" if action["record_id"] != anchor => Err(issue(
            "deviation_action_anchor",
            "skip_record must target the reserved record.",
            json!({}),
        )),
        "adjust_operation"
            if record_kind != "operation" || action["operation"]["id"] != base_id =>
        {
            Err(issue(
                "deviation_action_anchor",
                "adjust_operation must preserve the reserved operation ID.",
                json!({}),
            ))
        }
        "add_command"
            if action["after_record_id"] != anchor
                || action["command"]["id"]
                    .as_str()
                    .is_some_and(|id| formal_ids.contains(id)) =>
        {
            Err(issue(
                "deviation_action_identity",
                "add_command must follow the anchor and use a new formal ID.",
                json!({}),
            ))
        }
        "add_validation"
            if action["validation"]["id"]
                .as_str()
                .is_some_and(|id| formal_ids.contains(id)) =>
        {
            Err(issue(
                "deviation_action_identity",
                "add_validation must use a new formal ID.",
                json!({}),
            ))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_sequence_match_python() {
        assert_eq!(derive_overall_status(&["cancelled"]), "cancelled");
        assert_eq!(derive_overall_status(&["completed", "blocked"]), "blocked");
        assert_eq!(
            derive_overall_status(&["completed", "pending_retry"]),
            "pending"
        );
        assert_eq!(next_attempt_id(None).unwrap(), "ATTEMPT-001");
        assert_eq!(next_attempt_id(Some("ATTEMPT-009")).unwrap(), "ATTEMPT-010");
        assert_eq!(
            next_attempt_id(Some("ATTEMPT-999"))
                .unwrap_err()
                .reason_code,
            "attempt_start_id_exhausted"
        );
        assert_eq!(
            closed_task_status("stopped", Some("specification_defect")),
            "blocked"
        );
        for final_type in [
            "external_operation_failed",
            "instructions_changed",
            "specification_defect",
        ] {
            assert_eq!(closed_task_status("stopped", Some(final_type)), "blocked");
        }
        assert_eq!(
            closed_task_status("stopped", Some("other")),
            "pending_retry"
        );
        let attempt =
            json!({"carried_records":[{"record_id":"CMD-001"}],"records":[{"id":"CMD-001#2"}]});
        assert_eq!(next_record_id("CMD-001", &attempt).unwrap(), "CMD-001#3");
        let invalid_retries = json!({"records":[{"id":"CMD-001#0"},{"id":"CMD-001#01"},
            {"id":"CMD-001#"},{"id":"CMD-001#1#2"}]});
        assert_eq!(
            next_record_id("CMD-001", &invalid_retries).unwrap(),
            "CMD-001"
        );
        let large_retry = json!({"records":[{"id":"CMD-001#999999999999999999999999999999"}]});
        assert_eq!(
            next_record_id("CMD-001", &large_retry).unwrap(),
            "CMD-001#1000000000000000000000000000000"
        );
    }

    #[test]
    fn formal_record_kind_and_retry_sequence_match_python() {
        let task = json!({"commands":[{"id":"CMD-001"}],
            "operations":[{"id":"OP-001"}],"validations":[{"id":"VAL-001"}]});
        for (record_id, kind) in [
            ("CMD-001", "command"),
            ("OP-001", "operation"),
            ("VAL-001", "validation"),
        ] {
            assert_eq!(formal_record_kind(&task, record_id).unwrap(), kind);
        }
        let error =
            formal_record_kind(&json!({"commands":[{"id":"CMD-002"}]}), "CMD-001").unwrap_err();
        assert_eq!(error.reason_code, "record_begin_record_not_found");
        assert_eq!(error.details, json!({"record_id":"CMD-001"}));
        assert_eq!(
            next_record_id("VAL-001", &json!({"records":[]})).unwrap(),
            "VAL-001"
        );
        let attempt = json!({"carried_records":[{"record_id":"VAL-001#3"}],
            "records":[{"id":"VAL-001"},{"id":"VAL-001#1"},{"id":"VAL-002#9"}]});
        assert_eq!(next_record_id("VAL-001", &attempt).unwrap(), "VAL-001#4");
        assert_eq!(next_record_id("VAL-003", &attempt).unwrap(), "VAL-003");
        for record_id in ["VAL-001#1", "UNKNOWN-001", ""] {
            let error = next_record_id(record_id, &json!({})).unwrap_err();
            assert_eq!(error.reason_code, "record_begin_invalid_base_record_id");
            assert_eq!(error.details, json!({"record_id":record_id}));
        }
    }

    #[test]
    fn index_transitions_and_correction_impact_match_python() {
        let index = json!({"acceptance_results":[],"overall_status":"pending","tasks":[
            {"id":"TASK-001","status":"pending","acceptance_results":[]},{"id":"TASK-002","status":"completed","acceptance_results":[]}]});
        let started = start_index(&index, "TASK-001", "ATTEMPT-001").unwrap();
        assert_eq!(started["overall_status"], "in_progress");
        let closed = close_index(&started, "TASK-001", "ATTEMPT-001", "completed").unwrap();
        assert_eq!(closed["overall_status"], "completed");
        let collection = json!({"tasks":[{"id":"TASK-001","dependencies":[]},{"id":"TASK-002","dependencies":["TASK-001"]}]});
        let affected = affected_task_ids(&closed, &collection, "TASK-001", true).unwrap();
        assert_eq!(affected, ["TASK-001", "TASK-002"]);
        let corrected =
            corrected_index(&closed, "TASK-001", "ATTEMPT-001-CORRECTION-001", &affected).unwrap();
        assert_eq!(corrected["overall_status"], "pending");
        assert_eq!(corrected["tasks"][1]["status"], "pending_retry");
    }

    #[test]
    fn authorization_and_preflight_reject_drift() {
        let task = json!({"id":"TASK-001","requirement_id":"example","spec_id":"TASK-SPEC-001",
            "commands":[{"id":"CMD-001","mode":"argv","execution":{"working_directory":"."}}],
            "validations":[],"operations":[],"files":[{"path":"src/main.rs"}]});
        let manifest = json!({"task_id":"TASK-001","commands":[task["commands"][0]],"validations":[],
            "external_operations":[],"modifiable_files":["src/main.rs"],"working_directories":["."]});
        assert!(validate_authorization_scope(&manifest, &task, &json!({})).is_ok());
        let mut expanded = manifest.clone();
        expanded["modifiable_files"] = json!(["other.rs"]);
        assert_eq!(
            validate_authorization_scope(&expanded, &task, &json!({}))
                .unwrap_err()
                .reason_code,
            "attempt_authorization_scope_expansion"
        );
        let validation = json!({"task_collection_sha256":"a","task_index_sha256":"b","instructions_sha256":"c",
            "hierarchy_selection_sha256":"d","task_ids":["TASK-001"],"task_instructions_sha256":{"TASK-001":"e"}});
        let index = json!({"acceptance_results":[],"requirement_id":"example","task_spec_id":"TASK-SPEC-001","task_collection_sha256":"a",
            "task_index_sha256":"b","task_instructions_sha256":"c","hierarchy_selection_sha256":"d",
            "tasks":[{"id":"TASK-001","instructions_sha256":"e","acceptance_results":[]}]});
        let mut collection = task.clone();
        collection["tasks"] = json!([task.clone()]);
        assert!(validate_preflight_identity(&index, &collection, &validation).is_ok());
        let mut stale_document = index.clone();
        stale_document["task_instructions_sha256"] = json!("stale");
        assert_eq!(
            validate_preflight_identity(&stale_document, &collection, &validation)
                .unwrap_err()
                .reason_code,
            "execute_preflight_index_identity_mismatch"
        );
        let mut stale = index.clone();
        stale["tasks"][0]["instructions_sha256"] = json!("stale");
        assert_eq!(
            validate_preflight_identity(&stale, &collection, &validation)
                .unwrap_err()
                .reason_code,
            "execute_preflight_task_instructions_mismatch"
        );
    }

    #[test]
    fn authorization_accepts_exact_task_subset_with_windows_defaults() {
        let task = json!({"id":"TASK-001",
            "commands":[{"id":"CMD-001","mode":"argv","argv":["tool"]}],
            "validations":[],"operations":[],
            "files":[{"id":"FILE-001","action":"modify","path":"src.txt"}]});
        let manifest = json!({"task_id":"TASK-001","commands":[task["commands"][0]],
            "validations":[],"external_operations":[],"modifiable_files":["src.txt"],
            "working_directories":["."]});
        let defaults = json!({"working_directory":".","os":"windows","shell":"powershell"});
        assert_eq!(manifest["commands"], task["commands"]);
        validate_authorization_scope(&manifest, &task, &defaults).unwrap();
        let mut expanded = manifest;
        expanded["modifiable_files"] = json!(["other.txt"]);
        assert_eq!(
            validate_authorization_scope(&expanded, &task, &defaults)
                .unwrap_err()
                .reason_code,
            "attempt_authorization_scope_expansion"
        );
    }

    #[test]
    fn record_completion_and_deviation_rules_follow_python() {
        let attempt = json!({"records":[{"id":"VAL-001","kind":"validation","outcome":"failed"}]});
        let request =
            json!({"record":{"outcome":"success","state":"done"},"modified_files":["src/lib.rs"]});
        let finished =
            finish_attempt_candidate(&attempt, &request, "OP-001", "operation", None).unwrap();
        assert_eq!(
            finished["overall_result"],
            json!({"status":"complete_success","effective":["OP-001"]})
        );
        assert_eq!(finished["modified_files"], json!(["src/lib.rs"]));
        let task = json!({"validations":[{"id":"VAL-001"}],"operations":[{"id":"OP-001"}],"steps":[],"commands":[],"decisions":[]});
        assert_eq!(
            validate_completed_coverage(&task, &finished)
                .unwrap_err()
                .reason_code,
            "attempt_close_incomplete_validations"
        );
        let proposal = json!({"impact":{"scope_changed":true},"anchor_record_id":"OP-001","task_basis":["OP-001"],
            "action":{"kind":"adjust_operation","operation":{"id":"OP-001"}}});
        assert_eq!(
            deviation_reconciliation_target(&proposal),
            "task_and_execution"
        );
        assert!(validate_deviation_action(&proposal, &task, "operation").is_ok());
    }

    #[test]
    fn record_finish_identity_and_operation_aggregation_match_python() {
        assert_eq!(overall_operation_result(&[]), None);
        assert_eq!(
            overall_operation_result(&[json!({"id":"OP-001","kind":"operation",
            "status":"skipped","reason":"Not applicable.","deviation_id":"DEVIATION-001"})]),
            None
        );
        assert_eq!(
            overall_operation_result(&[json!({"id":"OP-001","kind":"operation",
            "outcome":"success"})]),
            Some(json!({"status":"complete_success","effective":["OP-001"]}))
        );
        assert_eq!(
            overall_operation_result(&[
                json!({"id":"OP-001","kind":"operation",
            "outcome":"success"}),
                json!({"id":"OP-002","kind":"operation","outcome":"failure"})
            ]),
            Some(json!({"status":"partial_success","effective":["OP-001"],
                "not_effective":["OP-002"]}))
        );
        assert_eq!(
            overall_operation_result(&[json!({"id":"OP-001","kind":"operation",
            "outcome":"unknown"})]),
            Some(json!({"status":"uncertain_result","unknown":["OP-001"]}))
        );
        let attempt = json!({"records":[]});
        let request = json!({"record":{"outcome":"passed","evidence":"Checked."}});
        let candidate =
            finish_attempt_candidate(&attempt, &request, "VAL-001", "validation", None).unwrap();
        assert_eq!(candidate["records"][0]["id"], "VAL-001");
        assert_eq!(candidate["records"][0]["kind"], "validation");
        for field in ["id", "kind", "correction"] {
            let mut invalid = request.clone();
            invalid["record"][field] = if field == "correction" {
                json!({})
            } else {
                json!("VAL-002")
            };
            assert_eq!(
                finish_attempt_candidate(&attempt, &invalid, "VAL-001", "validation", None)
                    .unwrap_err()
                    .reason_code,
                "record_finish_machine_fields"
            );
        }
    }

    #[test]
    fn completed_validation_coverage_follows_task_order_and_current_retry() {
        let task = json!({"validations":[{"id":"VAL-001"},{"id":"VAL-002"},{"id":"VAL-003"}]});
        let passed = json!({"records":[
            {"id":"VAL-001","kind":"validation","outcome":"passed"},
            {"id":"VAL-002","kind":"validation","outcome":"passed"},
            {"id":"VAL-003","kind":"validation","outcome":"passed"}]});
        validate_completed_coverage(&task, &passed).unwrap();
        let incomplete = json!({"records":[
            {"id":"VAL-001","kind":"validation","outcome":"failed"},
            {"id":"VAL-003","kind":"validation","outcome":"passed"}]});
        let error = validate_completed_coverage(&task, &incomplete).unwrap_err();
        assert_eq!(error.reason_code, "attempt_close_incomplete_validations");
        assert_eq!(
            error.message,
            "A completed Attempt requires every formal validation to pass or have approved skipped evidence."
        );
        assert_eq!(error.details["missing"], json!(["VAL-002"]));
        assert_eq!(error.details["failed"], json!(["VAL-001"]));
        let one = json!({"validations":[{"id":"VAL-001"}]});
        validate_completed_coverage(
            &one,
            &json!({"carried_records":[{"record_id":"VAL-001"}],"records":[]}),
        )
        .unwrap();
        validate_completed_coverage(
            &one,
            &json!({"records":[{"id":"VAL-001","kind":"validation","status":"skipped",
                "reason":"Not applicable.","deviation_id":"DEVIATION-001"}]}),
        )
        .unwrap();
        let retry = json!({"carried_records":[{"record_id":"VAL-001"}],
            "records":[{"id":"VAL-001#2","kind":"validation","outcome":"failed"}]});
        let error = validate_completed_coverage(&one, &retry).unwrap_err();
        assert_eq!(error.details["missing"], json!([]));
        assert_eq!(error.details["failed"], json!(["VAL-001"]));
    }

    #[test]
    fn record_recovery_and_formal_kind_follow_python() {
        let task =
            json!({"commands":[{"id":"CMD-001"}],"operations":[],"validations":[{"id":"VAL-001"}]});
        assert_eq!(formal_record_kind(&task, "CMD-001").unwrap(), "command");
        assert_eq!(
            formal_record_kind(&task, "VAL-002")
                .unwrap_err()
                .reason_code,
            "record_begin_record_not_found"
        );
        let index = json!({"lock":{"kind":"execution","record_id":"CMD-001",
            "command_correction":{"reason":"corrected"},"retry_authorization_evidence":"approved"}});
        assert_eq!(
            finished_record_index(&index).unwrap(),
            json!({"lock":{"kind":"execution"}})
        );
        let attempt = json!({"status":"stopped","final_type":"user_stopped","reason":"Paused",
            "closing_authorization_evidence":"Approved"});
        assert_eq!(
            attempt_close_request(&attempt),
            json!({"schema":"work-attempt-close-request/v1",
            "status":"stopped","final_type":"user_stopped","reason":"Paused","authorization_evidence":"Approved"})
        );
    }

    #[test]
    fn record_begin_candidate_preserves_retry_evidence_and_rejects_stale_lock() {
        let task = json!({"commands":[{"id":"CMD-001"}],"operations":[],"validations":[]});
        let attempt = json!({"attempt_id":"ATTEMPT-001","execute_instructions_sha256":"a",
            "authorization":{"commands":[{"id":"CMD-001"}],"authorization_evidence":"Original"},
            "carried_records":[],"records":[{"id":"CMD-001"}]});
        let index = json!({"lock":build_execution_lock("TASK-001", "ATTEMPT-001", "a")});
        let (reserved, id, kind) = record_begin_candidate(
            &task,
            &attempt,
            &index,
            "TASK-001",
            "CMD-001",
            Some("Approved retry"),
        )
        .unwrap();
        assert_eq!((id.as_str(), kind), ("CMD-001#1", "command"));
        assert_eq!(reserved["lock"]["record_id"], "CMD-001#1");
        assert_eq!(
            reserved["lock"]["retry_authorization_evidence"],
            "Approved retry"
        );
        assert_eq!(
            record_begin_candidate(&task, &attempt, &index, "TASK-001", "CMD-001", None)
                .unwrap_err()
                .reason_code,
            "execution_authorization_retry_required"
        );
        assert_eq!(
            record_begin_candidate(
                &task,
                &attempt,
                &reserved,
                "TASK-001",
                "CMD-001",
                Some("Approved retry")
            )
            .unwrap_err()
            .reason_code,
            "record_begin_record_already_reserved"
        );
        let stale = json!({"lock":build_execution_lock("TASK-002", "ATTEMPT-001", "a")});
        assert_eq!(
            record_begin_candidate(
                &task,
                &attempt,
                &stale,
                "TASK-001",
                "CMD-001",
                Some("Approved retry")
            )
            .unwrap_err()
            .reason_code,
            "record_begin_lock_mismatch"
        );
    }

    #[test]
    fn record_identity_fingerprints_and_drift_match_python_cases() {
        let collection = json!({"requirement_id":"example","spec_id":"TASK-SPEC-001","tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_ids":["TASK-001"],"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"1".repeat(64),"task_item_sha256":{"TASK-001":"2".repeat(64)},
            "instructions_sha256":"b".repeat(64),"task_instructions_sha256":{"TASK-001":"c".repeat(64)},
            "hierarchy_selection_sha256":"f".repeat(64)});
        let index = json!({"acceptance_results":[],"requirement_id":"example","task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"1".repeat(64),
            "task_instructions_sha256":"b".repeat(64),"hierarchy_selection_sha256":"f".repeat(64),
            "tasks":[{"id":"TASK-001","instructions_sha256":"c".repeat(64),"acceptance_results":[]}]});
        let attempt = json!({"acceptance_results":[],"task_spec_id":"TASK-SPEC-001","task_id":"TASK-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"1".repeat(64),
            "task_item_sha256":"2".repeat(64),"task_instructions_sha256":"c".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64)});
        assert_eq!(
            validate_execution_identity(&collection, &validation, &index, &attempt, "TASK-001")
                .unwrap()["instructions_sha256"],
            "c".repeat(64)
        );
        let mut stale = index.clone();
        stale["tasks"][0]["instructions_sha256"] = json!("0".repeat(64));
        assert_eq!(
            validate_execution_identity(&collection, &validation, &stale, &attempt, "TASK-001")
                .unwrap_err()
                .reason_code,
            "record_begin_task_instructions_mismatch"
        );
        let mut changed = index.clone();
        changed["task_collection_sha256"] = json!("0".repeat(64));
        assert_eq!(
            validate_execution_identity(&collection, &validation, &changed, &attempt, "TASK-001")
                .unwrap_err()
                .reason_code,
            "record_begin_index_identity_mismatch"
        );
        let mut changed = index.clone();
        changed["tasks"][0]["id"] = json!("TASK-002");
        assert_eq!(
            validate_execution_identity(&collection, &validation, &changed, &attempt, "TASK-001")
                .unwrap_err()
                .reason_code,
            "record_begin_index_task_set_mismatch"
        );
        let mut changed = attempt.clone();
        changed["task_item_sha256"] = json!("0".repeat(64));
        assert_eq!(
            validate_execution_identity(&collection, &validation, &index, &changed, "TASK-001")
                .unwrap_err()
                .reason_code,
            "record_begin_attempt_identity_mismatch"
        );
        let missing =
            validate_execution_identity(&collection, &validation, &index, &attempt, "TASK-002")
                .unwrap_err();
        assert_eq!(missing.reason_code, "record_begin_task_not_found");
        assert_eq!(missing.details, json!({"task_id":"TASK-002"}));
    }
}

pub mod acceptance;
pub mod attempt;
pub mod attempt_close;
pub mod attempt_prepare;
pub mod attempt_start;
pub mod authorization;
pub mod command_correction;
pub mod command_run;
pub mod correction;
pub mod deviation;
pub mod index;
pub mod preflight;
pub mod record_finish;
pub mod recovery;
pub mod requests;
pub mod worktree;
