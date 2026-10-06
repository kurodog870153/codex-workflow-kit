//! Canonical execution index creation and byte ordering.

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::execution::{ExecutionIssue, derive_overall_status};
use crate::protocol::{ATTEMPT_ID_PREFIX, INVALID_SHA256_ERROR_CODE, TASK_ID_PREFIX, valid_sha256};

const TOP_ORDER: &[&str] = &[
    "schema",
    "requirement_id",
    "title",
    "task_spec_id",
    "task_collection_sha256",
    "task_index_sha256",
    "task_instructions_sha256",
    "hierarchy_selection_sha256",
    "skill_selection_sha256",
    "instruction_selection_manifest",
    "latest_task_instruction_audit",
    "lock",
    "overall_status",
    "acceptance_results",
    "tasks",
];
const LOCK_ORDER: &[&str] = &[
    "kind",
    "record",
    "task_id",
    "attempt_id",
    "correction_id",
    "record_id",
    "retry_authorization_evidence",
    "command_correction",
    "execute_instructions_sha256",
    "invalidates_completion",
    "affected_task_ids",
];
const TASK_ORDER: &[&str] = &[
    "id",
    "status",
    "skill_id",
    "task_item_sha256",
    "instructions_sha256",
    "acceptance_results",
    "latest_attempt",
    "latest_correction",
    "status_reason",
];

struct Ordered<'a> {
    value: &'a Value,
    path: Vec<String>,
}

pub(crate) fn order(path: &[String]) -> &'static [&'static str] {
    if let Some(order) = crate::execution::acceptance::field_order(path) {
        return order;
    }
    if let Some(order) = crate::instruction::refresh::manifest_field_order(path) {
        return order;
    }
    if path.is_empty() {
        TOP_ORDER
    } else if path == ["lock"] {
        LOCK_ORDER
    } else if path == ["tasks"] {
        TASK_ORDER
    } else if path.last().is_some_and(|part| part == "status_reason") {
        &["kind", "ref"]
    } else if path.last().is_some_and(|part| part == "command_correction") {
        &[
            "original_command",
            "actual_command",
            "reason",
            "authorization_evidence",
        ]
    } else if path
        .last()
        .is_some_and(|part| part == "original_command" || part == "actual_command")
    {
        &["mode", "argv", "script"]
    } else {
        &[]
    }
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(object) = self.value.as_object() {
            let priority = order(&self.path);
            let mut result = serializer.serialize_map(Some(object.len()))?;
            for field in priority.iter().copied().chain(
                object
                    .keys()
                    .filter(|field| !priority.contains(&field.as_str()))
                    .map(String::as_str),
            ) {
                if let Some(value) = object.get(field) {
                    let mut path = self.path.clone();
                    path.push(field.into());
                    result.serialize_entry(field, &Ordered { value, path })?;
                }
            }
            result.end()
        } else if let Some(array) = self.value.as_array() {
            let mut result = serializer.serialize_seq(Some(array.len()))?;
            for value in array {
                result.serialize_element(&Ordered {
                    value,
                    path: self.path.clone(),
                })?;
            }
            result.end()
        } else {
            self.value.serialize(serializer)
        }
    }
}

pub fn render_execution_index(index: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut raw = serde_json::to_vec_pretty(&Ordered {
        value: index,
        path: vec![],
    })?;
    raw.push(b'\n');
    Ok(raw)
}

