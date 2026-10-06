//! Deviation proposal and approval bindings.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::canonical::canonical_json_sha256;
use crate::execution::ExecutionIssue;
use crate::protocol::valid_sha256;

fn issue(reason_code: &'static str, message: &'static str) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

fn sha(value: &Value) -> bool {
    value.as_str().is_some_and(valid_sha256)
}

fn fields(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), ExecutionIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "A deviation object is required.",
        ));
    };
    if required.iter().any(|field| !object.contains_key(*field))
        || object
            .keys()
            .any(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The deviation has missing or unknown fields.",
        ));
    }
    Ok(())
}

fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

fn numbered_id(value: &Value, prefix: &str) -> bool {
    value
        .as_str()
        .and_then(|text| text.strip_prefix(prefix))
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

fn record_id(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let base = text.split('#').next().unwrap_or(text);
    let valid_base = ["CMD-", "OP-", "VAL-"]
        .into_iter()
        .any(|prefix| numbered_id(&json!(base), prefix));
    let retry = text.split_once('#').is_none_or(|(_, suffix)| {
        !suffix.is_empty()
            && !suffix.starts_with('0')
            && suffix.bytes().all(|byte| byte.is_ascii_digit())
            && !suffix.contains('#')
    });
    valid_base && retry
}

fn file_scope(value: Option<&Value>) -> Result<(), ExecutionIssue> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(paths) = value.as_array() else {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "File scope must be an array.",
        ));
    };
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        let Some(path) = path.as_str() else {
            return Err(issue(
                "invalid_execution_deviation_fields",
                "File scope needs paths.",
            ));
        };
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
            || !seen.insert(path)
        {
            return Err(issue(
                "invalid_execution_deviation_fields",
                "File scope needs unique safe project-relative paths.",
            ));
        }
    }
    Ok(())
}

fn command_shape(value: &Value, replacement: bool) -> Result<(), ExecutionIssue> {
    fields(value, &["mode"], &["id", "argv", "script", "execution"])?;
    let invalid_id = if replacement {
        value.get("id").is_some()
    } else {
        !value["id"].is_string()
    };
    if invalid_id {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The command identity is invalid.",
        ));
    }
    match value["mode"].as_str() {
        Some("argv") => {
            let invalid = if replacement {
                value.get("script").is_some()
                    || !value["argv"]
                        .as_array()
                        .is_some_and(|args| !args.is_empty() && args.iter().all(nonempty))
            } else {
                value.get("argv").is_some_and(|argv| {
                    !argv.is_null()
                        && !argv
                            .as_array()
                            .is_some_and(|args| args.iter().all(Value::is_string))
                })
            };
            if invalid {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "An argv command needs non-empty arguments.",
                ));
            }
        }
        Some("shell") => {
            let invalid = if replacement {
                value.get("argv").is_some() || !nonempty(&value["script"])
            } else {
                value
                    .get("script")
                    .is_some_and(|script| !script.is_null() && !script.is_string())
            };
            if invalid {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "A shell command needs a script.",
                ));
            }
        }
        _ => {
            return Err(issue(
                "invalid_execution_deviation_fields",
                "The command mode is invalid.",
            ));
        }
    }
    if let Some(execution) = value.get("execution").filter(|value| !value.is_null()) {
        fields(execution, &["working_directory", "os", "shell"], &[])?;
        if !["working_directory", "os", "shell"]
            .iter()
            .all(|key| execution[*key].is_string())
        {
            return Err(issue(
                "invalid_execution_deviation_fields",
                "Execution defaults are invalid.",
            ));
        }
    }
    Ok(())
}

fn action_shape(action: &Value) -> Result<(), ExecutionIssue> {
    match action["kind"].as_str() {
        Some("replace_command") => {
            fields(action, &["kind", "record_id", "replacement"], &[])?;
            if !record_id(&action["record_id"]) {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The action record ID is invalid.",
                ));
            }
            command_shape(&action["replacement"], true)
        }
        Some("add_command") => {
            fields(action, &["kind", "after_record_id", "command"], &[])?;
            if !record_id(&action["after_record_id"]) {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The action anchor is invalid.",
                ));
            }
            command_shape(&action["command"], false)
        }
        Some("add_validation") => {
            fields(action, &["kind", "validation"], &[])?;
            let validation = &action["validation"];
            fields(
                validation,
                &["id", "kind"],
                &[
                    "command_ids",
                    "pass_condition",
                    "confirmer",
                    "criteria",
                    "acceptance_ids",
                ],
            )?;
            if !validation["id"].is_string()
                || !matches!(validation["kind"].as_str(), Some("automated" | "manual"))
            {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The validation is invalid.",
                ));
            }
            Ok(())
        }
        Some("skip_record") => {
            fields(action, &["kind", "record_id", "reason"], &[])?;
            if !record_id(&action["record_id"]) || !nonempty(&action["reason"]) {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The skipped record is invalid.",
                ));
            }
            Ok(())
        }
        Some("adjust_operation") => {
            fields(action, &["kind", "operation"], &[])?;
            let operation = &action["operation"];
            fields(
                operation,
                &["id", "kind", "action", "target", "validation_id"],
                &["command_id"],
            )?;
            if !operation["id"].is_string()
                || !operation["validation_id"].is_string()
                || !["kind", "action", "target"]
                    .iter()
                    .all(|key| operation[*key].is_string())
            {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The adjusted operation is invalid.",
                ));
            }
            Ok(())
        }
        _ => Err(issue(
            "invalid_execution_deviation_fields",
            "The deviation action kind is invalid.",
        )),
    }
}

fn positive_position(value: &Value) -> bool {
    value.as_u64().is_some_and(|number| number > 0)
}

