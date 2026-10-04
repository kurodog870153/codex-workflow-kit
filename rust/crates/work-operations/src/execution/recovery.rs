//! Pure recovery direction checks over already validated execution artifacts.

use serde_json::{Value, json};

use crate::derivation::identity::next_record_id;
use crate::execution::attempt::{render_attempt, validate_attempt_bytes};
use crate::execution::attempt_close::build_close_candidates;
use crate::execution::authorization::{
    effective_task, require_record_finish_result_evidence, require_retry_evidence,
};
use crate::execution::command_correction::validate_command_correction;
use crate::execution::deviation::validate_deviation_artifact;
use crate::execution::index::{render_execution_index, validate_execution_index};
use crate::execution::record_finish::build_record_finish_candidates;
use crate::execution::validate_completed_coverage;
use crate::execution::{ExecutionIssue, finished_record_index, formal_record_kind};

fn issue(reason_code: &'static str, message: &'static str) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

fn valid_id(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|tail| tail.len() == 3 && tail.bytes().all(|byte| byte.is_ascii_digit()))
}

fn record_temp_name(value: &str) -> bool {
    let base = value.split_once("-retry-").map_or(value, |(base, _)| base);
    if !["CMD-", "OP-", "VAL-"]
        .iter()
        .any(|prefix| valid_id(base, prefix))
    {
        return false;
    }
    value.split_once("-retry-").is_none_or(|(_, retry)| {
        !retry.is_empty() && retry.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn correction_file_identity<'a>(file: &'a str, prefix: &str) -> Option<&'a str> {
    let tail = file.strip_prefix(prefix)?;
    let (identity, suffix) = tail.rsplit_once('-')?;
    if !valid_id(identity, "CORRECTION-") {
        return None;
    }
    if !["artifact.tmp", "lock.tmp", "index.tmp"].contains(&suffix) {
        return None;
    }
    Some(identity)
}

pub fn validate_deviation_record_recovery(
    attempt: &Value,
    attempt_raw: &[u8],
    prepared: &Value,
    prepared_raw: &[u8],
) -> Result<(), ExecutionIssue> {
    validate_attempt_bytes(attempt, attempt_raw)?;
    validate_attempt_bytes(prepared, prepared_raw)?;
    let current = attempt["execution_deviations"].as_array();
    let future = prepared["execution_deviations"].as_array().ok_or_else(|| {
        issue(
            "execution_recovery_deviation_append_mismatch",
            "The prepared Attempt must append exactly one execution deviation.",
        )
    })?;
    let previous_count = current.map_or(0, Vec::len);
    if future.len() != previous_count + 1
        || current.is_some_and(|rows| future[..previous_count] != rows[..])
    {
        return Err(issue(
            "execution_recovery_deviation_append_mismatch",
            "The prepared Attempt must append exactly one execution deviation.",
        ));
    }
    validate_deviation_artifact(&future[previous_count])?;
    let mut expected = attempt.clone();
    expected["execution_deviations"] = json!(future);
    if render_attempt(&expected).map_err(|_| {
        issue(
            "execution_recovery_deviation_target_mismatch",
            "The prepared deviation Attempt is not the unique canonical target.",
        )
    })? != prepared_raw
    {
        return Err(issue(
            "execution_recovery_deviation_target_mismatch",
            "The prepared deviation Attempt is not the unique canonical target.",
        ));
    }
    Ok(())
}

pub struct CommandCorrectionRecovery<'a> {
    pub index: &'a Value,
    pub index_raw: &'a [u8],
    pub prepared: &'a Value,
    pub prepared_raw: &'a [u8],
    pub task: &'a Value,
}

