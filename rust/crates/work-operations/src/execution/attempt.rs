//! Attempt artifact identity and canonical rendering.

use std::collections::{HashMap, HashSet};

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};

use crate::canonical::canonical_json_sha256;
use crate::execution::ExecutionIssue;
use crate::identifiers::path_segment_issue;
use crate::protocol::{ATTEMPT_ID_PREFIX, TASK_ID_PREFIX, valid_sha256};

const ROOT: &[&str] = &[
    "schema",
    "attempt_id",
    "task_spec_id",
    "task_id",
    "skill_id",
    "status",
    "task_collection_sha256",
    "task_index_sha256",
    "task_item_sha256",
    "task_instructions_sha256",
    "execute_instructions_sha256",
    "hierarchy_selection_sha256",
    "execute_skill_selection_sha256",
    "authorization",
    "authorization_sha256",
    "started_at",
    "continued_from",
    "carried_records",
    "modified_files",
    "execution_deviations",
    "records",
    "overall_result",
    "final_type",
    "reason",
    "closing_authorization_evidence",
    "ended_at",
];
const REQUIRED: &[&str] = &[
    "schema",
    "attempt_id",
    "task_spec_id",
    "task_id",
    "skill_id",
    "status",
    "task_collection_sha256",
    "task_index_sha256",
    "task_item_sha256",
    "task_instructions_sha256",
    "execute_instructions_sha256",
    "hierarchy_selection_sha256",
    "execute_skill_selection_sha256",
    "authorization",
    "authorization_sha256",
    "started_at",
    "records",
];
const AUTHORIZATION: &[&str] = &[
    "schema",
    "task_id",
    "commands",
    "validations",
    "modifiable_files",
    "working_directories",
    "external_operations",
    "allowed_deviations",
    "reapproval_conditions",
    "authorization_evidence",
];
const REAPPROVAL: &[&str] = &[
    "scope_expansion",
    "source_or_worktree_drift",
    "failure_divergence",
    "retry",
    "recovery",
    "unknown_result",
];

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn validate_attempt_file_path(path: &str, attempt: &Value) -> Result<(), ExecutionIssue> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.last() != Some(&"attempt.json") {
        return Err(issue(
            "attempt_filename_mismatch",
            "Attempt files must use <TASK-ID>/<ATTEMPT-ID>/attempt.json; legacy flat paths are unsupported.",
            json!({}),
        ));
    }
    if parts.iter().rev().nth(1).copied() != attempt["attempt_id"].as_str() {
        return Err(issue(
            "attempt_parent_attempt_mismatch",
            "The Attempt directory does not match attempt_id.",
            json!({}),
        ));
    }
    if parts.iter().rev().nth(2).copied() != attempt["task_id"].as_str() {
        return Err(issue(
            "attempt_parent_task_mismatch",
            "The Attempt grandparent directory does not match task_id.",
            json!({}),
        ));
    }
    Ok(())
}

fn strict(
    value: &Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), ExecutionIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            "attempt_expected_object",
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
            "attempt_invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":location,"missing":missing,"unknown":unknown}),
        ));
    }
    Ok(())
}

fn identifier(
    value: &Value,
    prefix: &str,
    digits: usize,
    location: &str,
) -> Result<(), ExecutionIssue> {
    let text = value.as_str().unwrap_or("");
    if text.trim().is_empty() {
        return Err(issue(
            "attempt_empty_text_value",
            "A non-empty string is required.",
            json!({"location":location}),
        ));
    }
    if !text
        .strip_prefix(prefix)
        .is_some_and(|tail| tail.len() == digits && tail.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(issue(
            "attempt_invalid_identifier",
            "An Attempt contract identifier has an invalid format.",
            json!({"location":location,"value":text}),
        ));
    }
    Ok(())
}

fn sha(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    if value.as_str().is_none_or(|text| !valid_sha256(text)) {
        return Err(issue(
            "attempt_invalid_sha256",
            "A lowercase SHA-256 value is required.",
            json!({"location":location}),
        ));
    }
    Ok(())
}

fn nonempty(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    if value.as_str().is_none_or(|text| text.trim().is_empty()) {
        return Err(issue(
            "attempt_empty_text_value",
            "A non-empty string is required.",
            json!({"location":location}),
        ));
    }
    Ok(())
}

