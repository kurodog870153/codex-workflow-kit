//! Effective Attempt authorization, including approved supplemental scope.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::execution::{ExecutionIssue, deviation_reconciliation_target};

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

fn action_target(action: &Value) -> &str {
    match action["kind"].as_str().unwrap_or("") {
        "replace_command" | "skip_record" => action["record_id"]
            .as_str()
            .unwrap_or("")
            .split('#')
            .next()
            .unwrap_or(""),
        "add_command" => action["command"]["id"].as_str().unwrap_or(""),
        "add_validation" => action["validation"]["id"].as_str().unwrap_or(""),
        _ => action["operation"]["id"].as_str().unwrap_or(""),
    }
}

pub fn supplemental_authorizations(
    attempt: &Value,
) -> Result<HashMap<String, Value>, ExecutionIssue> {
    let mut resolved = HashMap::new();
    for deviation in attempt["execution_deviations"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if deviation["decision"]["outcome"] != "approved" {
            continue;
        }
        let proposal = &deviation["proposal"];
        let supplemental = &deviation["supplemental_authorization"];
        let action = &proposal["action"];
        if supplemental["preview_sha256"] != deviation["approved_preview_sha256"]
            || supplemental["action"] != *action
            || supplemental["modifiable_files"] != proposal["modifiable_files"]
            || supplemental["authorization_evidence"] != deviation["decision"]["evidence"]
        {
            return Err(issue(
                "execution_authorization_supplemental_mismatch",
                "The supplemental authorization does not match its approved deviation.",
                json!({"deviation_id":deviation["deviation_id"]}),
            ));
        }
        if deviation_reconciliation_target(proposal) == "task_and_execution" {
            return Err(issue(
                "execution_authorization_blocking_deviation",
                "A blocking deviation cannot extend the active Attempt authorization.",
                json!({"deviation_id":deviation["deviation_id"]}),
            ));
        }
        let anchor = proposal["anchor_record_id"].as_str().unwrap_or("");
        let base = anchor.split('#').next().unwrap_or(anchor);
        let anchored = match action["kind"].as_str().unwrap_or("") {
            "replace_command" | "skip_record" => action["record_id"] == anchor,
            "add_command" => action["after_record_id"] == anchor,
            "adjust_operation" => action["operation"]["id"] == base,
            "add_validation" => true,
            _ => false,
        };
        if !anchored {
            return Err(issue(
                "execution_authorization_supplemental_anchor",
                "The supplemental action does not match its approved anchor.",
                json!({"deviation_id":deviation["deviation_id"]}),
            ));
        }
        let target = action_target(action);
        if resolved
            .insert(target.to_owned(), supplemental.clone())
            .is_some()
        {
            return Err(issue(
                "execution_authorization_supplemental_duplicate",
                "More than one supplemental action targets the same record.",
                json!({"record_id":target}),
            ));
        }
    }
    Ok(resolved)
}

pub fn authorization_evidence(
    attempt: &Value,
    lock: &Value,
    base_record_id: &str,
) -> Result<String, ExecutionIssue> {
    if let Some(evidence) = lock["retry_authorization_evidence"].as_str() {
        return Ok(evidence.to_owned());
    }
    if let Some(supplemental) = supplemental_authorizations(attempt)?.get(base_record_id) {
        if let Some(evidence) = supplemental["authorization_evidence"].as_str() {
            return Ok(evidence.to_owned());
        }
    }
    attempt["authorization"]["authorization_evidence"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            issue(
                "execution_authorization_evidence_missing",
                "The active Attempt authorization evidence is missing.",
                json!({}),
            )
        })
}

