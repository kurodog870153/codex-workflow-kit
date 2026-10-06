//! Equivalent command correction contract.

use serde_json::{Value, json};

use crate::derivation::identity::next_record_id;
use crate::execution::ExecutionIssue;
use crate::execution::authorization::{authorization_evidence, effective_task, require_deviation};
use crate::execution::formal_record_kind;

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

fn fields(value: &Value, location: &str, required: &[&str]) -> Result<(), ExecutionIssue> {
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
        .filter(|field| !required.contains(&field.as_str()))
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

fn command(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    match value["mode"].as_str() {
        Some("argv") => {
            fields(value, location, &["mode", "argv"])?;
            let rows = value["argv"]
                .as_array()
                .filter(|rows| !rows.is_empty())
                .ok_or_else(|| {
                    issue(
                        "attempt_invalid_command_argv",
                        "A command argv must be a non-empty string array.",
                        json!({"location":format!("{location}.argv")}),
                    )
                })?;
            if rows
                .iter()
                .any(|row| row.as_str().is_none_or(|text| text.trim().is_empty()))
            {
                return Err(issue(
                    "attempt_empty_text_value",
                    "A non-empty string is required.",
                    json!({"location":format!("{location}.argv[]")}),
                ));
            }
        }
        Some("shell") => {
            fields(value, location, &["mode", "script"])?;
            if value["script"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
            {
                return Err(issue(
                    "attempt_empty_text_value",
                    "A non-empty string is required.",
                    json!({"location":format!("{location}.script")}),
                ));
            }
        }
        _ => {
            return Err(issue(
                "attempt_invalid_command_mode",
                "A command mode must be argv or shell.",
                json!({"location":format!("{location}.mode")}),
            ));
        }
    }
    Ok(())
}

pub fn validate_command_correction(value: &Value, location: &str) -> Result<(), ExecutionIssue> {
    fields(
        value,
        location,
        &[
            "original_command",
            "actual_command",
            "reason",
            "authorization_evidence",
        ],
    )?;
    command(
        &value["original_command"],
        &format!("{location}.original_command"),
    )?;
    command(
        &value["actual_command"],
        &format!("{location}.actual_command"),
    )?;
    if value["original_command"]["mode"] != value["actual_command"]["mode"] {
        return Err(issue(
            "attempt_command_correction_mode_mismatch",
            "An equivalent command correction must preserve the command mode.",
            json!({"location":location}),
        ));
    }
    if value["original_command"] == value["actual_command"] {
        return Err(issue(
            "attempt_command_correction_unchanged",
            "A command correction must change the command value.",
            json!({"location":location}),
        ));
    }
    for field in ["reason", "authorization_evidence"] {
        if value[field]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        {
            return Err(issue(
                "attempt_empty_text_value",
                "A non-empty string is required.",
                json!({"location":format!("{location}.{field}")}),
            ));
        }
    }
    Ok(())
}

pub fn validate_command_correction_request(request: &Value) -> Result<(), ExecutionIssue> {
    let object = request.as_object().ok_or_else(|| {
        issue(
            "command_correction_expected_object",
            "A JSON object is required.",
            json!({}),
        )
    })?;
    let missing: Vec<_> = ["schema", "actual_command", "reason"]
        .into_iter()
        .filter(|field| !object.contains_key(*field))
        .collect();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !["schema", "actual_command", "reason"].contains(&field.as_str()))
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "command_correction_invalid_fields",
            "The command-correction request has missing or unknown fields.",
            json!({"missing":missing,"unknown":unknown}),
        ));
    }
    if request["schema"] != "work-command-correction-request" {
        return Err(issue(
            "command_correction_invalid_schema",
            "The command-correction request schema is invalid.",
            json!({}),
        ));
    }
    if command(
        &request["actual_command"],
        "command_correction_request.actual_command",
    )
    .is_err()
        || !request["reason"].is_string()
    {
        return Err(issue(
            "command_correction_invalid_fields",
            "The command-correction request is invalid.",
            json!({}),
        ));
    }
    work_model::execution::request::verified::<
        work_model::execution::request::CommandCorrectionRequest,
    >(request);
    Ok(())
}