pub(crate) fn timestamp(value: &Value, location: &str) -> Result<i64, ExecutionIssue> {
    nonempty(value, location)?;
    let text = value.as_str().expect("checked text");
    let bytes = text.as_bytes();
    let shape = bytes.len() == 22
        && [4, 7, 10, 13, 16, 19]
            .iter()
            .zip(*b"--T:+:")
            .all(|(position, expected)| {
                bytes[*position] == expected || *position == 16 && bytes[*position] == b'-'
            })
        && bytes.iter().enumerate().all(|(position, byte)| {
            [4, 7, 10, 13, 16, 19].contains(&position) || byte.is_ascii_digit()
        });
    if !shape {
        return Err(issue(
            "attempt_invalid_timestamp",
            "A minute-precision ISO 8601 timestamp with a numeric offset is required.",
            json!({"location":location,"value":text}),
        ));
    }
    let number =
        |start: usize, end: usize| text[start..end].parse::<i64>().expect("checked digits");
    let year = number(0, 4);
    let month = number(5, 7);
    let day = number(8, 10);
    let hour = number(11, 13);
    let minute = number(14, 16);
    let offset_hour = number(17, 19);
    let offset_minute = number(20, 22);
    let leap = |year: i64| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year == 0
        || !(1..=12).contains(&month)
        || day < 1
        || day > days[(month - 1) as usize]
        || hour > 23
        || minute > 59
        || offset_hour > 23
        || offset_minute > 59
    {
        return Err(issue(
            "attempt_invalid_timestamp",
            "The Attempt timestamp is not a valid calendar time.",
            json!({"location":location,"value":text}),
        ));
    }
    let mut absolute_days = 0;
    for candidate in 1..year {
        absolute_days += if leap(candidate) { 366 } else { 365 };
    }
    for length in days.iter().take((month - 1) as usize) {
        absolute_days += length;
    }
    absolute_days += day - 1;
    let offset = (offset_hour * 60 + offset_minute) * if bytes[16] == b'+' { 1 } else { -1 };
    Ok(absolute_days * 1440 + hour * 60 + minute - offset)
}

fn record_identity(value: &Value, location: &str) -> Result<(String, String, u64), ExecutionIssue> {
    let text = value.as_str().unwrap_or("");
    if text.trim().is_empty() {
        nonempty(value, location)?;
    }
    let (base, suffix) = text.split_once('#').unwrap_or((text, ""));
    let valid = ["CMD-", "OP-", "VAL-"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|digits| {
            digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    });
    let retry = if suffix.is_empty() {
        0
    } else {
        suffix.parse::<u64>().unwrap_or(0)
    };
    if !valid
        || text.contains('#')
            && (suffix.starts_with('0')
                || retry == 0
                || !suffix.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(issue(
            "attempt_invalid_record_id",
            "A record ID must use CMD-, OP-, or VAL- with an optional retry suffix.",
            json!({"location":location,"value":text}),
        ));
    }
    Ok((text.into(), base.into(), retry))
}

fn validate_carried_records(
    value: &Value,
    source: &str,
) -> Result<HashMap<String, u64>, ExecutionIssue> {
    let rows = value
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            issue(
                "attempt_invalid_carried_records",
                "carried_records must be a non-empty array when present.",
                json!({}),
            )
        })?;
    let mut seen = HashSet::new();
    let mut retries = HashMap::new();
    for (position, row) in rows.iter().enumerate() {
        let location = format!("carried_records[{position}]");
        strict(
            row,
            &location,
            &["source_attempt_id", "record_id", "evidence"],
            &[],
        )?;
        identifier(
            &row["source_attempt_id"],
            ATTEMPT_ID_PREFIX,
            3,
            &format!("{location}.source_attempt_id"),
        )?;
        if row["source_attempt_id"] != source {
            return Err(issue(
                "attempt_carried_source_mismatch",
                "Every carried record must use continued_from as its source Attempt.",
                json!({"location":location}),
            ));
        }
        let (id, base, retry) =
            record_identity(&row["record_id"], &format!("{location}.record_id"))?;
        if !seen.insert(id.clone()) {
            return Err(issue(
                "attempt_duplicate_carried_record",
                "A carried record ID cannot be repeated.",
                json!({"record_id":id}),
            ));
        }
        retries
            .entry(base)
            .and_modify(|old: &mut u64| *old = (*old).max(retry))
            .or_insert(retry);
        nonempty(&row["evidence"], &format!("{location}.evidence"))?;
    }
    Ok(retries)
}