pub fn build_initial_execution_index(
    collection: &Value,
    validation: &Value,
) -> Result<Value, ExecutionIssue> {
    let tasks = collection["tasks"].as_array().ok_or(ExecutionIssue {
        reason_code: "invalid_task_collection",
        message: "TASK collection tasks must be an array.",
        details: json!({}),
    })?;
    let rows: Vec<_> = tasks
        .iter()
        .map(|task| {
            let id = task["id"].as_str().unwrap_or("");
            json!({"id":id,"status":"pending","skill_id":validation["task_skill_ids"][id],
            "task_item_sha256":validation["task_item_sha256"][id],
            "instructions_sha256":validation["task_instructions_sha256"][id],"acceptance_results":crate::execution::acceptance::pending(crate::execution::acceptance::task_ids(task))})
        })
        .collect();
    let index = json!({"schema":"work-execution-index","requirement_id":collection["requirement_id"],
        "title":"Execution","task_spec_id":collection["spec_id"],
        "task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_instructions_sha256":validation["instructions_sha256"],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"],
        "skill_selection_sha256":validation["skill_selection_sha256"],
        "overall_status":"pending","tasks":rows,"acceptance_results":crate::execution::acceptance::pending(collection["acceptance_criteria"].as_array().into_iter().flatten().filter_map(|row|row["id"].as_str().map(str::to_owned)))});
    let _: work_model::execution::index::ExecutionIndex =
        serde_json::from_value(index.clone()).expect("initial execution index matches model");
    Ok(index)
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

fn id(value: &Value, prefix: &str, digits: usize) -> bool {
    value
        .as_str()
        .and_then(|text| text.strip_prefix(prefix))
        .is_some_and(|tail| tail.len() == digits && tail.bytes().all(|byte| byte.is_ascii_digit()))
}

fn sha(value: &Value) -> bool {
    value.as_str().is_some_and(valid_sha256)
}

fn strict_fields(
    value: &Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), ExecutionIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location":location}),
        )
    })?;
    let missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":location,"missing":missing,"unknown":unknown}),
        ));
    }
    Ok(())
}