pub fn validate_semantic_deviation_request(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &[
            "gap",
            "action",
            "modifiable_files",
            "impact",
            "side_effects",
        ],
        &[],
    )?;
    if !nonempty(&value["gap"])
        || !value["side_effects"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(nonempty))
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The semantic deviation request is invalid.",
        ));
    }
    file_scope(value.get("modifiable_files"))?;
    let impact = &value["impact"];
    fields(
        impact,
        &[
            "summary",
            "requirement_changed",
            "acceptance_criteria_changed",
            "deliverables_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ],
        &["scope_changed"],
    )?;
    if !nonempty(&impact["summary"])
        || ![
            "requirement_changed",
            "acceptance_criteria_changed",
            "deliverables_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ]
        .iter()
        .all(|key| impact[*key].is_boolean())
        || impact
            .get("scope_changed")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The semantic deviation impact is invalid.",
        ));
    }
    let action = &value["action"];
    match action["kind"].as_str() {
        Some("replace_command") => {
            fields(action, &["kind", "replacement"], &[])?;
            command_shape(&action["replacement"], true)?;
        }
        Some("add_command") => {
            fields(action, &["kind", "command"], &[])?;
            command_shape(&action["command"], true)?;
        }
        Some("skip_record") => {
            fields(action, &["kind", "reason"], &[])?;
            if !nonempty(&action["reason"]) {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The semantic skip reason is invalid.",
                ));
            }
        }
        Some("add_validation") => {
            fields(action, &["kind", "validation"], &[])?;
            let validation = &action["validation"];
            fields(
                validation,
                &["kind"],
                &[
                    "command_positions",
                    "pass_condition",
                    "confirmer",
                    "criteria",
                    "acceptance_positions",
                ],
            )?;
            if !matches!(validation["kind"].as_str(), Some("automated" | "manual")) {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The semantic validation kind is invalid.",
                ));
            }
            for field in ["command_positions", "acceptance_positions"] {
                if validation.get(field).is_some_and(|positions| {
                    !positions.is_null()
                        && !positions
                            .as_array()
                            .is_some_and(|rows| rows.iter().all(positive_position))
                }) {
                    return Err(issue(
                        "invalid_execution_deviation_fields",
                        "Semantic positions are invalid.",
                    ));
                }
            }
            for field in ["pass_condition", "confirmer", "criteria"] {
                if validation
                    .get(field)
                    .is_some_and(|text| !text.is_null() && !nonempty(text))
                {
                    return Err(issue(
                        "invalid_execution_deviation_fields",
                        "Semantic validation text is invalid.",
                    ));
                }
            }
        }
        Some("adjust_operation") => {
            fields(action, &["kind", "operation"], &[])?;
            let operation = &action["operation"];
            fields(
                operation,
                &["kind", "action", "target", "validation_position"],
                &["command_position"],
            )?;
            if !["kind", "action", "target"]
                .iter()
                .all(|field| nonempty(&operation[*field]))
                || !positive_position(&operation["validation_position"])
                || operation
                    .get("command_position")
                    .is_some_and(|value| !value.is_null() && !positive_position(value))
            {
                return Err(issue(
                    "invalid_execution_deviation_fields",
                    "The semantic operation is invalid.",
                ));
            }
        }
        _ => {
            return Err(issue(
                "invalid_execution_deviation_fields",
                "The semantic action kind is invalid.",
            ));
        }
    }
    work_model::execution::request::verified::<
        work_model::execution::request::DeviationSemanticRequest,
    >(value);
    Ok(())
}

pub fn validate_deviation_proposal(proposal: &Value) -> Result<(), ExecutionIssue> {
    fields(
        proposal,
        &[
            "schema",
            "task_id",
            "attempt_id",
            "anchor_record_id",
            "task_basis",
            "gap",
            "action",
            "impact",
            "side_effects",
        ],
        &["modifiable_files"],
    )?;
    if proposal["schema"] != "work-execution-deviation-proposal" {
        return Err(issue(
            "invalid_execution_deviation_schema",
            "A nested deviation schema is invalid.",
        ));
    }
    if !numbered_id(&proposal["task_id"], "TASK-")
        || !numbered_id(&proposal["attempt_id"], "ATTEMPT-")
        || !record_id(&proposal["anchor_record_id"])
        || !nonempty(&proposal["gap"])
        || !proposal["side_effects"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(nonempty))
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The deviation proposal is invalid.",
        ));
    }
    let impact = &proposal["impact"];
    fields(
        impact,
        &[
            "summary",
            "requirement_changed",
            "acceptance_criteria_changed",
            "deliverables_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ],
        &["scope_changed"],
    )?;
    if !nonempty(&impact["summary"])
        || ![
            "requirement_changed",
            "acceptance_criteria_changed",
            "deliverables_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ]
        .iter()
        .all(|key| impact[*key].is_boolean())
        || impact
            .get("scope_changed")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The deviation impact is invalid.",
        ));
    }
    file_scope(proposal.get("modifiable_files"))?;
    action_shape(&proposal["action"])?;
    let anchor = proposal["anchor_record_id"].as_str().unwrap();
    let base = anchor.split('#').next().unwrap_or(anchor);
    if !proposal["task_basis"].as_array().is_some_and(|rows| {
        !rows.is_empty() && rows.iter().all(nonempty) && rows.iter().any(|row| row == base)
    }) {
        return Err(issue(
            "invalid_contract_value",
            "task_basis must contain the anchor base record ID.",
        ));
    }
    let action = &proposal["action"];
    let matches_anchor = match action["kind"].as_str().unwrap_or("") {
        "replace_command" | "skip_record" => action["record_id"] == anchor,
        "add_command" => action["after_record_id"] == anchor,
        "adjust_operation" => action["operation"]["id"] == base,
        "add_validation" => true,
        _ => false,
    };
    if !matches_anchor {
        return Err(issue(
            "invalid_contract_value",
            "The deviation action must target its anchor record.",
        ));
    }
    Ok(())
}

pub fn validate_deviation_preview(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &[
            "schema",
            "proposal",
            "record_kind",
            "action_validation",
            "semantic_review",
            "classification",
            "blocking",
            "sources",
            "preview_sha256",
        ],
        &[],
    )?;
    if value["schema"] != "work-execution-deviation-preview" {
        return Err(issue(
            "invalid_execution_deviation_schema",
            "The deviation preview schema is invalid.",
        ));
    }
    validate_deviation_proposal(&value["proposal"])?;
    if !matches!(
        value["record_kind"].as_str(),
        Some("command" | "operation" | "validation")
    ) || value["action_validation"] != "passed"
        || value["semantic_review"] != "required"
        || !sha(&value["preview_sha256"])
        || !value["sources"]
            .as_object()
            .is_some_and(|sources| sources.values().all(sha))
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The deviation preview is invalid.",
        ));
    }
    let blocking = crate::execution::deviation_reconciliation_target(&value["proposal"])
        == "task_and_execution";
    let classification = if blocking {
        "task_and_execution"
    } else {
        "task_only"
    };
    if value["blocking"] != blocking || value["classification"] != classification {
        return Err(issue(
            "deviation_preview_classification_mismatch",
            "Preview classification and blocking must match proposal impact.",
        ));
    }
    Ok(())
}

