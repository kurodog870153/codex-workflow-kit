//! Handoff direction and discussion-only transfer rules.

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::identifiers::RequirementId;
use crate::protocol::{ATTEMPT_ID_PREFIX, INVALID_SHA256_ERROR_CODE, TASK_ID_PREFIX, valid_sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> HandoffIssue {
    HandoffIssue {
        reason_code,
        message,
        details,
    }
}

pub fn direction_stages(direction: &str) -> Option<(&'static str, &'static str)> {
    match direction {
        "task_to_execute" => Some(("task", "execute")),
        "execute_to_task" => Some(("execute", "task")),
        _ => None,
    }
}

pub fn build_formal_handoff(
    direction: &str,
    requirement_id: &str,
    artifacts: &Value,
    source: &Value,
    payload: &Value,
) -> Result<Value, HandoffIssue> {
    let (from, to) = direction_stages(direction).ok_or_else(|| {
        issue(
            "invalid_handoff_direction",
            "The Handoff direction is invalid.",
            json!({"direction":direction}),
        )
    })?;
    let source = source.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "The Handoff source must be an object.",
            json!({"location":"source"}),
        )
    })?;
    let payload = payload.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "The Handoff payload must be an object.",
            json!({"location":"payload"}),
        )
    })?;
    if source.contains_key("stage")
        || payload.keys().any(|key| {
            [
                "schema",
                "marker",
                "direction",
                "requirement_id",
                "artifacts",
                "source",
                "target",
            ]
            .contains(&key.as_str())
        })
    {
        return Err(issue(
            "invalid_object_fields",
            "Handoff builder fields cannot override its identity.",
            json!({}),
        ));
    }
    let mut source = source.clone();
    source.insert("stage".into(), json!(from));
    let mut handoff = json!({
        "schema":"work-handoff","marker":"WORK-HANDOFF","direction":direction,
        "requirement_id":requirement_id,"artifacts":artifacts,"source":source,
        "target":{"stage":to},
    });
    for (key, value) in payload {
        handoff[key] = value.clone();
    }
    validate_handoff_structure(&handoff)?;
    let _: work_model::handoff::FormalHandoff =
        serde_json::from_value(handoff.clone()).expect("built handoff matches its model");
    Ok(handoff)
}

