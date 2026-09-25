//! Attempt-close artifact candidates after lifecycle and authorization checks.

use serde_json::{Value, json};

use crate::execution::attempt::render_attempt;
use crate::execution::index::{render_execution_index, validate_execution_index};
use crate::execution::requests::validate_attempt_close_request;
use crate::execution::{ExecutionIssue, close_index, closed_task_status};

pub fn build_close_candidates(
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
    render_attempt(&closed)?;
    let task_status = closed_task_status(
        request["status"].as_str().unwrap_or(""),
        request["final_type"].as_str(),
    );
    let updated_index = close_index(index, task_id, attempt_id, task_status)?;
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
        let authorization = json!({"schema":"work-attempt-authorization/v1",
            "task_id":"TASK-001","commands":[],"validations":[],"modifiable_files":[],
            "working_directories":[],"external_operations":[],"allowed_deviations":[],
            "reapproval_conditions":["scope_expansion","source_or_worktree_drift",
                "failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"schema":"work-attempt/v1","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let index = json!({"schema":"work-execution-index/v1","requirement_id":"demo",
            "title":"Execution","task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "skill_selection_sha256":"0".repeat(64),"overall_status":"in_progress",
            "lock":build_execution_lock("TASK-001","ATTEMPT-001",&"e".repeat(64)),
            "tasks":[{"id":"TASK-001","status":"in_progress","skill_id":null,
                "task_item_sha256":"c".repeat(64),"instructions_sha256":"d".repeat(64),
                "latest_attempt":"ATTEMPT-001"}]});
        let completed = json!({"schema":"work-attempt-close-request/v1","status":"completed"});
        let (closed, updated) =
            build_close_candidates(&index, &attempt, &completed, "2026-09-01T10:10+08:00").unwrap();
        assert_eq!(closed["status"], "completed");
        assert_eq!(updated["tasks"][0]["status"], "completed");
        assert!(updated.get("lock").is_none());
        let stopped = json!({"schema":"work-attempt-close-request/v1","status":"stopped",
            "final_type":"other","reason":"Need retry","authorization_evidence":"Approved close"});
        let (closed, updated) =
            build_close_candidates(&index, &attempt, &stopped, "2026-09-01T10:10+08:00").unwrap();
        assert_eq!(closed["final_type"], "other");
        assert_eq!(updated["tasks"][0]["status"], "pending_retry");
        assert_eq!(updated["tasks"][0]["status_reason"]["ref"], "ATTEMPT-001");
    }
}