pub fn build_command_correction_candidate(
    task: &Value,
    attempt: &Value,
    index: &Value,
    task_id: &str,
    request: &Value,
) -> Result<(Value, String), ExecutionIssue> {
    validate_command_correction_request(request)?;
    let lock = &index["lock"];
    if lock["kind"] != "execution"
        || lock["task_id"] != task_id
        || lock["attempt_id"] != attempt["attempt_id"]
        || lock["execute_instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(issue(
            "command_correction_lock_mismatch",
            "The execution lock does not match the reserved command.",
            json!({}),
        ));
    }
    if lock.get("command_correction").is_some() {
        return Err(issue(
            "command_correction_already_recorded",
            "The reserved command already has a correction.",
            json!({}),
        ));
    }
    let record_id = lock["record_id"].as_str().ok_or_else(|| {
        issue(
            "command_correction_lock_mismatch",
            "The execution lock has no reserved command.",
            json!({}),
        )
    })?;
    let base = record_id.split('#').next().unwrap_or(record_id);
    let effective = effective_task(task, attempt)?;
    if formal_record_kind(&effective, base)? != "command" {
        return Err(issue(
            "command_correction_non_command_record",
            "Only a formal CMD record can have a command correction.",
            json!({}),
        ));
    }
    let expected = next_record_id(base, attempt)?;
    if record_id != expected {
        return Err(issue(
            "command_correction_retry_sequence_mismatch",
            "The reserved command ID is not the next record instance.",
            json!({"expected":expected,"actual":record_id}),
        ));
    }
    let formal = effective["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == base)
        .ok_or_else(|| {
            issue(
                "command_correction_non_command_record",
                "Only a formal CMD record can have a command correction.",
                json!({}),
            )
        })?;
    let mut original = json!({"mode":formal["mode"]});
    let command_field = if formal["mode"] == "argv" {
        "argv"
    } else {
        "script"
    };
    original[command_field] = formal[command_field].clone();
    let correction = json!({"original_command":original,
        "actual_command":request["actual_command"],"reason":request["reason"],
        "authorization_evidence":authorization_evidence(attempt,lock,base)?});
    validate_command_correction(&correction, "formal_command_correction")?;
    require_deviation(
        attempt,
        &json!({"kind":"replace_command","record_id":record_id,
        "replacement":correction["actual_command"]}),
    )?;
    let mut candidate = index.clone();
    candidate["lock"]["command_correction"] = correction;
    Ok((candidate, record_id.to_owned()))
}

pub struct CommandCorrectionStagingInput<'a> {
    pub canonical_root: &'a str,
    pub requirement: &'a crate::identifiers::RequirementId,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub task: &'a Value,
    pub index_before: &'a [u8],
    pub attempt_before: &'a [u8],
    pub index_after: &'a [u8],
}