pub fn build_discussion_handoff(request: &Value) -> Result<Value, HandoffIssue> {
    let object = request
        .as_object()
        .filter(|object| {
            object.contains_key("schema")
                && object.contains_key("direction")
                && object.contains_key("requirement_id")
                && object.contains_key("summary")
                && object.keys().all(|field| {
                    [
                        "schema",
                        "direction",
                        "requirement_id",
                        "summary",
                        "confirmed_approach",
                        "requested_changes",
                        "preserve",
                        "affected_ids",
                        "validation_requirements",
                    ]
                    .contains(&field.as_str())
                })
        })
        .ok_or_else(|| {
            issue(
                "invalid_object_fields",
                "The discussion handoff request is invalid.",
                json!({}),
            )
        })?;
    if object["schema"] != "work-discussion-handoff-request" {
        return Err(issue(
            "invalid_contract_value",
            "The discussion handoff schema is invalid.",
            json!({}),
        ));
    }
    let direction = object["direction"]
        .as_str()
        .filter(|direction| matches!(*direction, "task_to_execute" | "execute_to_task"))
        .ok_or_else(|| {
            issue(
                "invalid_contract_value",
                "The discussion handoff direction is invalid.",
                json!({}),
            )
        })?;
    let requirement_id = object["requirement_id"].as_str().ok_or_else(|| {
        issue(
            "invalid_requirement_id",
            "The requirement ID is invalid.",
            json!({}),
        )
    })?;
    requirement_id.parse::<RequirementId>().map_err(|failure| {
        issue(
            failure.reason_code(),
            "The requirement ID is invalid.",
            json!({}),
        )
    })?;
    let summary = object["summary"]
        .as_str()
        .filter(|summary| !summary.trim().is_empty())
        .ok_or_else(|| {
            issue(
                "empty_text_value",
                "A non-empty summary is required.",
                json!({}),
            )
        })?;
    for field in [
        "requested_changes",
        "preserve",
        "affected_ids",
        "validation_requirements",
    ] {
        if object.get(field).is_some_and(|value| {
            !value.is_null()
                && !value
                    .as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
        }) {
            return Err(issue(
                "invalid_contract_value",
                "Discussion handoff entries must be string arrays.",
                json!({"field":field}),
            ));
        }
    }
    if object
        .get("confirmed_approach")
        .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err(issue(
            "invalid_contract_value",
            "The confirmed approach must be text.",
            json!({}),
        ));
    }
    let (source, target) = direction_stages(direction).expect("filtered direction");
    let _: work_model::handoff::DiscussionHandoffRequest = serde_json::from_value(request.clone())
        .expect("validated discussion request matches its model");
    let mut result = json!({"schema":"work-discussion-handoff","marker":"WORK-DISCUSSION-HANDOFF","direction":direction,"requirement_id":requirement_id,"source_stage":source,"target_stage":target,"source_status":"unsaved_discussion","source_validation":"not_checked","grants_authorization":false,"summary":summary});
    for field in [
        "confirmed_approach",
        "requested_changes",
        "preserve",
        "affected_ids",
        "validation_requirements",
    ] {
        if let Some(value) = object.get(field) {
            result[field] = value.clone();
        }
    }
    let _: work_model::handoff::DiscussionHandoff =
        serde_json::from_value(result.clone()).expect("discussion handoff matches its model");
    Ok(result)
}

pub fn require_known_affected_ids(
    affected: &[String],
    known: &std::collections::BTreeSet<String>,
    code: &'static str,
) -> Result<(), HandoffIssue> {
    let unknown: Vec<_> = affected
        .iter()
        .filter(|id| !known.contains(*id))
        .cloned()
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(issue(
            code,
            "Affected IDs must exist in the validated source scope.",
            json!({"affected_ids":unknown}),
        ))
    }
}

pub fn require_matching_source(incoming: &Value, expected: &Value) -> Result<(), HandoffIssue> {
    let mismatches: Vec<_> = ["requirement_id", "artifacts", "source"]
        .into_iter()
        .filter(|field| incoming[*field] != expected[*field])
        .collect();
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(issue(
            "handoff_source_mismatch",
            "The incoming handoff does not match the selected current source.",
            json!({"fields":mismatches}),
        ))
    }
}

fn strict<'a>(
    value: &'a Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, HandoffIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location":location}),
        )
    })?;
    let mut missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    missing.sort_unstable();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect();
    if missing.is_empty() && unknown.is_empty() {
        Ok(object)
    } else {
        Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":location,"missing":missing,"unknown":unknown}),
        ))
    }
}

fn nonempty(value: &Value, location: &str) -> Result<(), HandoffIssue> {
    if value.as_str().is_some_and(|text| !text.trim().is_empty()) {
        Ok(())
    } else {
        Err(issue(
            "empty_text_value",
            "A non-empty string is required.",
            json!({"location":location}),
        ))
    }
}

fn sha(value: &Value, location: &str) -> Result<(), HandoffIssue> {
    if value.as_str().is_some_and(valid_sha256) {
        Ok(())
    } else {
        Err(issue(
            INVALID_SHA256_ERROR_CODE,
            "A lowercase SHA-256 value is required.",
            json!({"location":location}),
        ))
    }
}

fn text_array(value: &Value, location: &str) -> Result<Vec<String>, HandoffIssue> {
    let rows = value
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_string_array",
                "A non-empty string array is required.",
                json!({"location":location}),
            )
        })?;
    let mut result = Vec::new();
    for row in rows {
        nonempty(row, &format!("{location}[]"))?;
        result.push(row.as_str().expect("checked string").to_owned());
    }
    if result.iter().collect::<BTreeSet<_>>().len() != result.len() {
        return Err(issue(
            "duplicate_array_value",
            "Array values must be unique.",
            json!({"location":location}),
        ));
    }
    Ok(result)
}

