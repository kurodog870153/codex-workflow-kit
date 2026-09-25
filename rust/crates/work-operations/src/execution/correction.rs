//! Correction artifact validation and canonical bytes.

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};

use crate::execution::ExecutionIssue;
use crate::execution::attempt::timestamp;
use crate::execution::index::{render_execution_index, validate_execution_index};
use crate::execution::requests::validate_correction_create_request;
use crate::execution::{affected_task_ids, corrected_index};
use crate::protocol::{ATTEMPT_ID_PREFIX, valid_sha256};

const ORDER: &[&str] = &[
    "schema",
    "correction_id",
    "created_at",
    "target_attempt_id",
    "task_collection_sha256",
    "task_index_sha256",
    "task_item_sha256",
    "task_instructions_sha256",
    "execute_instructions_sha256",
    "field",
    "correct_value",
    "reason",
];

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn validate_correction_file_path(path: &str, correction: &Value) -> Result<(), ExecutionIssue> {
    let parts: Vec<_> = path.split('/').collect();
    let expected = format!(
        "{}.json",
        correction["correction_id"].as_str().unwrap_or("")
    );
    if parts.last().copied() != Some(expected.as_str()) {
        return Err(issue(
            "correction_filename_mismatch",
            "The Correction filename does not match correction_id.",
            json!({}),
        ));
    }
    if parts.iter().rev().nth(1).copied() != Some("corrections")
        || parts.iter().rev().nth(2).copied() != correction["target_attempt_id"].as_str()
    {
        return Err(issue(
            "correction_parent_attempt_mismatch",
            "Corrections must use <TASK-ID>/<ATTEMPT-ID>/corrections/<CORRECTION-ID>.json; legacy flat paths are unsupported.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn validate_correction(value: &Value) -> Result<Value, ExecutionIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            "correction_expected_object",
            "A JSON object is required.",
            json!({}),
        )
    })?;
    if value["schema"] != "work-correction/v1" {
        return Err(issue(
            "correction_invalid_schema",
            "The Correction schema is invalid.",
            json!({}),
        ));
    }
    let mut missing: Vec<_> = ORDER
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    missing.sort_unstable();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !ORDER.contains(&field.as_str()))
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "correction_invalid_fields",
            "The Correction contract has missing or unknown fields.",
            json!({"missing":missing,"unknown":unknown}),
        ));
    }
    let id = value["correction_id"].as_str().unwrap_or("");
    let target = value["target_attempt_id"].as_str().unwrap_or("");
    let valid_attempt = |id: &str| {
        id.strip_prefix(ATTEMPT_ID_PREFIX).is_some_and(|digits| {
            digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    };
    let valid_correction = id
        .split_once("-CORRECTION-")
        .is_some_and(|(attempt, number)| {
            valid_attempt(attempt)
                && number.len() == 3
                && number.bytes().all(|byte| byte.is_ascii_digit())
        });
    if !valid_attempt(target) || !valid_correction {
        return Err(issue(
            "correction_invalid_identifier",
            "Correction and target Attempt IDs must use canonical formats.",
            json!({}),
        ));
    }
    if !id.starts_with(&format!("{target}-CORRECTION-")) {
        return Err(issue(
            "correction_target_mismatch",
            "correction_id must be scoped to target_attempt_id.",
            json!({}),
        ));
    }
    timestamp(&value["created_at"], "created_at").map_err(|_| {
        issue(
            "correction_invalid_timestamp",
            "created_at must use minute-precision ISO 8601 with a numeric offset.",
            json!({}),
        )
    })?;
    for field in [
        "task_collection_sha256",
        "task_index_sha256",
        "task_item_sha256",
        "task_instructions_sha256",
        "execute_instructions_sha256",
    ] {
        if value[field].as_str().is_none_or(|text| !valid_sha256(text)) {
            return Err(issue(
                "correction_invalid_sha256",
                "A lowercase SHA-256 value is required.",
                json!({"location":field}),
            ));
        }
    }
    for field in ["field", "correct_value", "reason"] {
        if value[field]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        {
            return Err(issue(
                "correction_empty_text_value",
                "A non-empty string is required.",
                json!({"location":field}),
            ));
        }
    }
    let _: work_model::execution::correction::Correction =
        serde_json::from_value(value.clone()).expect("validated Correction matches model");
    Ok(
        json!({"schema":"work-correction-validation/v1","correction_id":id,
        "target_attempt_id":target,"result":"valid"}),
    )
}

struct Ordered<'a>(&'a Value);
impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut result = serializer.serialize_map(self.0.as_object().map(|object| object.len()))?;
        for field in ORDER {
            if let Some(value) = self.0.get(*field) {
                result.serialize_entry(field, value)?;
            }
        }
        result.end()
    }
}