pub fn validate_command_correction_recovery(
    input: CommandCorrectionRecovery<'_>,
) -> Result<String, ExecutionIssue> {
    validate_execution_index(input.index, input.index_raw)?;
    validate_execution_index(input.prepared, input.prepared_raw)?;
    let lock = &input.index["lock"];
    let record_id = lock["record_id"]
        .as_str()
        .filter(|id| id.starts_with("CMD-"))
        .ok_or_else(|| {
            issue(
                "execution_recovery_command_lock_required",
                "command_correction recovery requires a reserved CMD record.",
            )
        })?;
    if lock.get("command_correction").is_some() {
        return Err(issue(
            "execution_recovery_command_correction_already_stored",
            "The command correction is already stored without a pending transaction file.",
        ));
    }
    let correction = &input.prepared["lock"]["command_correction"];
    validate_command_correction(correction, "lock.command_correction")?;
    let base = record_id.split('#').next().unwrap_or(record_id);
    let formal = input.task["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|command| command["id"] == base)
        .ok_or_else(|| {
            issue(
                "execution_recovery_original_command_mismatch",
                "The prepared correction does not use the formal TASK command.",
            )
        })?;
    let field = if formal["mode"] == "argv" {
        "argv"
    } else {
        "script"
    };
    let mut original = json!({"mode":formal["mode"]});
    original[field] = formal[field].clone();
    if correction["original_command"] != original {
        return Err(issue(
            "execution_recovery_original_command_mismatch",
            "The prepared correction does not use the formal TASK command.",
        ));
    }
    let mut expected = input.index.clone();
    expected["lock"]["command_correction"] = correction.clone();
    let rendered = render_execution_index(&expected).map_err(|_| {
        issue(
            "execution_recovery_command_correction_target_mismatch",
            "The prepared command_correction index is not the unique canonical target.",
        )
    })?;
    if input.prepared_raw != rendered {
        return Err(issue(
            "execution_recovery_command_correction_target_mismatch",
            "The prepared command_correction index is not the unique canonical target.",
        ));
    }
    Ok(record_id.to_owned())
}

pub fn validate_recovery_direction(
    transaction: &str,
    task_id: &str,
    attempt_id: &str,
    files: &[String],
    index: &Value,
    attempt: &Value,
) -> Result<Value, ExecutionIssue> {
    let prefix = format!(
        ".work-{}-{task_id}-{attempt_id}-",
        transaction.replace('_', "-")
    );
    if files.iter().any(|file| !file.starts_with(&prefix)) {
        return Err(issue(
            "recovery_prepare_mixed_transactions",
            "Preserve foreign, unknown or attempt-start transactions; do not omit them.",
        ));
    }
    let lock = &index["lock"];
    if transaction == "correction" {
        let identities: Vec<_> = files
            .iter()
            .map(|file| correction_file_identity(file, &prefix))
            .collect();
        let identity = identities.first().and_then(|identity| *identity);
        if files.is_empty() || identity.is_none() || identities.iter().any(|row| *row != identity) {
            return Err(issue(
                "recovery_prepare_correction_identity",
                "Exactly one preserved Correction transaction is required.",
            ));
        }
        let correction_id = format!("{attempt_id}-{}", identity.expect("checked identity"));
        if attempt["status"] == "in_progress"
            || !lock.is_null()
                && (lock["kind"] != "correction"
                    || lock["task_id"] != task_id
                    || lock["correction_id"] != correction_id
                    || lock["execute_instructions_sha256"]
                        != attempt["execute_instructions_sha256"])
        {
            return Err(issue(
                "recovery_prepare_lock",
                "The Correction direction conflicts with the Attempt or lock.",
            ));
        }
        return Ok(json!({"correction_id":correction_id,"record_id":null}));
    }
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id);
    if row.is_none_or(|row| row["latest_attempt"] != attempt_id || row["status"] != "in_progress")
        || lock["kind"] != "execution"
        || lock["task_id"] != task_id
        || lock["attempt_id"] != attempt_id
        || lock["execute_instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(issue(
            "recovery_prepare_lock",
            "The active index and execution lock must identify the requested Attempt.",
        ));
    }
    let record_id = lock["record_id"].as_str();
    let safe_record = record_id.map(|id| id.replace('#', "-retry-"));
    let invalid_record = || {
        issue(
            "recovery_prepare_record_identity",
            "The transaction files do not identify the reserved record.",
        )
    };
    match transaction {
        "record_begin" => {
            if record_id.is_some()
                || files.len() != 1
                || !files[0]
                    .strip_prefix(&prefix)
                    .and_then(|name| name.strip_suffix(".tmp"))
                    .is_some_and(record_temp_name)
            {
                return Err(issue(
                    "recovery_prepare_record_identity",
                    "Record-begin requires one prepared record and an unreserved lock.",
                ));
            }
        }
        "command_correction" | "record_finish" | "deviation_record" => {
            let Some(safe_record) = safe_record else {
                return Err(invalid_record());
            };
            let expected: Vec<_> = match transaction {
                "command_correction" => vec![format!("{prefix}{safe_record}.tmp")],
                "record_finish" => vec![
                    format!("{prefix}{safe_record}-attempt.tmp"),
                    format!("{prefix}{safe_record}-index.tmp"),
                ],
                _ => vec![format!("{prefix}{safe_record}-attempt.tmp")],
            };
            if files.iter().any(|file| !expected.contains(file))
                || transaction == "command_correction"
                    && (record_id.is_none_or(|id| !id.starts_with("CMD-"))
                        || lock.get("command_correction").is_some())
            {
                return Err(invalid_record());
            }
        }
        "attempt_close" => {
            let expected = [format!("{prefix}attempt.tmp"), format!("{prefix}index.tmp")];
            if record_id.is_some() || files.iter().any(|file| !expected.contains(file)) {
                return Err(invalid_record());
            }
        }
        _ => return Err(invalid_record()),
    }
    let post_write_evidence = match transaction {
        "attempt_close" => attempt["status"] != "in_progress" && record_id.is_none(),
        "record_finish" => record_id.is_some_and(|id| {
            attempt["records"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|record| record["id"] == id)
        }),
        "deviation_record" => {
            record_id.is_some()
                && attempt["execution_deviations"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
        }
        _ => false,
    };
    if files.is_empty() && !post_write_evidence {
        return Err(issue(
            "recovery_prepare_missing_evidence",
            "No preserved files or supported post-write evidence establish this direction.",
        ));
    }
    Ok(json!({"correction_id":null,"record_id":record_id}))
}