pub fn validate_deviation_record_response(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &[
            "schema",
            "task_id",
            "attempt_id",
            "deviation_id",
            "attempt_path",
            "classification",
            "blocking",
            "record_status",
            "lock_status",
        ],
        &[],
    )?;
    if value["schema"] != "work-execution-deviation-record" {
        return Err(issue(
            "invalid_execution_deviation_schema",
            "The deviation record schema is invalid.",
        ));
    }
    if !numbered_id(&value["task_id"], "TASK-")
        || !numbered_id(&value["attempt_id"], "ATTEMPT-")
        || !numbered_id(&value["deviation_id"], "DEVIATION-")
        || !matches!(
            value["classification"].as_str(),
            Some("task_only" | "task_and_execution")
        )
        || value["record_status"] != "recorded"
        || value["lock_status"] != "record_reserved"
    {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "The deviation record is invalid.",
        ));
    }
    let suffix = format!(
        "/{}/{}/attempt.json",
        value["task_id"].as_str().unwrap(),
        value["attempt_id"].as_str().unwrap()
    );
    if value["blocking"] != (value["classification"] == "task_and_execution")
        || !value["attempt_path"]
            .as_str()
            .is_some_and(|path| path.ends_with(&suffix))
    {
        return Err(issue(
            "deviation_record_identity_mismatch",
            "Record blocking and Attempt path must match its classification and identity.",
        ));
    }
    Ok(())
}

pub fn validate_deviation_artifact(value: &Value) -> Result<(), ExecutionIssue> {
    fields(
        value,
        &[
            "schema",
            "deviation_id",
            "approved_preview_sha256",
            "proposal",
            "supplemental_authorization",
            "decision",
            "reconciliation_status",
        ],
        &[],
    )?;
    if value["schema"] != "work-execution-deviation" {
        return Err(issue(
            "invalid_execution_deviation_schema",
            "The execution deviation schema is invalid.",
        ));
    }
    let id = value["deviation_id"].as_str().unwrap_or("");
    if !id
        .strip_prefix("DEVIATION-")
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(issue(
            "invalid_execution_deviation_id",
            "The deviation ID is invalid.",
        ));
    }
    let authorization = &value["supplemental_authorization"];
    let proposal = &value["proposal"];
    let decision = &value["decision"];
    validate_deviation_proposal(proposal)?;
    fields(
        authorization,
        &[
            "schema",
            "preview_sha256",
            "action",
            "authorization_evidence",
        ],
        &["modifiable_files"],
    )?;
    file_scope(authorization.get("modifiable_files"))?;
    action_shape(&authorization["action"])?;
    if !nonempty(&authorization["authorization_evidence"]) {
        return Err(issue(
            "invalid_execution_deviation_fields",
            "Authorization evidence is required.",
        ));
    }
    fields(decision, &["outcome", "evidence"], &[])?;
    if authorization["schema"] != "work-execution-deviation-authorization" {
        return Err(issue(
            "invalid_execution_deviation_schema",
            "A nested deviation schema is invalid.",
        ));
    }
    if !sha(&value["approved_preview_sha256"])
        || authorization["preview_sha256"] != value["approved_preview_sha256"]
    {
        return Err(issue(
            "deviation_preview_binding_mismatch",
            "Supplemental authorization must bind the approved preview fingerprint.",
        ));
    }
    if authorization["action"] != proposal["action"] {
        return Err(issue(
            "deviation_action_binding_mismatch",
            "Supplemental action must match the approved proposal.",
        ));
    }
    if authorization["authorization_evidence"] != decision["evidence"] {
        return Err(issue(
            "deviation_evidence_mismatch",
            "The deviation decision must use its supplemental authorization evidence.",
        ));
    }
    if authorization
        .get("modifiable_files")
        .cloned()
        .unwrap_or(json!([]))
        != proposal
            .get("modifiable_files")
            .cloned()
            .unwrap_or(json!([]))
    {
        return Err(issue(
            "deviation_file_scope_mismatch",
            "Supplemental file scope must match the approved proposal.",
        ));
    }
    let outcome = decision["outcome"].as_str().unwrap_or("");
    let status = value["reconciliation_status"].as_str().unwrap_or("");
    if outcome == "rejected" && status != "not_needed"
        || outcome == "approved" && status == "not_needed"
    {
        return Err(issue(
            "deviation_reconciliation_status_mismatch",
            "The reconciliation status conflicts with the deviation decision.",
        ));
    }
    if !matches!(outcome, "approved" | "rejected")
        || !matches!(
            status,
            "pending" | "incorporated" | "declined" | "not_needed"
        )
    {
        return Err(issue(
            "invalid_deviation_decision",
            "The deviation decision or reconciliation status is invalid.",
        ));
    }
    let _: work_model::execution::deviation::ExecutionDeviation =
        serde_json::from_value(value.clone()).expect("validated deviation matches model");
    Ok(())
}

pub fn build_deviation_preview(
    proposal: &Value,
    record_kind: &str,
    sources: &BTreeMap<String, String>,
) -> Result<Value, ExecutionIssue> {
    if !matches!(record_kind, "command" | "operation" | "validation") {
        return Err(issue(
            "deviation_active_record",
            "A formal reserved record is required for deviation review.",
        ));
    }
    validate_deviation_proposal(proposal)?;
    let blocking =
        crate::execution::deviation_reconciliation_target(proposal) == "task_and_execution";
    let mut preview = json!({
        "schema":"work-execution-deviation-preview",
        "proposal":proposal,
        "record_kind":record_kind,
        "action_validation":"passed",
        "semantic_review":"required",
        "classification":if blocking { "task_and_execution" } else { "task_only" },
        "blocking":blocking,
        "sources":sources,
    });
    preview["preview_sha256"] = json!(canonical_json_sha256(&preview).expect("JSON value"));
    validate_deviation_preview(&preview)?;
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::DeviationPreview,
    >(preview))
}