fn validate_records(
    value: &Value,
    carried_retries: &HashMap<String, u64>,
) -> Result<Vec<Value>, ExecutionIssue> {
    let rows = value.as_array().ok_or_else(|| {
        issue(
            "attempt_invalid_records",
            "records must be an array.",
            json!({}),
        )
    })?;
    let mut previous = carried_retries.clone();
    let mut seen = HashSet::new();
    let mut operations = Vec::new();
    for (position, row) in rows.iter().enumerate() {
        let location = format!("records[{position}]");
        if !row.is_object() {
            return Err(issue(
                "attempt_expected_object",
                "A JSON object is required.",
                json!({"location":location}),
            ));
        }
        let kind = row["kind"].as_str().unwrap_or("");
        let skipped = row["status"] == "skipped";
        if !matches!(kind, "command" | "operation" | "validation") {
            return Err(issue(
                "attempt_invalid_record_kind",
                "A record kind must be command, operation, or validation.",
                json!({"location":format!("{location}.kind")}),
            ));
        }
        if skipped {
            strict(
                row,
                &location,
                &["id", "kind", "status", "reason", "deviation_id"],
                &[],
            )?;
        } else {
            match kind {
                "command" => strict(
                    row,
                    &location,
                    &["id", "kind", "exit_code", "result"],
                    &["correction"],
                )?,
                "operation" => strict(row, &location, &["id", "kind", "outcome", "state"], &[])?,
                _ => strict(row, &location, &["id", "kind", "outcome", "evidence"], &[])?,
            }
        }
        let (id, base, retry) = record_identity(&row["id"], &format!("{location}.id"))?;
        let prefix = match kind {
            "command" => "CMD-",
            "operation" => "OP-",
            _ => "VAL-",
        };
        if !id.starts_with(prefix) {
            return Err(issue(
                "attempt_record_kind_mismatch",
                "The record ID prefix does not match its kind.",
                json!({"record_id":id,"kind":kind}),
            ));
        }
        if !seen.insert(id.clone()) {
            return Err(issue(
                "attempt_duplicate_record",
                "A record ID cannot be repeated.",
                json!({"record_id":id}),
            ));
        }
        let expected = previous.get(&base).map_or(0, |last| last + 1);
        if retry != expected {
            return Err(issue(
                "attempt_record_retry_sequence",
                "Record retries must begin with the original ID and increase without gaps.",
                json!({"record_id":id,"expected_retry":expected}),
            ));
        }
        previous.insert(base, retry);
        if skipped {
            nonempty(&row["reason"], &format!("{location}.reason"))?;
            identifier(
                &row["deviation_id"],
                "DEVIATION-",
                3,
                &format!("{location}.deviation_id"),
            )?;
        } else {
            match kind {
                "command" => {
                    if !row["exit_code"].is_i64() && !row["exit_code"].is_u64() {
                        return Err(issue(
                            "attempt_invalid_exit_code",
                            "A command exit_code must be an integer.",
                            json!({"location":format!("{location}.exit_code")}),
                        ));
                    }
                    nonempty(&row["result"], &format!("{location}.result"))?;
                    if let Some(correction) = row.get("correction") {
                        crate::execution::command_correction::validate_command_correction(
                            correction,
                            &format!("{location}.correction"),
                        )?;
                    }
                }
                "operation" => {
                    if !matches!(
                        row["outcome"].as_str(),
                        Some("success" | "failure" | "unknown")
                    ) {
                        return Err(issue(
                            "attempt_invalid_operation_outcome",
                            "An operation outcome must be success, failure, or unknown.",
                            json!({"location":format!("{location}.outcome")}),
                        ));
                    }
                    nonempty(&row["state"], &format!("{location}.state"))?;
                    operations.push(row.clone());
                }
                _ => {
                    if !matches!(row["outcome"].as_str(), Some("passed" | "failed")) {
                        return Err(issue(
                            "attempt_invalid_validation_outcome",
                            "A validation outcome must be passed or failed.",
                            json!({"location":format!("{location}.outcome")}),
                        ));
                    }
                    nonempty(&row["evidence"], &format!("{location}.evidence"))?;
                }
            }
        }
    }
    Ok(operations)
}

fn validate_modified_files(value: &Value) -> Result<(), ExecutionIssue> {
    let rows = value
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            issue(
                "attempt_invalid_modified_files",
                "modified_files must be a non-empty array when present.",
                json!({}),
            )
        })?;
    let mut seen = HashSet::new();
    for row in rows {
        let path = row.as_str().unwrap_or("");
        if path.is_empty() {
            return Err(issue(
                "empty_relative_path",
                "The project-relative path cannot be empty.",
                json!({"field":"modified_files[]"}),
            ));
        }
        if path.starts_with(['/', '\\']) || path.as_bytes().get(1) == Some(&b':') {
            return Err(issue(
                "absolute_path_rejected",
                "The path must be project-relative.",
                json!({"field":"modified_files[]","path":path}),
            ));
        }
        let normalized = path
            .strip_prefix("./")
            .or_else(|| path.strip_prefix(".\\"))
            .unwrap_or(path)
            .replace('\\', "/");
        for segment in normalized.split('/') {
            if let Some(problem) = path_segment_issue(segment) {
                return Err(issue(
                    problem.reason_code(),
                    "The path contains an unsafe segment.",
                    json!({"field":"modified_files[]","segment":segment}),
                ));
            }
        }
        if normalized != path {
            return Err(issue(
                "attempt_noncanonical_modified_file",
                "Modified-file paths must use normalized project-relative form.",
                json!({}),
            ));
        }
        if !seen.insert(crate::canonical::portable_path_identity(path)) {
            return Err(issue(
                "attempt_duplicate_modified_file",
                "Modified-file paths must have unique portable path identities.",
                json!({}),
            ));
        }
    }
    Ok(())
}