pub struct RecordBeginRecovery<'a> {
    pub index: &'a Value,
    pub index_raw: &'a [u8],
    pub prepared: &'a Value,
    pub prepared_raw: &'a [u8],
    pub attempt: &'a Value,
    pub task: &'a Value,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub temporary_name: &'a str,
}

pub fn validate_record_begin_recovery(
    input: RecordBeginRecovery<'_>,
) -> Result<String, ExecutionIssue> {
    validate_execution_index(input.index, input.index_raw)?;
    validate_execution_index(input.prepared, input.prepared_raw)?;
    let lock = &input.index["lock"];
    if lock.get("record_id").is_some()
        || lock.get("command_correction").is_some()
        || lock.get("retry_authorization_evidence").is_some()
    {
        return Err(issue(
            "execution_recovery_record_begin_state_conflict",
            "record_begin recovery requires an unreserved execution lock.",
        ));
    }
    let record_id = input.prepared["lock"]["record_id"]
        .as_str()
        .ok_or_else(|| {
            issue(
                "execution_recovery_record_id_missing",
                "The prepared record_begin index does not reserve a record.",
            )
        })?;
    let expected_name = format!(
        ".work-record-begin-{}-{}-{}.tmp",
        input.task_id,
        input.attempt_id,
        record_id.replace('#', "-retry-")
    );
    if input.temporary_name != expected_name {
        return Err(ExecutionIssue {
            reason_code: "execution_recovery_file_identity_mismatch",
            message: "The transaction file name does not match its prepared state.",
            details: json!({"expected":expected_name,"actual":input.temporary_name}),
        });
    }
    let base = record_id.split('#').next().unwrap_or(record_id);
    formal_record_kind(input.task, base)?;
    if record_id != next_record_id(base, input.attempt)? {
        return Err(issue(
            "execution_recovery_retry_sequence_mismatch",
            "The prepared record ID is not the next record instance.",
        ));
    }
    let evidence = input.prepared["lock"]["retry_authorization_evidence"].as_str();
    require_retry_evidence(input.attempt, record_id, evidence)?;
    let mut expected = input.index.clone();
    expected["lock"]["record_id"] = json!(record_id);
    if let Some(evidence) = evidence {
        expected["lock"]["retry_authorization_evidence"] = json!(evidence);
    }
    if render_execution_index(&expected).expect("validated JSON") != input.prepared_raw {
        return Err(issue(
            "execution_recovery_record_begin_target_mismatch",
            "The prepared record_begin index is not the unique canonical target.",
        ));
    }
    Ok(record_id.to_owned())
}