pub fn append_approved_deviation(
    attempt: &Value,
    preview: &Value,
    approved_sha256: &str,
    evidence: &str,
    execution_dir: &str,
) -> Result<(Value, Value), ExecutionIssue> {
    if evidence.trim().is_empty() || evidence == attempt["authorization"]["authorization_evidence"]
    {
        return Err(issue(
            "deviation_record_authorization_evidence_reused",
            "A runtime deviation requires fresh authorization evidence.",
        ));
    }
    let existing = attempt["execution_deviations"].as_array();
    if existing.is_some_and(|rows| {
        rows.iter()
            .any(|row| row["approved_preview_sha256"] == approved_sha256)
    }) {
        return Err(issue(
            "deviation_record_duplicate",
            "This approved deviation is already recorded.",
        ));
    }
    let mut unsigned = preview.clone();
    let stated_sha = unsigned
        .as_object_mut()
        .and_then(|object| object.remove("preview_sha256"));
    let computed_sha = canonical_json_sha256(&unsigned).expect("JSON value");
    if stated_sha.as_ref().and_then(Value::as_str) != Some(computed_sha.as_str())
        || computed_sha != approved_sha256
    {
        return Err(issue(
            "deviation_record_approval_changed",
            "Sources or proposal content changed after review.",
        ));
    }
    let count = existing.map_or(0, Vec::len);
    let deviation_id = crate::derivation::identity::next_deviation_id(count).ok_or_else(|| {
        issue(
            "deviation_record_limit",
            "The Attempt cannot allocate another DEVIATION-nnn ID.",
        )
    })?;
    let proposal = &preview["proposal"];
    let artifact = json!({
        "schema":"work-execution-deviation",
        "deviation_id":deviation_id,
        "approved_preview_sha256":approved_sha256,
        "proposal":proposal,
        "supplemental_authorization":{
            "schema":"work-execution-deviation-authorization",
            "preview_sha256":approved_sha256,
            "action":proposal["action"],
            "modifiable_files":proposal["modifiable_files"],
            "authorization_evidence":evidence,
        },
        "decision":{"outcome":"approved","evidence":evidence},
        "reconciliation_status":"pending",
    });
    validate_deviation_artifact(&artifact)?;
    let mut candidate = attempt.clone();
    let mut deviations = existing.cloned().unwrap_or_default();
    deviations.push(artifact);
    candidate["execution_deviations"] = json!(deviations);
    let response = json!({
        "schema":"work-execution-deviation-record",
        "task_id":proposal["task_id"],
        "attempt_id":proposal["attempt_id"],
        "deviation_id":deviation_id,
        "attempt_path":format!("{execution_dir}/{}/{}/attempt.json", proposal["task_id"].as_str().unwrap_or(""), proposal["attempt_id"].as_str().unwrap_or("")),
        "classification":preview["classification"],
        "blocking":preview["blocking"],
        "record_status":"recorded",
        "lock_status":"record_reserved",
    });
    validate_deviation_record_response(&response)?;
    Ok((
        candidate,
        work_model::execution::response::verified::<
            work_model::execution::response::DeviationRecordResponse,
        >(response),
    ))
}

pub fn formalize_semantic_action(
    action: &Value,
    task: &Value,
    attempt: &Value,
    anchor: &str,
) -> Result<Value, ExecutionIssue> {
    let base = anchor.split('#').next().unwrap_or(anchor);
    let prefix = base.split('-').next().unwrap_or(base);
    let position = |group: &str, value: &Value| -> Result<Value, ExecutionIssue> {
        let number = value.as_u64().filter(|number| *number > 0).ok_or_else(|| {
            issue(
                "deviation_semantic_reference",
                "A semantic position must be a positive integer.",
            )
        })?;
        let rows = if group == "acceptance" {
            &task["traceability"]["acceptance_ids"]
        } else {
            &task[group]
        };
        let selected = rows
            .as_array()
            .and_then(|rows| rows.get((number - 1) as usize))
            .ok_or_else(|| {
                issue(
                    "deviation_semantic_reference",
                    "A semantic position does not exist in the current TASK.",
                )
            })?;
        Ok(if group == "acceptance" {
            selected.clone()
        } else {
            selected["id"].clone()
        })
    };
    let positions = |group: &str, values: &Value| -> Result<Value, ExecutionIssue> {
        let rows = values
            .as_array()
            .filter(|rows| !rows.is_empty())
            .ok_or_else(|| {
                issue(
                    "deviation_semantic_reference",
                    "Semantic positions must be nonempty and unique.",
                )
            })?;
        let mut seen = std::collections::HashSet::new();
        let mut formal = Vec::new();
        for value in rows {
            let number = value.as_u64().filter(|number| *number > 0).ok_or_else(|| {
                issue(
                    "deviation_semantic_reference",
                    "Semantic positions must be nonempty and unique.",
                )
            })?;
            if !seen.insert(number) {
                return Err(issue(
                    "deviation_semantic_reference",
                    "Semantic positions must be nonempty and unique.",
                ));
            }
            formal.push(position(group, value)?);
        }
        Ok(json!(formal))
    };
    let next_id = |group: &str, prefix: &str| -> Result<String, ExecutionIssue> {
        let mut maximum = 0u16;
        for row in task[group].as_array().into_iter().flatten() {
            if let Some(number) = row["id"]
                .as_str()
                .and_then(|id| id.strip_prefix(&format!("{prefix}-")))
                .and_then(|number| number.parse::<u16>().ok())
            {
                maximum = maximum.max(number);
            }
        }
        for deviation in attempt["execution_deviations"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let prior = &deviation["proposal"]["action"];
            let field = if group == "commands" {
                "command"
            } else {
                "validation"
            };
            if prior["kind"]
                == if group == "commands" {
                    "add_command"
                } else {
                    "add_validation"
                }
            {
                if let Some(number) = prior[field]["id"]
                    .as_str()
                    .and_then(|id| id.strip_prefix(&format!("{prefix}-")))
                    .and_then(|number| number.parse::<u16>().ok())
                {
                    maximum = maximum.max(number);
                }
            }
        }
        if maximum >= 999 {
            return Err(issue(
                "deviation_semantic_id_limit",
                "The current TASK has no available formal deviation ID.",
            ));
        }
        Ok(format!("{prefix}-{:03}", maximum + 1))
    };
    match action["kind"].as_str().unwrap_or("") {
        "replace_command" => {
            if prefix != "CMD" {
                return Err(issue(
                    "deviation_action_anchor",
                    "replace_command requires a reserved command record.",
                ));
            }
            Ok(json!({"kind":"replace_command","record_id":anchor,
                "replacement":action["replacement"]}))
        }
        "skip_record" => Ok(json!({"kind":"skip_record","record_id":anchor,
            "reason":action["reason"]})),
        "add_command" => {
            let mut command = action["command"].clone();
            command["id"] = json!(next_id("commands", "CMD")?);
            Ok(json!({"kind":"add_command","after_record_id":anchor,"command":command}))
        }
        "add_validation" => {
            let mut validation = action["validation"].clone();
            if let Some(values) = validation
                .as_object_mut()
                .and_then(|object| object.remove("command_positions"))
            {
                validation["command_ids"] = positions("commands", &values)?;
            }
            if let Some(values) = validation
                .as_object_mut()
                .and_then(|object| object.remove("acceptance_positions"))
            {
                validation["acceptance_ids"] = positions("acceptance", &values)?;
            }
            validation["id"] = json!(next_id("validations", "VAL")?);
            Ok(json!({"kind":"add_validation","validation":validation}))
        }
        "adjust_operation" => {
            if prefix != "OP" {
                return Err(issue(
                    "deviation_action_anchor",
                    "adjust_operation requires a reserved operation record.",
                ));
            }
            let mut operation = action["operation"].clone();
            let validation_position = operation
                .as_object_mut()
                .and_then(|object| object.remove("validation_position"))
                .ok_or_else(|| {
                    issue(
                        "deviation_semantic_reference",
                        "A validation position is required.",
                    )
                })?;
            operation["validation_id"] = position("validations", &validation_position)?;
            if let Some(command_position) = operation
                .as_object_mut()
                .and_then(|object| object.remove("command_position"))
            {
                operation["command_id"] = position("commands", &command_position)?;
            }
            operation["id"] = json!(base);
            Ok(json!({"kind":"adjust_operation","operation":operation}))
        }
        _ => Err(issue(
            "deviation_semantic_action",
            "The semantic deviation action is invalid.",
        )),
    }
}