fn numbered_id(value: &Value, prefix: &str, location: &str) -> Result<(), HandoffIssue> {
    nonempty(value, location)?;
    let id = value.as_str().expect("checked string");
    if id.starts_with(prefix)
        && id.len() == prefix.len() + 3
        && id[prefix.len()..].bytes().all(|byte| byte.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(issue(
            "invalid_handoff_identifier",
            "A Handoff identifier has an invalid format.",
            json!({"location":location,"value":id}),
        ))
    }
}

pub fn validate_handoff_structure(contract: &Value) -> Result<Value, HandoffIssue> {
    let direction = contract["direction"].as_str().ok_or_else(|| {
        issue(
            "invalid_handoff_direction",
            "The Handoff direction is invalid.",
            json!({"direction":contract["direction"]}),
        )
    })?;
    let (source_stage, target_stage) = direction_stages(direction).ok_or_else(|| {
        issue(
            "invalid_handoff_direction",
            "The Handoff direction is invalid.",
            json!({"direction":direction}),
        )
    })?;
    let common = [
        "schema",
        "marker",
        "direction",
        "requirement_id",
        "artifacts",
        "source",
        "target",
        "summary",
    ];
    let return_fields = [
        "confirmed_approach",
        "requested_changes",
        "preserve",
        "affected_ids",
        "validation_requirements",
    ];
    let mut required = common.to_vec();
    if direction == "execute_to_task" {
        required.extend(return_fields);
    }
    strict(contract, "handoff", &required, &[])?;
    if contract["schema"] != "work-handoff" {
        return Err(issue(
            "invalid_handoff_schema",
            "The Handoff schema is invalid.",
            json!({}),
        ));
    }
    if contract["marker"] != "WORK-HANDOFF" {
        return Err(issue(
            "invalid_handoff_marker",
            "The Handoff marker is invalid.",
            json!({}),
        ));
    }
    nonempty(&contract["requirement_id"], "requirement_id")?;
    let requirement_id = contract["requirement_id"].as_str().expect("checked ID");
    requirement_id.parse::<RequirementId>().map_err(|failure| {
        issue(
            failure.reason_code(),
            "The requirement ID is invalid.",
            json!({}),
        )
    })?;
    let source = &contract["source"];
    {
        let mut required = vec![
            "stage",
            "task_spec_id",
            "task_id",
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
            "task_instructions_sha256",
            "skill_id",
        ];
        let executing = direction.starts_with("execute_to_");
        if executing {
            required.extend(["execution_context", "execute_skill_selection_sha256"]);
        } else {
            required.push("skill_selection_sha256");
        }
        strict(
            source,
            "source",
            &required,
            if executing { &["attempt_sha256"] } else { &[] },
        )?;
        numbered_id(&source["task_spec_id"], "TASK-SPEC-", "source.task_spec_id")?;
        numbered_id(&source["task_id"], TASK_ID_PREFIX, "source.task_id")?;
        for field in [
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
            "task_instructions_sha256",
        ] {
            sha(&source[field], &format!("source.{field}"))?;
        }
        if source["skill_id"]
            .as_str()
            .is_some_and(|text| text.trim().is_empty())
        {
            nonempty(&source["skill_id"], "source.skill_id")?;
        }
        if executing {
            sha(
                &source["execute_skill_selection_sha256"],
                "source.execute_skill_selection_sha256",
            )?;
            if source.get("attempt_sha256").is_some() {
                sha(&source["attempt_sha256"], "source.attempt_sha256")?;
            }
            let context = &source["execution_context"];
            strict(
                context,
                "source.execution_context",
                &["attempt", "phase", "issue_type", "reason"],
                &[],
            )?;
            let attempt = &context["attempt"];
            strict(
                attempt,
                "source.execution_context.attempt",
                &["status"],
                &["id"],
            )?;
            match attempt["status"].as_str() {
                Some("not_created") if attempt.get("id").is_some() => {
                    return Err(issue(
                        "unexpected_attempt_id",
                        "A not-created Attempt cannot have an ID.",
                        json!({}),
                    ));
                }
                Some("stopped" | "blocked") if attempt.get("id").is_none() => {
                    return Err(issue(
                        "missing_attempt_id",
                        "A stopped or blocked Attempt must have an ID.",
                        json!({}),
                    ));
                }
                Some("stopped" | "blocked") => numbered_id(
                    &attempt["id"],
                    ATTEMPT_ID_PREFIX,
                    "source.execution_context.attempt.id",
                )?,
                Some("not_created") => {}
                _ => {
                    return Err(issue(
                        "invalid_attempt_status",
                        "The Handoff Attempt status is invalid.",
                        json!({}),
                    ));
                }
            }
            if !matches!(context["phase"].as_str(), Some("preflight" | "execution")) {
                return Err(issue(
                    "invalid_execution_phase",
                    "The Handoff execution phase is invalid.",
                    json!({}),
                ));
            }
            if context["issue_type"] != "specification_defect" {
                return Err(issue(
                    "invalid_issue_type",
                    "An Execute return Handoff must describe a specification defect.",
                    json!({}),
                ));
            }
            nonempty(&context["reason"], "source.execution_context.reason")?;
        } else {
            sha(
                &source["skill_selection_sha256"],
                "source.skill_selection_sha256",
            )?;
        }
    }
    if source["stage"] != source_stage {
        return Err(issue(
            "handoff_source_stage_mismatch",
            "The Handoff source stage does not match its direction.",
            json!({"expected":source_stage,"actual":source["stage"]}),
        ));
    }
    strict(&contract["target"], "target", &["stage"], &[])?;
    if contract["target"]["stage"] != target_stage {
        return Err(issue(
            "handoff_target_stage_mismatch",
            "The Handoff target stage does not match its direction.",
            json!({"expected":target_stage,"actual":contract["target"]["stage"]}),
        ));
    }
    nonempty(&contract["summary"], "summary")?;
    if direction == "execute_to_task" {
        let ids = text_array(&contract["affected_ids"], "affected_ids")?;
        for id in ids {
            let Some((prefix, number)) = id.rsplit_once('-') else {
                return Err(issue(
                    "invalid_handoff_identifier",
                    "A Handoff identifier has an invalid format.",
                    json!({"location":"affected_ids[]","value":id}),
                ));
            };
            if prefix.is_empty()
                || !prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
                || number.len() != 3
                || !number.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(issue(
                    "invalid_handoff_identifier",
                    "A Handoff identifier has an invalid format.",
                    json!({"location":"affected_ids[]","value":id}),
                ));
            }
        }
    }
    if direction == "execute_to_task" {
        nonempty(&contract["confirmed_approach"], "confirmed_approach")?;
        for field in ["requested_changes", "preserve", "validation_requirements"] {
            text_array(&contract[field], field)?;
        }
    }
    let result = json!({"schema":"work-handoff-validation","marker":"WORK-HANDOFF","direction":direction,"requirement_id":requirement_id,"source_stage":source_stage,"target_stage":target_stage,"status":"valid"});
    let _: work_model::handoff::HandoffValidation =
        serde_json::from_value(result.clone()).expect("handoff validation matches its model");
    Ok(result)
}

