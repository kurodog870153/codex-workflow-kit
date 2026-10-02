//! Pure workflow state decisions before execution artifacts are available.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDecision {
    pub status: String,
    pub next_action: String,
    pub target_artifact: &'static str,
    pub requires_user_confirmation: bool,
    pub required_checks: Vec<String>,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
}

pub fn decide_pre_execution(
    plan_validation: Option<&Value>,
    draft: Option<&Value>,
    task_validation: Option<&Value>,
    execution_index_exists: bool,
) -> Result<Option<WorkflowDecision>, WorkflowIssue> {
    let decision = match (plan_validation, task_validation, execution_index_exists) {
        (None, _, _) => WorkflowDecision {
            status: "plan_required".into(),
            next_action: "prepare_plan".into(),
            target_artifact: "plan",
            requires_user_confirmation: true,
            required_checks: vec![],
            details: json!({}),
        },
        (Some(plan_validation), None, _) => {
            let draft = draft.ok_or(WorkflowIssue {
                reason_code: "workflow_draft_required",
                message: "A TASK draft state is required before formal TASK creation.",
            })?;
            let status = draft["status"].as_str().ok_or(WorkflowIssue {
                reason_code: "workflow_draft_invalid",
                message: "The TASK draft state is invalid.",
            })?;
            let action = draft["next_action"].as_str().ok_or(WorkflowIssue {
                reason_code: "workflow_draft_invalid",
                message: "The TASK draft action is invalid.",
            })?;
            let checks = draft["required_checks"]
                .as_array()
                .filter(|checks| checks.iter().all(Value::is_string))
                .ok_or(WorkflowIssue {
                    reason_code: "workflow_draft_invalid",
                    message: "The TASK draft checks are invalid.",
                })?
                .iter()
                .map(|value| value.as_str().expect("checked string").to_owned())
                .collect();
            let confirmation =
                draft["requires_user_confirmation"]
                    .as_bool()
                    .ok_or(WorkflowIssue {
                        reason_code: "workflow_draft_invalid",
                        message: "The TASK draft confirmation state is invalid.",
                    })?;
            WorkflowDecision {
                status: format!("task_{status}"),
                next_action: action.into(),
                target_artifact: "task",
                requires_user_confirmation: confirmation,
                required_checks: checks,
                details: json!({"plan_sha256":plan_validation["plan_sha256"],"draft":draft}),
            }
        }
        (Some(_), Some(task_validation), false) => WorkflowDecision {
            status: "execution_recovery_required".into(),
            next_action: "inspect_recovery".into(),
            target_artifact: "execution",
            requires_user_confirmation: true,
            required_checks: vec!["task validate".into()],
            details: json!({"task_collection_sha256":task_validation["task_collection_sha256"]}),
        },
        _ => return Ok(None),
    };
    Ok(Some(decision))
}