fn validate_deviations_and_skips(value: &Value) -> Result<(), ExecutionIssue> {
    let mut deviations = Vec::new();
    if let Some(raw) = value.get("execution_deviations") {
        deviations = raw
            .as_array()
            .filter(|rows| !rows.is_empty())
            .ok_or_else(|| {
                issue(
                    "attempt_invalid_execution_deviations",
                    "execution_deviations must be a non-empty array when present.",
                    json!({}),
                )
            })?
            .clone();
        for (position, deviation) in deviations.iter().enumerate() {
            if deviation["deviation_id"] != format!("DEVIATION-{:03}", position + 1) {
                return Err(issue(
                    "attempt_invalid_execution_deviation_sequence",
                    "execution_deviations must use contiguous DEVIATION-nnn IDs in stored order.",
                    json!({}),
                ));
            }
            crate::execution::deviation::validate_deviation_artifact(deviation)?;
        }
    }
    let approved: HashMap<_, _> = deviations
        .iter()
        .filter(|deviation| {
            deviation["decision"]["outcome"] == "approved"
                && deviation["proposal"]["action"]["kind"] == "skip_record"
                && crate::execution::deviation_reconciliation_target(&deviation["proposal"])
                    == "task_only"
        })
        .filter_map(|deviation| {
            deviation["proposal"]["action"]["record_id"]
                .as_str()
                .map(|id| (id, deviation))
        })
        .collect();
    let mut recorded = HashSet::new();
    for record in value["records"].as_array().into_iter().flatten() {
        let Some(id) = record["id"].as_str() else {
            continue;
        };
        let matched = approved.get(id);
        if record["status"] == "skipped" {
            let deviation = matched.ok_or_else(|| {
                issue(
                    "attempt_unapproved_skipped_record",
                    "A skipped record requires an approved skip_record deviation.",
                    json!({"record_id":id}),
                )
            })?;
            if record["deviation_id"] != deviation["deviation_id"]
                || record["reason"] != deviation["proposal"]["action"]["reason"]
            {
                return Err(issue(
                    "attempt_skipped_record_mismatch",
                    "Skipped record evidence must match its approved deviation.",
                    json!({"record_id":id}),
                ));
            }
            recorded.insert(id);
        } else if matched.is_some() {
            return Err(issue(
                "attempt_skip_record_result_mismatch",
                "A record with an approved skip_record deviation must be recorded as skipped.",
                json!({"record_id":id}),
            ));
        }
    }
    if value["status"] == "completed" {
        let mut missing: Vec<_> = approved
            .keys()
            .filter(|id| !recorded.contains(**id))
            .copied()
            .collect();
        missing.sort();
        if !missing.is_empty() {
            return Err(issue(
                "attempt_missing_skipped_records",
                "A completed Attempt must record every approved skip_record deviation.",
                json!({"record_ids":missing}),
            ));
        }
    }
    Ok(())
}

pub fn validate_authorization(value: &Value) -> Result<(), ExecutionIssue> {
    strict(value, "authorization", AUTHORIZATION, &[])?;
    if value["schema"] != "work-attempt-authorization/v1" {
        return Err(issue(
            "attempt_invalid_authorization",
            "The Attempt authorization schema is invalid.",
            json!({}),
        ));
    }
    identifier(
        &value["task_id"],
        TASK_ID_PREFIX,
        3,
        "authorization.task_id",
    )?;
    for field in [
        "commands",
        "validations",
        "modifiable_files",
        "working_directories",
        "external_operations",
        "allowed_deviations",
    ] {
        let rows = value[field].as_array().ok_or_else(|| {
            issue(
                "attempt_invalid_authorization",
                "The Attempt authorization manifest is invalid.",
                json!({"field":field}),
            )
        })?;
        let mut unique = std::collections::HashSet::new();
        for row in rows {
            let identity = if matches!(field, "commands" | "validations" | "external_operations") {
                row["id"].as_str()
            } else {
                row.as_str()
            };
            if let Some(identity) = identity {
                if !unique.insert(identity) {
                    return Err(issue(
                        "attempt_invalid_authorization",
                        "The Attempt authorization scope contains duplicates.",
                        json!({"field":field}),
                    ));
                }
            }
        }
    }
    if value["reapproval_conditions"] != json!(REAPPROVAL) {
        return Err(issue(
            "attempt_invalid_authorization",
            "Every fixed reapproval condition is required in canonical order.",
            json!({}),
        ));
    }
    nonempty(
        &value["authorization_evidence"],
        "authorization.authorization_evidence",
    )?;
    let _: work_model::execution::attempt::AttemptAuthorization =
        serde_json::from_value(value.clone()).expect("validated authorization matches model");
    Ok(())
}