struct OrderedDiscussion<'a>(&'a Value);

impl Serialize for OrderedDiscussion<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let object = self.0.as_object().expect("built handoff");
        let mut output = serializer.serialize_map(Some(object.len()))?;
        for field in [
            "schema",
            "marker",
            "direction",
            "requirement_id",
            "source_stage",
            "target_stage",
            "source_status",
            "source_validation",
            "grants_authorization",
            "summary",
            "confirmed_approach",
            "requested_changes",
            "preserve",
            "affected_ids",
            "validation_requirements",
        ] {
            if let Some(value) = object.get(field) {
                output.serialize_entry(field, value)?;
            }
        }
        output.end()
    }
}

pub fn render_discussion_handoff(value: &Value) -> Result<Vec<u8>, HandoffIssue> {
    if value["schema"] != "work-discussion-handoff" || value["marker"] != "WORK-DISCUSSION-HANDOFF"
    {
        return Err(issue(
            "invalid_contract_value",
            "The discussion handoff is invalid.",
            json!({}),
        ));
    }
    let (source, target) = value["direction"]
        .as_str()
        .and_then(direction_stages)
        .ok_or_else(|| {
            issue(
                "invalid_handoff_direction",
                "The discussion direction is invalid.",
                json!({}),
            )
        })?;
    if value["source_stage"] != source
        || value["target_stage"] != target
        || value["grants_authorization"] != false
    {
        return Err(issue(
            "invalid_contract_value",
            "Discussion handoffs cannot grant authorization or change their stages.",
            json!({}),
        ));
    }
    let mut raw =
        serde_json::to_vec_pretty(&OrderedDiscussion(value)).expect("JSON value serializes");
    raw.push(b'\n');
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discussion_handoff_never_grants_authorization() {
        let request = json!({"schema":"work-discussion-handoff-request","direction":"task_to_execute","requirement_id":"example","summary":"Review the unfinished discussion."});
        let result = build_discussion_handoff(&request).unwrap();
        assert_eq!(result["source_stage"], "task");
        assert_eq!(result["target_stage"], "execute");
        assert_eq!(result["grants_authorization"], false);
        for direction in ["plan_to_task", "task_to_plan", "execute_to_plan"] {
            let mut legacy_request = request.clone();
            legacy_request["direction"] = json!(direction);
            assert!(build_discussion_handoff(&legacy_request).is_err());
            let mut legacy = result.clone();
            legacy["direction"] = json!(direction);
            assert!(render_discussion_handoff(&legacy).is_err());
            assert!(
                serde_json::from_value::<work_model::handoff::DiscussionHandoff>(legacy).is_err()
            );
        }
        assert_eq!(
            crate::canonical::sha256_hex(&render_discussion_handoff(&result).unwrap()),
            "d6ad0e89128bdeb5dd5060aa316602bccb90b159886c08d3331a58b0f32426f3"
        );
        let mut wrong = result.clone();
        wrong["source"] = json!({"sha":"changed"});
        assert_eq!(
            require_matching_source(&wrong, &result)
                .unwrap_err()
                .reason_code,
            "handoff_source_mismatch"
        );
    }

    #[test]
    fn current_contract_formal_handoff_directions_and_rejections() {
        for direction in ["task_to_execute", "execute_to_task"] {
            let (from, to) = direction_stages(direction).unwrap();
            let source = {
                let mut source = json!({"stage":from,"task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","task_collection_sha256":"1".repeat(64),"task_index_sha256":"2".repeat(64),"task_item_sha256":"3".repeat(64),"task_instructions_sha256":"c".repeat(64),"skill_id":null});
                if direction.starts_with("execute_to_") {
                    source["execute_skill_selection_sha256"] = json!("d".repeat(64));
                    source["execution_context"] = json!({"attempt":{"status":"not_created"},"phase":"preflight","issue_type":"specification_defect","reason":"The specification needs clarification."});
                } else {
                    source["skill_selection_sha256"] = json!("d".repeat(64));
                }
                source
            };
            let mut contract = json!({"schema":"work-handoff","marker":"WORK-HANDOFF","direction":direction,"requirement_id":"example","artifacts":{"source":"outputs/work/sources/example","task":"outputs/work/tasks/example/index.json","execution":"outputs/work/executions/example"},"source":source,"target":{"stage":to},"summary":"Continue the workflow."});
            if direction == "execute_to_task" {
                contract["confirmed_approach"] = json!("Update the source artifact.");
                contract["requested_changes"] = json!(["Clarify the expected behavior."]);
                contract["preserve"] = json!(["Keep existing identifiers."]);
                contract["affected_ids"] = json!(["TASK-001"]);
                contract["validation_requirements"] = json!(["Revalidate the artifact."]);
            }
            assert_eq!(
                validate_handoff_structure(&contract).unwrap()["status"],
                "valid",
                "{direction}"
            );
            for legacy_direction in ["plan_to_task", "task_to_plan", "execute_to_plan"] {
                let mut legacy = contract.clone();
                legacy["direction"] = json!(legacy_direction);
                assert_eq!(
                    validate_handoff_structure(&legacy).unwrap_err().reason_code,
                    "invalid_handoff_direction"
                );
                assert!(
                    serde_json::from_value::<work_model::handoff::FormalHandoff>(legacy).is_err()
                );
            }
            let mut legacy_source = contract.clone();
            legacy_source["source"]["plan_sha256"] = json!("a".repeat(64));
            assert_eq!(
                validate_handoff_structure(&legacy_source)
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields"
            );
            if direction == "task_to_execute" {
                let valid = contract.clone();
                contract["source"]
                    .as_object_mut()
                    .unwrap()
                    .remove("task_instructions_sha256");
                contract["source"]["task_rules_sha256"] = json!("c".repeat(64));
                let failure = validate_handoff_structure(&contract).unwrap_err();
                assert_eq!(failure.reason_code, "invalid_object_fields");
                assert_eq!(
                    failure.details["missing"],
                    json!(["task_instructions_sha256"])
                );
                assert_eq!(failure.details["unknown"], json!(["task_rules_sha256"]));
                let mut legacy_task = valid.clone();
                let source = legacy_task["source"].as_object_mut().unwrap();
                let collection = source.remove("task_collection_sha256").unwrap();
                source.remove("task_index_sha256");
                source.remove("task_item_sha256");
                source.insert("task_sha256".into(), collection);
                let failure = validate_handoff_structure(&legacy_task).unwrap_err();
                assert_eq!(failure.reason_code, "invalid_object_fields");
                assert_eq!(failure.details["unknown"], json!(["task_sha256"]));
                let mut legacy_target = valid.clone();
                legacy_target["target"]["selection_topology"] = json!(["general", "web"]);
                let failure = validate_handoff_structure(&legacy_target).unwrap_err();
                assert_eq!(failure.reason_code, "invalid_object_fields");
                assert_eq!(failure.details["unknown"], json!(["selection_topology"]));
                let mut missing_skill = valid;
                missing_skill["source"]
                    .as_object_mut()
                    .unwrap()
                    .remove("skill_selection_sha256");
                let failure = validate_handoff_structure(&missing_skill).unwrap_err();
                assert_eq!(failure.reason_code, "invalid_object_fields");
                assert_eq!(
                    failure.details["missing"],
                    json!(["skill_selection_sha256"])
                );
            }
        }
    }

    #[test]
    fn formal_builder_sets_direction_stages_and_validates_payload() {
        let built = build_formal_handoff(
            "task_to_execute", "example",
            &json!({"source":"outputs/work/sources/example","task":"outputs/work/tasks/example/index.json","execution":"outputs/work/executions/example"}),
            &json!({"task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","task_collection_sha256":"a".repeat(64),"task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),"task_instructions_sha256":"d".repeat(64),"skill_selection_sha256":"e".repeat(64),"skill_id":null}),
            &json!({"summary":"Continue the workflow."}),
        ).unwrap();
        assert_eq!(built["source"]["stage"], "task");
        assert_eq!(built["target"]["stage"], "execute");
        assert_eq!(built["marker"], "WORK-HANDOFF");
    }
}