pub fn validate_execution_index(index: &Value, raw: &[u8]) -> Result<Value, ExecutionIssue> {
    if index["schema"] != "work-execution-index" {
        return Err(issue(
            "invalid_execution_index_schema",
            "Invalid execution index schema.",
            json!({}),
        ));
    }
    strict_fields(
        index,
        "execution_index",
        &[
            "schema",
            "requirement_id",
            "title",
            "task_spec_id",
            "task_collection_sha256",
            "task_index_sha256",
            "task_instructions_sha256",
            "hierarchy_selection_sha256",
            "skill_selection_sha256",
            "overall_status",
            "acceptance_results",
            "tasks",
        ],
        &[
            "instruction_selection_manifest",
            "latest_task_instruction_audit",
            "lock",
        ],
    )?;
    if !index["title"]
        .as_str()
        .is_some_and(|text| !text.trim().is_empty() && !text.contains(['\n', '\r']))
    {
        return Err(issue(
            "invalid_execution_index_title",
            "The execution index title must fit on one line.",
            json!({}),
        ));
    }
    if index["requirement_id"]
        .as_str()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(issue(
            "empty_text_value",
            "A non-empty string is required.",
            json!({"location":"requirement_id"}),
        ));
    }
    if !id(&index["task_spec_id"], "TASK-SPEC-", 3) {
        return Err(issue(
            "invalid_task_spec_id",
            "The execution index TASK spec ID is invalid.",
            json!({}),
        ));
    }
    for field in [
        "task_collection_sha256",
        "task_index_sha256",
        "task_instructions_sha256",
        "hierarchy_selection_sha256",
        "skill_selection_sha256",
    ] {
        if !sha(&index[field]) {
            return Err(issue(
                INVALID_SHA256_ERROR_CODE,
                "A lowercase SHA-256 value is required.",
                json!({"location":field}),
            ));
        }
    }
    if index
        .get("latest_task_instruction_audit")
        .is_some_and(|value| !id(value, "TASK-INSTRUCTION-AUDIT-", 3))
    {
        return Err(issue(
            "invalid_task_instruction_audit_id",
            "The latest Task instruction audit ID is invalid.",
            json!({}),
        ));
    }
    let bindings: std::collections::BTreeMap<_, _> = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                row["id"].as_str()?.to_owned(),
                (
                    row["task_item_sha256"].as_str()?.to_owned(),
                    row["instructions_sha256"].as_str()?.to_owned(),
                ),
            ))
        })
        .collect();
    crate::execution::acceptance::validate(&index["acceptance_results"], &bindings, None, None)?;
    for row in index["tasks"].as_array().into_iter().flatten() {
        crate::execution::acceptance::validate(
            &row["acceptance_results"],
            &bindings,
            None,
            row["id"].as_str(),
        )?;
    }
    if let Some(lock) = index.get("lock") {
        validate_lock(lock)?;
    }
    let rows = index["tasks"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_item_array",
                "The execution index tasks array must be non-empty.",
                json!({}),
            )
        })?;
    let mut previous = 0;
    let mut statuses = Vec::with_capacity(rows.len());
    for (position, row) in rows.iter().enumerate() {
        strict_fields(
            row,
            &format!("tasks[{position}]"),
            &[
                "id",
                "status",
                "skill_id",
                "task_item_sha256",
                "instructions_sha256",
                "acceptance_results",
            ],
            &["latest_attempt", "latest_correction", "status_reason"],
        )?;
        let number = row["id"]
            .as_str()
            .and_then(|text| text.strip_prefix(TASK_ID_PREFIX))
            .filter(|digits| digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|digits| digits.parse::<u16>().ok())
            .unwrap_or(0);
        if number <= previous {
            return Err(issue(
                "invalid_or_unsorted_id",
                "Execution index TASK IDs must be ascending.",
                json!({}),
            ));
        }
        previous = number;
        let status = row["status"].as_str().unwrap_or("");
        if !matches!(
            status,
            "pending" | "in_progress" | "pending_retry" | "blocked" | "completed" | "cancelled"
        ) {
            return Err(issue(
                "invalid_task_status",
                "Invalid TASK status.",
                json!({}),
            ));
        }
        if !row["skill_id"].is_null()
            && row["skill_id"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(issue(
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location":format!("{}.skill_id",row["id"])}),
            ));
        }
        for field in ["task_item_sha256", "instructions_sha256"] {
            if !sha(&row[field]) {
                return Err(issue(
                    INVALID_SHA256_ERROR_CODE,
                    "A lowercase SHA-256 value is required.",
                    json!({"location":format!("{}.{}",row["id"],field)}),
                ));
            }
        }
        if row
            .get("latest_attempt")
            .is_some_and(|value| !id(value, ATTEMPT_ID_PREFIX, 3))
        {
            return Err(issue(
                "invalid_attempt_id",
                "Invalid latest Attempt ID.",
                json!({}),
            ));
        }
        if row
            .get("latest_correction")
            .is_some_and(|value| !correction_id(value))
        {
            return Err(issue(
                "invalid_correction_id",
                "Invalid latest Correction ID.",
                json!({}),
            ));
        }
        if let Some(reason) = row.get("status_reason") {
            strict_fields(
                reason,
                &format!("{}.status_reason", row["id"]),
                &["kind", "ref"],
                &[],
            )?;
            if !matches!(
                reason["kind"].as_str(),
                Some("attempt" | "correction" | "task_change" | "instruction_audit")
            ) || reason["ref"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
                || reason["kind"] == "correction" && !correction_id(&reason["ref"])
                || reason["kind"] == "instruction_audit"
                    && !id(&reason["ref"], "TASK-INSTRUCTION-AUDIT-", 3)
            {
                return Err(issue(
                    "invalid_status_reason",
                    "Invalid status reason kind or reference.",
                    json!({}),
                ));
            }
        }
        if matches!(status, "in_progress" | "completed") && row.get("latest_attempt").is_none() {
            return Err(issue(
                "latest_attempt_required",
                "This TASK status requires latest_attempt.",
                json!({}),
            ));
        }
        if matches!(status, "pending_retry" | "blocked" | "cancelled")
            && row.get("status_reason").is_none()
        {
            return Err(issue(
                "status_reason_required",
                "This TASK status requires status_reason.",
                json!({}),
            ));
        }
        if status == "pending"
            && ["latest_attempt", "latest_correction", "status_reason"]
                .iter()
                .any(|field| row.get(*field).is_some())
        {
            return Err(issue(
                "invalid_pending_metadata",
                "An initial pending TASK cannot have attempt metadata.",
                json!({}),
            ));
        }
        statuses.push(status);
    }
    if index["overall_status"] != derive_overall_status(&statuses) {
        return Err(issue(
            "overall_status_mismatch",
            "overall_status does not match the TASK statuses.",
            json!({}),
        ));
    }
    let expected = render_execution_index(index).expect("JSON value serializes");
    if raw != expected {
        return Err(issue(
            "noncanonical_json_contract",
            "The JSON document must use canonical Work formatting.",
            json!({}),
        ));
    }
    let _: work_model::execution::index::ExecutionIndex =
        serde_json::from_value(index.clone()).expect("validated execution index matches model");
    Ok(
        json!({"schema":"work-execution-index-validation","requirement_id":index["requirement_id"],
        "task_spec_id":index["task_spec_id"],"overall_status":index["overall_status"],
        "index_sha256":sha256_hex(raw),"task_count":rows.len()}),
    )
}