pub struct DeviationStagingInput<'a> {
    pub canonical_root: &'a str,
    pub requirement: &'a crate::identifiers::RequirementId,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub task: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
    pub attempt_after: &'a [u8],
    pub sources: &'a BTreeMap<String, Vec<u8>>,
}

pub fn build_deviation_staging(
    input: DeviationStagingInput<'_>,
) -> Result<work_model::runtime::RuntimeManifest, ExecutionIssue> {
    use crate::canonical::parse_json_contract;
    use crate::derivation::fingerprint;
    use work_model::runtime::{RuntimeBytes, RuntimeTarget};
    let parse = |raw| {
        parse_json_contract(raw).map_err(|_| {
            issue(
                "deviation_staging_contract",
                "Canonical artifacts are required.",
            )
        })
    };
    let index = parse(input.index_before)?;
    let before = parse(input.attempt_before)?;
    let after = parse(input.attempt_after)?;
    crate::execution::index::validate_execution_index(&index, input.index_before)?;
    crate::execution::recovery::validate_deviation_record_recovery(
        &before,
        input.attempt_before,
        &after,
        input.attempt_after,
    )?;
    let lock = &index["lock"];
    if index["requirement_id"] != input.requirement.as_str()
        || before["task_id"] != input.task_id
        || before["attempt_id"] != input.attempt_id
        || before["status"] != "in_progress"
        || lock["kind"] != "execution"
        || lock["task_id"] != input.task_id
        || lock["attempt_id"] != input.attempt_id
        || lock["record_id"] != input.record_id
        || lock["execute_instructions_sha256"] != before["execute_instructions_sha256"]
    {
        return Err(issue(
            "deviation_staging_identity",
            "The active record identities disagree.",
        ));
    }
    let artifact = after["execution_deviations"]
        .as_array()
        .and_then(|rows| rows.last())
        .ok_or_else(|| {
            issue(
                "deviation_staging_contract",
                "The appended deviation is missing.",
            )
        })?;
    let proposal = &artifact["proposal"];
    if proposal["task_id"] != input.task_id
        || proposal["attempt_id"] != input.attempt_id
        || proposal["anchor_record_id"] != input.record_id
    {
        return Err(issue(
            "deviation_staging_identity",
            "The deviation proposal identifies another record.",
        ));
    }
    let kind = crate::execution::formal_record_kind(
        input.task,
        input.record_id.split('#').next().unwrap_or(""),
    )?;
    crate::execution::validate_deviation_action(proposal, input.task, kind)?;
    let index_path = format!("{}/index.json", input.execution_dir);
    let attempt_path = format!(
        "{}/{}/{}/attempt.json",
        input.execution_dir, input.task_id, input.attempt_id
    );
    if input.sources.get(&index_path).map(Vec::as_slice) != Some(input.index_before)
        || input.sources.get(&attempt_path).map(Vec::as_slice) != Some(input.attempt_before)
    {
        return Err(issue(
            "deviation_staging_sources",
            "The complete reviewed execution sources are required.",
        ));
    }
    let sources = input
        .sources
        .iter()
        .map(|(path, raw)| (path.clone(), fingerprint::raw(raw)))
        .collect();
    let preview = build_deviation_preview(proposal, kind, &sources)?;
    let approval = artifact["approved_preview_sha256"].as_str().unwrap_or("");
    let (expected, _) = append_approved_deviation(
        &before,
        &preview,
        approval,
        artifact["decision"]["evidence"].as_str().unwrap_or(""),
        input.execution_dir,
    )?;
    if crate::execution::attempt::render_attempt(&expected)? != input.attempt_after {
        return Err(issue(
            "deviation_staging_transition",
            "The prepared Attempt differs from its approved preview.",
        ));
    }
    let bytes = |raw: &[u8]| RuntimeBytes {
        bytes: raw.to_vec(),
        sha256: fingerprint::raw(raw),
    };
    let payloads = BTreeMap::from([("attempt.json.tmp".into(), input.attempt_after.to_vec())]);
    crate::execution::recovery::build_execution_staging_manifest(
        crate::execution::recovery::ExecutionStagingInput {
            canonical_root: input.canonical_root,
            requirement: input.requirement,
            execution_dir: input.execution_dir,
            operation: crate::derivation::publication::RuntimeOperation::DeviationRecord,
            approval_sha256: approval,
            business_identity: json!({"task_id":input.task_id,"attempt_id":input.attempt_id,"record_id":input.record_id,"publication_sources":input.sources}),
            targets: vec![
                RuntimeTarget {
                    path: attempt_path,
                    before: Some(bytes(input.attempt_before)),
                    after: Some(bytes(input.attempt_after)),
                },
                RuntimeTarget {
                    path: index_path,
                    before: Some(bytes(input.index_before)),
                    after: Some(bytes(input.index_before)),
                },
            ],
            payloads: &payloads,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_and_record_classification_follow_impact_and_identity() {
        let proposal = strict_proposal();
        let sources = BTreeMap::from([("task.json".to_owned(), "a".repeat(64))]);
        let mut preview = build_deviation_preview(&proposal, "command", &sources).unwrap();
        validate_deviation_preview(&preview).unwrap();
        preview["proposal"]["impact"]["scope_changed"] = json!(true);
        assert_eq!(
            validate_deviation_preview(&preview)
                .unwrap_err()
                .reason_code,
            "deviation_preview_classification_mismatch"
        );
        preview["classification"] = json!("task_and_execution");
        preview["blocking"] = json!(true);
        validate_deviation_preview(&preview).unwrap();
        let mut legacy = preview.clone();
        legacy["classification"] = json!("plan_and_task");
        assert_eq!(
            validate_deviation_preview(&legacy).unwrap_err().reason_code,
            "deviation_preview_classification_mismatch"
        );
        let mut record = json!({"schema":"work-execution-deviation-record",
            "task_id":"TASK-001","attempt_id":"ATTEMPT-001","deviation_id":"DEVIATION-001",
            "attempt_path":"outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
            "classification":"task_only","blocking":false,"record_status":"recorded",
            "lock_status":"record_reserved"});
        validate_deviation_record_response(&record).unwrap();
        let mut legacy = record.clone();
        legacy["classification"] = json!("plan_and_task");
        assert_eq!(
            validate_deviation_record_response(&legacy)
                .unwrap_err()
                .reason_code,
            "invalid_execution_deviation_fields"
        );
        record["blocking"] = json!(true);
        assert_eq!(
            validate_deviation_record_response(&record)
                .unwrap_err()
                .reason_code,
            "deviation_record_identity_mismatch"
        );
        record["blocking"] = json!(false);
        record["attempt_path"] =
            json!("outputs/work/executions/example/TASK-002/ATTEMPT-001/attempt.json");
        assert_eq!(
            validate_deviation_record_response(&record)
                .unwrap_err()
                .reason_code,
            "deviation_record_identity_mismatch"
        );
    }

    #[test]
    fn semantic_request_rejects_formal_ids_and_references() {
        let mut request = json!({"gap":"The executable is unavailable.",
            "action":{"kind":"replace_command","replacement":{"mode":"argv","argv":["tool"]}},
            "modifiable_files":[],
            "impact":{"summary":"Equivalent command.","requirement_changed":false,
                "scope_changed":false,"acceptance_criteria_changed":false,
                "deliverables_changed":false,"safety_boundary_changed":false,
                "external_side_effect_boundary_changed":false},"side_effects":[]});
        validate_semantic_deviation_request(&request).unwrap();
        let invalid = [
            json!({"kind":"replace_command","record_id":"CMD-001",
                "replacement":{"mode":"argv","argv":["tool"]}}),
            json!({"kind":"skip_record","record_id":"CMD-001","reason":"Skip."}),
            json!({"kind":"add_command","after_record_id":"CMD-001",
                "command":{"mode":"argv","argv":["tool"]}}),
            json!({"kind":"add_command","command":{"id":"CMD-002","mode":"argv","argv":["tool"]}}),
            json!({"kind":"add_validation","validation":{"id":"VAL-002","kind":"manual",
                "confirmer":"user","criteria":"Reviewed."}}),
            json!({"kind":"add_validation","validation":{"kind":"automated",
                "command_ids":["CMD-001"]}}),
            json!({"kind":"adjust_operation","operation":{"id":"OP-001","kind":"file",
                "action":"Update.","target":"src/app.py","validation_position":1}}),
            json!({"kind":"adjust_operation","operation":{"kind":"file","action":"Update.",
                "target":"src/app.py","validation_id":"VAL-001"}}),
        ];
        for action in invalid {
            request["action"] = action;
            assert!(validate_semantic_deviation_request(&request).is_err());
        }
    }

    pub(super) fn strict_proposal() -> Value {
        json!({"schema":"work-execution-deviation-proposal","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","anchor_record_id":"CMD-001",
            "task_basis":["CMD-001","STEP-001"],"gap":"The executable is unavailable.",
            "action":{"kind":"replace_command","record_id":"CMD-001",
                "replacement":{"mode":"argv","argv":["tool","test"]}},
            "impact":{"summary":"Equivalent command.","requirement_changed":false,
                "scope_changed":false,"acceptance_criteria_changed":false,
                "deliverables_changed":false,"safety_boundary_changed":false,
                "external_side_effect_boundary_changed":false},
            "side_effects":["Runs the existing command."]})
    }

    #[test]
    fn proposal_accepts_strict_actions_and_rejects_invalid_shapes_and_bindings() {
        let mut proposal = strict_proposal();
        validate_deviation_proposal(&proposal).unwrap();
        assert!(proposal.get("modifiable_files").is_none());
        let actions = [
            json!({"kind":"replace_command","record_id":"CMD-001",
                "replacement":{"mode":"shell","script":"tool test"}}),
            json!({"kind":"add_command","after_record_id":"CMD-001",
                "command":{"id":"CMD-002","mode":"argv","argv":["tool","test"]}}),
            json!({"kind":"add_validation","validation":{"id":"VAL-002",
                "kind":"manual","confirmer":"user","criteria":"Reviewed."}}),
            json!({"kind":"skip_record","record_id":"OP-001",
                "reason":"The operation is not applicable."}),
            json!({"kind":"adjust_operation","operation":{"id":"OP-001",
                "kind":"file","action":"Update target.","target":"src/app.py",
                "validation_id":"VAL-001"}}),
        ];
        for action in actions {
            let operation = matches!(
                action["kind"].as_str(),
                Some("skip_record" | "adjust_operation")
            );
            proposal["action"] = action;
            proposal["anchor_record_id"] = json!(if operation { "OP-001" } else { "CMD-001" });
            proposal["task_basis"] = json!([if operation { "OP-001" } else { "CMD-001" }]);
            validate_deviation_proposal(&proposal).unwrap();
        }
        let mut wrong = strict_proposal();
        wrong["action"]["replacement"]["script"] = json!("tool");
        assert!(validate_deviation_proposal(&wrong).is_err());
        let mut missing = strict_proposal();
        missing.as_object_mut().unwrap().remove("gap");
        assert!(validate_deviation_proposal(&missing).is_err());
        let mut unknown = strict_proposal();
        unknown["unexpected"] = json!(true);
        assert!(validate_deviation_proposal(&unknown).is_err());
        let mut mixed = strict_proposal();
        mixed["action"]["reason"] = json!("Not allowed.");
        assert!(validate_deviation_proposal(&mixed).is_err());
        let mut wrong_basis = strict_proposal();
        wrong_basis["task_basis"] = json!(["STEP-001"]);
        assert_eq!(
            validate_deviation_proposal(&wrong_basis)
                .unwrap_err()
                .reason_code,
            "invalid_contract_value"
        );
        let mut wrong_anchor = strict_proposal();
        wrong_anchor["action"]["record_id"] = json!("CMD-002");
        assert_eq!(
            validate_deviation_proposal(&wrong_anchor)
                .unwrap_err()
                .reason_code,
            "invalid_contract_value"
        );
        let mut wrong_position = strict_proposal();
        wrong_position["action"] = json!({"kind":"add_command","after_record_id":"CMD-002",
            "command":{"id":"CMD-003","mode":"argv","argv":["tool"]}});
        assert_eq!(
            validate_deviation_proposal(&wrong_position)
                .unwrap_err()
                .reason_code,
            "invalid_contract_value"
        );
        let mut wrong_operation = strict_proposal();
        wrong_operation["anchor_record_id"] = json!("OP-001");
        wrong_operation["task_basis"] = json!(["OP-001"]);
        wrong_operation["action"] = json!({"kind":"adjust_operation","operation":{
            "id":"OP-002","kind":"file","action":"Update.","target":"src/app.py",
            "validation_id":"VAL-001"}});
        assert_eq!(
            validate_deviation_proposal(&wrong_operation)
                .unwrap_err()
                .reason_code,
            "invalid_contract_value"
        );
        let mut boundary = strict_proposal();
        boundary["impact"]["requirement_changed"] = json!(true);
        validate_deviation_proposal(&boundary).unwrap();
        boundary["impact"]["scope_changed"] = json!(true);
        validate_deviation_proposal(&boundary).unwrap();
        for paths in [
            json!(["../outside.py"]),
            json!(["src\\file.py"]),
            json!(["src/a.py", "src/a.py"]),
        ] {
            let mut unsafe_proposal = strict_proposal();
            unsafe_proposal["modifiable_files"] = paths;
            assert!(validate_deviation_proposal(&unsafe_proposal).is_err());
        }
        let mut safe_scope = strict_proposal();
        safe_scope["modifiable_files"] = json!(["src/extra.py"]);
        validate_deviation_proposal(&safe_scope).unwrap();
    }

    #[test]
    fn semantic_actions_resolve_formal_positions_and_allocate_ids() {
        let task = json!({"commands":[{"id":"CMD-001"}],"operations":[{"id":"OP-001"}],
            "validations":[{"id":"VAL-001"}],
            "traceability":{"acceptance_ids":["ACCEPTANCE-001"]}});
        let attempt = json!({"execution_deviations":[{"proposal":{"action":{
            "kind":"add_command","command":{"id":"CMD-002"}}}}]});
        let added_command = formalize_semantic_action(
            &json!({"kind":"add_command","command":{"mode":"argv","argv":["tool"]}}),
            &task,
            &attempt,
            "CMD-001",
        )
        .unwrap();
        assert_eq!(added_command["command"]["id"], "CMD-003");
        assert_eq!(added_command["after_record_id"], "CMD-001");
        assert_eq!(added_command["command"]["argv"], json!(["tool"]));
        let added_validation = formalize_semantic_action(
            &json!({"kind":"add_validation","validation":{"kind":"automated",
                "command_positions":[1],"acceptance_positions":[1],"pass_condition":"Success."}}),
            &task,
            &attempt,
            "CMD-001",
        )
        .unwrap();
        assert_eq!(added_validation["validation"]["id"], "VAL-002");
        assert_eq!(
            added_validation["validation"]["command_ids"],
            json!(["CMD-001"])
        );
        assert_eq!(
            added_validation["validation"]["acceptance_ids"],
            json!(["ACCEPTANCE-001"])
        );
        let adjusted = formalize_semantic_action(
            &json!({"kind":"adjust_operation","operation":{"kind":"file",
                "action":"Update.","target":"src/app.py",
                "validation_position":1,"command_position":1}}),
            &task,
            &attempt,
            "OP-001",
        )
        .unwrap();
        assert_eq!(adjusted["operation"]["id"], "OP-001");
        assert_eq!(adjusted["operation"]["validation_id"], "VAL-001");
        assert_eq!(adjusted["operation"]["command_id"], "CMD-001");
        assert_eq!(adjusted["operation"]["action"], "Update.");
        assert_eq!(adjusted["operation"]["target"], "src/app.py");
        assert_eq!(
            formalize_semantic_action(
                &json!({"kind":"replace_command","replacement":{"mode":"argv","argv":["tool"]}}),
                &task,
                &attempt,
                "CMD-001"
            )
            .unwrap(),
            json!({"kind":"replace_command","record_id":"CMD-001",
                "replacement":{"mode":"argv","argv":["tool"]}})
        );
        assert_eq!(
            formalize_semantic_action(
                &json!({"kind":"skip_record","reason":"Cannot run."}),
                &task,
                &attempt,
                "CMD-001"
            )
            .unwrap(),
            json!({"kind":"skip_record","record_id":"CMD-001","reason":"Cannot run."})
        );
        let recorded = json!({"execution_deviations":[
            {"proposal":{"action":{"kind":"add_command","command":{"id":"CMD-002"}}}},
            {"proposal":{"action":{"kind":"add_validation","validation":{"id":"VAL-002"}}}}]});
        assert_eq!(
            formalize_semantic_action(
                &json!({"kind":"add_validation","validation":{"kind":"manual",
                "confirmer":"user","criteria":"Reviewed."}}),
                &task,
                &recorded,
                "CMD-001"
            )
            .unwrap()["validation"]["id"],
            "VAL-003"
        );
        assert_eq!(
            formalize_semantic_action(
                &json!({"kind":"add_validation","validation":{"command_positions":[2]}}),
                &task,
                &attempt,
                "CMD-001",
            )
            .unwrap_err()
            .reason_code,
            "deviation_semantic_reference"
        );
    }

    #[test]
    fn reviewed_preview_and_record_match_current_contract_binding() {
        let proposal = json!({
            "schema":"work-execution-deviation-proposal", "task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001", "anchor_record_id":"CMD-001",
            "task_basis":["CMD-001","STEP-001"],
            "gap":"The executable is not available through PATH.",
            "action":{"kind":"replace_command","record_id":"CMD-001",
                "replacement":{"mode":"argv","argv":["C:/tools/tool.cmd","test"]}},
            "modifiable_files":[],
            "impact":{"summary":"Use the absolute executable path without changing command effects.",
                "requirement_changed":false,"scope_changed":false,"acceptance_criteria_changed":false,
                "deliverables_changed":false,"safety_boundary_changed":false,
                "external_side_effect_boundary_changed":false},
            "side_effects":["Runs the existing validation command."]
        });
        let sources = BTreeMap::from([("task.json".to_owned(), "a".repeat(64))]);
        let preview = build_deviation_preview(&proposal, "command", &sources).unwrap();
        assert_eq!(
            preview["preview_sha256"],
            "dcd6eb7d977ffd940df4cebffdf78ed993a4ba7c720dc03b197e4caab5c16ff3"
        );
        let attempt = json!({"authorization":{"authorization_evidence":"Original"}});
        let approved = preview["preview_sha256"].as_str().unwrap();
        let (candidate, response) = append_approved_deviation(
            &attempt,
            &preview,
            approved,
            "Fresh approval",
            "outputs/work/executions/example",
        )
        .unwrap();
        assert_eq!(
            candidate["execution_deviations"][0]["deviation_id"],
            "DEVIATION-001"
        );
        assert_eq!(
            response["attempt_path"],
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json"
        );
        assert_eq!(
            append_approved_deviation(
                &candidate,
                &preview,
                approved,
                "Fresh approval",
                "outputs/work/executions/example"
            )
            .unwrap_err()
            .reason_code,
            "deviation_record_duplicate"
        );
        assert_eq!(
            append_approved_deviation(
                &attempt,
                &preview,
                approved,
                "Original",
                "outputs/work/executions/example"
            )
            .unwrap_err()
            .reason_code,
            "deviation_record_authorization_evidence_reused"
        );
    }

    #[test]
    fn approved_deviation_binds_preview_action_scope_and_evidence() {
        let action = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":{"mode":"argv","argv":["tool","test"]}});
        let value = json!({"schema":"work-execution-deviation","deviation_id":"DEVIATION-001",
            "approved_preview_sha256":"c".repeat(64),
            "proposal":{"schema":"work-execution-deviation-proposal","task_id":"TASK-001",
                "attempt_id":"ATTEMPT-001","anchor_record_id":"CMD-001","task_basis":["CMD-001"],
                "gap":"A command needs an equivalent path.","action":action,"modifiable_files":[],
                "impact":{"summary":"Equivalent command path.","requirement_changed":false,
                    "scope_changed":false,"acceptance_criteria_changed":false,"deliverables_changed":false,
                    "safety_boundary_changed":false,"external_side_effect_boundary_changed":false},
                "side_effects":["Runs the existing command."]},
            "supplemental_authorization":{"schema":"work-execution-deviation-authorization",
                "preview_sha256":"c".repeat(64),"action":action,
                "modifiable_files":[],"authorization_evidence":"Approved"},
            "decision":{"outcome":"approved","evidence":"Approved"},"reconciliation_status":"pending"});
        assert!(validate_deviation_artifact(&value).is_ok());
        let mut defaults = value.clone();
        defaults["proposal"]
            .as_object_mut()
            .unwrap()
            .remove("modifiable_files");
        defaults["supplemental_authorization"]
            .as_object_mut()
            .unwrap()
            .remove("modifiable_files");
        validate_deviation_artifact(&defaults).unwrap();
        defaults["supplemental_authorization"]["modifiable_files"] = json!([]);
        validate_deviation_artifact(&defaults).unwrap();
        let mut incomplete = value.clone();
        incomplete["proposal"]
            .as_object_mut()
            .unwrap()
            .remove("gap");
        assert_eq!(
            validate_deviation_artifact(&incomplete)
                .unwrap_err()
                .reason_code,
            "invalid_execution_deviation_fields"
        );
        let mut rejected = value.clone();
        rejected["decision"]["outcome"] = json!("rejected");
        assert_eq!(
            validate_deviation_artifact(&rejected)
                .unwrap_err()
                .reason_code,
            "deviation_reconciliation_status_mismatch"
        );
        rejected["reconciliation_status"] = json!("not_needed");
        validate_deviation_artifact(&rejected).unwrap();
        let mut approved = value.clone();
        approved["reconciliation_status"] = json!("not_needed");
        assert_eq!(
            validate_deviation_artifact(&approved)
                .unwrap_err()
                .reason_code,
            "deviation_reconciliation_status_mismatch"
        );
        let mut fingerprint = value.clone();
        fingerprint["supplemental_authorization"]["preview_sha256"] = json!("d".repeat(64));
        assert_eq!(
            validate_deviation_artifact(&fingerprint)
                .unwrap_err()
                .reason_code,
            "deviation_preview_binding_mismatch"
        );
        let mut action_drift = value.clone();
        action_drift["supplemental_authorization"]["action"]["replacement"]["argv"] =
            json!(["other"]);
        assert_eq!(
            validate_deviation_artifact(&action_drift)
                .unwrap_err()
                .reason_code,
            "deviation_action_binding_mismatch"
        );
        let mut evidence = value.clone();
        evidence["decision"]["evidence"] = json!("Different evidence.");
        assert_eq!(
            validate_deviation_artifact(&evidence)
                .unwrap_err()
                .reason_code,
            "deviation_evidence_mismatch"
        );
        let mut drift = value;
        drift["supplemental_authorization"]["modifiable_files"] = json!(["other.txt"]);
        assert_eq!(
            validate_deviation_artifact(&drift).unwrap_err().reason_code,
            "deviation_file_scope_mismatch"
        );
    }
}

#[cfg(test)]
mod task_impact_tests {
    use super::*;

    #[test]
    fn every_specification_impact_requires_task_and_execution_revision() {
        let source = super::tests::strict_proposal();
        let sources = BTreeMap::from([("task/index.json".to_owned(), "a".repeat(64))]);
        for field in [
            "requirement_changed",
            "scope_changed",
            "deliverables_changed",
            "acceptance_criteria_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ] {
            let mut proposal = source.clone();
            proposal["impact"][field] = json!(true);
            let preview = build_deviation_preview(&proposal, "command", &sources).unwrap();
            assert_eq!(preview["classification"], "task_and_execution", "{field}");
            assert_eq!(preview["blocking"], true, "{field}");
            validate_deviation_preview(&preview).unwrap();
        }
    }
}
