//! Pure acceptance evidence shape and immutable Task binding checks.
use super::ExecutionIssue;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use work_model::execution::acceptance::{AcceptanceProgress, AcceptanceStatus};
use work_model::execution::attempt::ValidationOutcome;

fn issue(code: &'static str, message: &'static str) -> ExecutionIssue {
    ExecutionIssue {
        reason_code: code,
        message,
        details: json!({}),
    }
}

pub fn pending(ids: impl IntoIterator<Item = String>) -> Value {
    json!(
        ids.into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|id| json!({"id":id,"status":"pending","evidence":[]}))
            .collect::<Vec<_>>()
    )
}

pub fn task_ids(task: &Value) -> BTreeSet<String> {
    task["traceability"]["acceptance_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .chain(
            task["acceptance_criteria"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row["id"].as_str())
                .map(str::to_owned),
        )
        .collect()
}

pub fn reset(value: &Value) -> Result<Value, ExecutionIssue> {
    let rows: Vec<AcceptanceProgress> = serde_json::from_value(value.clone()).map_err(|_| {
        issue(
            "invalid_acceptance_results",
            "Acceptance progress is required.",
        )
    })?;
    Ok(pending(rows.into_iter().map(|row| row.id)))
}

pub fn validate(
    value: &Value,
    bindings: &BTreeMap<String, (String, String)>,
    attempt: Option<&str>,
    owner: Option<&str>,
) -> Result<(), ExecutionIssue> {
    let rows: Vec<AcceptanceProgress> = serde_json::from_value(value.clone()).map_err(|_| {
        issue(
            "invalid_acceptance_results",
            "Acceptance progress requires exact IDs, states and evidence fields.",
        )
    })?;
    let mut ids = BTreeSet::new();
    for row in rows {
        let main = crate::task::item::numbered(&row.id, "ACCEPTANCE").is_some();
        let technical = row
            .id
            .split_once("-ACCEPTANCE-")
            .is_some_and(|(task, number)| {
                crate::task::item::numbered(task, "TASK").is_some()
                    && crate::task::item::numbered(&format!("ACCEPTANCE-{number}"), "ACCEPTANCE")
                        .is_some()
                    && owner == Some(task)
            });
        if !ids.insert(row.id.clone()) || (!main && !technical) {
            return Err(issue(
                "invalid_acceptance_id",
                "Acceptance IDs must be unique and belong to their main or Task scope.",
            ));
        }
        if row.status == AcceptanceStatus::Completed
            && (row.evidence.is_empty()
                || row
                    .evidence
                    .iter()
                    .any(|e| e.outcome != ValidationOutcome::Passed))
        {
            return Err(issue(
                "acceptance_without_passed_evidence",
                "Completed acceptance requires passed validation evidence.",
            ));
        }
        let mut seen = BTreeSet::new();
        for evidence in row.evidence {
            if crate::task::item::numbered(&evidence.attempt_id, "ATTEMPT").is_none()
                || crate::task::item::numbered(&evidence.validation_id, "VAL").is_none()
                || evidence.evidence.trim().is_empty()
                || !seen.insert((
                    evidence.task_id.clone(),
                    evidence.attempt_id.clone(),
                    evidence.record_id.clone(),
                ))
            {
                return Err(issue(
                    "invalid_acceptance_evidence",
                    "Evidence requires distinct exact Attempt and VAL identities and nonempty results.",
                ));
            }
            let (base, retry) = evidence
                .record_id
                .split_once('#')
                .unwrap_or((&evidence.record_id, ""));
            if base != evidence.validation_id
                || (evidence.record_id.contains('#') && retry.is_empty())
                || (!retry.is_empty()
                    && retry
                        .parse::<u64>()
                        .ok()
                        .is_none_or(|n| n == 0 || n.to_string() != retry))
            {
                return Err(issue(
                    "invalid_acceptance_evidence",
                    "Evidence must identify the exact recorded VAL instance.",
                ));
            }
            if owner.is_some_and(|id| id != evidence.task_id)
                || attempt.is_some_and(|id| id != evidence.attempt_id)
                || bindings.get(&evidence.task_id)
                    != Some(&(evidence.task_item_sha256, evidence.task_instructions_sha256))
            {
                return Err(issue(
                    "acceptance_evidence_binding_mismatch",
                    "Acceptance evidence must match the current Task and instruction binding.",
                ));
            }
        }
    }
    Ok(())
}