pub fn ordered_correction(value: &Value) -> impl Serialize + '_ {
    Ordered(value)
}

pub fn render_correction(value: &Value) -> Result<Vec<u8>, ExecutionIssue> {
    validate_correction(value)?;
    let mut bytes = serde_json::to_vec_pretty(&Ordered(value)).expect("JSON value serializes");
    bytes.push(b'\n');
    Ok(bytes)
}

pub struct CorrectionCandidates {
    pub artifact: Value,
    pub locked_index: Value,
    pub final_index: Value,
    pub affected_task_ids: Vec<String>,
}

pub fn build_correction_candidates(
    collection: &Value,
    index: &Value,
    attempt: &Value,
    task_id: &str,
    request: &Value,
    correction_id: &str,
    created_at: &str,
) -> Result<CorrectionCandidates, ExecutionIssue> {
    validate_correction_create_request(request)?;
    if index.get("lock").is_some() {
        return Err(issue(
            "correction_create_lock_present",
            "A Correction requires an unlocked execution index.",
            json!({"lock":index["lock"]}),
        ));
    }
    if attempt["status"] == "in_progress" {
        return Err(issue(
            "correction_create_attempt_not_closed",
            "A Correction must target a closed Attempt.",
            json!({}),
        ));
    }
    let attempt_id = request["target_attempt_id"].as_str().unwrap_or("");
    if attempt["attempt_id"] != attempt_id || attempt["task_id"] != task_id {
        return Err(issue(
            "correction_create_attempt_identity",
            "The target Attempt identity does not match the request.",
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
                "correction_create_task_not_found",
                "The target TASK is not in the execution index.",
                json!({}),
            )
        })?;
    let invalidates = request["invalidates_completion"] == true;
    if invalidates && row["latest_attempt"] != attempt_id {
        return Err(issue(
            "correction_create_not_latest_attempt",
            "Only the latest Attempt can invalidate the current TASK completion.",
            json!({}),
        ));
    }
    if !correction_id.starts_with(&format!("{attempt_id}-CORRECTION-")) {
        return Err(issue(
            "correction_invalid_identifier",
            "The Correction ID does not match its target Attempt.",
            json!({}),
        ));
    }
    let affected = affected_task_ids(index, collection, task_id, invalidates)?;
    let artifact = json!({
        "schema":"work-correction/v1","correction_id":correction_id,
        "created_at":created_at,"target_attempt_id":attempt_id,
        "task_collection_sha256":attempt["task_collection_sha256"],
        "task_index_sha256":attempt["task_index_sha256"],
        "task_item_sha256":attempt["task_item_sha256"],
        "task_instructions_sha256":attempt["task_instructions_sha256"],
        "execute_instructions_sha256":attempt["execute_instructions_sha256"],
        "field":request["field"],"correct_value":request["correct_value"],
        "reason":request["reason"],
    });
    render_correction(&artifact)?;
    let mut locked_index = index.clone();
    locked_index["lock"] = json!({
        "kind":"correction","task_id":task_id,"attempt_id":attempt_id,
        "correction_id":correction_id,
        "execute_instructions_sha256":attempt["execute_instructions_sha256"],
        "invalidates_completion":invalidates,"affected_task_ids":affected,
    });
    let locked_raw = render_execution_index(&locked_index).expect("JSON index");
    validate_execution_index(&locked_index, &locked_raw)?;
    let final_index = corrected_index(&locked_index, task_id, correction_id, &affected)?;
    let final_raw = render_execution_index(&final_index).expect("JSON index");
    validate_execution_index(&final_index, &final_raw)?;
    Ok(CorrectionCandidates {
        artifact,
        locked_index,
        final_index,
        affected_task_ids: affected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;
    use crate::execution::index::build_initial_execution_index;

    #[test]
    fn correction_candidates_reserve_and_release_formal_index() {
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001","dependencies":[]}]});
        let validation = json!({"task_instructions_sha256":{"TASK-001":"a".repeat(64)},
            "task_skill_ids":{"TASK-001":null},"task_item_sha256":{"TASK-001":"b".repeat(64)},
            "instructions_sha256":"c".repeat(64),"hierarchy_selection_sha256":"d".repeat(64),
            "skill_selection_sha256":"e".repeat(64),"task_collection_sha256":"f".repeat(64),
            "task_index_sha256":"0".repeat(64)});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["tasks"][0]["status"] = json!("completed");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["overall_status"] = json!("completed");
        let attempt = json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001",
            "status":"completed","task_collection_sha256":"f".repeat(64),
            "task_index_sha256":"0".repeat(64),"task_item_sha256":"b".repeat(64),
            "task_instructions_sha256":"a".repeat(64),
            "execute_instructions_sha256":"1".repeat(64)});
        let request = json!({"schema":"work-correction-create-request/v1",
            "target_attempt_id":"ATTEMPT-001","field":"records[0].outcome",
            "correct_value":"passed","reason":"Correct recorded outcome.",
            "invalidates_completion":true});
        let candidates = build_correction_candidates(
            &collection,
            &index,
            &attempt,
            "TASK-001",
            &request,
            "ATTEMPT-001-CORRECTION-001",
            "2026-09-01T10:05+08:00",
        )
        .unwrap();
        assert_eq!(candidates.affected_task_ids, ["TASK-001"]);
        assert_eq!(candidates.locked_index["lock"]["kind"], "correction");
        assert_eq!(
            candidates.locked_index["lock"]["execute_instructions_sha256"],
            "1".repeat(64)
        );
        assert!(
            candidates.locked_index["lock"]
                .get("execute_rules_sha256")
                .is_none()
        );
        assert!(candidates.final_index.get("lock").is_none());
        assert_eq!(
            candidates.final_index["tasks"][0]["status"],
            "pending_retry"
        );
    }

    #[test]
    fn correction_example_matches_python_bytes_and_scope() {
        let value = json!({"schema":"work-correction/v1","correction_id":"ATTEMPT-001-CORRECTION-001",
            "created_at":"2026-09-01T10:05+08:00","target_attempt_id":"ATTEMPT-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),
            "task_item_sha256":"c".repeat(64),"task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),"field":"records[0].outcome",
            "correct_value":"passed","reason":"Correct the recorded outcome."});
        assert_eq!(
            sha256_hex(&render_correction(&value).unwrap()),
            "de80fab70437417d51b6b1f51d4dcdc2c8a5f9f24748c1508fba4c914d293943"
        );
        assert_eq!(validate_correction(&value).unwrap()["result"], "valid");
        let mut mismatch = value;
        mismatch["target_attempt_id"] = json!("ATTEMPT-002");
        assert_eq!(
            validate_correction(&mismatch).unwrap_err().reason_code,
            "correction_target_mismatch"
        );
    }

    #[test]
    fn correction_v1_fingerprints_and_legacy_rejections_match_python() {
        let value = json!({"schema":"work-correction/v1","correction_id":"ATTEMPT-001-CORRECTION-001",
            "created_at":"2026-09-01T10:05+08:00","target_attempt_id":"ATTEMPT-001",
            "task_collection_sha256":"1".repeat(64),"task_index_sha256":"2".repeat(64),
            "task_item_sha256":"3".repeat(64),"task_instructions_sha256":"b".repeat(64),
            "execute_instructions_sha256":"c".repeat(64),"field":"records[0].outcome",
            "correct_value":"passed","reason":"Correct the recorded outcome."});
        assert_eq!(
            validate_correction(&value).unwrap()["schema"],
            "work-correction-validation/v1"
        );
        assert_eq!(value["task_instructions_sha256"], "b".repeat(64));
        assert!(value.get("task_rules_sha256").is_none());
        assert!(value.get("execute_rules_sha256").is_none());
        let mut legacy = value.clone();
        legacy["task_rules_sha256"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("task_instructions_sha256")
            .unwrap();
        legacy["execute_rules_sha256"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("execute_instructions_sha256")
            .unwrap();
        let error = validate_correction(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "correction_invalid_fields");
        assert_eq!(
            error.details["missing"],
            json!(["execute_instructions_sha256", "task_instructions_sha256"])
        );
        assert_eq!(
            error.details["unknown"],
            json!(["execute_rules_sha256", "task_rules_sha256"])
        );
        let mut missing = value.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("task_collection_sha256");
        assert_eq!(
            validate_correction(&missing).unwrap_err().reason_code,
            "correction_invalid_fields"
        );
        let mut retired = value;
        retired["schema"] = json!("work-correction/v2");
        assert_eq!(
            validate_correction(&retired).unwrap_err().reason_code,
            "correction_invalid_schema"
        );
    }

    #[test]
    fn correction_file_path_binds_filename_and_attempt_directory() {
        let correction = json!({"correction_id":"ATTEMPT-001-CORRECTION-001",
            "target_attempt_id":"ATTEMPT-001"});
        validate_correction_file_path(
            "execution/TASK-001/ATTEMPT-001/corrections/ATTEMPT-001-CORRECTION-001.json",
            &correction,
        )
        .unwrap();
        for path in [
            "execution/TASK-001/ATTEMPT-001-CORRECTION-001.json",
            "execution/TASK-001/ATTEMPT-002/corrections/ATTEMPT-001-CORRECTION-001.json",
        ] {
            assert_eq!(
                validate_correction_file_path(path, &correction)
                    .unwrap_err()
                    .reason_code,
                "correction_parent_attempt_mismatch"
            );
        }
    }
}
