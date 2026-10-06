//! Attempt-close artifact candidates after lifecycle and authorization checks.

use serde_json::{Value, json};

use crate::execution::attempt::render_attempt;
use crate::execution::index::{render_execution_index, validate_execution_index};
use crate::execution::requests::validate_attempt_close_request;
use crate::execution::{ExecutionIssue, close_index, closed_task_status};

pub struct AttemptCloseStagingInput<'a> {
    pub canonical_root: &'a str,
    pub requirement: &'a crate::identifiers::RequirementId,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub task: &'a Value,
    pub index_before: &'a [u8],
    pub index_after: &'a [u8],
    pub attempt_before: &'a [u8],
    pub attempt_after: &'a [u8],
}

pub fn build_attempt_close_staging(
    input: AttemptCloseStagingInput<'_>,
) -> Result<work_model::runtime::RuntimeManifest, ExecutionIssue> {
    use crate::canonical::parse_json_contract;
    use crate::derivation::fingerprint;
    use work_model::runtime::{RuntimeBytes, RuntimeTarget};
    let issue = |reason_code, message| ExecutionIssue {
        reason_code,
        message,
        details: json!({}),
    };
    let parse = |raw| {
        parse_json_contract(raw).map_err(|_| {
            issue(
                "attempt_close_staging_contract",
                "Canonical artifacts are required.",
            )
        })
    };
    let index = parse(input.index_before)?;
    let before = parse(input.attempt_before)?;
    let after = parse(input.attempt_after)?;
    validate_execution_index(&index, input.index_before)?;
    crate::execution::attempt::validate_attempt_bytes(&before, input.attempt_before)?;
    crate::execution::attempt::validate_attempt_bytes(&after, input.attempt_after)?;
    let lock = &index["lock"];
    if index["requirement_id"] != input.requirement.as_str()
        || before["task_id"] != input.task_id
        || before["attempt_id"] != input.attempt_id
        || before["status"] != "in_progress"
        || lock["kind"] != "execution"
        || lock["task_id"] != input.task_id
        || lock["attempt_id"] != input.attempt_id
        || lock["execute_instructions_sha256"] != before["execute_instructions_sha256"]
    {
        return Err(issue(
            "attempt_close_staging_identity",
            "The active Attempt identities disagree.",
        ));
    }
    let mut request = json!({"schema":"work-attempt-close-request","status":after["status"]});
    if after["status"] != "completed" {
        request["final_type"] = after["final_type"].clone();
        request["reason"] = after["reason"].clone();
        request["authorization_evidence"] = after["closing_authorization_evidence"].clone();
    }
    if lock.get("record_id").is_some() || lock.get("command_correction").is_some() {
        let blocking = before["execution_deviations"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|row| {
                row["decision"]["outcome"] == "approved"
                    && row["reconciliation_status"] == "pending"
                    && row["proposal"]["anchor_record_id"] == lock["record_id"]
                    && crate::execution::deviation_reconciliation_target(&row["proposal"])
                        == "task_and_execution"
            });
        if lock.get("command_correction").is_some()
            || !blocking
            || request["status"] != "stopped"
            || request["final_type"] != "specification_defect"
        {
            return Err(issue(
                "attempt_close_record_reserved",
                "The reserved record cannot close without its blocking specification deviation.",
            ));
        }
    }
    if request["status"] == "completed" {
        crate::execution::validate_completed_coverage(input.task, &before)?;
    }
    let (expected_attempt, expected_index) = build_close_candidates(
        input.task,
        &index,
        &before,
        &request,
        after["ended_at"].as_str().unwrap_or(""),
    )?;
    if render_attempt(&expected_attempt)? != input.attempt_after
        || render_execution_index(&expected_index).map_err(|_| {
            issue(
                "attempt_close_staging_contract",
                "The index cannot be rendered.",
            )
        })? != input.index_after
    {
        return Err(issue(
            "attempt_close_staging_transition",
            "The prepared artifacts are not the unique close transition.",
        ));
    }
    let bytes = |raw: &[u8]| RuntimeBytes {
        bytes: raw.to_vec(),
        sha256: fingerprint::raw(raw),
    };
    let payloads = std::collections::BTreeMap::from([
        ("attempt.json.tmp".into(), input.attempt_after.to_vec()),
        ("index.json.tmp".into(), input.index_after.to_vec()),
    ]);
    crate::execution::recovery::build_execution_staging_manifest(
        crate::execution::recovery::ExecutionStagingInput {
            canonical_root: input.canonical_root,
            requirement: input.requirement,
            execution_dir: input.execution_dir,
            operation: crate::derivation::publication::RuntimeOperation::AttemptClose,
            approval_sha256: &fingerprint::structured(&request).map_err(|_| {
                issue(
                    "attempt_close_staging_contract",
                    "The request cannot be fingerprinted.",
                )
            })?,
            business_identity: json!({"task_id":input.task_id,"attempt_id":input.attempt_id,"attempt_close_request":request}),
            targets: vec![
                RuntimeTarget {
                    path: format!(
                        "{}/{}/{}/attempt.json",
                        input.execution_dir, input.task_id, input.attempt_id
                    ),
                    before: Some(bytes(input.attempt_before)),
                    after: Some(bytes(input.attempt_after)),
                },
                RuntimeTarget {
                    path: format!("{}/index.json", input.execution_dir),
                    before: Some(bytes(input.index_before)),
                    after: Some(bytes(input.index_after)),
                },
            ],
            payloads: &payloads,
        },
    )
}

