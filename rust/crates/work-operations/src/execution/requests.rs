//! Execution request contracts and their stable rejection reasons.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::execution::ExecutionIssue;
use crate::execution::attempt::validate_authorization;
use crate::protocol::{ATTEMPT_ID_PREFIX, valid_sha256};

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

fn fields(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    reason: &'static str,
    message: &'static str,
) -> Result<(), ExecutionIssue> {
    let object = value
        .as_object()
        .ok_or_else(|| issue(reason, message, json!({})))?;
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
            reason,
            message,
            json!({"missing":missing,"unknown":unknown}),
        ));
    }
    Ok(())
}

fn attempt_id(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|id| id.strip_prefix(ATTEMPT_ID_PREFIX))
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

fn sha(value: &Value) -> bool {
    value.as_str().is_some_and(valid_sha256)
}

pub fn validate_attempt_start_request(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &["schema", "worktree_snapshot_sha256", "authorization"],
        &["continuation"],
        "attempt_start_invalid_object_fields",
        "The JSON object has missing or unknown fields.",
    )?;
    if value["schema"] != "work-attempt-start-request/v1" {
        return Err(issue(
            "attempt_start_invalid_schema",
            "The Attempt-start request schema is invalid.",
            json!({}),
        ));
    }
    if !sha(&value["worktree_snapshot_sha256"]) {
        return Err(issue(
            "attempt_start_invalid_worktree_snapshot",
            "A lowercase worktree snapshot SHA-256 is required.",
            json!({}),
        ));
    }
    if !value["authorization"].is_object() {
        return Err(issue(
            "attempt_start_invalid_object_fields",
            "The Attempt-start request is invalid.",
            json!({}),
        ));
    }
    validate_authorization(&value["authorization"]).map_err(|authorization_issue| {
        if authorization_issue.reason_code == "attempt_invalid_object_fields" {
            let mut missing = authorization_issue.details["missing"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            missing.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            return issue(
                "attempt_start_invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                json!({"location":"authorization","missing":missing,
                    "unknown":authorization_issue.details["unknown"]}),
            );
        }
        issue(
            "attempt_start_invalid_object_fields",
            "The Attempt-start request is invalid.",
            json!({}),
        )
    })?;
    if let Some(continuation) = value.get("continuation").filter(|value| !value.is_null()) {
        fields(
            continuation,
            &["source_attempt_id", "carried_records"],
            &[],
            "attempt_start_invalid_object_fields",
            "The JSON object has missing or unknown fields.",
        )?;
        if !attempt_id(&continuation["source_attempt_id"]) {
            return Err(issue(
                "attempt_start_invalid_source_attempt",
                "The continuation source Attempt ID is invalid.",
                json!({}),
            ));
        }
        let carried = continuation["carried_records"].as_array().ok_or_else(|| {
            issue(
                "attempt_start_invalid_carried_records",
                "carried_records must be an array.",
                json!({}),
            )
        })?;
        let mut seen = HashSet::new();
        for row in carried {
            fields(
                row,
                &["record_id", "evidence"],
                &[],
                "attempt_start_invalid_object_fields",
                "The JSON object has missing or unknown fields.",
            )?;
            let id = row["record_id"].as_str().unwrap_or("");
            if !record_id(id) {
                return Err(issue(
                    "attempt_start_invalid_carried_record_id",
                    "A carried record ID is invalid.",
                    json!({}),
                ));
            }
            if row["evidence"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
            {
                return Err(issue(
                    "attempt_start_empty_text_value",
                    "A non-empty string is required.",
                    json!({}),
                ));
            }
            if !seen.insert(id) {
                return Err(issue(
                    "attempt_start_duplicate_carried_record",
                    "A carried record ID cannot be repeated.",
                    json!({"record_id":id}),
                ));
            }
        }
    }
    work_model::execution::request::verified::<work_model::execution::request::AttemptStartRequest>(
        value,
    );
    Ok(())
}

fn record_id(id: &str) -> bool {
    let (base, retry) = id.split_once('#').unwrap_or((id, ""));
    let base_valid = ["STEP-", "CMD-", "OP-", "VAL-"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|digits| {
            digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    });
    base_valid
        && (retry.is_empty() && !id.contains('#')
            || !retry.is_empty() && retry.bytes().all(|byte| byte.is_ascii_digit()))
}

