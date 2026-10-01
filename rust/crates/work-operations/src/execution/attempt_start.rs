//! Attempt-start artifact construction after authorization and preflight.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::canonical::canonical_json_sha256;
use crate::derivation::identity::next_attempt_id;
use crate::execution::attempt::render_attempt;
use crate::execution::index::{render_execution_index, validate_execution_index};
use crate::execution::{ExecutionIssue, derive_overall_status};
use crate::protocol::ATTEMPT_ID_PREFIX;

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn validate_attempt_namespace(
    existing: &[String],
    original_status: &str,
    latest_attempt: Option<&str>,
    attempt_id: &str,
    allow_current: bool,
) -> Result<(), ExecutionIssue> {
    let mut names: Vec<&str> = existing
        .iter()
        .map(String::as_str)
        .filter(|name| {
            name.strip_prefix(ATTEMPT_ID_PREFIX).is_some_and(|digits| {
                digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .collect();
    names.sort_unstable();
    if original_status == "pending" {
        let allowed = if allow_current {
            vec![attempt_id]
        } else {
            vec![]
        };
        if !names.is_empty() && names != allowed {
            return Err(issue(
                "attempt_start_unexpected_attempt_history",
                "An initial pending TASK has unexpected Attempt history.",
                json!({"attempts":names}),
            ));
        }
    }
    if original_status == "pending_retry" {
        let latest = latest_attempt.unwrap_or("");
        if !names.contains(&latest) {
            return Err(issue(
                "attempt_start_latest_attempt_missing",
                "The latest Attempt file is missing.",
                json!({"latest_attempt":latest_attempt}),
            ));
        }
        let later = names
            .iter()
            .any(|name| *name != attempt_id && *name > latest);
        if later || names.contains(&attempt_id) && !allow_current {
            return Err(issue(
                "attempt_start_attempt_history_conflict",
                "Attempt history conflicts with the next Attempt ID.",
                json!({"attempts":names}),
            ));
        }
    }
    Ok(())
}

pub fn recovery_candidate(
    temporary_files: &[String],
    index: &Value,
    task_id: &str,
) -> Result<String, ExecutionIssue> {
    if index["lock"]["kind"] == "execution" {
        if index["lock"]["task_id"] != task_id {
            return Err(issue(
                "attempt_start_recovery_lock_task_mismatch",
                "The execution lock belongs to a different TASK.",
                json!({"lock":index["lock"]}),
            ));
        }
        return Ok(index["lock"]["attempt_id"]
            .as_str()
            .unwrap_or("")
            .to_owned());
    }
    let mut candidates = std::collections::BTreeSet::new();
    for file in temporary_files {
        let Some(tail) = file.strip_prefix(".work-attempt-start-") else {
            continue;
        };
        let Some((task, tail)) = tail.split_once("-ATTEMPT-") else {
            continue;
        };
        let Some((digits, stage)) = tail.split_once('-') else {
            continue;
        };
        if task == task_id
            && digits.len() == 3
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && matches!(stage, "lock.tmp" | "started.tmp")
        {
            candidates.insert(format!("ATTEMPT-{digits}"));
        }
    }
    if candidates.len() != 1 {
        return Err(issue(
            "attempt_start_recovery_candidate_ambiguous",
            "Recovery requires exactly one matching Attempt-start transaction.",
            json!({"candidates":candidates}),
        ));
    }
    Ok(candidates.into_iter().next().expect("one candidate"))
}

pub fn recovery_base_index(
    index: &Value,
    task_id: &str,
    attempt_id: &str,
    original_status: &str,
    source_attempt_id: Option<&str>,
) -> Result<Value, ExecutionIssue> {
    if !matches!(original_status, "pending" | "pending_retry") {
        return Err(issue(
            "attempt_start_invalid_original_status",
            "The original TASK status is not eligible for Attempt start.",
            json!({}),
        ));
    }
    let mut base = index.clone();
    if let Some(lock) = base.get("lock") {
        if lock["kind"] != "execution"
            || lock["task_id"] != task_id
            || lock["attempt_id"] != attempt_id
        {
            return Err(issue(
                "attempt_start_recovery_lock_mismatch",
                "The execution lock does not match the recovery candidate.",
                json!({}),
            ));
        }
        base.as_object_mut().expect("index object").remove("lock");
    }
    let rows = base["tasks"].as_array_mut().ok_or_else(|| {
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
                "attempt_start_task_not_found",
                "The target TASK is not in the execution index.",
                json!({}),
            )
        })?;
    match row["status"].as_str() {
        Some("in_progress") => {
            row["status"] = json!(original_status);
            if original_status == "pending" {
                row.as_object_mut()
                    .expect("TASK row")
                    .remove("latest_attempt");
            } else {
                row["latest_attempt"] = json!(source_attempt_id);
            }
        }
        Some(status) if status == original_status => {}
        _ => {
            return Err(issue(
                "attempt_start_recovery_status_mismatch",
                "The execution index status conflicts with the Attempt-start request.",
                json!({}),
            ));
        }
    }
    let statuses = rows
        .iter()
        .filter_map(|row| row["status"].as_str())
        .collect::<Vec<_>>();
    base["overall_status"] = json!(derive_overall_status(&statuses));
    let raw = render_execution_index(&base).expect("JSON index");
    validate_execution_index(&base, &raw)?;
    Ok(base)
}

pub fn build_attempt_candidate(
    preflight: &Value,
    index: &Value,
    request: &Value,
    attempt_id: &str,
    started_at: &str,
    source_attempt: Option<&Value>,
) -> Result<Value, ExecutionIssue> {
    let task_id = preflight["task_id"].as_str().unwrap_or("");
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            issue(
                "attempt_start_task_not_found",
                "The target TASK is not present in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    let status = row["status"].as_str().unwrap_or("");
    let continuation = request.get("continuation").filter(|value| !value.is_null());
    let mut attempt = json!({
        "schema":"work-attempt/v1", "attempt_id":attempt_id,
        "task_spec_id":preflight["task_spec_id"], "task_id":task_id,
        "skill_id":preflight["skill_id"], "status":"in_progress",
        "task_collection_sha256":preflight["task_collection_sha256"],
        "task_index_sha256":preflight["task_index_sha256"],
        "task_item_sha256":preflight["task_item_sha256"],
        "task_instructions_sha256":preflight["task_instructions_sha256"],
        "execute_instructions_sha256":preflight["execute_instructions_sha256"],
        "hierarchy_selection_sha256":index["hierarchy_selection_sha256"],
        "execute_skill_selection_sha256":index["skill_selection_sha256"],
        "authorization":request["authorization"],
        "authorization_sha256":canonical_json_sha256(&request["authorization"])
            .expect("JSON authorization serializes"),
        "started_at":started_at,"records":[],
    });
    let expected = match status {
        "pending" => {
            if continuation.is_some() {
                return Err(issue(
                    "attempt_start_unexpected_continuation",
                    "An initial pending TASK cannot use continuation data.",
                    json!({}),
                ));
            }
            next_attempt_id(None)?
        }
        "pending_retry" => {
            let continuation = continuation.ok_or_else(|| {
                issue(
                    "attempt_start_continuation_required",
                    "A pending_retry TASK requires continuation data.",
                    json!({}),
                )
            })?;
            let latest = row["latest_attempt"].as_str().ok_or_else(|| {
                issue(
                    "attempt_start_latest_attempt_required",
                    "A pending_retry TASK must identify its latest Attempt.",
                    json!({}),
                )
            })?;
            if continuation["source_attempt_id"] != latest {
                return Err(issue(
                    "attempt_start_continuation_source_mismatch",
                    "The continuation source must be the latest Attempt.",
                    json!({"expected":latest,"actual":continuation["source_attempt_id"]}),
                ));
            }
            let source = source_attempt.ok_or_else(|| {
                issue(
                    "attempt_start_source_not_closed",
                    "A continuation source Attempt must be closed.",
                    json!({}),
                )
            })?;
            if source["status"] == "in_progress" {
                return Err(issue(
                    "attempt_start_source_not_closed",
                    "A continuation source Attempt must be closed.",
                    json!({}),
                ));
            }
            for field in [
                "skill_id",
                "hierarchy_selection_sha256",
                "execute_skill_selection_sha256",
            ] {
                if source[field] != attempt[field] {
                    return Err(issue(
                        "attempt_start_continuation_skill_identity_mismatch",
                        "A continuation must preserve its source Attempt selection identity.",
                        json!({"field":field,"expected":source[field],"actual":attempt[field]}),
                    ));
                }
            }
            let available: HashSet<_> = source["records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|record| record["id"].as_str())
                .chain(
                    source["carried_records"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|record| record["record_id"].as_str()),
                )
                .collect();
            let mut unknown: Vec<_> = continuation["carried_records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|record| record["record_id"].as_str())
                .filter(|id| !available.contains(id))
                .collect();
            unknown.sort_unstable();
            unknown.dedup();
            if !unknown.is_empty() {
                return Err(issue(
                    "attempt_start_unknown_carried_record",
                    "A requested carried record does not exist in the source Attempt.",
                    json!({"record_ids":unknown}),
                ));
            }
            attempt["continued_from"] = json!(latest);
            if let Some(carried) = continuation["carried_records"]
                .as_array()
                .filter(|rows| !rows.is_empty())
            {
                attempt["carried_records"] = json!(
                    carried
                        .iter()
                        .map(|record| json!({"source_attempt_id":latest,
                        "record_id":record["record_id"],"evidence":record["evidence"]}))
                        .collect::<Vec<_>>()
                );
            }
            if source["modified_files"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
            {
                attempt["modified_files"] = source["modified_files"].clone();
            }
            next_attempt_id(Some(latest))?
        }
        _ => {
            return Err(issue(
                "attempt_start_invalid_original_status",
                "The original TASK status is not eligible for Attempt start.",
                json!({"status":status}),
            ));
        }
    };
    if attempt_id != expected {
        return Err(issue(
            "attempt_start_id_mismatch",
            "The Attempt ID is not the next ID for the TASK.",
            json!({"expected":expected,"actual":attempt_id}),
        ));
    }
    render_attempt(&attempt)?;
    Ok(attempt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::index::build_initial_execution_index;

    #[test]
    fn recovery_base_restores_pending_index_from_started_state() {
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"}]});
        let validation = json!({"task_instructions_sha256":{"TASK-001":"a".repeat(64)},
            "task_skill_ids":{"TASK-001":null},"task_item_sha256":{"TASK-001":"b".repeat(64)},
            "instructions_sha256":"c".repeat(64),"hierarchy_selection_sha256":"d".repeat(64),
            "skill_selection_sha256":"e".repeat(64),"task_collection_sha256":"f".repeat(64),
            "task_index_sha256":"0".repeat(64)});
        let initial = build_initial_execution_index(&collection, &validation).unwrap();
        let mut started = initial.clone();
        started["tasks"][0]["status"] = json!("in_progress");
        started["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        started["overall_status"] = json!("in_progress");
        started["lock"] = json!({"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","execute_instructions_sha256":"1".repeat(64)});
        assert_eq!(
            recovery_base_index(&started, "TASK-001", "ATTEMPT-001", "pending", None).unwrap(),
            initial
        );
    }

    #[test]
    fn namespace_rejects_unexpected_or_conflicting_attempts() {
        assert_eq!(
            validate_attempt_namespace(
                &["ATTEMPT-001".into()],
                "pending",
                None,
                "ATTEMPT-001",
                false
            )
            .unwrap_err()
            .reason_code,
            "attempt_start_unexpected_attempt_history"
        );
        let names = vec!["ATTEMPT-001".into(), "unrelated".into()];
        assert_eq!(
            validate_attempt_namespace(&names, "pending", None, "ATTEMPT-002", false)
                .unwrap_err()
                .reason_code,
            "attempt_start_unexpected_attempt_history"
        );
        assert_eq!(
            validate_attempt_namespace(
                &[],
                "pending_retry",
                Some("ATTEMPT-001"),
                "ATTEMPT-002",
                false
            )
            .unwrap_err()
            .reason_code,
            "attempt_start_latest_attempt_missing"
        );
        let names = vec!["ATTEMPT-001".into(), "ATTEMPT-002".into()];
        assert_eq!(
            validate_attempt_namespace(
                &names,
                "pending_retry",
                Some("ATTEMPT-001"),
                "ATTEMPT-002",
                false
            )
            .unwrap_err()
            .reason_code,
            "attempt_start_attempt_history_conflict"
        );
        validate_attempt_namespace(
            &names,
            "pending_retry",
            Some("ATTEMPT-001"),
            "ATTEMPT-002",
            true,
        )
        .unwrap();
    }

    #[test]
    fn recovery_candidate_uses_lock_or_exactly_one_transaction_identity() {
        let index = json!({"lock":{"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-002"}});
        assert_eq!(
            recovery_candidate(&[], &index, "TASK-001").unwrap(),
            "ATTEMPT-002"
        );
        assert_eq!(
            recovery_candidate(&[], &index, "TASK-002")
                .unwrap_err()
                .reason_code,
            "attempt_start_recovery_lock_task_mismatch"
        );
        let files = vec![
            ".work-attempt-start-TASK-001-ATTEMPT-003-lock.tmp".into(),
            ".work-attempt-start-TASK-001-ATTEMPT-003-started.tmp".into(),
            ".work-attempt-start-TASK-002-ATTEMPT-001-lock.tmp".into(),
        ];
        assert_eq!(
            recovery_candidate(&files, &json!({}), "TASK-001").unwrap(),
            "ATTEMPT-003"
        );
        let mut ambiguous = files;
        ambiguous.push(".work-attempt-start-TASK-001-ATTEMPT-004-lock.tmp".into());
        assert_eq!(
            recovery_candidate(&ambiguous, &json!({}), "TASK-001")
                .unwrap_err()
                .reason_code,
            "attempt_start_recovery_candidate_ambiguous"
        );
    }

    #[test]
    fn initial_and_retry_attempts_preserve_source_selection_and_records() {
        let authorization = json!({"schema":"work-attempt-authorization/v1",
            "task_id":"TASK-001","commands":[],"validations":[],"modifiable_files":[],
            "working_directories":[],"external_operations":[],"allowed_deviations":[],
            "reapproval_conditions":["scope_expansion","source_or_worktree_drift",
                "failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let request = json!({"authorization":authorization});
        let preflight = json!({"task_id":"TASK-001","task_spec_id":"TASK-SPEC-001",
            "skill_id":null,"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64)});
        let index = json!({"tasks":[{"id":"TASK-001","status":"pending"}],
            "hierarchy_selection_sha256":"f".repeat(64),"skill_selection_sha256":"0".repeat(64)});
        let initial = build_attempt_candidate(
            &preflight,
            &index,
            &request,
            "ATTEMPT-001",
            "2026-09-01T10:00+08:00",
            None,
        )
        .unwrap();
        assert_eq!(initial["status"], "in_progress");
        assert_eq!(initial["schema"], "work-attempt/v1");
        for (field, expected) in [
            ("task_collection_sha256", "a"),
            ("task_index_sha256", "b"),
            ("task_item_sha256", "c"),
            ("task_instructions_sha256", "d"),
            ("execute_instructions_sha256", "e"),
            ("hierarchy_selection_sha256", "f"),
            ("execute_skill_selection_sha256", "0"),
        ] {
            assert_eq!(initial[field], expected.repeat(64), "{field}");
        }
        assert!(initial["skill_id"].is_null());
        for legacy in ["task_rules_sha256", "execute_rules_sha256", "task_sha256"] {
            assert!(initial.get(legacy).is_none(), "{legacy}");
        }
        let mut retry_index = index;
        retry_index["tasks"][0]["status"] = json!("pending_retry");
        retry_index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        let mut source = initial;
        source["status"] = json!("stopped");
        source["records"] = json!([{"id":"CMD-001"}]);
        source["modified_files"] = json!(["src/main.rs"]);
        let retry_request = json!({"authorization":authorization,
            "continuation":{"source_attempt_id":"ATTEMPT-001",
                "carried_records":[{"record_id":"CMD-001","evidence":"Carry result"}]}});
        let retry = build_attempt_candidate(
            &preflight,
            &retry_index,
            &retry_request,
            "ATTEMPT-002",
            "2026-09-01T10:10+08:00",
            Some(&source),
        )
        .unwrap();
        assert_eq!(retry["continued_from"], "ATTEMPT-001");
        assert_eq!(
            retry["carried_records"][0]["source_attempt_id"],
            "ATTEMPT-001"
        );
        assert_eq!(retry["modified_files"], json!(["src/main.rs"]));
        let mut changed_skill = source.clone();
        changed_skill["skill_id"] = json!("different");
        assert_eq!(
            build_attempt_candidate(
                &preflight,
                &retry_index,
                &retry_request,
                "ATTEMPT-002",
                "2026-09-01T10:10+08:00",
                Some(&changed_skill),
            )
            .unwrap_err()
            .reason_code,
            "attempt_start_continuation_skill_identity_mismatch"
        );
    }
}