pub fn build_command_correction_staging(
    input: CommandCorrectionStagingInput<'_>,
) -> Result<work_model::runtime::RuntimeManifest, ExecutionIssue> {
    use crate::canonical::parse_json_contract;
    use crate::derivation::fingerprint;
    use work_model::runtime::{RuntimeBytes, RuntimeTarget};
    let parse = |raw| {
        parse_json_contract(raw).map_err(|_| {
            issue(
                "command_correction_staging_contract",
                "Canonical execution artifacts are required.",
                json!({}),
            )
        })
    };
    let before = parse(input.index_before)?;
    let after = parse(input.index_after)?;
    let attempt = parse(input.attempt_before)?;
    crate::execution::index::validate_execution_index(&before, input.index_before)?;
    crate::execution::index::validate_execution_index(&after, input.index_after)?;
    crate::execution::attempt::validate_attempt_bytes(&attempt, input.attempt_before)?;
    if before["requirement_id"] != input.requirement.as_str()
        || attempt["task_id"] != input.task_id
        || attempt["attempt_id"] != input.attempt_id
        || attempt["status"] != "in_progress"
    {
        return Err(issue(
            "command_correction_staging_identity",
            "The execution identities disagree.",
            json!({}),
        ));
    }
    let request = json!({"schema":"work-command-correction-request",
        "actual_command":after["lock"]["command_correction"]["actual_command"],
        "reason":after["lock"]["command_correction"]["reason"]});
    let (expected, record) =
        build_command_correction_candidate(input.task, &attempt, &before, input.task_id, &request)?;
    if record != input.record_id
        || crate::execution::index::render_execution_index(&expected).map_err(|_| {
            issue(
                "command_correction_staging_contract",
                "The index cannot be rendered.",
                json!({}),
            )
        })? != input.index_after
    {
        return Err(issue(
            "command_correction_staging_transition",
            "The prepared index is not the unique command correction.",
            json!({}),
        ));
    }
    let evidence = |raw: &[u8]| RuntimeBytes {
        bytes: raw.to_vec(),
        sha256: fingerprint::raw(raw),
    };
    let payloads =
        std::collections::BTreeMap::from([("index.json.tmp".into(), input.index_after.to_vec())]);
    crate::execution::recovery::build_execution_staging_manifest(
        crate::execution::recovery::ExecutionStagingInput {
            canonical_root: input.canonical_root,
            requirement: input.requirement,
            execution_dir: input.execution_dir,
            operation: crate::derivation::publication::RuntimeOperation::CommandCorrection,
            approval_sha256: &fingerprint::raw(input.index_after),
            business_identity: json!({"task_id":input.task_id,"attempt_id":input.attempt_id,"record_id":input.record_id}),
            targets: vec![
                RuntimeTarget {
                    path: format!("{}/index.json", input.execution_dir),
                    before: Some(evidence(input.index_before)),
                    after: Some(evidence(input.index_after)),
                },
                RuntimeTarget {
                    path: format!(
                        "{}/{}/{}/attempt.json",
                        input.execution_dir, input.task_id, input.attempt_id
                    ),
                    before: Some(evidence(input.attempt_before)),
                    after: Some(evidence(input.attempt_before)),
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
    fn command_correction_request_matches_current_contract_contract_cases() {
        let mut request = json!({"schema":"work-command-correction-request",
            "actual_command":{"mode":"argv","argv":["tool","new"]},
            "reason":"Use the authorized argument."});
        validate_command_correction_request(&request).unwrap();
        request["actual_command"] = json!({"mode":"shell","script":"echo hello"});
        validate_command_correction_request(&request).unwrap();
        for invalid in [
            json!({"mode":"argv","script":"tool"}),
            json!({"mode":"shell","argv":["tool"]}),
            json!({"mode":"argv","argv":["tool"],"script":"tool"}),
            json!({"mode":"argv","argv":["tool"],"id":"CMD-001"}),
            json!({"mode":"argv","argv":[]}),
            json!({"mode":"argv","argv":[" "]}),
            json!({"mode":"shell","script":"  "}),
            json!({"mode":"invalid","argv":["tool"]}),
        ] {
            request["actual_command"] = invalid;
            assert_eq!(
                validate_command_correction_request(&request)
                    .unwrap_err()
                    .reason_code,
                "command_correction_invalid_fields"
            );
        }
        request["actual_command"] = json!({"mode":"argv","argv":["tool","new"]});
        request.as_object_mut().unwrap().remove("reason");
        request["extra"] = json!(true);
        let error = validate_command_correction_request(&request).unwrap_err();
        assert_eq!(error.reason_code, "command_correction_invalid_fields");
        assert_eq!(
            error.details,
            json!({"missing":["reason"],"unknown":["extra"]})
        );
        request.as_object_mut().unwrap().remove("extra");
        request["reason"] = json!("Use the authorized argument.");
        request["schema"] = json!("invalid");
        assert_eq!(
            validate_command_correction_request(&request)
                .unwrap_err()
                .reason_code,
            "command_correction_invalid_schema"
        );
        request["schema"] = json!("work-command-correction-request");
        for (field, value) in [
            ("record_id", json!("CMD-001")),
            (
                "original_command",
                json!({"mode":"argv","argv":["tool","old"]}),
            ),
        ] {
            request[field] = value;
            assert_eq!(
                validate_command_correction_request(&request)
                    .unwrap_err()
                    .reason_code,
                "command_correction_invalid_fields"
            );
            request.as_object_mut().unwrap().remove(field);
        }
    }

    #[test]
    fn candidate_requires_formal_reserved_command_and_preapproved_replacement() {
        let original = json!({"mode":"argv","argv":["tool"]});
        let actual = json!({"mode":"argv","argv":["/bin/tool"]});
        let action = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":actual});
        let task = json!({"commands":[{"id":"CMD-001","mode":"argv","argv":["tool"]}],
            "operations":[],"validations":[]});
        let attempt = json!({"attempt_id":"ATTEMPT-001","status":"in_progress",
            "execute_instructions_sha256":"a".repeat(64),"records":[],
            "authorization":{"authorization_evidence":"Approved",
                "allowed_deviations":[action]}});
        let index = json!({"lock":{"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","record_id":"CMD-001",
            "execute_instructions_sha256":"a".repeat(64)}});
        let request = json!({"schema":"work-command-correction-request",
            "actual_command":actual,"reason":"Use full path"});
        let (candidate, id) =
            build_command_correction_candidate(&task, &attempt, &index, "TASK-001", &request)
                .unwrap();
        assert_eq!(id, "CMD-001");
        assert_eq!(
            candidate["lock"]["command_correction"]["original_command"],
            original
        );
        assert_eq!(
            candidate["lock"]["command_correction"]["authorization_evidence"],
            "Approved"
        );
        let previous = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":{"mode":"argv","argv":["/usr/bin/tool"]}});
        let mut effective = attempt.clone();
        effective["execution_deviations"] = json!([{
            "decision":{"outcome":"approved","evidence":"Approved"},
            "proposal":{"action":previous,"anchor_record_id":"CMD-001",
                "impact":{},"modifiable_files":[]},
            "approved_preview_sha256":"b".repeat(64),
            "supplemental_authorization":{"preview_sha256":"b".repeat(64),
                "action":previous,"modifiable_files":[],
                "authorization_evidence":"Approved"}
        }]);
        let (corrected, identity) =
            build_command_correction_candidate(&task, &effective, &index, "TASK-001", &request)
                .unwrap();
        assert_eq!(identity, "CMD-001");
        assert_eq!(
            corrected["lock"]["command_correction"]["original_command"],
            json!({"mode":"argv","argv":["/usr/bin/tool"]})
        );
        assert_eq!(
            corrected["lock"]["command_correction"]["actual_command"],
            actual
        );
        let mut denied = attempt;
        denied["authorization"]["allowed_deviations"] = json!([]);
        assert_eq!(
            build_command_correction_candidate(&task, &denied, &index, "TASK-001", &request)
                .unwrap_err()
                .reason_code,
            "execution_authorization_deviation_required"
        );
    }

    #[test]
    fn equivalent_command_must_change_value_without_changing_mode() {
        let correction = json!({"original_command":{"mode":"argv","argv":["tool"]},
            "actual_command":{"mode":"argv","argv":["/bin/tool"]},
            "reason":"Use absolute path.","authorization_evidence":"Approved replacement."});
        assert!(validate_command_correction(&correction, "correction").is_ok());
        let mut unchanged = correction.clone();
        unchanged["actual_command"] = unchanged["original_command"].clone();
        assert_eq!(
            validate_command_correction(&unchanged, "correction")
                .unwrap_err()
                .reason_code,
            "attempt_command_correction_unchanged"
        );
    }

    #[test]
    fn canonical_correction_accepts_argv_and_shell_and_rejects_invalid_fields() {
        let mut correction = json!({"original_command":{"mode":"argv","argv":["tool","old"]},
            "actual_command":{"mode":"argv","argv":["tool","new"]},
            "reason":"Use the authorized argument.","authorization_evidence":"Manifest authorization."});
        validate_command_correction(&correction, "lock.correction").unwrap();
        correction["original_command"] = json!({"mode":"shell","script":"tool old"});
        correction["actual_command"] = json!({"mode":"shell","script":"tool new"});
        validate_command_correction(&correction, "lock.correction").unwrap();
        correction["actual_command"] = json!({"mode":"argv","argv":["tool","new"]});
        assert_eq!(
            validate_command_correction(&correction, "lock.correction")
                .unwrap_err()
                .reason_code,
            "attempt_command_correction_mode_mismatch"
        );
        correction["original_command"] = json!({"mode":"argv","argv":["tool","old"]});
        correction.as_object_mut().unwrap().remove("reason");
        correction["extra"] = json!(true);
        let error = validate_command_correction(&correction, "lock.correction").unwrap_err();
        assert_eq!(error.reason_code, "attempt_invalid_object_fields");
        assert_eq!(error.details["location"], "lock.correction");
        assert_eq!(error.details["missing"], json!(["reason"]));
        assert_eq!(error.details["unknown"], json!(["extra"]));
    }
}