pub fn build_close_candidates(
    task: &Value,
    index: &Value,
    attempt: &Value,
    request: &Value,
    ended_at: &str,
) -> Result<(Value, Value), ExecutionIssue> {
    validate_attempt_close_request(request)?;
    let task_id = attempt["task_id"].as_str().unwrap_or("");
    let attempt_id = attempt["attempt_id"].as_str().unwrap_or("");
    let mut closed = attempt.clone();
    closed["status"] = request["status"].clone();
    if request["status"] == "completed" {
        if let Some(object) = closed.as_object_mut() {
            object.remove("final_type");
            object.remove("reason");
            object.remove("closing_authorization_evidence");
        }
    } else {
        closed["final_type"] = request["final_type"].clone();
        closed["reason"] = request["reason"].clone();
        closed["closing_authorization_evidence"] = request["authorization_evidence"].clone();
    }
    closed["ended_at"] = json!(ended_at);
    closed["acceptance_results"] =
        crate::execution::acceptance::reset(&closed["acceptance_results"])?;
    if request["status"] == "completed" {
        closed["acceptance_results"] =
            crate::execution::acceptance::aggregate_attempt(task, &closed)?;
    }
    render_attempt(&closed)?;
    let task_status = closed_task_status(
        request["status"].as_str().unwrap_or(""),
        request["final_type"].as_str(),
    );
    let updated_index = close_index(index, task_id, attempt_id, task_status)?;
    let updated_index =
        crate::execution::acceptance::update_index_attempt(&updated_index, &closed)?;
    let raw_index = render_execution_index(&updated_index).expect("JSON index serializes");
    validate_execution_index(&updated_index, &raw_index)?;
    Ok((closed, updated_index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_json_sha256;
    use crate::execution::build_execution_lock;

    #[test]
    fn completed_and_stopped_candidates_update_attempt_and_index_together() {
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[],"validations":[],"modifiable_files":[],
            "working_directories":[],"external_operations":[],"allowed_deviations":[],
            "reapproval_conditions":["scope_expansion","source_or_worktree_drift",
                "failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":[],"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let index = json!({"acceptance_results":[],"schema":"work-execution-index","requirement_id":"demo",
            "title":"Execution","task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),"overall_status":"in_progress",
            "lock":build_execution_lock("TASK-001","ATTEMPT-001",&"e".repeat(64)),
            "tasks":[{"id":"TASK-001","status":"in_progress","skill_id":null,
                "task_item_sha256":"c".repeat(64),"acceptance_results":[],"instructions_sha256":"d".repeat(64),
                "latest_attempt":"ATTEMPT-001"}]});
        let completed = json!({"schema":"work-attempt-close-request","status":"completed"});
        let (closed, updated) = build_close_candidates(
            &json!({"id":"TASK-001"}),
            &index,
            &attempt,
            &completed,
            "2026-09-01T10:10+08:00",
        )
        .unwrap();
        assert_eq!(closed["status"], "completed");
        assert_eq!(updated["tasks"][0]["status"], "completed");
        assert!(updated.get("lock").is_none());
        let stopped = json!({"schema":"work-attempt-close-request","status":"stopped",
            "final_type":"other","reason":"Need retry","authorization_evidence":"Approved close"});
        let (closed, updated) = build_close_candidates(
            &json!({"id":"TASK-001"}),
            &index,
            &attempt,
            &stopped,
            "2026-09-01T10:10+08:00",
        )
        .unwrap();
        assert_eq!(closed["final_type"], "other");
        assert_eq!(updated["tasks"][0]["status"], "pending_retry");
        assert_eq!(updated["tasks"][0]["status_reason"]["ref"], "ATTEMPT-001");
    }
}