fn correction_id(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|text| text.split_once("-CORRECTION-"))
        .is_some_and(|(attempt, tail)| {
            id(&json!(attempt), ATTEMPT_ID_PREFIX, 3)
                && tail.len() == 3
                && tail.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn validate_lock(lock: &Value) -> Result<(), ExecutionIssue> {
    match lock["kind"].as_str() {
        Some("spec_update") => {
            strict_fields(lock, "lock", &["kind", "record"], &[])?;
            if !id(&lock["record"], "SPEC-UPDATE-", 3) {
                return Err(issue(
                    "invalid_spec_lock",
                    "Invalid spec lock record.",
                    json!({}),
                ));
            }
        }
        Some("execution") => {
            strict_fields(
                lock,
                "lock",
                &[
                    "kind",
                    "task_id",
                    "attempt_id",
                    "execute_instructions_sha256",
                ],
                &[
                    "record_id",
                    "retry_authorization_evidence",
                    "command_correction",
                ],
            )?;
            if !id(&lock["task_id"], TASK_ID_PREFIX, 3)
                || !id(&lock["attempt_id"], ATTEMPT_ID_PREFIX, 3)
            {
                return Err(issue(
                    "invalid_execution_lock",
                    "Invalid execution lock IDs.",
                    json!({}),
                ));
            }
            if !sha(&lock["execute_instructions_sha256"]) {
                return Err(issue(
                    INVALID_SHA256_ERROR_CODE,
                    "A lowercase SHA-256 value is required.",
                    json!({"location":"lock.execute_instructions_sha256"}),
                ));
            }
            if let Some(record) = lock.get("record_id") {
                if !record_id(record) {
                    return Err(issue(
                        "invalid_execution_lock_record_id",
                        "Invalid execution lock record ID.",
                        json!({}),
                    ));
                }
            }
            let retry_record = lock["record_id"]
                .as_str()
                .is_some_and(|record| record.contains('#'));
            let retry_evidence = lock.get("retry_authorization_evidence");
            if retry_record != retry_evidence.is_some()
                || retry_record
                    && retry_evidence
                        .and_then(Value::as_str)
                        .is_none_or(|text| text.trim().is_empty())
            {
                return Err(issue(
                    "invalid_execution_lock_retry_evidence",
                    "A reserved retry record requires fresh authorization evidence; a base record cannot carry it.",
                    json!({}),
                ));
            }
            if lock.get("command_correction").is_some()
                && !lock["record_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("CMD-"))
            {
                return Err(issue(
                    "invalid_execution_lock_command_correction",
                    "A command correction requires a reserved CMD record.",
                    json!({}),
                ));
            }
            if let Some(correction) = lock.get("command_correction") {
                crate::execution::command_correction::validate_command_correction(
                    correction,
                    "lock.command_correction",
                )?;
            }
        }
        Some("correction") => {
            strict_fields(
                lock,
                "lock",
                &[
                    "kind",
                    "task_id",
                    "attempt_id",
                    "correction_id",
                    "execute_instructions_sha256",
                    "invalidates_completion",
                    "affected_task_ids",
                ],
                &[],
            )?;
            if !id(&lock["task_id"], TASK_ID_PREFIX, 3)
                || !id(&lock["attempt_id"], ATTEMPT_ID_PREFIX, 3)
                || !correction_id(&lock["correction_id"])
                || !lock["correction_id"].as_str().is_some_and(|id| {
                    id.starts_with(&format!(
                        "{}-CORRECTION-",
                        lock["attempt_id"].as_str().unwrap_or("")
                    ))
                })
            {
                return Err(issue(
                    "invalid_correction_lock",
                    "Invalid Correction lock IDs.",
                    json!({}),
                ));
            }
            if !sha(&lock["execute_instructions_sha256"]) {
                return Err(issue(
                    INVALID_SHA256_ERROR_CODE,
                    "A lowercase SHA-256 value is required.",
                    json!({"location":"lock.execute_instructions_sha256"}),
                ));
            }
            let invalidates = lock["invalidates_completion"].as_bool().ok_or_else(|| {
                issue(
                    "invalid_correction_lock_invalidation",
                    "A Correction lock invalidation flag must be boolean.",
                    json!({}),
                )
            })?;
            let affected = lock["affected_task_ids"].as_array().ok_or_else(|| {
                issue(
                    "invalid_correction_lock_affected_tasks",
                    "A Correction lock affected TASK list is invalid.",
                    json!({}),
                )
            })?;
            let ids: Vec<_> = affected.iter().filter_map(Value::as_str).collect();
            let mut sorted = ids.clone();
            sorted.sort();
            sorted.dedup();
            if ids.len() != affected.len()
                || ids != sorted
                || ids.iter().any(|id| !self_id(id))
                || !invalidates && !ids.is_empty()
                || invalidates && !ids.contains(&lock["task_id"].as_str().unwrap_or(""))
            {
                return Err(issue(
                    "invalid_correction_lock_affected_tasks",
                    "A Correction lock affected TASK list is invalid.",
                    json!({}),
                ));
            }
        }
        _ => {
            return Err(issue(
                "invalid_lock_kind",
                "Invalid execution index lock kind.",
                json!({}),
            ));
        }
    }
    Ok(())
}

fn self_id(value: &str) -> bool {
    id(&json!(value), TASK_ID_PREFIX, 3)
}

fn record_id(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let (base, retry) = text.split_once('#').unwrap_or((text, ""));
    let base_valid = ["CMD-", "OP-", "VAL-"]
        .iter()
        .any(|prefix| id(&json!(base), prefix, 3));
    base_valid
        && (retry.is_empty()
            || retry.starts_with(|c: char| ('1'..='9').contains(&c))
                && retry.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;

    fn current_contract_instruction_index() -> Value {
        let collection = json!({"requirement_id":"example","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001","skill_id":null}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"1".repeat(64),"task_item_sha256":{"TASK-001":"2".repeat(64)},
            "instructions_sha256":"b".repeat(64),
            "task_instructions_sha256":{"TASK-001":"c".repeat(64)},
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"d".repeat(64),"task_skill_ids":{"TASK-001":null}});
        build_initial_execution_index(&collection, &validation).unwrap()
    }

    fn validate_instruction_fixture(index: &Value) -> Result<Value, ExecutionIssue> {
        validate_execution_index(index, &render_execution_index(index).unwrap())
    }

    #[test]
    fn initial_progress_covers_shared_and_technical_criteria_without_completion() {
        let collection = json!({"requirement_id":"example","spec_id":"TASK-SPEC-001","acceptance_criteria":[{"id":"ACCEPTANCE-001","criterion":"Shared result."}],"tasks":[
            {"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001","criterion":"First technical result."}]},
            {"id":"TASK-002","traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":"TASK-002-ACCEPTANCE-001","criterion":"Second technical result."}]}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),"instructions_sha256":"c".repeat(64),"hierarchy_selection_sha256":"d".repeat(64),"skill_selection_sha256":"e".repeat(64),"task_skill_ids":{"TASK-001":null,"TASK-002":null},"task_item_sha256":{"TASK-001":"f".repeat(64),"TASK-002":"0".repeat(64)},"task_instructions_sha256":{"TASK-001":"1".repeat(64),"TASK-002":"2".repeat(64)}});
        let index = build_initial_execution_index(&collection, &validation).unwrap();
        assert_eq!(
            index["acceptance_results"],
            json!([{"id":"ACCEPTANCE-001","status":"pending","evidence":[]}])
        );
        for (position, id) in ["TASK-001", "TASK-002"].into_iter().enumerate() {
            assert_eq!(index["tasks"][position]["status"], "pending");
            assert_eq!(
                index["tasks"][position]["acceptance_results"],
                json!([{"id":"ACCEPTANCE-001","status":"pending","evidence":[]},{"id":format!("{id}-ACCEPTANCE-001"),"status":"pending","evidence":[]}])
            );
        }
        let raw = render_execution_index(&index).unwrap();
        validate_execution_index(&index, &raw).unwrap();
        let typed: work_model::execution::index::ExecutionIndex =
            serde_json::from_slice(&raw).unwrap();
        assert_eq!(serde_json::to_value(typed).unwrap(), index);
        let mut missing = index.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("acceptance_results");
        assert!(
            validate_execution_index(&missing, &render_execution_index(&missing).unwrap()).is_err()
        );
        let mut missing_row = index.clone();
        missing_row["tasks"][0]
            .as_object_mut()
            .unwrap()
            .remove("acceptance_results");
        assert!(
            validate_execution_index(&missing_row, &render_execution_index(&missing_row).unwrap())
                .is_err()
        );
        let mut partial = index.clone();
        partial["tasks"][1]["acceptance_results"] = json!([]);
        assert_eq!(
            crate::execution::acceptance::require_collection(&partial, &collection)
                .unwrap_err()
                .reason_code,
            "acceptance_definition_mismatch"
        );
    }

    #[test]
    fn instruction_index_current_fields_order_and_legacy_rejections_match_current_contract() {
        let index = current_contract_instruction_index();
        let result = validate_instruction_fixture(&index).unwrap();
        assert_eq!(index["schema"], "work-execution-index");
        assert_eq!(result["schema"], "work-execution-index-validation");
        assert_eq!(result["overall_status"], "pending");
        for (field, value) in [
            ("task_collection_sha256", "a"),
            ("task_index_sha256", "1"),
            ("task_instructions_sha256", "b"),
            ("hierarchy_selection_sha256", "f"),
            ("skill_selection_sha256", "d"),
        ] {
            assert_eq!(index[field], value.repeat(64));
        }
        assert_eq!(index["tasks"][0]["skill_id"], Value::Null);
        assert_eq!(index["tasks"][0]["task_item_sha256"], "2".repeat(64));
        assert_eq!(index["tasks"][0]["instructions_sha256"], "c".repeat(64));
        assert!(index.get("task_sha256").is_none());
        assert!(index.get("task_rules_sha256").is_none());
        assert!(index["tasks"][0].get("rules_sha256").is_none());
        let raw = String::from_utf8(render_execution_index(&index).unwrap()).unwrap();
        let fields = [
            "task_collection_sha256",
            "task_index_sha256",
            "task_instructions_sha256",
            "hierarchy_selection_sha256",
            "skill_selection_sha256",
        ];
        let positions: Vec<_> = fields
            .iter()
            .map(|field| raw.find(&format!("  \"{field}\"")).unwrap())
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        let task_fields = [
            "id",
            "status",
            "skill_id",
            "task_item_sha256",
            "instructions_sha256",
        ];
        let task = raw.split("\"tasks\": [").nth(1).unwrap();
        let positions: Vec<_> = task_fields
            .iter()
            .map(|field| task.find(&format!("\"{field}\"")).unwrap())
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        let mut missing_skill = index.clone();
        missing_skill
            .as_object_mut()
            .unwrap()
            .remove("skill_selection_sha256");
        let error = validate_instruction_fixture(&missing_skill).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["missing"], json!(["skill_selection_sha256"]));
        let mut legacy = index.clone();
        legacy["task_rules_sha256"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("task_instructions_sha256")
            .unwrap();
        let error = validate_instruction_fixture(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(
            error.details["missing"],
            json!(["task_instructions_sha256"])
        );
        assert_eq!(error.details["unknown"], json!(["task_rules_sha256"]));
        let mut legacy = index.clone();
        legacy["tasks"][0]["rules_sha256"] = legacy["tasks"][0]
            .as_object_mut()
            .unwrap()
            .remove("instructions_sha256")
            .unwrap();
        let error = validate_instruction_fixture(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["missing"], json!(["instructions_sha256"]));
        assert_eq!(error.details["unknown"], json!(["rules_sha256"]));
        let mut legacy = index.clone();
        legacy["task_sha256"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("task_collection_sha256")
            .unwrap();
        legacy.as_object_mut().unwrap().remove("task_index_sha256");
        let error = validate_instruction_fixture(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(
            error.details["missing"],
            json!(["task_collection_sha256", "task_index_sha256"])
        );
        assert_eq!(error.details["unknown"], json!(["task_sha256"]));
        let mut legacy = index.clone();
        legacy["source_v1_task_sha256"] = json!("8".repeat(64));
        let error = validate_instruction_fixture(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["unknown"], json!(["source_v1_task_sha256"]));
        let mut retired = index;
        retired["schema"] = json!("work-execution-index/v2");
        assert_eq!(
            validate_instruction_fixture(&retired)
                .unwrap_err()
                .reason_code,
            "invalid_execution_index_schema"
        );
    }

    #[test]
    fn instruction_audit_fields_and_legacy_rejections_match_current_contract() {
        let mut index = current_contract_instruction_index();
        index["latest_task_instruction_audit"] = json!("TASK-INSTRUCTION-AUDIT-001");
        index["overall_status"] = json!("blocked");
        index["tasks"][0]["status"] = json!("blocked");
        index["tasks"][0]["status_reason"] = json!({"kind":"instruction_audit",
            "ref":"TASK-INSTRUCTION-AUDIT-001"});
        assert_eq!(
            validate_instruction_fixture(&index).unwrap()["overall_status"],
            "blocked"
        );
        let raw = String::from_utf8(render_execution_index(&index).unwrap()).unwrap();
        assert!(
            raw.find("\"skill_selection_sha256\"").unwrap()
                < raw.find("\"latest_task_instruction_audit\"").unwrap()
        );
        assert!(
            raw.find("\"latest_task_instruction_audit\"").unwrap()
                < raw.find("\"overall_status\"").unwrap()
        );
        let mut legacy = index.clone();
        legacy["latest_task_rule_audit"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("latest_task_instruction_audit")
            .unwrap();
        let error = validate_instruction_fixture(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["unknown"], json!(["latest_task_rule_audit"]));
        let mut legacy = index.clone();
        legacy["tasks"][0]["status_reason"]["kind"] = json!("rule_audit");
        assert_eq!(
            validate_instruction_fixture(&legacy)
                .unwrap_err()
                .reason_code,
            "invalid_status_reason"
        );
        index["latest_task_instruction_audit"] = json!("TASK-RULE-AUDIT-001");
        assert_eq!(
            validate_instruction_fixture(&index)
                .unwrap_err()
                .reason_code,
            "invalid_task_instruction_audit_id"
        );
    }

    #[test]
    fn retry_lock_requires_evidence_and_clears_after_finish() {
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_instructions_sha256":{"TASK-001":"a".repeat(64)},
            "task_skill_ids":{"TASK-001":null},"task_item_sha256":{"TASK-001":"b".repeat(64)},
            "instructions_sha256":"c".repeat(64),"hierarchy_selection_sha256":"d".repeat(64),
            "skill_selection_sha256":"e".repeat(64),"task_collection_sha256":"f".repeat(64),
            "task_index_sha256":"0".repeat(64)});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["overall_status"] = json!("in_progress");
        index["lock"] = json!({"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","execute_instructions_sha256":"1".repeat(64),
            "record_id":"CMD-001#1","retry_authorization_evidence":"Approved retry 1."});
        validate_execution_index(&index, &render_execution_index(&index).unwrap()).unwrap();
        let mut invalid = index.clone();
        invalid["lock"]["retry_authorization_evidence"] = json!(" ");
        assert_eq!(
            validate_execution_index(&invalid, &render_execution_index(&invalid).unwrap())
                .unwrap_err()
                .reason_code,
            "invalid_execution_lock_retry_evidence"
        );
        invalid["lock"]
            .as_object_mut()
            .unwrap()
            .remove("retry_authorization_evidence");
        assert_eq!(
            validate_execution_index(&invalid, &render_execution_index(&invalid).unwrap())
                .unwrap_err()
                .reason_code,
            "invalid_execution_lock_retry_evidence"
        );
        invalid = index.clone();
        invalid["lock"]["record_id"] = json!("CMD-001");
        assert_eq!(
            validate_execution_index(&invalid, &render_execution_index(&invalid).unwrap())
                .unwrap_err()
                .reason_code,
            "invalid_execution_lock_retry_evidence"
        );
        let finished = crate::execution::finished_record_index(&index).unwrap();
        validate_execution_index(&finished, &render_execution_index(&finished).unwrap()).unwrap();
        assert!(
            finished["lock"]
                .get("retry_authorization_evidence")
                .is_none()
        );
    }

    #[test]
    fn initial_index_matches_contract_bytes() {
        let collection =
            json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001","tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_instructions_sha256":{"TASK-001":"a".repeat(64)},"task_skill_ids":{"TASK-001":null},
            "task_item_sha256":{"TASK-001":"b".repeat(64)},"instructions_sha256":"c".repeat(64),
            "hierarchy_selection_sha256":"d".repeat(64),"skill_selection_sha256":"e".repeat(64),
            "task_collection_sha256":"f".repeat(64),"task_index_sha256":"0".repeat(64)});
        let index = build_initial_execution_index(&collection, &validation).unwrap();
        let raw = render_execution_index(&index).unwrap();
        assert_eq!(
            sha256_hex(&raw),
            "7a41e7ea05816a69b9d456cd84e3c3598d450d40027ddc397182fa8e6ec87b7e"
        );
        assert_eq!(
            validate_execution_index(&index, &raw).unwrap()["task_count"],
            1
        );
        let mut locked = index.clone();
        locked["lock"] =
            crate::execution::build_execution_lock("TASK-001", "ATTEMPT-001", &"c".repeat(64));
        assert_eq!(
            locked["lock"]["execute_instructions_sha256"],
            "c".repeat(64)
        );
        assert!(locked["lock"].get("execute_rules_sha256").is_none());
        assert_eq!(
            validate_execution_index(&locked, &render_execution_index(&locked).unwrap()).unwrap()["overall_status"],
            "pending"
        );
        let mut stale = index.clone();
        stale["tasks"][0]["status"] = json!("completed");
        assert_eq!(
            validate_execution_index(&stale, &render_execution_index(&stale).unwrap())
                .unwrap_err()
                .reason_code,
            "latest_attempt_required"
        );
    }

    #[test]
    fn ordering_preserves_task_reason_lock_and_command_fields() {
        let value = json!({"acceptance_results":[],
            "zzz": 2, "aaa": 1,
            "schema": "work-execution-index",
            "task_collection_sha256": "a", "task_index_sha256": "b",
            "overall_status": "in_progress",
            "tasks": [{"status_reason": {"ref": "ATTEMPT-001", "kind": "attempt"},
                "latest_attempt": "ATTEMPT-001", "status": "in_progress", "id": "TASK-001"}],
            "lock": {"execute_instructions_sha256": "c", "record_id": "CMD-001",
                "attempt_id": "ATTEMPT-001", "task_id": "TASK-001", "kind": "execution",
                "command_correction": {"authorization_evidence": "Approved.", "reason": "Use new argument.",
                    "actual_command": {"argv": ["tool", "new"], "mode": "argv"},
                    "original_command": {"argv": ["tool", "old"], "mode": "argv"}}}
        });
        let output = String::from_utf8(render_execution_index(&value).unwrap()).unwrap();
        let positions = |indent: &str, fields: &[&str]| -> Vec<usize> {
            fields
                .iter()
                .map(|field| output.find(&format!("\n{indent}\"{field}\"")).unwrap())
                .collect()
        };
        for (indent, fields) in [
            (
                "  ",
                vec![
                    "schema",
                    "task_collection_sha256",
                    "task_index_sha256",
                    "lock",
                    "overall_status",
                    "tasks",
                    "aaa",
                    "zzz",
                ],
            ),
            (
                "    ",
                vec![
                    "kind",
                    "task_id",
                    "attempt_id",
                    "record_id",
                    "command_correction",
                    "execute_instructions_sha256",
                ],
            ),
            (
                "      ",
                vec![
                    "original_command",
                    "actual_command",
                    "reason",
                    "authorization_evidence",
                ],
            ),
            ("        ", vec!["mode", "argv"]),
        ] {
            let positions = positions(indent, &fields);
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        }
        assert!(output.contains(
            "\n      \"id\": \"TASK-001\",
      \"status\": \"in_progress\","
        ));
        assert!(output.contains(
            "\n        \"kind\": \"attempt\",
        \"ref\": \"ATTEMPT-001\""
        ));
    }
}
