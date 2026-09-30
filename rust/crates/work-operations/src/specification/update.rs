//! Pure shape validation for a reviewed Specification collection update.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::execution::index::build_initial_execution_index;
use crate::execution::{ExecutionIssue, derive_overall_status};

use crate::protocol::valid_sha256;

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
    pub artifact_integrity: bool,
}

fn issue(
    reason_code: &'static str,
    message: &'static str,
    details: Value,
    artifact_integrity: bool,
) -> UpdateIssue {
    UpdateIssue {
        reason_code,
        message,
        details,
        artifact_integrity,
    }
}

fn fields(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    location: &str,
) -> Result<(), UpdateIssue> {
    let object = value.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location":location}),
            false,
        )
    })?;
    let missing = required
        .iter()
        .filter(|key| !object.contains_key(**key))
        .collect::<Vec<_>>();
    let unknown = object
        .keys()
        .filter(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":location,"missing":missing,"unknown":unknown}),
            false,
        ));
    }
    Ok(())
}

pub fn validate_update_request(value: &Value) -> Result<(), UpdateIssue> {
    fields(
        value,
        &[
            "schema",
            "reason",
            "expected",
            "plan",
            "task_index",
            "task_items",
        ],
        &[],
        "spec__update_collection",
    )?;
    if value["schema"] != "work-spec-update-request/v1" {
        return Err(issue(
            "spec_update_schema",
            "Use work-spec-update-request/v1.",
            json!({}),
            true,
        ));
    }
    if !value["reason"].is_string() {
        return Err(issue(
            "invalid_contract_value",
            "A Specification reason is required.",
            json!({"location":"reason"}),
            false,
        ));
    }
    fields(
        &value["expected"],
        &[
            "plan_sha256",
            "task_index_sha256",
            "execution_index_sha256",
            "task_item_sha256",
        ],
        &[],
        "expected",
    )?;
    for key in ["plan_sha256", "task_index_sha256", "execution_index_sha256"] {
        if !value["expected"][key].as_str().is_some_and(valid_sha256) {
            return Err(issue(
                "spec_update_source_changed",
                "The reviewed source fingerprints changed.",
                json!({}),
                true,
            ));
        }
    }
    let item_hashes = value["expected"]["task_item_sha256"].as_object();
    if item_hashes.is_none_or(|hashes| {
        hashes
            .values()
            .any(|hash| !hash.as_str().is_some_and(valid_sha256))
    }) {
        return Err(issue(
            "spec_update_source_changed",
            "The reviewed source fingerprints changed.",
            json!({}),
            true,
        ));
    }
    if !value["plan"].is_object()
        || !value["task_index"].is_object()
        || !value["task_items"].is_object()
    {
        return Err(issue(
            "spec_update_candidate",
            "A complete Plan, TASK index and item map are required.",
            json!({}),
            true,
        ));
    }
    fields(
        &value["plan"],
        &[
            "schema",
            "requirement_id",
            "status",
            "title",
            "summary",
            "artifacts",
            "hierarchy_selection",
            "work_instruction_selection",
            "skill_selection",
            "goals",
            "scope",
            "deliverables",
            "acceptance_criteria",
        ],
        &[
            "constraints",
            "dependencies",
            "risks",
            "milestones",
            "decisions",
            "changes",
        ],
        "plan",
    )?;
    fields(
        &value["task_index"],
        &[
            "schema",
            "requirement_id",
            "spec_id",
            "status",
            "title",
            "summary",
            "artifacts",
            "source_plan",
            "instruction_selection",
            "tasks",
            "readiness",
        ],
        &["execution_defaults", "decisions", "changes"],
        "task_index",
    )?;
    for (task_id, item) in value["task_items"].as_object().expect("checked item map") {
        fields(
            item,
            &[
                "schema",
                "id",
                "title",
                "skill_id",
                "instruction_selection",
                "traceability",
                "goal",
                "steps",
                "validations",
            ],
            &[
                "dependencies",
                "inputs",
                "decisions",
                "files",
                "risks",
                "commands",
                "operations",
            ],
            &format!("task_items.{task_id}"),
        )?;
    }
    Ok(())
}