pub fn require_recorded_evidence(attempt: &Value) -> Result<(), ExecutionIssue> {
    for progress in attempt["acceptance_results"]
        .as_array()
        .expect("validated progress")
    {
        for evidence in progress["evidence"].as_array().expect("validated evidence") {
            let record = attempt["records"]
                .as_array()
                .expect("validated records")
                .iter()
                .find(|row| row["id"] == evidence["record_id"]);
            let validation = attempt["authorization"]["validations"]
                .as_array()
                .expect("validated authorization")
                .iter()
                .find(|row| row["id"] == evidence["validation_id"]);
            if record.is_none_or(|row| {
                row["kind"] != "validation"
                    || row["status"] == "skipped"
                    || row["outcome"] != evidence["outcome"]
                    || row["evidence"] != evidence["evidence"]
            }) || validation.is_none_or(|row| {
                !row["acceptance_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&progress["id"]))
            }) {
                return Err(issue(
                    "acceptance_evidence_not_recorded",
                    "Acceptance evidence must match an actually recorded, authorized VAL result covering that criterion.",
                ));
            }
        }
    }
    Ok(())
}

pub fn require_ids(value: &Value, expected: &BTreeSet<String>) -> Result<(), ExecutionIssue> {
    let rows = value.as_array().ok_or_else(|| {
        issue(
            "invalid_acceptance_results",
            "Acceptance progress is required.",
        )
    })?;
    let observed: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    if observed != *expected || observed.len() != rows.len() {
        return Err(issue(
            "acceptance_definition_mismatch",
            "Acceptance progress must exactly cover the immutable Task criteria.",
        ));
    }
    Ok(())
}

pub fn require_collection(index: &Value, collection: &Value) -> Result<(), ExecutionIssue> {
    let main = collection["acceptance_criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect();
    require_ids(&index["acceptance_results"], &main)?;
    for row in index["tasks"].as_array().into_iter().flatten() {
        let task = collection["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == row["id"])
            .ok_or_else(|| {
                issue(
                    "acceptance_definition_mismatch",
                    "Every progress row requires its immutable Task definition.",
                )
            })?;
        require_ids(&row["acceptance_results"], &task_ids(task))?;
    }
    Ok(())
}

/// Derive progress from the latest actual result of every VAL covering each criterion.
pub fn aggregate_attempt(task: &Value, attempt: &Value) -> Result<Value, ExecutionIssue> {
    super::attempt::render_attempt(attempt)?;
    if task["id"] != attempt["task_id"] {
        return Err(issue(
            "acceptance_evidence_binding_mismatch",
            "Attempt must belong to the immutable Task.",
        ));
    }
    let ids = task_ids(task);
    require_ids(&attempt["acceptance_results"], &ids)?;
    let mut result = pending(ids);
    for progress in result.as_array_mut().expect("pending progress") {
        let validations: Vec<_> = task["validations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|validation| {
                validation["acceptance_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&progress["id"]))
            })
            .collect();
        if validations.is_empty() {
            return Err(issue(
                "acceptance_validation_coverage_missing",
                "Every criterion requires an immutable Task VAL.",
            ));
        }
        let mut passed = true;
        let mut evidence = Vec::new();
        for validation in validations {
            let id = validation["id"]
                .as_str()
                .ok_or_else(|| issue("invalid_acceptance_evidence", "VAL identity is required."))?;
            let authorized = attempt["authorization"]["validations"]
                .as_array()
                .expect("validated authorization")
                .iter()
                .any(|row| {
                    row["id"] == id
                        && row["acceptance_ids"]
                            .as_array()
                            .is_some_and(|ids| ids.contains(&progress["id"]))
                });
            let record = attempt["records"]
                .as_array()
                .expect("validated records")
                .iter()
                .rev()
                .find(|row| {
                    row["kind"] == "validation"
                        && row["id"]
                            .as_str()
                            .is_some_and(|record| record.split('#').next() == Some(id))
                });
            if !authorized
                || record.is_none_or(|row| row["status"] == "skipped" || row["outcome"] != "passed")
            {
                passed = false;
                continue;
            }
            let record = record.expect("passed actual record");
            evidence.push(json!({"task_id":attempt["task_id"],"attempt_id":attempt["attempt_id"],"validation_id":id,"record_id":record["id"],"outcome":record["outcome"],"evidence":record["evidence"],"task_item_sha256":attempt["task_item_sha256"],"task_instructions_sha256":attempt["task_instructions_sha256"]}));
        }
        progress["status"] = json!(if passed { "completed" } else { "pending" });
        progress["evidence"] = json!(evidence);
    }
    let mut checked = attempt.clone();
    checked["acceptance_results"] = result.clone();
    super::attempt::render_attempt(&checked)?;
    Ok(result)
}

/// Recompute Task and shared criteria using only the index's current Attempts.
pub fn aggregate_collection(
    collection: &Value,
    index: &Value,
    attempts: &BTreeMap<String, Value>,
) -> Result<Value, ExecutionIssue> {
    let raw = super::index::render_execution_index(index)
        .map_err(|_| issue("invalid_execution_index", "Execution index must serialize."))?;
    super::index::validate_execution_index(index, &raw)?;
    require_collection(index, collection)?;
    let mut result = index.clone();
    for row in result["tasks"].as_array_mut().expect("validated tasks") {
        let task_id = row["id"].as_str().expect("validated Task identity");
        let task = collection["tasks"]
            .as_array()
            .expect("validated collection")
            .iter()
            .find(|task| task["id"] == task_id)
            .expect("required collection");
        row["acceptance_results"] = if let Some(attempt) = attempts.get(task_id) {
            if row["latest_attempt"] != attempt["attempt_id"]
                || attempt["task_item_sha256"] != row["task_item_sha256"]
                || attempt["task_instructions_sha256"] != row["instructions_sha256"]
                || attempt["task_collection_sha256"] != index["task_collection_sha256"]
                || attempt["task_index_sha256"] != index["task_index_sha256"]
            {
                return Err(issue(
                    "acceptance_evidence_binding_mismatch",
                    "Only the current Attempt and immutable Task binding may contribute evidence.",
                ));
            }
            aggregate_attempt(task, attempt)?
        } else {
            pending(task_ids(task))
        };
    }
    let tasks = result["tasks"].as_array().expect("validated tasks").clone();
    for progress in result["acceptance_results"]
        .as_array_mut()
        .expect("validated progress")
    {
        let responsible: Vec<_> = tasks
            .iter()
            .filter_map(|task| {
                task["acceptance_results"]
                    .as_array()
                    .expect("computed progress")
                    .iter()
                    .find(|row| row["id"] == progress["id"])
            })
            .collect();
        if responsible.is_empty() {
            return Err(issue(
                "acceptance_responsibility_missing",
                "Main acceptance requires a responsible Task.",
            ));
        }
        progress["status"] = json!(
            if responsible.iter().all(|row| row["status"] == "completed") {
                "completed"
            } else {
                "pending"
            }
        );
        progress["evidence"] = json!(
            responsible
                .iter()
                .flat_map(|row| row["evidence"]
                    .as_array()
                    .expect("computed evidence")
                    .iter()
                    .cloned())
                .collect::<Vec<_>>()
        );
    }
    let raw = super::index::render_execution_index(&result)
        .map_err(|_| issue("invalid_execution_index", "Execution index must serialize."))?;
    super::index::validate_execution_index(&result, &raw)?;
    Ok(result)
}

/// Recompute shared criteria from Task progress without altering historical Attempts.
pub fn aggregate_index_progress(index: &Value) -> Result<Value, ExecutionIssue> {
    let mut result = index.clone();
    let tasks = result["tasks"]
        .as_array()
        .ok_or_else(|| {
            issue(
                "invalid_execution_index_tasks",
                "Task progress is required.",
            )
        })?
        .clone();
    for progress in result["acceptance_results"].as_array_mut().ok_or_else(|| {
        issue(
            "invalid_acceptance_results",
            "Acceptance progress is required.",
        )
    })? {
        let responsible: Vec<_> = tasks
            .iter()
            .filter_map(|task| {
                task["acceptance_results"]
                    .as_array()?
                    .iter()
                    .find(|row| row["id"] == progress["id"])
            })
            .collect();
        if responsible.is_empty() {
            return Err(issue(
                "acceptance_responsibility_missing",
                "Main acceptance requires a responsible Task.",
            ));
        }
        progress["status"] = json!(
            if responsible.iter().all(|row| row["status"] == "completed") {
                "completed"
            } else {
                "pending"
            }
        );
        progress["evidence"] = json!(
            responsible
                .iter()
                .flat_map(|row| row["evidence"].as_array().into_iter().flatten().cloned())
                .collect::<Vec<_>>()
        );
    }
    Ok(result)
}

pub fn invalidate(index: &Value, affected: &[String]) -> Result<Value, ExecutionIssue> {
    let mut result = index.clone();
    for row in result["tasks"].as_array_mut().ok_or_else(|| {
        issue(
            "invalid_execution_index_tasks",
            "Task progress is required.",
        )
    })? {
        if row["id"]
            .as_str()
            .is_some_and(|id| affected.iter().any(|affected| affected == id))
        {
            row["acceptance_results"] = reset(&row["acceptance_results"])?;
        }
    }
    aggregate_index_progress(&result)
}

pub fn update_index_attempt(index: &Value, attempt: &Value) -> Result<Value, ExecutionIssue> {
    super::attempt::render_attempt(attempt)?;
    let mut result = index.clone();
    let row = result["tasks"]
        .as_array_mut()
        .ok_or_else(|| {
            issue(
                "invalid_execution_index_tasks",
                "Task progress is required.",
            )
        })?
        .iter_mut()
        .find(|row| row["id"] == attempt["task_id"])
        .ok_or_else(|| issue("execution_task_not_found", "Attempt Task is absent."))?;
    if row["latest_attempt"] != attempt["attempt_id"]
        || row["task_item_sha256"] != attempt["task_item_sha256"]
        || row["instructions_sha256"] != attempt["task_instructions_sha256"]
    {
        return Err(issue(
            "acceptance_evidence_binding_mismatch",
            "Attempt evidence must match the current Task binding.",
        ));
    }
    require_ids(
        &attempt["acceptance_results"],
        &reset(&row["acceptance_results"])?
            .as_array()
            .expect("pending progress")
            .iter()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
            .collect(),
    )?;
    row["acceptance_results"] = attempt["acceptance_results"].clone();
    aggregate_index_progress(&result)
}

pub(crate) fn field_order(path: &[String]) -> Option<&'static [&'static str]> {
    if path
        .last()
        .is_some_and(|field| field == "acceptance_results")
    {
        Some(&["id", "status", "evidence"])
    } else if path.last().is_some_and(|field| field == "evidence")
        && path.iter().any(|field| field == "acceptance_results")
    {
        Some(&[
            "task_id",
            "attempt_id",
            "validation_id",
            "record_id",
            "outcome",
            "evidence",
            "task_item_sha256",
            "task_instructions_sha256",
        ])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_acceptance_has_no_evidence_and_completion_requires_exact_binding() {
        let bindings = BTreeMap::from([("TASK-001".into(), ("a".repeat(64), "b".repeat(64)))]);
        let mut value = pending(["ACCEPTANCE-001".into(), "TASK-001-ACCEPTANCE-001".into()]);
        validate(&value, &bindings, Some("ATTEMPT-001"), Some("TASK-001")).unwrap();
        assert!(
            value
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["status"] == "pending" && row["evidence"] == json!([]))
        );
        value[0]["status"] = json!("completed");
        assert_eq!(
            validate(&value, &bindings, Some("ATTEMPT-001"), Some("TASK-001"))
                .unwrap_err()
                .reason_code,
            "acceptance_without_passed_evidence"
        );
        value[0]["evidence"] = json!([{"task_id":"TASK-001","attempt_id":"ATTEMPT-001","validation_id":"VAL-001","record_id":"VAL-001","outcome":"passed","evidence":"Verified actual result.","task_item_sha256":"a".repeat(64),"task_instructions_sha256":"b".repeat(64)}]);
        validate(&value, &bindings, Some("ATTEMPT-001"), Some("TASK-001")).unwrap();
        for field in ["task_item_sha256", "task_instructions_sha256"] {
            let mut drift = value.clone();
            drift[0]["evidence"][0][field] = json!("c".repeat(64));
            assert_eq!(
                validate(&drift, &bindings, Some("ATTEMPT-001"), Some("TASK-001"))
                    .unwrap_err()
                    .reason_code,
                "acceptance_evidence_binding_mismatch"
            );
        }
        for record in [
            "VAL-001#",
            "VAL-001#0",
            "VAL-001#01",
            "VAL-002",
            "VAL-001#2#3",
        ] {
            let mut invalid = value.clone();
            invalid[0]["evidence"][0]["record_id"] = json!(record);
            assert_eq!(
                validate(&invalid, &bindings, Some("ATTEMPT-001"), Some("TASK-001"))
                    .unwrap_err()
                    .reason_code,
                "invalid_acceptance_evidence"
            );
        }
        let reset = reset(&value).unwrap();
        assert!(
            reset
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["status"] == "pending" && row["evidence"] == json!([]))
        );
    }
    #[test]
    fn shared_and_owned_acceptance_require_all_actual_latest_validations() {
        let registry = work_model::contract_data::registry_value();
        let mut index = registry["items"]["work-execution-index"]["description"]["example"].clone();
        index["acceptance_results"] = pending(["ACCEPTANCE-001".into()]);
        index["overall_status"] = json!("in_progress");
        let mut tasks = Vec::new();
        let mut rows = Vec::new();
        let mut attempts = BTreeMap::new();
        for id in ["TASK-001", "TASK-002"] {
            let own = format!("{id}-ACCEPTANCE-001");
            let task = json!({"id":id,"traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":own}],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001",own]},{"id":"VAL-002","acceptance_ids":["ACCEPTANCE-001"]}]});
            let mut attempt = registry["items"]["work-attempt"]["description"]["example"].clone();
            attempt["task_id"] = json!(id);
            attempt["authorization"]["task_id"] = json!(id);
            attempt["authorization"]["validations"] = json!([{"id":"VAL-001","kind":"manual","confirmer":"user","criteria":"Verified.","acceptance_ids":["ACCEPTANCE-001",own]},{"id":"VAL-002","kind":"manual","confirmer":"user","criteria":"Verified.","acceptance_ids":["ACCEPTANCE-001"]}]);
            attempt["authorization_sha256"] =
                json!(crate::canonical::canonical_json_sha256(&attempt["authorization"]).unwrap());
            attempt["task_collection_sha256"] = index["task_collection_sha256"].clone();
            attempt["task_index_sha256"] = index["task_index_sha256"].clone();
            attempt["acceptance_results"] = pending(task_ids(&task));
            attempt["records"] = json!([{"id":"VAL-001","kind":"validation","outcome":"passed","evidence":"Verified."},{"id":"VAL-002","kind":"validation","outcome":"passed","evidence":"Verified."}]);
            let mut row = index["tasks"][0].clone();
            row["id"] = json!(id);
            row["status"] = json!("in_progress");
            row["latest_attempt"] = attempt["attempt_id"].clone();
            row["task_item_sha256"] = attempt["task_item_sha256"].clone();
            row["instructions_sha256"] = attempt["task_instructions_sha256"].clone();
            row["acceptance_results"] = pending(task_ids(&task));
            rows.push(row);
            tasks.push(task);
            attempts.insert(id.into(), attempt);
        }
        index["tasks"] = json!(rows);
        let collection = json!({"acceptance_criteria":[{"id":"ACCEPTANCE-001"}],"tasks":tasks});
        let complete = aggregate_collection(&collection, &index, &attempts).unwrap();
        assert_eq!(complete["acceptance_results"][0]["status"], "completed");
        assert_eq!(
            complete["acceptance_results"][0]["evidence"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            complete["tasks"][1]["acceptance_results"][1]["status"],
            "completed"
        );
        let history = attempts.clone();
        let invalidated = invalidate(&complete, &["TASK-002".into()]).unwrap();
        assert_eq!(invalidated["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            invalidated["tasks"][0]["acceptance_results"],
            complete["tasks"][0]["acceptance_results"]
        );
        assert!(
            invalidated["tasks"][1]["acceptance_results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["status"] == "pending" && row["evidence"] == json!([]))
        );
        let restarted = super::super::start_index(&complete, "TASK-002", "ATTEMPT-002").unwrap();
        assert_eq!(restarted["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            restarted["tasks"][0]["acceptance_results"],
            complete["tasks"][0]["acceptance_results"]
        );
        let corrected = super::super::corrected_index(
            &complete,
            "TASK-002",
            "ATTEMPT-001-CORRECTION-001",
            &["TASK-002".into()],
        )
        .unwrap();
        assert_eq!(corrected["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            corrected["tasks"][0]["acceptance_results"],
            complete["tasks"][0]["acceptance_results"]
        );
        assert_eq!(invalidate(&complete, &[]).unwrap(), complete);
        let (closed, closed_index) = super::super::attempt_close::build_close_candidates(
            &collection["tasks"][1],
            &complete,
            &attempts["TASK-002"],
            &json!({"schema":"work-attempt-close-request","status":"completed"}),
            "2026-10-04T12:00+08:00",
        )
        .unwrap();
        assert_eq!(closed["acceptance_results"][0]["status"], "completed");
        assert_eq!(closed_index["acceptance_results"][0]["status"], "completed");
        let (stopped, stopped_index) = super::super::attempt_close::build_close_candidates(&collection["tasks"][1], &complete, &attempts["TASK-002"], &json!({"schema":"work-attempt-close-request","status":"stopped","final_type":"validation_failed","reason":"Failed verification.","authorization_evidence":"Approved stop."}), "2026-10-04T12:00+08:00").unwrap();
        assert_eq!(stopped["acceptance_results"][0]["status"], "pending");
        assert_eq!(stopped_index["acceptance_results"][0]["status"], "pending");
        let mut logical = collection.clone();
        logical["requirement_id"] = index["requirement_id"].clone();
        logical["spec_id"] = index["task_spec_id"].clone();
        let mut validation = json!({"task_collection_sha256":index["task_collection_sha256"],"task_index_sha256":index["task_index_sha256"],"instructions_sha256":index["task_instructions_sha256"],"hierarchy_selection_sha256":index["hierarchy_selection_sha256"],"skill_selection_sha256":index["skill_selection_sha256"],"task_skill_ids":{},"task_item_sha256":{},"task_instructions_sha256":{}});
        for row in index["tasks"].as_array().unwrap() {
            let id = row["id"].as_str().unwrap();
            validation["task_skill_ids"][id] = row["skill_id"].clone();
            validation["task_item_sha256"][id] = row["task_item_sha256"].clone();
            validation["task_instructions_sha256"][id] = row["instructions_sha256"].clone();
        }
        let revised = crate::specification::update::rebuild_execution_index(
            &complete,
            &logical,
            &validation,
            &["TASK-002".into()],
            "TASK-CHANGE-001",
        )
        .unwrap();
        assert_eq!(revised["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            revised["tasks"][0]["acceptance_results"],
            complete["tasks"][0]["acceptance_results"]
        );
        validation["task_instructions_sha256"]["TASK-002"] = json!("0".repeat(64));
        let audited = crate::specification::update::rebuild_execution_index(
            &complete,
            &logical,
            &validation,
            &[],
            "TASK-CHANGE-002",
        )
        .unwrap();
        assert_eq!(audited["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            audited["tasks"][0]["acceptance_results"],
            complete["tasks"][0]["acceptance_results"]
        );
        assert_eq!(attempts, history);
        let mut missing = attempts.clone();
        missing.get_mut("TASK-002").unwrap()["records"]
            .as_array_mut()
            .unwrap()
            .pop();
        let pending_result = aggregate_collection(&collection, &index, &missing).unwrap();
        assert_eq!(pending_result["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            pending_result["tasks"][1]["acceptance_results"][1]["status"],
            "completed"
        );
        let mut retry = attempts.clone();
        retry.get_mut("TASK-002").unwrap()["records"].as_array_mut().unwrap().push(json!({"id":"VAL-001#1","kind":"validation","outcome":"failed","evidence":"Latest verification failed."}));
        let failed = aggregate_collection(&collection, &index, &retry).unwrap();
        assert_eq!(failed["acceptance_results"][0]["status"], "pending");
        assert_eq!(
            failed["tasks"][1]["acceptance_results"][1]["status"],
            "pending"
        );
        assert_eq!(
            failed["tasks"][0]["acceptance_results"][1]["status"],
            "completed"
        );
        assert!(aggregate_collection(&collection, &index, &BTreeMap::new()).unwrap()["acceptance_results"][0]["evidence"].as_array().unwrap().is_empty());
        retry.get_mut("TASK-002").unwrap()["records"].as_array_mut().unwrap().push(json!({"id":"VAL-001#2","kind":"validation","outcome":"passed","evidence":"Latest retry verified."}));
        let repaired = aggregate_collection(&collection, &index, &retry).unwrap();
        assert_eq!(repaired["acceptance_results"][0]["status"], "completed");
        assert!(
            repaired["acceptance_results"][0]["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|proof| proof["task_id"] == "TASK-002" && proof["record_id"] == "VAL-001#2")
        );
        let mut unauthorized = attempts.clone();
        let active = unauthorized.get_mut("TASK-002").unwrap();
        active["authorization"]["validations"][0]["acceptance_ids"] = json!([]);
        active["authorization_sha256"] =
            json!(crate::canonical::canonical_json_sha256(&active["authorization"]).unwrap());
        assert_eq!(
            aggregate_collection(&collection, &index, &unauthorized).unwrap()["acceptance_results"]
                [0]["status"],
            "pending"
        );
        let mut stale = attempts.clone();
        stale.get_mut("TASK-002").unwrap()["task_item_sha256"] = json!("0".repeat(64));
        assert_eq!(
            aggregate_collection(&collection, &index, &stale)
                .unwrap_err()
                .reason_code,
            "acceptance_evidence_binding_mismatch"
        );
    }
}