pub fn validate_attempt_close_request(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &["schema", "status"],
        &["final_type", "reason", "authorization_evidence"],
        "attempt_close_invalid_fields",
        "The attempt-close request has missing or unknown fields.",
    )?;
    if value["schema"] != "work-attempt-close-request/v1" {
        return Err(issue(
            "attempt_close_invalid_schema",
            "The attempt-close request schema is invalid.",
            json!({}),
        ));
    }
    let status = value["status"].as_str().unwrap_or("");
    if !matches!(status, "completed" | "stopped" | "blocked") {
        return Err(issue(
            "attempt_close_invalid_status",
            "status must be completed, stopped, or blocked.",
            json!({"status":value["status"]}),
        ));
    }
    let present: Vec<_> = ["final_type", "reason"]
        .iter()
        .filter(|field| value.get(**field).is_some_and(|value| !value.is_null()))
        .copied()
        .collect();
    if status == "completed" {
        if !present.is_empty() {
            return Err(issue(
                "attempt_close_unexpected_final_details",
                "A completed Attempt cannot include final_type or reason.",
                json!({"fields":present}),
            ));
        }
        if value
            .get("authorization_evidence")
            .is_some_and(|value| !value.is_null())
        {
            return Err(issue(
                "attempt_close_unexpected_authorization_evidence",
                "A completed Attempt reuses its manifest authorization.",
                json!({}),
            ));
        }
    } else {
        let missing: Vec<_> = ["final_type", "reason"]
            .iter()
            .filter(|field| !present.contains(field))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(issue(
                "attempt_close_missing_final_details",
                "A stopped or blocked Attempt requires final_type and reason.",
                json!({"missing":missing}),
            ));
        }
        if value["authorization_evidence"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        {
            return Err(issue(
                "attempt_close_missing_authorization_evidence",
                "A stopped or blocked Attempt requires fresh authorization evidence.",
                json!({}),
            ));
        }
        if ["final_type", "reason"].iter().any(|field| {
            value[*field]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
        }) {
            return Err(issue(
                "attempt_close_empty_final_detail",
                "final_type and reason must be non-empty strings.",
                json!({}),
            ));
        }
    }
    work_model::execution::request::verified::<work_model::execution::request::AttemptCloseRequest>(
        value,
    );
    Ok(())
}

pub fn validate_record_finish_request(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &["schema", "record"],
        &["modified_files", "authorization_evidence"],
        "record_finish_invalid_fields",
        "The record-finish request has missing or unknown fields.",
    )?;
    if value["schema"] != "work-record-finish-request/v1" {
        return Err(issue(
            "record_finish_invalid_schema",
            "The record-finish request schema is invalid.",
            json!({}),
        ));
    }
    let record = value["record"].as_object().ok_or_else(|| {
        issue(
            "record_finish_invalid_record",
            "record must be a JSON object.",
            json!({}),
        )
    })?;
    let repeated: Vec<_> = ["correction", "id", "kind"]
        .iter()
        .filter(|field| record.contains_key(**field))
        .copied()
        .collect();
    if !repeated.is_empty() {
        return Err(issue(
            "record_finish_machine_fields",
            "Record identity and command correction are derived from the active lock.",
            json!({"fields":repeated}),
        ));
    }
    if let Some(modified) = value.get("modified_files").filter(|value| !value.is_null()) {
        let rows = modified
            .as_array()
            .filter(|rows| !rows.is_empty())
            .ok_or_else(|| {
                issue(
                    "record_finish_invalid_modified_files",
                    "modified_files must be a non-empty array when present.",
                    json!({}),
                )
            })?;
        if rows
            .iter()
            .any(|value| value.as_str().is_none_or(str::is_empty))
        {
            return Err(issue(
                "record_finish_invalid_modified_file",
                "Every modified file must be a non-empty string.",
                json!({}),
            ));
        }
    }
    Ok(())
}