pub struct RecordFinishRecovery<'a> {
    pub index: &'a Value,
    pub index_raw: &'a [u8],
    pub attempt: &'a Value,
    pub attempt_raw: &'a [u8],
    pub prepared_attempt: Option<(&'a Value, &'a [u8])>,
    pub task: &'a Value,
    pub authorization_evidence: Option<&'a str>,
}

#[derive(Debug)]
pub struct RecordFinishRecoveryTarget {
    pub record_id: String,
    pub attempt: Option<Vec<u8>>,
    pub index: Vec<u8>,
}

pub fn validate_record_finish_recovery(
    input: RecordFinishRecovery<'_>,
) -> Result<RecordFinishRecoveryTarget, ExecutionIssue> {
    validate_execution_index(input.index, input.index_raw)?;
    validate_attempt_bytes(input.attempt, input.attempt_raw)?;
    let record_id = input.index["lock"]["record_id"].as_str().ok_or_else(|| {
        issue(
            "execution_recovery_record_lock_required",
            "record_finish recovery requires a reserved record.",
        )
    })?;
    let base = record_id.split('#').next().unwrap_or(record_id);
    let effective = effective_task(input.task, input.attempt)?;
    let kind = formal_record_kind(&effective, base)?;
    let current_has_record = input.attempt["records"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|record| record["id"] == record_id);
    let (expected_attempt, finished_attempt) = if let Some((prepared, prepared_raw)) =
        input.prepared_attempt
    {
        if current_has_record {
            return Err(issue(
                "execution_recovery_duplicate_attempt_target",
                "The current Attempt already contains the prepared record.",
            ));
        }
        validate_attempt_bytes(prepared, prepared_raw)?;
        let original_records = input.attempt["records"].as_array().map_or(0, Vec::len);
        let prepared_records = prepared["records"].as_array().map_or(0, Vec::len);
        if prepared_records != original_records + 1 {
            return Err(issue(
                "execution_recovery_record_append_mismatch",
                "The prepared Attempt must append exactly one record.",
            ));
        }
        let mut result_record = prepared["records"][original_records].clone();
        for field in ["correction", "id", "kind"] {
            result_record
                .as_object_mut()
                .expect("validated record")
                .remove(field);
        }
        let mut request = json!({"schema":"work-record-finish-request","record":result_record});
        if let Some(evidence) = input.authorization_evidence {
            request["authorization_evidence"] = json!(evidence);
        }
        if let Some(files) = prepared
            .get("modified_files")
            .filter(|files| Some(*files) != input.attempt.get("modified_files"))
        {
            request["modified_files"] = files.clone();
        }
        let candidate = build_record_finish_candidates(
            input.task,
            input.attempt,
            input.index,
            input.attempt["task_id"].as_str().unwrap_or(""),
            &request,
        )?;
        let expected = candidate.attempt;
        let expected_raw = render_attempt(&expected)?;
        if expected_raw != prepared_raw {
            return Err(issue(
                "execution_recovery_record_finish_target_mismatch",
                "The prepared record_finish Attempt is not the unique canonical target.",
            ));
        }
        require_record_finish_result_evidence(
            input.attempt,
            &prepared["records"][original_records],
            record_id,
            kind,
            input.authorization_evidence,
        )?;
        (Some(expected_raw), expected)
    } else {
        if !current_has_record {
            return Err(issue(
                "execution_recovery_record_result_missing",
                "The record result is not preserved in an Attempt transaction target.",
            ));
        }
        let recorded = input.attempt["records"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|record| record["id"] == record_id)
            .expect("record exists");
        require_record_finish_result_evidence(
            input.attempt,
            recorded,
            record_id,
            kind,
            input.authorization_evidence,
        )?;
        let mut checked = input.attempt.clone();
        checked["acceptance_results"] =
            crate::execution::acceptance::reset(&checked["acceptance_results"])?;
        checked["acceptance_results"] =
            crate::execution::acceptance::aggregate_attempt(&effective, &checked)?;
        if checked["acceptance_results"] != input.attempt["acceptance_results"] {
            return Err(issue(
                "execution_recovery_acceptance_result_mismatch",
                "The installed record result must retain its exact derived acceptance evidence.",
            ));
        }
        (None, checked)
    };
    let expected_index = finished_record_index(input.index)?;
    let expected_index =
        crate::execution::acceptance::update_index_attempt(&expected_index, &finished_attempt)?;
    let index_raw = render_execution_index(&expected_index).expect("validated index serializes");
    validate_execution_index(&expected_index, &index_raw)?;
    Ok(RecordFinishRecoveryTarget {
        record_id: record_id.to_owned(),
        attempt: expected_attempt,
        index: index_raw,
    })
}