pub fn effective_task(task: &Value, attempt: &Value) -> Result<Value, ExecutionIssue> {
    let mut result = task.clone();
    let supplemental = supplemental_authorizations(attempt)?;
    for deviation in attempt["execution_deviations"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if deviation["decision"]["outcome"] != "approved" {
            continue;
        }
        let target = action_target(&deviation["proposal"]["action"]);
        let authorization = supplemental
            .get(target)
            .expect("validated supplemental target");
        let action = &authorization["action"];
        match action["kind"].as_str().unwrap_or("") {
            "replace_command" => {
                let id = action_target(action);
                if let Some(commands) = result["commands"].as_array_mut() {
                    for command in commands.iter_mut().filter(|command| command["id"] == id) {
                        let mut replacement = action["replacement"].clone();
                        replacement["id"] = json!(id);
                        *command = replacement;
                    }
                }
            }
            "add_command" => result["commands"]
                .as_array_mut()
                .expect("validated task commands")
                .push(action["command"].clone()),
            "add_validation" => result["validations"]
                .as_array_mut()
                .expect("validated task validations")
                .push(action["validation"].clone()),
            "adjust_operation" => {
                if let Some(operations) = result["operations"].as_array_mut() {
                    for operation in operations
                        .iter_mut()
                        .filter(|operation| operation["id"] == action["operation"]["id"])
                    {
                        *operation = action["operation"].clone();
                    }
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

pub fn require_record_scope(attempt: &Value, base_id: &str) -> Result<(), ExecutionIssue> {
    let field = if base_id.starts_with("CMD-") {
        "commands"
    } else if base_id.starts_with("VAL-") {
        "validations"
    } else if base_id.starts_with("OP-") {
        "external_operations"
    } else {
        ""
    };
    let original = attempt["authorization"][field]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["id"] == base_id);
    if !original && !supplemental_authorizations(attempt)?.contains_key(base_id) {
        return Err(issue(
            "execution_authorization_scope_expansion",
            "The record is outside the authorized Attempt scope.",
            json!({"record_id":base_id}),
        ));
    }
    Ok(())
}

pub fn require_retry_evidence(
    attempt: &Value,
    record_id: &str,
    evidence: Option<&str>,
) -> Result<(), ExecutionIssue> {
    if !record_id.contains('#') {
        if evidence.is_some() {
            return Err(issue(
                "execution_authorization_unexpected_retry_evidence",
                "Normal record execution reuses the Attempt authorization.",
                json!({}),
            ));
        }
        return Ok(());
    }
    if evidence.is_none_or(|text| text.trim().is_empty()) {
        return Err(issue(
            "execution_authorization_retry_required",
            "A retry requires fresh authorization evidence.",
            json!({"record_id":record_id}),
        ));
    }
    if evidence == attempt["authorization"]["authorization_evidence"].as_str() {
        return Err(issue(
            "execution_authorization_retry_evidence_reused",
            "A retry cannot reuse the original Attempt authorization evidence.",
            json!({"record_id":record_id}),
        ));
    }
    Ok(())
}

pub fn require_modified_files(
    attempt: &Value,
    paths: &[String],
    base_id: Option<&str>,
) -> Result<(), ExecutionIssue> {
    let mut allowed: HashSet<_> = attempt["authorization"]["modifiable_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let supplemental = supplemental_authorizations(attempt)?;
    if let Some(source) = base_id.and_then(|id| supplemental.get(id)) {
        allowed.extend(
            source["modifiable_files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    let mut outside: Vec<_> = paths
        .iter()
        .filter(|path| !allowed.contains(path.as_str()))
        .collect();
    outside.sort();
    outside.dedup();
    if !outside.is_empty() {
        return Err(issue(
            "execution_authorization_scope_expansion",
            "Modified files are outside the authorized Attempt scope.",
            json!({"files":outside}),
        ));
    }
    Ok(())
}

pub fn require_result_evidence(
    attempt: &Value,
    record: &Value,
    evidence: Option<&str>,
) -> Result<(), ExecutionIssue> {
    let failed = match record["kind"].as_str() {
        Some("command") => record["exit_code"] != 0,
        Some("operation") => record["outcome"] == "failure" || record["outcome"] == "unknown",
        Some("validation") => record["outcome"] == "failed",
        _ => false,
    };
    if failed && evidence.is_none_or(|text| text.trim().is_empty()) {
        return Err(issue(
            "execution_authorization_result_required",
            "A failed or unknown result requires fresh authorization evidence.",
            json!({"record_id":record["id"]}),
        ));
    }
    if failed && evidence == attempt["authorization"]["authorization_evidence"].as_str() {
        return Err(issue(
            "execution_authorization_result_evidence_reused",
            "A failed or unknown result cannot reuse the original Attempt authorization evidence.",
            json!({"record_id":record["id"]}),
        ));
    }
    if !failed && evidence.is_some() {
        return Err(issue(
            "execution_authorization_unexpected_result_evidence",
            "A successful result reuses the Attempt authorization.",
            json!({"record_id":record["id"]}),
        ));
    }
    Ok(())
}

pub fn require_record_finish_result_evidence(
    attempt: &Value,
    request_record: &Value,
    record_id: &str,
    record_kind: &str,
    evidence: Option<&str>,
) -> Result<(), ExecutionIssue> {
    let mut record = request_record.clone();
    record["id"] = json!(record_id);
    record["kind"] = json!(record_kind);
    require_result_evidence(attempt, &record, evidence)
}

pub fn require_deviation(attempt: &Value, action: &Value) -> Result<(), ExecutionIssue> {
    if !attempt["authorization"]["allowed_deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|allowed| allowed == action)
    {
        return Err(issue(
            "execution_authorization_deviation_required",
            "The deviation is outside the preauthorized actions.",
            json!({"action":action}),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_execution_evidence_prefers_retry_then_supplemental() {
        let action = json!({"kind":"add_command","after_record_id":"CMD-001",
            "command":{"id":"CMD-002"}});
        let attempt = json!({"authorization":{"authorization_evidence":"Initial"},
            "execution_deviations":[{"decision":{"outcome":"approved","evidence":"Supplemental"},
                "proposal":{"action":action,"anchor_record_id":"CMD-001",
                    "modifiable_files":[]},"approved_preview_sha256":"a".repeat(64),
                "supplemental_authorization":{"preview_sha256":"a".repeat(64),
                    "action":action,"modifiable_files":[],
                    "authorization_evidence":"Supplemental"}}]});
        assert_eq!(
            authorization_evidence(&attempt, &json!({}), "CMD-001").unwrap(),
            "Initial"
        );
        assert_eq!(
            authorization_evidence(&attempt, &json!({}), "CMD-002").unwrap(),
            "Supplemental"
        );
        assert_eq!(
            authorization_evidence(
                &attempt,
                &json!({"retry_authorization_evidence":"Retry"}),
                "CMD-002"
            )
            .unwrap(),
            "Retry"
        );
    }

    #[test]
    fn effective_task_keeps_approved_additions_in_deviation_order() {
        let deviation = |id: &str, anchor: &str| {
            let action = json!({"kind":"add_command","after_record_id":anchor,
                "command":{"id":id}});
            json!({"decision":{"outcome":"approved","evidence":"Approved"},
                "proposal":{"action":action,"anchor_record_id":anchor,"impact":{},"modifiable_files":[]},
                "approved_preview_sha256":"a".repeat(64),
                "supplemental_authorization":{"preview_sha256":"a".repeat(64),
                    "action":action,"modifiable_files":[],"authorization_evidence":"Approved"}})
        };
        let task = json!({"commands":[],"validations":[],"operations":[]});
        let attempt = json!({"execution_deviations":[
            deviation("CMD-003", "CMD-001"), deviation("CMD-002", "CMD-001")]});
        let result = effective_task(&task, &attempt).unwrap();
        assert_eq!(
            result["commands"],
            json!([{"id":"CMD-003"},{"id":"CMD-002"}])
        );
    }

    #[test]
    fn original_scope_and_retry_evidence_follow_python() {
        let attempt = json!({"authorization":{"commands":[{"id":"CMD-001"}],"validations":[],
            "external_operations":[],"modifiable_files":["src/lib.rs"],"allowed_deviations":[],
            "authorization_evidence":"Initial approval"}});
        assert!(require_record_scope(&attempt, "CMD-001").is_ok());
        assert_eq!(
            require_record_scope(&attempt, "CMD-002")
                .unwrap_err()
                .reason_code,
            "execution_authorization_scope_expansion"
        );
        assert_eq!(
            require_retry_evidence(&attempt, "CMD-001#1", Some("Initial approval"))
                .unwrap_err()
                .reason_code,
            "execution_authorization_retry_evidence_reused"
        );
        assert!(require_retry_evidence(&attempt, "CMD-001#1", Some("Fresh approval")).is_ok());
        assert!(require_modified_files(&attempt, &["src/lib.rs".into()], None).is_ok());
        assert_eq!(
            require_result_evidence(
                &attempt,
                &json!({"id":"VAL-001","kind":"validation","outcome":"failed"}),
                None
            )
            .unwrap_err()
            .reason_code,
            "execution_authorization_result_required"
        );
    }

    #[test]
    fn record_finish_checks_derived_kind_for_failed_result() {
        let attempt = json!({"authorization":{"authorization_evidence":"Original"}});
        let request_record = json!({"outcome":"failed","evidence":"Validation failed."});
        assert_eq!(
            require_record_finish_result_evidence(
                &attempt,
                &request_record,
                "VAL-001",
                "validation",
                None
            )
            .unwrap_err()
            .reason_code,
            "execution_authorization_result_required"
        );
        assert_eq!(
            require_record_finish_result_evidence(
                &attempt,
                &request_record,
                "VAL-001",
                "validation",
                Some("Original")
            )
            .unwrap_err()
            .reason_code,
            "execution_authorization_result_evidence_reused"
        );
        assert!(
            require_record_finish_result_evidence(
                &attempt,
                &request_record,
                "VAL-001",
                "validation",
                Some("Reviewed failure")
            )
            .is_ok()
        );
    }

    #[test]
    fn original_scope_deviation_retry_and_result_evidence_match_python_flow() {
        let action = json!({"kind":"skip_record","record_id":"VAL-001",
            "reason":"Covered by equivalent evidence."});
        let attempt = json!({"authorization":{"commands":[{"id":"CMD-001"}],
            "validations":[{"id":"VAL-001"}],"external_operations":[{"id":"OP-001"}],
            "modifiable_files":["src/example.txt"],"allowed_deviations":[action],
            "authorization_evidence":"User approved this exact Attempt scope."}});
        for id in ["CMD-001", "VAL-001", "OP-001"] {
            require_record_scope(&attempt, id).unwrap();
        }
        require_modified_files(&attempt, &["src/example.txt".into()], None).unwrap();
        for record in [
            json!({"id":"CMD-001","kind":"command","exit_code":0}),
            json!({"id":"VAL-001","kind":"validation","outcome":"passed"}),
        ] {
            require_result_evidence(&attempt, &record, None).unwrap();
        }
        assert_eq!(
            authorization_evidence(&attempt, &json!({}), "CMD-001").unwrap(),
            "User approved this exact Attempt scope."
        );
        assert_eq!(
            require_record_scope(&attempt, "CMD-002")
                .unwrap_err()
                .reason_code,
            "execution_authorization_scope_expansion"
        );
        assert_eq!(
            require_modified_files(&attempt, &["src/unapproved.txt".into()], None)
                .unwrap_err()
                .reason_code,
            "execution_authorization_scope_expansion"
        );
        require_deviation(&attempt, &action).unwrap();
        let mut changed_action = action.clone();
        changed_action["reason"] = json!("Different reason.");
        assert_eq!(
            require_deviation(&attempt, &changed_action)
                .unwrap_err()
                .reason_code,
            "execution_authorization_deviation_required"
        );
        require_retry_evidence(&attempt, "CMD-001", None).unwrap();
        assert_eq!(
            require_retry_evidence(&attempt, "CMD-001#1", None)
                .unwrap_err()
                .reason_code,
            "execution_authorization_retry_required"
        );
        assert_eq!(
            require_retry_evidence(
                &attempt,
                "CMD-001#1",
                Some("User approved this exact Attempt scope.")
            )
            .unwrap_err()
            .reason_code,
            "execution_authorization_retry_evidence_reused"
        );
        require_retry_evidence(&attempt, "CMD-001#1", Some("User approved retry 1.")).unwrap();
        assert_eq!(
            authorization_evidence(
                &attempt,
                &json!({"retry_authorization_evidence":"User approved retry 1."}),
                "CMD-001"
            )
            .unwrap(),
            "User approved retry 1."
        );
        for record in [
            json!({"id":"CMD-001","kind":"command","exit_code":1}),
            json!({"id":"OP-001","kind":"operation","outcome":"failure"}),
            json!({"id":"OP-001","kind":"operation","outcome":"unknown"}),
            json!({"id":"VAL-001","kind":"validation","outcome":"failed"}),
        ] {
            assert_eq!(
                require_result_evidence(&attempt, &record, None)
                    .unwrap_err()
                    .reason_code,
                "execution_authorization_result_required"
            );
            assert_eq!(
                require_result_evidence(
                    &attempt,
                    &record,
                    Some("User approved this exact Attempt scope.")
                )
                .unwrap_err()
                .reason_code,
                "execution_authorization_result_evidence_reused"
            );
            require_result_evidence(&attempt, &record, Some("Reviewed this result.")).unwrap();
        }
    }

    #[test]
    fn supplemental_scope_is_exact_nonblocking_unique_and_record_specific() {
        let original = json!({"authorization":{"commands":[{"id":"CMD-001"}],
            "validations":[],"external_operations":[],
            "modifiable_files":["src/example.txt"],"authorization_evidence":"Original"}});
        let make_deviation = |action: Value, blocking: bool, files: Vec<&str>| {
            json!({"deviation_id":"DEVIATION-001","approved_preview_sha256":"a".repeat(64),
                "proposal":{"anchor_record_id":"CMD-001","action":action,
                    "modifiable_files":files,"impact":{"requirement_changed":blocking}},
                "supplemental_authorization":{"preview_sha256":"a".repeat(64),
                    "action":action,"modifiable_files":files,
                    "authorization_evidence":"Approved supplemental action."},
                "decision":{"outcome":"approved","evidence":"Approved supplemental action."}})
        };
        let add_action = json!({"kind":"add_command","after_record_id":"CMD-001",
            "command":{"id":"CMD-002","mode":"argv","argv":["tool","new"]}});
        let mut attempt = original.clone();
        attempt["execution_deviations"] = json!([make_deviation(add_action, false, vec![])]);
        require_record_scope(&attempt, "CMD-002").unwrap();
        let task = json!({"commands":[{"id":"CMD-001"}],"validations":[],"operations":[]});
        assert_eq!(
            effective_task(&task, &attempt).unwrap()["commands"],
            json!([{"id":"CMD-001"},{"id":"CMD-002","mode":"argv",
                "argv":["tool","new"]}])
        );
        assert_eq!(
            authorization_evidence(&attempt, &json!({}), "CMD-002").unwrap(),
            "Approved supplemental action."
        );
        let replace = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":{"mode":"argv","argv":["tool","new"]}});
        let mut mismatch = make_deviation(replace.clone(), false, vec![]);
        mismatch["supplemental_authorization"]["action"]["replacement"]["argv"] = json!(["other"]);
        attempt["execution_deviations"] = json!([mismatch]);
        assert_eq!(
            supplemental_authorizations(&attempt)
                .unwrap_err()
                .reason_code,
            "execution_authorization_supplemental_mismatch"
        );
        attempt["execution_deviations"] = json!([make_deviation(replace.clone(), true, vec![])]);
        assert_eq!(
            supplemental_authorizations(&attempt)
                .unwrap_err()
                .reason_code,
            "execution_authorization_blocking_deviation"
        );
        let first = make_deviation(replace.clone(), false, vec![]);
        let mut second = first.clone();
        second["deviation_id"] = json!("DEVIATION-002");
        attempt["execution_deviations"] = json!([first, second]);
        assert_eq!(
            supplemental_authorizations(&attempt)
                .unwrap_err()
                .reason_code,
            "execution_authorization_supplemental_duplicate"
        );
        attempt["execution_deviations"] =
            json!([make_deviation(replace, false, vec!["src/supplemental.py"])]);
        assert_eq!(
            effective_task(&task, &attempt).unwrap()["commands"],
            json!([{"id":"CMD-001","mode":"argv","argv":["tool","new"]}])
        );
        assert_eq!(
            authorization_evidence(&attempt, &json!({}), "CMD-001").unwrap(),
            "Approved supplemental action."
        );
        require_modified_files(
            &attempt,
            &["src/example.txt".into(), "src/supplemental.py".into()],
            Some("CMD-001"),
        )
        .unwrap();
        assert_eq!(
            require_modified_files(&attempt, &["src/supplemental.py".into()], None)
                .unwrap_err()
                .reason_code,
            "execution_authorization_scope_expansion"
        );
    }
}