pub fn validate_correction_create_request(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &[
            "schema",
            "target_attempt_id",
            "field",
            "correct_value",
            "reason",
            "invalidates_completion",
        ],
        &[],
        "correction_create_invalid_fields",
        "The Correction create request has missing or unknown fields.",
    )?;
    if value["schema"] != "work-correction-create-request/v1" {
        return Err(issue(
            "correction_create_invalid_schema",
            "The Correction create request schema is invalid.",
            json!({}),
        ));
    }
    if !value["invalidates_completion"].is_boolean() {
        return Err(issue(
            "correction_create_invalid_invalidation_flag",
            "invalidates_completion must be a boolean.",
            json!({}),
        ));
    }
    if !attempt_id(&value["target_attempt_id"]) {
        return Err(issue(
            "correction_create_invalid_attempt_id",
            "target_attempt_id must use ATTEMPT-nnn.",
            json!({}),
        ));
    }
    for field in ["field", "correct_value", "reason"] {
        if value[field]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        {
            return Err(issue(
                "correction_create_empty_text",
                "Correction text fields must be non-empty strings.",
                json!({"field":field}),
            ));
        }
    }
    work_model::execution::request::verified::<
        work_model::execution::request::CorrectionCreateRequest,
    >(value);
    Ok(())
}