pub fn validate_attempt_identity(value: &Value) -> Result<(), ExecutionIssue> {
    if value["schema"] != "work-attempt/v1" {
        return Err(issue(
            "attempt_invalid_schema",
            "The Attempt schema is invalid.",
            json!({}),
        ));
    }
    let optional: Vec<_> = ROOT
        .iter()
        .copied()
        .filter(|field| !REQUIRED.contains(field))
        .collect();
    strict(value, "attempt", REQUIRED, &optional)?;
    identifier(&value["attempt_id"], ATTEMPT_ID_PREFIX, 3, "attempt_id")?;
    identifier(&value["task_spec_id"], "TASK-SPEC-", 3, "task_spec_id")?;
    identifier(&value["task_id"], TASK_ID_PREFIX, 3, "task_id")?;
    if !value["skill_id"].is_null() {
        nonempty(&value["skill_id"], "skill_id")?;
    }
    for field in [
        "task_collection_sha256",
        "task_index_sha256",
        "task_item_sha256",
        "task_instructions_sha256",
        "execute_instructions_sha256",
        "hierarchy_selection_sha256",
        "execute_skill_selection_sha256",
        "authorization_sha256",
    ] {
        sha(&value[field], field)?;
    }
    validate_authorization(&value["authorization"])?;
    let actual = canonical_json_sha256(&value["authorization"]).expect("JSON value serializes");
    if value["authorization_sha256"] != actual {
        return Err(issue(
            "attempt_authorization_fingerprint_mismatch",
            "The authorization manifest does not match its fingerprint.",
            json!({}),
        ));
    }
    if !matches!(
        value["status"].as_str(),
        Some("in_progress" | "completed" | "stopped" | "blocked")
    ) {
        return Err(issue(
            "attempt_invalid_status",
            "Attempt status must be in_progress, completed, stopped, or blocked.",
            json!({}),
        ));
    }
    let started = timestamp(&value["started_at"], "started_at")?;
    let mut carried_retries = HashMap::new();
    if let Some(source) = value.get("continued_from") {
        identifier(source, ATTEMPT_ID_PREFIX, 3, "continued_from")?;
        let current = value["attempt_id"].as_str().expect("validated Attempt ID");
        if source.as_str().expect("validated source Attempt") >= current {
            return Err(issue(
                "attempt_invalid_continuation",
                "continued_from must identify an earlier Attempt.",
                json!({}),
            ));
        }
    }
    if let Some(carried) = value.get("carried_records") {
        let source = value["continued_from"].as_str().ok_or_else(|| {
            issue(
                "attempt_missing_continuation",
                "carried_records requires continued_from.",
                json!({}),
            )
        })?;
        carried_retries = validate_carried_records(carried, source)?;
    }
    let operations = validate_records(&value["records"], &carried_retries)?;
    if let Some(files) = value.get("modified_files") {
        validate_modified_files(files)?;
    }
    validate_deviations_and_skips(value)?;
    let overall = crate::execution::overall_operation_result(&operations);
    if let Some(result) = value.get("overall_result") {
        if operations.is_empty() {
            return Err(issue(
                "attempt_unexpected_overall_result",
                "overall_result requires at least one operation record.",
                json!({}),
            ));
        }
        if Some(result) != overall.as_ref() {
            return Err(issue(
                "attempt_overall_result_mismatch",
                "Overall-result details must exactly classify every operation record.",
                json!({}),
            ));
        }
    } else if !operations.is_empty() && value["status"] != "in_progress" {
        return Err(issue(
            "attempt_missing_overall_result",
            "A closed Attempt with operation records requires overall_result.",
            json!({}),
        ));
    }
    let status = value["status"].as_str().expect("checked status");
    let closing: Vec<_> = [
        "final_type",
        "reason",
        "closing_authorization_evidence",
        "ended_at",
    ]
    .iter()
    .filter(|field| value.get(**field).is_some())
    .copied()
    .collect();
    if status == "in_progress" {
        if !closing.is_empty() {
            return Err(issue(
                "attempt_unexpected_closing_fields",
                "An in-progress Attempt cannot contain closing fields.",
                json!({"fields":closing}),
            ));
        }
    } else {
        let ended = value.get("ended_at").ok_or_else(|| {
            issue(
                "attempt_missing_end_time",
                "A closed Attempt requires ended_at.",
                json!({}),
            )
        })?;
        if timestamp(ended, "ended_at")? < started {
            return Err(issue(
                "attempt_end_before_start",
                "ended_at cannot be earlier than started_at.",
                json!({}),
            ));
        }
        if status == "completed" {
            let unexpected: Vec<_> = ["final_type", "reason", "closing_authorization_evidence"]
                .iter()
                .filter(|field| value.get(**field).is_some())
                .copied()
                .collect();
            if !unexpected.is_empty() {
                return Err(issue(
                    "attempt_unexpected_final_details",
                    "A completed Attempt cannot contain final_type or reason.",
                    json!({"fields":unexpected}),
                ));
            }
            if overall
                .as_ref()
                .is_some_and(|result| result["status"] != "complete_success")
            {
                return Err(issue(
                    "attempt_incomplete_operation_result",
                    "A completed Attempt cannot have a partial, failed, or uncertain operation result.",
                    json!({}),
                ));
            }
        } else {
            let missing: Vec<_> = ["final_type", "reason", "closing_authorization_evidence"]
                .iter()
                .filter(|field| value.get(**field).is_none())
                .copied()
                .collect();
            if !missing.is_empty() {
                return Err(issue(
                    "attempt_missing_final_details",
                    "A stopped or blocked Attempt requires final_type and reason.",
                    json!({"missing":missing}),
                ));
            }
            let allowed = if status == "stopped" {
                &[
                    "specification_defect",
                    "instructions_changed",
                    "validation_failed",
                    "unexpected_change",
                    "external_operation_failed",
                    "user_stopped",
                    "other",
                ][..]
            } else {
                &[
                    "environment",
                    "external_service",
                    "permission",
                    "required_input",
                    "other",
                ][..]
            };
            if !value["final_type"]
                .as_str()
                .is_some_and(|kind| allowed.contains(&kind))
            {
                return Err(issue(
                    "attempt_invalid_final_type",
                    "final_type is invalid for the Attempt status.",
                    json!({"status":status,"final_type":value["final_type"]}),
                ));
            }
            nonempty(&value["reason"], "reason")?;
            nonempty(
                &value["closing_authorization_evidence"],
                "closing_authorization_evidence",
            )?;
        }
    }
    let _: work_model::execution::attempt::Attempt =
        serde_json::from_value(value.clone()).expect("validated Attempt matches model");
    Ok(())
}