pub fn decide_execution(index: &Value, attempts: &Value) -> WorkflowDecision {
    let target = "execution";
    let checks = vec!["task validate".into(), "execute preflight".into()];
    if !index["lock"].is_null() {
        let lock = &index["lock"];
        let task_id = lock["task_id"].as_str().unwrap_or("");
        let row = index["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == task_id);
        let attempt = &attempts[task_id];
        let active = lock["kind"] == "execution"
            && row.is_some_and(|row| {
                row["status"] == "in_progress" && row["latest_attempt"] == lock["attempt_id"]
            })
            && attempt["status"] == "in_progress"
            && attempt["task_id"] == lock["task_id"]
            && attempt["attempt_id"] == lock["attempt_id"]
            && attempt["execute_instructions_sha256"] == lock["execute_instructions_sha256"];
        if !active {
            return WorkflowDecision {
                status: "execution_locked".into(),
                next_action: "inspect_recovery".into(),
                target_artifact: target,
                requires_user_confirmation: true,
                required_checks: vec!["execute preflight".into()],
                details: json!({"lock":lock}),
            };
        }
    }
    let overall = index["overall_status"].as_str().unwrap_or("");
    let mut unresolved = Vec::new();
    if let Some(attempts) = attempts.as_object() {
        for attempt in attempts.values() {
            if attempt["status"] == "in_progress" {
                continue;
            }
            let resolved = attempt["_reconciliation_resolved_ids"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for deviation in attempt["execution_deviations"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if deviation["decision"]["outcome"] == "approved"
                    && deviation["reconciliation_status"] == "pending"
                    && !resolved.contains(&deviation["deviation_id"])
                {
                    unresolved.push(deviation["deviation_id"].clone());
                }
            }
        }
    }
    if matches!(overall, "completed" | "cancelled") && !unresolved.is_empty() {
        unresolved.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        return WorkflowDecision {
            status: "execution_reconciliation_pending".into(),
            next_action: "review_reconciliation".into(),
            target_artifact: target,
            requires_user_confirmation: true,
            required_checks: checks,
            details: json!({"overall_status":overall,"pending_deviation_ids":unresolved}),
        };
    }
    let (action, confirmation) = match overall {
        "pending" => ("select_task_for_execution", true),
        "in_progress" => ("continue_execution", false),
        "pending_retry" => ("confirm_retry", true),
        "blocked" => ("review_reconciliation", true),
        "completed" | "cancelled" => ("review_completion", true),
        _ => ("review_state", true),
    };
    WorkflowDecision {
        status: format!("execution_{overall}"),
        next_action: action.into(),
        target_artifact: target,
        requires_user_confirmation: confirmation,
        required_checks: checks,
        details: json!({"overall_status":overall}),
    }
}

pub fn mode_for_action(status: &str, next_action: &str) -> &'static str {
    if next_action == "prepare_plan" {
        "plan"
    } else if status.starts_with("task_") {
        "task"
    } else if next_action == "inspect_recovery" {
        "execute"
    } else if next_action == "review_reconciliation" {
        "specification"
    } else {
        "execute"
    }
}

pub fn lifecycle_for_status(status: &str) -> &str {
    if status.ends_with("required") || status == "plan_required" {
        "missing"
    } else {
        status
    }
}

pub fn next_action_guidance(next_action: &str, requirement_id: &str, artifacts: &Value) -> Value {
    let common = json!({"user_config_root":"<user-config-root>"});
    let task = json!({"requirement_id":requirement_id,"plan_path":artifacts["plan"],"user_config_root":"<user-config-root>"});
    let execute = json!({"task_path":artifacts["task"],"execution_dir":artifacts["execution"],"task_id":"<confirmed-task-id>","user_config_root":"<user-config-root>"});
    let (command, contract, arguments): (Option<&str>, Option<&str>, Value) = match next_action {
        "prepare_plan" => (
            Some("plan semantic-prepare"),
            Some("work-plan-semantic-request/v1"),
            json!({"input_file":"<semantic-input-file>","user_config_root":"<user-config-root>"}),
        ),
        "confirm_task_list" => (
            Some("task prepare"),
            Some("work-task-semantic-request/v1"),
            {
                let mut value = task;
                value["input_file"] = json!("<semantic-input-file>");
                value
            },
        ),
        "choose_task" => (Some("task status"), None, task.clone()),
        "confirm_start" | "confirm_resume" | "confirm_review" => (
            Some("task status"),
            None,
            json!({"requirement_id":requirement_id,"plan_path":artifacts["plan"],"user_config_root":"<user-config-root>","task_id":"<confirmed-task-id>"}),
        ),
        "select_task_for_execution" | "confirm_retry" => (Some("execute preflight"), None, execute),
        "review_reconciliation" => (
            Some("specification reconciliation-prepare"),
            Some("work-spec-reconciliation-prepare-request/v1"),
            {
                let mut value = common;
                value["input_file"] = json!("<semantic-input-file>");
                value
            },
        ),
        _ => (None, None, json!({})),
    };
    json!({"request_contract_id":contract,"command":command,"arguments":arguments,"semantic_input_contract":contract})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_draft_and_recovery_states_follow_python_order() {
        let missing = decide_pre_execution(None, None, None, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                missing.status.as_str(),
                missing.next_action.as_str(),
                missing.target_artifact
            ),
            ("plan_required", "prepare_plan", "plan")
        );
        assert!(missing.requires_user_confirmation);
        let plan = json!({"plan_sha256":"1".repeat(64)});
        let draft = json!({"status":"list_pending","next_action":"confirm_task_list","requires_user_confirmation":true,"required_checks":["plan validate"]});
        let pending = decide_pre_execution(Some(&plan), Some(&draft), None, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                pending.status.as_str(),
                pending.next_action.as_str(),
                pending.target_artifact
            ),
            ("task_list_pending", "confirm_task_list", "task")
        );
        assert_eq!(pending.details["plan_sha256"], plan["plan_sha256"]);
        let artifacts = json!({"plan":"outputs/work/plans/example.json"});
        for (action, command) in [
            ("confirm_task_list", "task prepare"),
            ("choose_task", "task status"),
            ("confirm_start", "task status"),
            ("confirm_resume", "task status"),
            ("confirm_review", "task status"),
        ] {
            let guidance = next_action_guidance(action, "example", &artifacts);
            assert_eq!(guidance["command"], command);
            assert_eq!(guidance["arguments"]["plan_path"], artifacts["plan"]);
            assert_eq!(
                guidance["arguments"]["user_config_root"],
                "<user-config-root>"
            );
        }
        let task = json!({"task_collection_sha256":"2".repeat(64)});
        let recovery = decide_pre_execution(Some(&plan), None, Some(&task), false)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                recovery.status.as_str(),
                recovery.next_action.as_str(),
                recovery.target_artifact
            ),
            (
                "execution_recovery_required",
                "inspect_recovery",
                "execution"
            )
        );
        assert_eq!(
            mode_for_action(&recovery.status, &recovery.next_action),
            "execute"
        );
        assert!(
            decide_pre_execution(Some(&plan), None, Some(&task), true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn active_lock_and_reconciliation_routes_match_python() {
        let fingerprint = "c".repeat(64);
        let index = json!({
            "overall_status":"in_progress",
            "lock":{"kind":"execution","task_id":"TASK-001","attempt_id":"ATTEMPT-001","execute_instructions_sha256":fingerprint},
            "tasks":[{"id":"TASK-001","status":"in_progress","latest_attempt":"ATTEMPT-001"}]
        });
        let attempts = json!({"TASK-001":{
            "status":"in_progress","task_id":"TASK-001","attempt_id":"ATTEMPT-001",
            "execute_instructions_sha256":fingerprint
        }});
        let active = decide_execution(&index, &attempts);
        assert_eq!(
            (active.status.as_str(), active.next_action.as_str()),
            ("execution_in_progress", "continue_execution")
        );
        assert!(!active.requires_user_confirmation);
        assert!(
            next_action_guidance(&active.next_action, "example", &json!({}))["command"].is_null()
        );

        let mut drifted = attempts;
        drifted["TASK-001"]["execute_instructions_sha256"] = json!("d".repeat(64));
        let locked = decide_execution(&index, &drifted);
        assert_eq!(locked.next_action, "inspect_recovery");

        let completed = json!({"overall_status":"completed","lock":null,"tasks":[{
            "id":"TASK-001","status":"completed","latest_attempt":"ATTEMPT-001"
        }]});
        let mut attempts = json!({"TASK-001":{
            "status":"completed","execution_deviations":[{
                "deviation_id":"DEVIATION-001","decision":{"outcome":"approved"},
                "reconciliation_status":"pending"
            }]
        }});
        let pending = decide_execution(&completed, &attempts);
        assert_eq!(
            (pending.status.as_str(), pending.next_action.as_str()),
            ("execution_reconciliation_pending", "review_reconciliation")
        );
        assert_eq!(
            pending.details["pending_deviation_ids"],
            json!(["DEVIATION-001"])
        );
        let guidance = next_action_guidance(&pending.next_action, "example", &json!({}));
        assert_eq!(guidance["command"], "specification reconciliation-prepare");
        assert_eq!(
            guidance["request_contract_id"],
            "work-spec-reconciliation-prepare-request/v1"
        );
        assert_eq!(
            guidance["semantic_input_contract"],
            guidance["request_contract_id"]
        );
        assert_eq!(guidance["arguments"]["input_file"], "<semantic-input-file>");

        attempts["TASK-001"]["_reconciliation_resolved_ids"] = json!(["DEVIATION-001"]);
        let resolved = decide_execution(&completed, &attempts);
        assert_eq!(
            (resolved.status.as_str(), resolved.next_action.as_str()),
            ("execution_completed", "review_completion")
        );
    }
}