pub fn validate_recovery_request(value: &Value, prepare: bool) -> Result<(), ExecutionIssue> {
    let schema = if prepare {
        "work-execution-recovery-prepare-request/v1"
    } else {
        "work-execution-recovery-request/v1"
    };
    let required = if prepare {
        &["schema", "transaction", "attempt_id"][..]
    } else {
        &["schema", "transaction", "attempt_id", "transaction_files"][..]
    };
    let optional = if prepare {
        &[][..]
    } else {
        &["authorization_evidence"][..]
    };
    fields(
        value,
        required,
        optional,
        "execution_recovery_invalid_fields",
        "The execution-recovery request has missing or unknown fields.",
    )?;
    if value["schema"] != schema {
        return Err(issue(
            if prepare {
                "recovery_prepare_schema"
            } else {
                "execution_recovery_invalid_schema"
            },
            "The execution-recovery request schema is invalid.",
            json!({}),
        ));
    }
    if !matches!(
        value["transaction"].as_str(),
        Some(
            "record_begin"
                | "command_correction"
                | "record_finish"
                | "attempt_close"
                | "correction"
                | "deviation_record"
        )
    ) {
        return Err(issue(
            "execution_recovery_invalid_transaction",
            "transaction is not supported by general execution recovery.",
            json!({"transaction":value["transaction"]}),
        ));
    }
    if !attempt_id(&value["attempt_id"]) {
        return Err(issue(
            "execution_recovery_invalid_attempt_id",
            "attempt_id must use the canonical ATTEMPT-nnn format.",
            json!({}),
        ));
    }
    if !prepare {
        if value
            .get("authorization_evidence")
            .is_some_and(|evidence| !evidence.is_null() && !evidence.is_string())
        {
            return Err(issue(
                "execution_recovery_invalid_authorization_evidence",
                "authorization_evidence must be a string.",
                json!({}),
            ));
        }
        let rows = value["transaction_files"].as_array().ok_or_else(|| {
            issue(
                "execution_recovery_invalid_file_list",
                "transaction_files must be an array.",
                json!({}),
            )
        })?;
        let mut names = Vec::new();
        for row in rows {
            let name = row.as_str().unwrap_or("");
            if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
                return Err(issue(
                    "execution_recovery_invalid_file_name",
                    "Every transaction file must be a plain non-empty file name.",
                    json!({}),
                ));
            }
            names.push(name);
        }
        let mut canonical = names.clone();
        canonical.sort();
        canonical.dedup();
        if canonical != names {
            return Err(issue(
                "execution_recovery_noncanonical_file_list",
                "transaction_files must be unique and sorted.",
                json!({}),
            ));
        }
    }
    if prepare {
        work_model::execution::request::verified::<
            work_model::execution::request::ExecutionRecoveryPrepareRequest,
        >(value);
    } else {
        work_model::execution::request::verified::<
            work_model::execution::request::ExecutionRecoveryRequest,
        >(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_and_recovery_rejections_match_python_contracts() {
        assert!(
            validate_attempt_close_request(
                &json!({"schema":"work-attempt-close-request/v1","status":"completed"})
            )
            .is_ok()
        );
        let stopped = json!({"schema":"work-attempt-close-request/v1","status":"stopped","final_type":"other","reason":"Paused","authorization_evidence":"Approved"});
        assert!(validate_attempt_close_request(&stopped).is_ok());
        let mut incomplete = stopped;
        incomplete
            .as_object_mut()
            .unwrap()
            .remove("authorization_evidence");
        assert_eq!(
            validate_attempt_close_request(&incomplete)
                .unwrap_err()
                .reason_code,
            "attempt_close_missing_authorization_evidence"
        );
        let recovery = json!({"schema":"work-execution-recovery-request/v1","transaction":"record_begin","attempt_id":"ATTEMPT-001",
            "transaction_files":[".work-a.tmp",".work-b.tmp"]});
        assert!(validate_recovery_request(&recovery, false).is_ok());
        let mut with_evidence = recovery.clone();
        with_evidence["transaction"] = json!("record_finish");
        with_evidence["authorization_evidence"] = json!("Fresh failure authorization");
        assert!(validate_recovery_request(&with_evidence, false).is_ok());
        with_evidence["authorization_evidence"] = json!(123);
        assert_eq!(
            validate_recovery_request(&with_evidence, false)
                .unwrap_err()
                .reason_code,
            "execution_recovery_invalid_authorization_evidence"
        );
        let mut unsorted = recovery;
        unsorted["transaction_files"] = json!(["b", "a"]);
        assert_eq!(
            validate_recovery_request(&unsorted, false)
                .unwrap_err()
                .reason_code,
            "execution_recovery_noncanonical_file_list"
        );
    }

    #[test]
    fn attempt_close_request_field_and_status_cases_match_python() {
        let base = json!({"schema":"work-attempt-close-request/v1","status":"completed"});
        assert!(validate_attempt_close_request(&base).is_ok());
        for status in ["stopped", "blocked"] {
            assert!(
                validate_attempt_close_request(&json!({
                    "schema":"work-attempt-close-request/v1","status":status,
                    "final_type":"other","reason":"Recorded reason.",
                    "authorization_evidence":"User approved this closure."
                }))
                .is_ok()
            );
        }
        let invalid = json!({"schema":"work-attempt-close-request/v1","status":"in_progress"});
        assert_eq!(
            validate_attempt_close_request(&invalid)
                .unwrap_err()
                .reason_code,
            "attempt_close_invalid_status"
        );
        let mut unexpected = base.clone();
        unexpected["final_type"] = json!("other");
        unexpected["reason"] = json!("Unexpected.");
        let issue = validate_attempt_close_request(&unexpected).unwrap_err();
        assert_eq!(issue.reason_code, "attempt_close_unexpected_final_details");
        assert_eq!(issue.details["fields"], json!(["final_type", "reason"]));
        let mut missing = base.clone();
        missing["status"] = json!("stopped");
        let issue = validate_attempt_close_request(&missing).unwrap_err();
        assert_eq!(issue.reason_code, "attempt_close_missing_final_details");
        assert_eq!(issue.details["missing"], json!(["final_type", "reason"]));
        let blank = json!({"schema":"work-attempt-close-request/v1","status":"blocked",
            "final_type":"other","reason":" ","authorization_evidence":"Approved"});
        assert_eq!(
            validate_attempt_close_request(&blank)
                .unwrap_err()
                .reason_code,
            "attempt_close_empty_final_detail"
        );
        let fields = json!({"schema":"work-attempt-close-request/v1","extra":true});
        let issue = validate_attempt_close_request(&fields).unwrap_err();
        assert_eq!(issue.reason_code, "attempt_close_invalid_fields");
        assert_eq!(issue.details["missing"], json!(["status"]));
        assert_eq!(issue.details["unknown"], json!(["extra"]));
    }

    #[test]
    fn attempt_start_request_continuation_and_errors_match_python() {
        let authorization = json!({
            "schema":"work-attempt-authorization/v1","task_id":"TASK-001",
            "commands":[],"validations":[],"modifiable_files":[],
            "working_directories":[],"external_operations":[],"allowed_deviations":[],
            "reapproval_conditions":["scope_expansion","source_or_worktree_drift",
                "failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"User approved this exact Attempt scope."
        });
        let base = json!({"schema":"work-attempt-start-request/v1",
            "worktree_snapshot_sha256":"a".repeat(64),"authorization":authorization});
        assert!(validate_attempt_start_request(&base).is_ok());
        let mut continuation = base.clone();
        continuation["continuation"] = json!({"source_attempt_id":"ATTEMPT-001",
        "carried_records":[
            {"record_id":"VAL-001","evidence":"Passed previously."},
            {"record_id":"CMD-001#2","evidence":"Authorized retry."}
        ]});
        assert!(validate_attempt_start_request(&continuation).is_ok());
        let mut invalid = base.clone();
        invalid["schema"] = json!("invalid");
        assert_eq!(
            validate_attempt_start_request(&invalid)
                .unwrap_err()
                .reason_code,
            "attempt_start_invalid_schema"
        );
        invalid = base.clone();
        invalid["worktree_snapshot_sha256"] = json!("A".repeat(64));
        assert_eq!(
            validate_attempt_start_request(&invalid)
                .unwrap_err()
                .reason_code,
            "attempt_start_invalid_worktree_snapshot"
        );
        continuation["continuation"]["carried_records"] = json!([
            {"record_id":"VAL-001","evidence":"First."},
            {"record_id":"VAL-001","evidence":"Duplicate."}
        ]);
        let issue = validate_attempt_start_request(&continuation).unwrap_err();
        assert_eq!(issue.reason_code, "attempt_start_duplicate_carried_record");
        assert_eq!(issue.details["record_id"], "VAL-001");
        invalid = base;
        invalid
            .as_object_mut()
            .unwrap()
            .remove("worktree_snapshot_sha256");
        invalid["extra"] = json!(true);
        let issue = validate_attempt_start_request(&invalid).unwrap_err();
        assert_eq!(issue.reason_code, "attempt_start_invalid_object_fields");
        assert_eq!(
            issue.details["missing"],
            json!(["worktree_snapshot_sha256"])
        );
        assert_eq!(issue.details["unknown"], json!(["extra"]));
    }

    #[test]
    fn record_finish_request_values_match_python() {
        let base = json!({"schema":"work-record-finish-request/v1",
            "record":{"outcome":"passed"}});
        assert!(validate_record_finish_request(&base).is_ok());
        let mut modified = base.clone();
        modified["modified_files"] = json!(["src/app.py"]);
        assert!(validate_record_finish_request(&modified).is_ok());
        for (request, code) in [
            (
                json!({"schema":"invalid","record":{"outcome":"passed"}}),
                "record_finish_invalid_schema",
            ),
            (
                json!({"schema":"work-record-finish-request/v1","record":[]}),
                "record_finish_invalid_record",
            ),
            (
                json!({"schema":"work-record-finish-request/v1","record":{"outcome":"passed"},
                "modified_files":[]}),
                "record_finish_invalid_modified_files",
            ),
            (
                json!({"schema":"work-record-finish-request/v1","record":{"outcome":"passed"},
                "modified_files":[""]}),
                "record_finish_invalid_modified_file",
            ),
            (
                json!({"schema":"work-record-finish-request/v1","record":{"id":"VAL-001"}}),
                "record_finish_machine_fields",
            ),
            (
                json!({"schema":"work-record-finish-request/v1","record":{"kind":"validation"}}),
                "record_finish_machine_fields",
            ),
        ] {
            assert_eq!(
                validate_record_finish_request(&request)
                    .unwrap_err()
                    .reason_code,
                code
            );
        }
    }

    #[test]
    fn execution_recovery_request_variants_match_python() {
        let base = json!({"schema":"work-execution-recovery-request/v1",
            "transaction":"record_begin","attempt_id":"ATTEMPT-001",
            "transaction_files":["prepared.tmp"]});
        assert!(validate_recovery_request(&base, false).is_ok());
        let mut deviation = base.clone();
        deviation["transaction"] = json!("deviation_record");
        assert!(validate_recovery_request(&deviation, false).is_ok());
        let mut finish = base.clone();
        finish["transaction"] = json!("record_finish");
        finish["authorization_evidence"] = json!("Fresh failure authorization.");
        assert!(validate_recovery_request(&finish, false).is_ok());
        for (field, value, code) in [
            (
                "schema",
                json!("invalid"),
                "execution_recovery_invalid_schema",
            ),
            (
                "transaction",
                json!("invalid"),
                "execution_recovery_invalid_transaction",
            ),
            (
                "attempt_id",
                json!("ATTEMPT-1"),
                "execution_recovery_invalid_attempt_id",
            ),
            (
                "transaction_files",
                json!("bad"),
                "execution_recovery_invalid_file_list",
            ),
            (
                "transaction_files",
                json!(["a/b"]),
                "execution_recovery_invalid_file_name",
            ),
            (
                "transaction_files",
                json!(["b", "a"]),
                "execution_recovery_noncanonical_file_list",
            ),
            (
                "authorization_evidence",
                json!(123),
                "execution_recovery_invalid_authorization_evidence",
            ),
        ] {
            let mut invalid = base.clone();
            invalid[field] = value;
            assert_eq!(
                validate_recovery_request(&invalid, false)
                    .unwrap_err()
                    .reason_code,
                code
            );
        }
    }

    #[test]
    fn start_correction_and_record_requests_reject_invalid_fields() {
        let authorization = json!({"schema":"work-attempt-authorization/v1","task_id":"TASK-001",
            "commands":[],"validations":[],"modifiable_files":[],"working_directories":[],
            "external_operations":[],"allowed_deviations":[],
            "reapproval_conditions":["scope_expansion","source_or_worktree_drift","failure_divergence",
                "retry","recovery","unknown_result"],"authorization_evidence":"Approved"});
        let start = json!({"schema":"work-attempt-start-request/v1","worktree_snapshot_sha256":"0".repeat(64),"authorization":authorization});
        assert!(validate_attempt_start_request(&start).is_ok());
        let mut missing_authorization = start.clone();
        missing_authorization["authorization"] = json!({});
        let missing_issue = validate_attempt_start_request(&missing_authorization).unwrap_err();
        assert_eq!(
            missing_issue.reason_code,
            "attempt_start_invalid_object_fields"
        );
        assert_eq!(missing_issue.details["location"], "authorization");
        assert_eq!(
            missing_issue.details["missing"].as_array().unwrap().len(),
            10
        );
        let mut with_null_continuation = start.clone();
        with_null_continuation["continuation"] = Value::Null;
        assert!(validate_attempt_start_request(&with_null_continuation).is_ok());
        assert!(!record_id("CMD-001#"));
        let mut changed = start;
        changed["worktree_snapshot_sha256"] = json!("bad");
        assert_eq!(
            validate_attempt_start_request(&changed)
                .unwrap_err()
                .reason_code,
            "attempt_start_invalid_worktree_snapshot"
        );
        let correction = json!({"schema":"work-correction-create-request/v1","target_attempt_id":"ATTEMPT-001",
            "field":"records[0]","correct_value":"passed","reason":"Fix","invalidates_completion":true});
        assert!(validate_correction_create_request(&correction).is_ok());
        let finish = json!({"schema":"work-record-finish-request/v1","record":{"outcome":"passed","evidence":"ok"}});
        assert!(validate_record_finish_request(&finish).is_ok());
    }

    #[test]
    fn correction_create_request_acceptance_and_rejections_match_python() {
        let request = json!({"schema":"work-correction-create-request/v1",
            "target_attempt_id":"ATTEMPT-001","field":"records[0].outcome",
            "correct_value":"passed","reason":"Correct the recorded outcome.",
            "invalidates_completion":true});
        assert!(validate_correction_create_request(&request).is_ok());
        for field in ["field", "correct_value", "reason"] {
            let mut blank = request.clone();
            blank[field] = json!(" ");
            let error = validate_correction_create_request(&blank).unwrap_err();
            assert_eq!(error.reason_code, "correction_create_empty_text");
            assert_eq!(error.details["field"], field);
        }
        for (field, value, code) in [
            (
                "schema",
                json!("invalid"),
                "correction_create_invalid_schema",
            ),
            (
                "target_attempt_id",
                json!("ATTEMPT-1"),
                "correction_create_invalid_attempt_id",
            ),
            (
                "invalidates_completion",
                json!(1),
                "correction_create_invalid_invalidation_flag",
            ),
        ] {
            let mut invalid = request.clone();
            invalid[field] = value;
            assert_eq!(
                validate_correction_create_request(&invalid)
                    .unwrap_err()
                    .reason_code,
                code
            );
        }
        let mut invalid_fields = request;
        invalid_fields.as_object_mut().unwrap().remove("reason");
        invalid_fields["extra"] = json!(true);
        let error = validate_correction_create_request(&invalid_fields).unwrap_err();
        assert_eq!(error.reason_code, "correction_create_invalid_fields");
        assert_eq!(
            error.details,
            json!({"missing":["reason"],"unknown":["extra"]})
        );
    }
}