struct Ordered<'a> {
    value: &'a Value,
    path: Vec<String>,
}
fn order(path: &[String]) -> &'static [&'static str] {
    if path.is_empty() {
        ROOT
    } else if path == ["authorization"] {
        AUTHORIZATION
    } else if path == ["authorization", "commands"] {
        &["id", "mode", "argv", "script", "execution"]
    } else if path == ["authorization", "commands", "execution"] {
        &["working_directory", "os", "shell"]
    } else if path == ["authorization", "validations"] {
        &[
            "id",
            "kind",
            "command_ids",
            "pass_condition",
            "confirmer",
            "criteria",
            "acceptance_ids",
        ]
    } else if path == ["authorization", "external_operations"] {
        &[
            "id",
            "kind",
            "action",
            "target",
            "validation_id",
            "command_id",
        ]
    } else if path == ["execution_deviations"] {
        &[
            "schema",
            "deviation_id",
            "approved_preview_sha256",
            "proposal",
            "supplemental_authorization",
            "decision",
            "reconciliation_status",
        ]
    } else if path == ["execution_deviations", "proposal"] {
        &[
            "schema",
            "task_id",
            "attempt_id",
            "anchor_record_id",
            "task_basis",
            "gap",
            "action",
            "modifiable_files",
            "impact",
            "side_effects",
        ]
    } else if path == ["execution_deviations", "proposal", "impact"] {
        &[
            "summary",
            "requirement_changed",
            "scope_changed",
            "acceptance_criteria_changed",
            "deliverables_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ]
    } else if path == ["execution_deviations", "supplemental_authorization"] {
        &[
            "schema",
            "preview_sha256",
            "action",
            "modifiable_files",
            "authorization_evidence",
        ]
    } else if path == ["execution_deviations", "decision"] {
        &["outcome", "evidence"]
    } else if path.last().is_some_and(|part| part == "action") {
        &["kind", "record_id", "reason"]
    } else if path == ["carried_records"] {
        &["source_attempt_id", "record_id", "evidence"]
    } else if path == ["records"] {
        &[
            "id",
            "kind",
            "status",
            "reason",
            "deviation_id",
            "correction",
            "exit_code",
            "result",
            "outcome",
            "state",
            "evidence",
        ]
    } else if path == ["overall_result"] {
        &["status", "effective", "not_effective", "unknown"]
    } else if path
        .last()
        .is_some_and(|part| part == "correction" || part == "command_correction")
    {
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

pub fn ordered_attempt(value: &Value) -> impl Serialize + '_ {
    Ordered {
        value,
        path: vec![],
    }
}

pub fn render_attempt(value: &Value) -> Result<Vec<u8>, ExecutionIssue> {
    validate_attempt_identity(value)?;
    let mut bytes = serde_json::to_vec_pretty(&Ordered {
        value,
        path: vec![],
    })
    .expect("JSON value serializes");
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn validate_attempt_bytes(value: &Value, raw: &[u8]) -> Result<Value, ExecutionIssue> {
    let rendered = render_attempt(value)?;
    if rendered != raw {
        return Err(issue(
            "noncanonical_json_contract",
            "The JSON contract does not match the required canonical rendering.",
            json!({}),
        ));
    }
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::AttemptValidation,
    >(
        json!({"schema":"work-attempt-validation/v1","attempt_id":value["attempt_id"],
        "task_spec_id":value["task_spec_id"],"task_id":value["task_id"],
        "status":value["status"],"record_count":value["records"].as_array().expect("checked records").len(),
        "result":"valid"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;

    #[test]
    fn attempt_file_path_binds_task_and_attempt_directory() {
        let attempt = json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001"});
        validate_attempt_file_path("execution/TASK-001/ATTEMPT-001/attempt.json", &attempt)
            .unwrap();
        for (path, expected) in [
            (
                "execution/TASK-001/ATTEMPT-001.json",
                "attempt_filename_mismatch",
            ),
            (
                "execution/TASK-001/ATTEMPT-002/attempt.json",
                "attempt_parent_attempt_mismatch",
            ),
            (
                "execution/TASK-002/ATTEMPT-001/attempt.json",
                "attempt_parent_task_mismatch",
            ),
        ] {
            assert_eq!(
                validate_attempt_file_path(path, &attempt)
                    .unwrap_err()
                    .reason_code,
                expected
            );
        }
    }

    #[test]
    fn authorization_evidence_changes_fingerprint_and_reapproval_is_fixed() {
        let mut authorization = json!({"schema":"work-attempt-authorization/v1","task_id":"TASK-001",
            "commands":[],"validations":[],"modifiable_files":[],"working_directories":[],
            "external_operations":[],"allowed_deviations":[],"reapproval_conditions":REAPPROVAL,
            "authorization_evidence":"User approved this exact Attempt scope."});
        validate_authorization(&authorization).unwrap();
        let first = canonical_json_sha256(&authorization).unwrap();
        authorization["authorization_evidence"] = json!("A different reviewed authorization.");
        validate_authorization(&authorization).unwrap();
        assert_ne!(first, canonical_json_sha256(&authorization).unwrap());
        authorization["reapproval_conditions"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert_eq!(
            validate_authorization(&authorization)
                .unwrap_err()
                .reason_code,
            "attempt_invalid_authorization"
        );
    }

    #[test]
    fn attempt_v1_accepts_current_fingerprints_and_rejects_retired_fields() {
        let authorization = json!({"schema":"work-attempt-authorization/v1","task_id":"TASK-001",
            "commands":[],"validations":[],"modifiable_files":[],"working_directories":[],
            "external_operations":[],"allowed_deviations":[],"reapproval_conditions":REAPPROVAL,
            "authorization_evidence":"User approved this exact Attempt scope."});
        let mut attempt = json!({"schema":"work-attempt/v1","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"1".repeat(64),"task_item_sha256":"2".repeat(64),
            "task_instructions_sha256":"b".repeat(64),
            "execute_instructions_sha256":"c".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"d".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00",
            "records":[]});
        let raw = render_attempt(&attempt).unwrap();
        assert_eq!(
            validate_attempt_bytes(&attempt, &raw).unwrap()["schema"],
            "work-attempt-validation/v1"
        );
        let mut legacy = attempt.clone();
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
        let error = render_attempt(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "attempt_invalid_object_fields");
        assert_eq!(
            error.details["missing"],
            json!(["task_instructions_sha256", "execute_instructions_sha256"])
        );
        assert_eq!(
            error.details["unknown"],
            json!(["execute_rules_sha256", "task_rules_sha256"])
        );
        let mut legacy = attempt.clone();
        legacy["task_sha256"] = legacy
            .as_object_mut()
            .unwrap()
            .remove("task_collection_sha256")
            .unwrap();
        legacy.as_object_mut().unwrap().remove("task_index_sha256");
        legacy.as_object_mut().unwrap().remove("task_item_sha256");
        let error = render_attempt(&legacy).unwrap_err();
        assert_eq!(error.reason_code, "attempt_invalid_object_fields");
        assert_eq!(
            error.details["missing"],
            json!([
                "task_collection_sha256",
                "task_index_sha256",
                "task_item_sha256"
            ])
        );
        assert_eq!(error.details["unknown"], json!(["task_sha256"]));
        let mut retired = attempt.clone();
        retired["schema"] = json!("work-attempt/v2");
        assert_eq!(
            render_attempt(&retired).unwrap_err().reason_code,
            "attempt_invalid_schema"
        );
        attempt["status"] = json!("stopped");
        attempt["final_type"] = json!("instructions_changed");
        attempt["reason"] = json!("The Execute instructions changed.");
        attempt["closing_authorization_evidence"] = json!("User approved this closure.");
        attempt["ended_at"] = json!("2026-09-01T10:05+08:00");
        render_attempt(&attempt).unwrap();
        assert_eq!(attempt["final_type"], "instructions_changed");
        attempt["final_type"] = json!("rules_changed");
        assert_eq!(
            render_attempt(&attempt).unwrap_err().reason_code,
            "attempt_invalid_final_type"
        );
    }

    #[test]
    fn attempt_deviation_ids_are_contiguous_after_validation() {
        let authorization = json!({"schema":"work-attempt-authorization/v1","task_id":"TASK-001",
            "commands":[],"validations":[],"modifiable_files":[],"working_directories":[],
            "external_operations":[],"allowed_deviations":[],"reapproval_conditions":REAPPROVAL,
            "authorization_evidence":"User approved this exact Attempt scope."});
        let action = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":{"mode":"argv","argv":["tool","test"]}});
        let deviation = json!({"schema":"work-execution-deviation/v1","deviation_id":"DEVIATION-001",
            "approved_preview_sha256":"c".repeat(64),
            "proposal":{"schema":"work-execution-deviation-proposal/v1","task_id":"TASK-001",
                "attempt_id":"ATTEMPT-001","anchor_record_id":"CMD-001","task_basis":["CMD-001"],
                "gap":"A command needs an equivalent path.","action":action,"modifiable_files":[],
                "impact":{"summary":"Equivalent command path.","requirement_changed":false,
                    "scope_changed":false,"acceptance_criteria_changed":false,"deliverables_changed":false,
                    "safety_boundary_changed":false,"external_side_effect_boundary_changed":false},
                "side_effects":["Runs the existing command."]},
            "supplemental_authorization":{"schema":"work-execution-deviation-authorization/v1",
                "preview_sha256":"c".repeat(64),"action":action,
                "modifiable_files":[],"authorization_evidence":"Approved"},
            "decision":{"outcome":"approved","evidence":"Approved"},"reconciliation_status":"pending"});
        let mut attempt = json!({"schema":"work-attempt/v1","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"1".repeat(64),"task_item_sha256":"2".repeat(64),
            "task_instructions_sha256":"b".repeat(64),
            "execute_instructions_sha256":"c".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"d".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00",
            "execution_deviations":[deviation],"records":[]});
        render_attempt(&attempt).unwrap();
        assert_eq!(
            attempt["execution_deviations"][0]["deviation_id"],
            "DEVIATION-001"
        );
        attempt["execution_deviations"][0]["deviation_id"] = json!("DEVIATION-002");
        assert_eq!(
            render_attempt(&attempt).unwrap_err().reason_code,
            "attempt_invalid_execution_deviation_sequence"
        );
    }

    #[test]
    fn python_attempt_example_has_identical_bytes() {
        let authorization = json!({"schema":"work-attempt-authorization/v1","task_id":"TASK-001",
            "commands":[],"validations":[],"modifiable_files":[],"working_directories":[],
            "external_operations":[],"allowed_deviations":[],"reapproval_conditions":REAPPROVAL,
            "authorization_evidence":"User approved this exact Attempt scope."});
        assert_eq!(
            canonical_json_sha256(&authorization).unwrap(),
            "30a8dd5343d43861e6a2f0846add3362ed674ca0be96dcdb4020d9004e6ff7aa"
        );
        let attempt = json!({"schema":"work-attempt/v1","attempt_id":"ATTEMPT-001","task_spec_id":"TASK-SPEC-001",
            "task_id":"TASK-001","skill_id":null,"status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),"execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"execute_skill_selection_sha256":"0".repeat(64),
            "authorization":authorization,"authorization_sha256":"30a8dd5343d43861e6a2f0846add3362ed674ca0be96dcdb4020d9004e6ff7aa",
            "started_at":"2026-09-01T10:00+08:00","records":[]});
        let raw = render_attempt(&attempt).unwrap();
        assert_eq!(
            sha256_hex(&raw),
            "7d3a31c951f64355b8b0d5ba460c75a93caedeb39c579d9efb40c2ad9b359bff"
        );
        assert_eq!(
            validate_attempt_bytes(&attempt, &raw).unwrap(),
            json!({"schema":"work-attempt-validation/v1","attempt_id":"ATTEMPT-001",
                "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","status":"in_progress",
                "record_count":0,"result":"valid"})
        );
        let mut with_operation = attempt.clone();
        with_operation["records"] =
            json!([{"id":"OP-001","kind":"operation","outcome":"success","state":"done"}]);
        with_operation["overall_result"] =
            json!({"status":"complete_success","effective":["OP-001"]});
        assert_eq!(
            sha256_hex(&render_attempt(&with_operation).unwrap()),
            "a8033a70fc7ae6473006eb90c57edd99af213631d48d5113da34cb1cd59e4d88"
        );
        with_operation["records"][0]["id"] = json!("OP-001#2");
        assert_eq!(
            render_attempt(&with_operation).unwrap_err().reason_code,
            "attempt_record_retry_sequence"
        );
        let mut corrected_command = attempt.clone();
        corrected_command["records"] = json!([{"id":"CMD-001","kind":"command",
            "correction":{"original_command":{"mode":"argv","argv":["tool"]},
                "actual_command":{"mode":"argv","argv":["/bin/tool"]},
                "reason":"Use absolute path.","authorization_evidence":"Approved replacement."},
            "exit_code":0,"result":"ok"}]);
        assert_eq!(
            sha256_hex(&render_attempt(&corrected_command).unwrap()),
            "c4c547a1f5e4e8940031c2bcea7deb7f20e375ccde03d810dcc81a6e7fdec0b8"
        );
        let mut completed = attempt.clone();
        completed["status"] = json!("completed");
        completed["ended_at"] = json!("2026-09-01T10:05+08:00");
        assert_eq!(
            sha256_hex(&render_attempt(&completed).unwrap()),
            "0f8051fbba68c32bec88da0706e0f1554befc7d0eec29761553b98b8e5f70bc5"
        );
        completed["ended_at"] = json!("2026-08-31T23:00+08:00");
        assert_eq!(
            render_attempt(&completed).unwrap_err().reason_code,
            "attempt_end_before_start"
        );
        let mut noncanonical_file = attempt.clone();
        noncanonical_file["modified_files"] = json!(["./src/main.rs"]);
        assert_eq!(
            render_attempt(&noncanonical_file).unwrap_err().reason_code,
            "attempt_noncanonical_modified_file"
        );
        let mut duplicate_unicode_file = attempt.clone();
        duplicate_unicode_file["modified_files"] =
            json!(["Straße/CAFÉ.txt", "STRASSE/cafe\u{301}.txt"]);
        assert_eq!(
            render_attempt(&duplicate_unicode_file)
                .unwrap_err()
                .reason_code,
            "attempt_duplicate_modified_file"
        );
        let mut unapproved_skip = attempt;
        unapproved_skip["records"] = json!([{"id":"VAL-001","kind":"validation","status":"skipped",
            "reason":"Approved skip","deviation_id":"DEVIATION-001"}]);
        assert_eq!(
            render_attempt(&unapproved_skip).unwrap_err().reason_code,
            "attempt_unapproved_skipped_record"
        );
    }
}