pub fn content_fingerprints(contents: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, String> {
    contents
        .iter()
        .map(|(path, raw)| (path.clone(), sha256_hex(raw)))
        .collect()
}

pub fn rebuild_execution_index(
    old: &Value,
    logical: &Value,
    validation: &Value,
    affected: &[String],
    change_id: &str,
) -> Result<Value, ExecutionIssue> {
    let mut result = build_initial_execution_index(logical, validation)?;
    let old_rows = old["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["id"].as_str()?.to_owned(), row.clone())))
        .collect::<BTreeMap<_, _>>();
    let affected = affected.iter().collect::<BTreeSet<_>>();
    let rows = result["tasks"].as_array_mut().expect("initial index tasks");
    for fresh in rows.iter_mut() {
        let id = fresh["id"].as_str().unwrap_or("").to_owned();
        if let Some(mut prior) = old_rows.get(&id).cloned() {
            for field in ["skill_id", "task_item_sha256", "instructions_sha256"] {
                prior[field] = fresh[field].clone();
            }
            if affected.contains(&id) && prior["status"] != "pending" {
                if prior["status"] == "cancelled" {
                    return Err(ExecutionIssue {
                        reason_code: "spec_update_cancelled_task",
                        message: "A cancelled TASK requires an explicit lifecycle decision.",
                        details: json!({}),
                    });
                }
                prior["status"] = json!(if prior.get("latest_attempt").is_some() {
                    "pending_retry"
                } else {
                    "blocked"
                });
                prior["status_reason"] = json!({"kind":"task_change","ref":change_id});
            }
            *fresh = prior;
        }
    }
    let statuses = rows
        .iter()
        .map(|row| row["status"].as_str().unwrap_or("pending"))
        .collect::<Vec<_>>();
    result["overall_status"] = json!(derive_overall_status(&statuses));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        work_model::contract_data::registry_value()
    }

    #[test]
    fn update_contract_preserves_null_and_rejects_invalid_hash_and_nested_fields() {
        let registry = example();
        let value =
            registry["items"]["work-spec-update-request/v1"]["description"]["example"].clone();
        validate_update_request(&value).unwrap();
        let mut nullable = value.clone();
        nullable["task_index"]["decisions"] = Value::Null;
        validate_update_request(&nullable).unwrap();
        let round_trip: Value =
            serde_json::from_slice(&crate::canonical::canonical_json(&nullable).unwrap()).unwrap();
        assert_eq!(round_trip, nullable);
        for invalid in [
            json!("A".repeat(64)),
            json!("0".repeat(63)),
            json!(123),
            Value::Null,
        ] {
            let mut changed = value.clone();
            changed["expected"]["task_item_sha256"]["TASK-001"] = invalid;
            assert_eq!(
                validate_update_request(&changed).unwrap_err().reason_code,
                "spec_update_source_changed"
            );
        }
        let mut changed = value.clone();
        changed["task_items"]["TASK-001"]["unexpected"] = json!(true);
        let error = validate_update_request(&changed).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["location"], "task_items.TASK-001");
    }

    #[test]
    fn old_schema_and_single_file_request_are_rejected_before_io() {
        let registry = example();
        let value =
            registry["items"]["work-spec-update-request/v1"]["description"]["example"].clone();
        let mut old = value.clone();
        old["schema"] = json!("work-spec-update-request/v2");
        assert_eq!(
            validate_update_request(&old).unwrap_err().reason_code,
            "spec_update_schema"
        );
        let mut single = value;
        single["task"] = single
            .as_object_mut()
            .unwrap()
            .remove("task_index")
            .unwrap();
        assert_eq!(
            validate_update_request(&single).unwrap_err().reason_code,
            "invalid_object_fields"
        );
    }
    #[test]
    fn rebuild_preserves_history_and_requires_cancelled_decision() {
        let collection = json!({"requirement_id":"example","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"},{"id":"TASK-002"}]});
        let validation = json!({"task_skill_ids":{"TASK-001":"new","TASK-002":null},
            "task_item_sha256":{"TASK-001":"a".repeat(64),"TASK-002":"1".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"b".repeat(64),"TASK-002":"2".repeat(64)},
            "task_collection_sha256":"c".repeat(64),"task_index_sha256":"d".repeat(64),
            "instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"skill_selection_sha256":"0".repeat(64)});
        let mut old = build_initial_execution_index(&collection, &validation).unwrap();
        old["tasks"][0]["status"] = json!("completed");
        old["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        old["tasks"][0]["skill_id"] = json!("old");
        old["tasks"][1]["status"] = json!("blocked");
        old["tasks"][1]["skill_id"] = json!("old");
        let rebuilt = rebuild_execution_index(
            &old,
            &collection,
            &validation,
            &["TASK-001".into(), "TASK-002".into()],
            "TASK-CHANGE-ABC",
        )
        .unwrap();
        assert_eq!(rebuilt["tasks"][0]["status"], "pending_retry");
        assert_eq!(rebuilt["tasks"][0]["latest_attempt"], "ATTEMPT-001");
        assert_eq!(rebuilt["tasks"][0]["skill_id"], "new");
        assert_eq!(rebuilt["tasks"][0]["task_item_sha256"], "a".repeat(64));
        assert_eq!(rebuilt["tasks"][1]["status"], "blocked");
        assert_eq!(rebuilt["tasks"][1]["task_item_sha256"], "1".repeat(64));
        assert_eq!(
            rebuilt["tasks"][0]["status_reason"],
            json!({"kind":"task_change","ref":"TASK-CHANGE-ABC"})
        );
        assert_eq!(
            rebuilt["tasks"][1]["status_reason"],
            json!({"kind":"task_change","ref":"TASK-CHANGE-ABC"})
        );
        old["tasks"][0]["status"] = json!("cancelled");
        assert_eq!(
            rebuild_execution_index(
                &old,
                &collection,
                &validation,
                &["TASK-001".into()],
                "TASK-CHANGE-ABC"
            )
            .unwrap_err()
            .reason_code,
            "spec_update_cancelled_task"
        );
    }
}