pub struct AttemptCloseRecovery<'a> {
    pub index: &'a Value,
    pub index_raw: &'a [u8],
    pub attempt: &'a Value,
    pub attempt_raw: &'a [u8],
    pub prepared_attempt: Option<(&'a Value, &'a [u8])>,
    pub task: &'a Value,
}

#[derive(Debug)]
pub struct AttemptCloseRecoveryTarget {
    pub attempt: Option<Vec<u8>>,
    pub index: Vec<u8>,
    pub status: String,
}

fn close_request(attempt: &Value) -> Value {
    let mut request = json!({"schema":"work-attempt-close-request",
        "status":attempt["status"]});
    if attempt["status"] != "completed" {
        request["final_type"] = attempt["final_type"].clone();
        request["reason"] = attempt["reason"].clone();
        request["authorization_evidence"] = attempt["closing_authorization_evidence"].clone();
    }
    request
}

pub fn validate_attempt_close_recovery(
    input: AttemptCloseRecovery<'_>,
) -> Result<AttemptCloseRecoveryTarget, ExecutionIssue> {
    validate_execution_index(input.index, input.index_raw)?;
    validate_attempt_bytes(input.attempt, input.attempt_raw)?;
    let (closed, expected_attempt) = if let Some((prepared, prepared_raw)) = input.prepared_attempt
    {
        if input.attempt["status"] != "in_progress" {
            return Err(issue(
                "execution_recovery_duplicate_close_target",
                "The current Attempt is already closed while a close target remains.",
            ));
        }
        validate_attempt_bytes(prepared, prepared_raw)?;
        let request = close_request(prepared);
        if request["status"] == "completed" {
            validate_completed_coverage(input.task, input.attempt)?;
        }
        let ended_at = prepared["ended_at"].as_str().unwrap_or("");
        let (expected, _) =
            build_close_candidates(input.task, input.index, input.attempt, &request, ended_at)?;
        let expected_raw = render_attempt(&expected)?;
        if expected_raw != prepared_raw {
            return Err(issue(
                "execution_recovery_attempt_close_target_mismatch",
                "The prepared closed Attempt is not the unique canonical target.",
            ));
        }
        (expected, Some(expected_raw))
    } else {
        if input.attempt["status"] == "in_progress" {
            return Err(issue(
                "execution_recovery_closed_attempt_missing",
                "The closed Attempt is not preserved in a transaction target.",
            ));
        }
        (input.attempt.clone(), None)
    };
    let request = close_request(&closed);
    let (_, expected_index) = build_close_candidates(
        input.task,
        input.index,
        &closed,
        &request,
        closed["ended_at"].as_str().unwrap_or(""),
    )?;
    let index_raw = render_execution_index(&expected_index).expect("validated index serializes");
    validate_execution_index(&expected_index, &index_raw)?;
    Ok(AttemptCloseRecoveryTarget {
        attempt: expected_attempt,
        index: index_raw,
        status: closed["status"].as_str().unwrap_or("").to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_json_sha256;
    use crate::execution::build_execution_lock;
    use crate::execution::index::build_initial_execution_index;

    #[test]
    fn command_correction_recovery_requires_unique_formal_target() {
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"e".repeat(64));
        index["lock"]["record_id"] = json!("CMD-001");
        let task = json!({"commands":[{"id":"CMD-001","mode":"argv",
            "argv":["tool"]}]});
        let correction = json!({"original_command":{"mode":"argv","argv":["tool"]},
            "actual_command":{"mode":"argv","argv":["/bin/tool"]},
            "reason":"Use full path","authorization_evidence":"Approved"});
        let mut prepared = index.clone();
        prepared["lock"]["command_correction"] = correction;
        let index_raw = render_execution_index(&index).unwrap();
        let prepared_raw = render_execution_index(&prepared).unwrap();
        assert_eq!(
            validate_command_correction_recovery(CommandCorrectionRecovery {
                index: &index,
                index_raw: &index_raw,
                prepared: &prepared,
                prepared_raw: &prepared_raw,
                task: &task
            })
            .unwrap(),
            "CMD-001"
        );
        let mut tampered = prepared.clone();
        tampered["lock"]["command_correction"]["original_command"]["argv"] = json!(["other"]);
        let tampered_raw = render_execution_index(&tampered).unwrap();
        assert_eq!(
            validate_command_correction_recovery(CommandCorrectionRecovery {
                index: &index,
                index_raw: &index_raw,
                prepared: &tampered,
                prepared_raw: &tampered_raw,
                task: &task
            })
            .unwrap_err()
            .reason_code,
            "execution_recovery_original_command_mismatch"
        );
    }

    #[test]
    fn record_finish_recovery_rejects_tampered_prepared_attempt() {
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001"]}],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":crate::execution::acceptance::pending(["ACCEPTANCE-001".to_owned()]),"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "acceptance_criteria":[{"id":"ACCEPTANCE-001"}],"tasks":[{"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]}}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"skill_selection_sha256":"0".repeat(64),
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"e".repeat(64));
        index["lock"]["record_id"] = json!("VAL-001");
        let task = json!({"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"commands":[],"operations":[],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001"]}]});
        let request = json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Passed"}});
        let prepared =
            build_record_finish_candidates(&task, &attempt, &index, "TASK-001", &request)
                .unwrap()
                .attempt;
        let attempt_raw = render_attempt(&attempt).unwrap();
        let index_raw = render_execution_index(&index).unwrap();
        let prepared_raw = render_attempt(&prepared).unwrap();
        let target = validate_record_finish_recovery(RecordFinishRecovery {
            index: &index,
            index_raw: &index_raw,
            attempt: &attempt,
            attempt_raw: &attempt_raw,
            prepared_attempt: Some((&prepared, &prepared_raw)),
            task: &task,
            authorization_evidence: None,
        })
        .unwrap();
        assert_eq!(target.record_id, "VAL-001");
        assert_eq!(target.attempt.unwrap(), prepared_raw);
        let finished = crate::canonical::parse_json_contract(&target.index).unwrap();
        assert!(finished["lock"].get("record_id").is_none());
        assert_eq!(finished["acceptance_results"][0]["status"], "completed");
        assert_eq!(
            finished["tasks"][0]["acceptance_results"],
            prepared["acceptance_results"]
        );
        let mut altered = prepared.clone();
        altered["started_at"] = json!("2026-09-01T10:01+08:00");
        let altered_raw = render_attempt(&altered).unwrap();
        assert_eq!(
            validate_record_finish_recovery(RecordFinishRecovery {
                index: &index,
                index_raw: &index_raw,
                attempt: &attempt,
                attempt_raw: &attempt_raw,
                prepared_attempt: Some((&altered, &altered_raw)),
                task: &task,
                authorization_evidence: None,
            })
            .unwrap_err()
            .reason_code,
            "execution_recovery_record_finish_target_mismatch"
        );
        let failed_request = json!({"schema":"work-record-finish-request","record":{"outcome":"failed","evidence":"Failed"},"authorization_evidence":"Fresh failure authorization"});
        let failed_candidate =
            build_record_finish_candidates(&task, &attempt, &index, "TASK-001", &failed_request)
                .unwrap();
        let failed = failed_candidate.attempt;
        let failed_raw = render_attempt(&failed).unwrap();
        let check = |evidence| {
            validate_record_finish_recovery(RecordFinishRecovery {
                index: &index,
                index_raw: &index_raw,
                attempt: &attempt,
                attempt_raw: &attempt_raw,
                prepared_attempt: Some((&failed, &failed_raw)),
                task: &task,
                authorization_evidence: evidence,
            })
        };
        assert_eq!(
            check(None).unwrap_err().reason_code,
            "execution_authorization_result_required"
        );
        assert_eq!(
            check(Some("Approved")).unwrap_err().reason_code,
            "execution_authorization_result_evidence_reused"
        );
        assert!(check(Some("Fresh failure authorization")).is_ok());
        let failed_target = check(Some("Fresh failure authorization")).unwrap();
        let failed_index = crate::canonical::parse_json_contract(&failed_target.index).unwrap();
        assert_eq!(failed_index["acceptance_results"][0]["status"], "pending");
        let mut retry_index = failed_index;
        retry_index["lock"]["record_id"] = json!("VAL-001#1");
        retry_index["lock"]["retry_authorization_evidence"] = json!("Fresh retry authorization");
        let retry_index_raw = render_execution_index(&retry_index).unwrap();
        let retry_request = json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Retry passed"}});
        let retried = build_record_finish_candidates(
            &task,
            &failed,
            &retry_index,
            "TASK-001",
            &retry_request,
        )
        .unwrap()
        .attempt;
        let retried_raw = render_attempt(&retried).unwrap();
        let target = validate_record_finish_recovery(RecordFinishRecovery {
            index: &retry_index,
            index_raw: &retry_index_raw,
            attempt: &failed,
            attempt_raw: &failed_raw,
            prepared_attempt: Some((&retried, &retried_raw)),
            task: &task,
            authorization_evidence: None,
        })
        .unwrap();
        assert_eq!(target.record_id, "VAL-001#1");
        assert_eq!(target.attempt.unwrap(), retried_raw);
        assert_eq!(retried["records"][0], failed["records"][0]);
        assert_eq!(
            retried["acceptance_results"][0]["evidence"][0]["record_id"],
            "VAL-001#1"
        );
        assert_eq!(
            crate::canonical::parse_json_contract(&target.index).unwrap()["acceptance_results"][0]
                ["status"],
            "completed"
        );
    }

    #[test]
    fn retry_record_begin_recovery_requires_exact_authorized_target() {
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"c".repeat(64),
            "hierarchy_selection_sha256":"d".repeat(64),
            "skill_selection_sha256":"e".repeat(64),
            "task_item_sha256":{"TASK-001":"f".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"0".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"1".repeat(64));
        let index_raw = render_execution_index(&index).unwrap();
        let attempt = json!({"records":[{"id":"VAL-001"}],
            "authorization":{"authorization_evidence":"Original"}});
        let task =
            json!({"id":"TASK-001","commands":[],"operations":[],"validations":[{"id":"VAL-001"}]});
        let mut prepared = index.clone();
        prepared["lock"]["record_id"] = json!("VAL-001#1");
        prepared["lock"]["retry_authorization_evidence"] = json!("Approved retry");
        let prepared_raw = render_execution_index(&prepared).unwrap();
        let name = ".work-record-begin-TASK-001-ATTEMPT-001-VAL-001-retry-1.tmp";
        let input = RecordBeginRecovery {
            index: &index,
            index_raw: &index_raw,
            prepared: &prepared,
            prepared_raw: &prepared_raw,
            attempt: &attempt,
            task: &task,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            temporary_name: name,
        };
        assert_eq!(validate_record_begin_recovery(input).unwrap(), "VAL-001#1");
        let mut altered = prepared.clone();
        altered["title"] = json!("Changed");
        let altered_raw = render_execution_index(&altered).unwrap();
        assert_eq!(
            validate_record_begin_recovery(RecordBeginRecovery {
                index: &index,
                index_raw: &index_raw,
                prepared: &altered,
                prepared_raw: &altered_raw,
                attempt: &attempt,
                task: &task,
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                temporary_name: name,
            })
            .unwrap_err()
            .reason_code,
            "execution_recovery_record_begin_target_mismatch"
        );
    }

    #[test]
    fn recovery_direction_requires_matching_files_lock_and_post_write_evidence() {
        let fingerprint = "a".repeat(64);
        let index = json!({"tasks":[{"id":"TASK-001","status":"in_progress",
            "latest_attempt":"ATTEMPT-001"}],"lock":{"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","execute_instructions_sha256":fingerprint}});
        let attempt = json!({"status":"in_progress","execute_instructions_sha256":fingerprint,
            "records":[]});
        let files = vec![".work-record-begin-TASK-001-ATTEMPT-001-CMD-001.tmp".into()];
        assert!(
            validate_recovery_direction(
                "record_begin",
                "TASK-001",
                "ATTEMPT-001",
                &files,
                &index,
                &attempt
            )
            .is_ok()
        );
        assert_eq!(
            validate_recovery_direction(
                "record_begin",
                "TASK-001",
                "ATTEMPT-001",
                &[],
                &index,
                &attempt
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_record_identity"
        );
        let finish_files =
            vec![".work-record-finish-TASK-001-ATTEMPT-001-CMD-001-attempt.tmp".into()];
        assert_eq!(
            validate_recovery_direction(
                "record_finish",
                "TASK-001",
                "ATTEMPT-001",
                &finish_files,
                &index,
                &attempt
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_record_identity"
        );
        let mut reserved = index.clone();
        reserved["lock"]["record_id"] = json!("CMD-001");
        let mut written = attempt.clone();
        written["records"] = json!([{"id":"CMD-001"}]);
        assert!(
            validate_recovery_direction(
                "record_finish",
                "TASK-001",
                "ATTEMPT-001",
                &[],
                &reserved,
                &written
            )
            .is_ok()
        );
        assert_eq!(
            validate_recovery_direction(
                "attempt_close",
                "TASK-001",
                "ATTEMPT-001",
                &[],
                &index,
                &attempt
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_missing_evidence"
        );
        let mut closed_index = reserved.clone();
        closed_index["lock"]
            .as_object_mut()
            .unwrap()
            .remove("record_id");
        let mut closed_attempt = written.clone();
        closed_attempt["status"] = json!("stopped");
        assert!(
            validate_recovery_direction(
                "attempt_close",
                "TASK-001",
                "ATTEMPT-001",
                &[],
                &closed_index,
                &closed_attempt
            )
            .is_ok()
        );
        let foreign = vec![".work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp".into()];
        assert_eq!(
            validate_recovery_direction(
                "record_finish",
                "TASK-001",
                "ATTEMPT-001",
                &foreign,
                &reserved,
                &written
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_mixed_transactions"
        );
    }

    #[test]
    fn correction_and_command_recovery_keep_their_own_identity() {
        let fingerprint = "b".repeat(64);
        let correction_files = vec![
            ".work-correction-TASK-001-ATTEMPT-001-CORRECTION-001-artifact.tmp".into(),
            ".work-correction-TASK-001-ATTEMPT-001-CORRECTION-001-index.tmp".into(),
        ];
        let attempt = json!({"status":"stopped","execute_instructions_sha256":fingerprint});
        let correction_index = json!({"lock":{"kind":"correction","task_id":"TASK-001",
            "correction_id":"ATTEMPT-001-CORRECTION-001",
            "execute_instructions_sha256":fingerprint}});
        let direction = validate_recovery_direction(
            "correction",
            "TASK-001",
            "ATTEMPT-001",
            &correction_files,
            &correction_index,
            &attempt,
        )
        .unwrap();
        assert_eq!(direction["correction_id"], "ATTEMPT-001-CORRECTION-001");
        let mut mixed = correction_files;
        mixed[1] = ".work-correction-TASK-001-ATTEMPT-001-CORRECTION-002-index.tmp".into();
        assert_eq!(
            validate_recovery_direction(
                "correction",
                "TASK-001",
                "ATTEMPT-001",
                &mixed,
                &correction_index,
                &attempt
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_correction_identity"
        );

        let active = json!({"tasks":[{"id":"TASK-001","status":"in_progress",
            "latest_attempt":"ATTEMPT-001"}],"lock":{"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","record_id":"CMD-001#2",
            "execute_instructions_sha256":fingerprint}});
        let mut in_progress = attempt;
        in_progress["status"] = json!("in_progress");
        let command_files =
            vec![".work-command-correction-TASK-001-ATTEMPT-001-CMD-001-retry-2.tmp".into()];
        assert!(
            validate_recovery_direction(
                "command_correction",
                "TASK-001",
                "ATTEMPT-001",
                &command_files,
                &active,
                &in_progress
            )
            .is_ok()
        );
        let mut corrected = active;
        corrected["lock"]["command_correction"] = json!({});
        assert_eq!(
            validate_recovery_direction(
                "command_correction",
                "TASK-001",
                "ATTEMPT-001",
                &command_files,
                &corrected,
                &in_progress
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_record_identity"
        );
    }
}
