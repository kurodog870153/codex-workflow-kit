//! Handoff validation against the selected project's artifact paths.

use std::collections::BTreeSet;

use serde_json::{Value, json};

pub enum HandoffStorageAction<'a> {
    TaskToExecute {
        verify: bool,
        task_path: &'a str,
        task_id: &'a str,
    },
    ExecuteReturn {
        verify: bool,
        preflight: bool,
        direction: &'a str,
        task_path: &'a str,
        task_id: &'a str,
        attempt_id: Option<&'a str>,
    },
}

pub trait HandoffCommandRepository {
    fn execute(
        &self,
        action: HandoffStorageAction<'_>,
        request: &Value,
    ) -> Result<Value, crate::error::WorkError>;
}
use work_operations::handoff::{
    HandoffIssue, build_discussion_handoff, build_formal_handoff, require_known_affected_ids,
    require_matching_source, validate_handoff_structure,
};
use work_operations::identifiers::RequirementId;

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};

fn domain(error: HandoffIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        error.reason_code,
        error.message,
        error.details,
    )
}

pub fn validate_return_direction(
    incoming: &Value,
    direction: &str,
    execute_return: bool,
) -> Result<(), WorkError> {
    if (execute_return && direction != "execute_to_task") || incoming["direction"] != direction {
        return Err(WorkError::new(
            ExitCode::Contract,
            "handoff_direction_mismatch",
            "The incoming return direction does not match this receiver.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn validate_closed_return_index(
    validation: &Value,
    index: &Value,
    task_id: &str,
    attempt_id: &str,
) -> Result<(), WorkError> {
    if validation["task_skill_ids"].get(task_id).is_none() {
        return Err(WorkError::new(
            ExitCode::Contract,
            "handoff_task_not_found",
            "The explicitly selected TASK does not exist in the formal specification.",
            json!({"task_id":task_id}),
        ));
    }
    let row = index["tasks"]
        .as_array()
        .expect("validated execution index")
        .iter()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::WorkflowState,
                "record_begin_task_not_found",
                "The requested TASK is not present in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    if index.get("lock").is_some() || row["latest_attempt"] != attempt_id {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "handoff_attempt_not_current",
            "The selected Attempt must be latest and the execution index unlocked.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn validate_closed_return_attempt(
    attempt: &Value,
    task_id: &str,
    attempt_id: &str,
) -> Result<(), WorkError> {
    if attempt["task_id"] != task_id || attempt["attempt_id"] != attempt_id {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "attempt_path_identity",
            "The Attempt path does not match its identity.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn closed_return_instruction_inputs(
    validation: &Value,
    attempt: &Value,
    task_id: &str,
) -> (Vec<String>, Vec<String>) {
    let task = validation["collection_contract"]["tasks"]
        .as_array()
        .expect("validated TASKs")
        .iter()
        .find(|row| row["id"] == task_id);
    let selected_paths = task
        .and_then(|row| row["instruction_selection"]["selected_paths"].as_array())
        .map(|paths| {
            paths
                .iter()
                .filter_map(|path| path.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut references = vec!["execute.general.execution-records".to_owned()];
    if attempt.get("continued_from").is_some() {
        references.push("execute.general.execution-recovery".into());
    }
    (selected_paths, references)
}

pub fn return_request(incoming: &Value) -> Value {
    json!({"summary":incoming["summary"],
        "confirmed_approach":incoming["confirmed_approach"],
        "requested_changes":incoming["requested_changes"],"preserve":incoming["preserve"],
        "affected_ids":incoming["affected_ids"],
        "validation_requirements":incoming["validation_requirements"],
        "reason":incoming["source"]["execution_context"]["reason"]})
}

pub fn build_discussion(request: &Value) -> Result<Value, WorkError> {
    build_discussion_handoff(request).map_err(domain)
}

pub fn validate_task_to_execute_request(request: &Value) -> Result<(), WorkError> {
    let fields = request.as_object().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "expected_object",
            "A JSON object is required.",
            json!({"location":"handoff_request"}),
        )
    })?;
    let missing = (!fields.contains_key("summary"))
        .then_some("summary")
        .into_iter()
        .collect::<Vec<_>>();
    let unknown: Vec<_> = fields
        .keys()
        .filter(|field| field.as_str() != "summary")
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"handoff_request","missing":missing,"unknown":unknown}),
        ));
    }
    Ok(())
}

pub fn build_task_to_execute(
    repo: &impl ArtifactPathRepository,
    validation: &Value,
    task_id: &str,
    request: &Value,
) -> Result<Value, WorkError> {
    validate_task_to_execute_request(request)?;
    let skill_id = validation["task_skill_ids"].get(task_id).ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "handoff_task_not_found",
            "The explicitly selected TASK does not exist in the formal specification.",
            json!({"task_id":task_id}),
        )
    })?;
    let collection = &validation["collection_contract"];
    build_task_formal(
        repo,
        "task_to_execute",
        validation["requirement_id"]
            .as_str()
            .expect("validated requirement"),
        &collection["artifacts"],
        &json!({"task_spec_id":validation["spec_id"],"task_id":task_id,
            "task_collection_sha256":validation["task_collection_sha256"],
            "task_index_sha256":validation["task_index_sha256"],
            "task_item_sha256":validation["task_item_sha256"][task_id],
            "task_instructions_sha256":validation["task_instructions_sha256"][task_id],
            "skill_id":skill_id,"skill_selection_sha256":validation["skill_selection_sha256"]}),
        request,
    )
}

pub fn verify_task_to_execute(
    repo: &impl ArtifactPathRepository,
    validation: &Value,
    task_id: &str,
    incoming: &Value,
) -> Result<Value, WorkError> {
    let mut result = validate_task_handoff(repo, incoming)?;
    if incoming["direction"] != "task_to_execute" {
        return Err(WorkError::new(
            ExitCode::Contract,
            "handoff_direction_mismatch",
            "This receiver requires a Task-to-Execute handoff.",
            json!({}),
        ));
    }
    let expected = build_task_to_execute(
        repo,
        validation,
        task_id,
        &json!({"summary":incoming["summary"]}),
    )?;
    require_matching_source(incoming, &expected).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    result["schema"] = json!("work-handoff-source-validation/v1");
    result["task_path"] = expected["artifacts"]["task"].clone();
    result["source"] = expected["source"].clone();
    let _: work_model::handoff::HandoffSourceValidation = serde_json::from_value(result.clone())
        .expect("handoff source validation matches its model");
    Ok(result)
}

pub fn validate_preflight_return_request(
    request: &Value,
    direction: &str,
) -> Result<(), WorkError> {
    if direction != "execute_to_task" {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_handoff_direction",
            "An Execute return direction is required.",
            json!({}),
        ));
    }
    let fields = request.as_object().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "expected_object",
            "A JSON object is required.",
            json!({"location":"handoff_request"}),
        )
    })?;
    let required = [
        "summary",
        "reason",
        "confirmed_approach",
        "requested_changes",
        "preserve",
        "affected_ids",
        "validation_requirements",
    ];
    let missing: Vec<_> = required
        .iter()
        .filter(|field| !fields.contains_key(**field))
        .collect();
    let unknown: Vec<_> = fields
        .keys()
        .filter(|field| !required.contains(&field.as_str()))
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"handoff_request","missing":missing,"unknown":unknown}),
        ));
    }
    Ok(())
}

pub fn build_preflight_return(
    repo: &impl ArtifactPathRepository,
    validation: &Value,
    index: &Value,
    direction: &str,
    task_id: &str,
    request: &Value,
) -> Result<Value, WorkError> {
    validate_preflight_return_request(request, direction)?;
    let skill_id = validation["task_skill_ids"].get(task_id).ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "handoff_task_not_found",
            "The explicitly selected TASK does not exist in the formal specification.",
            json!({"task_id":task_id}),
        )
    })?;
    let collection = &validation["collection_contract"];
    let expected_identity = json!({"requirement_id":validation["requirement_id"],
        "task_spec_id":validation["spec_id"],
        "task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_instructions_sha256":validation["instructions_sha256"],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"]});
    let changed = expected_identity
        .as_object()
        .expect("identity object")
        .iter()
        .any(|(field, value)| index.get(field) != Some(value));
    if changed {
        let observed_identity = expected_identity
            .as_object()
            .expect("identity object")
            .keys()
            .map(|field| (field.clone(), index[field].clone()))
            .collect::<serde_json::Map<_, _>>();
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            json!({"expected":expected_identity,"observed":observed_identity}),
        ));
    }
    let rows = index["tasks"]
        .as_array()
        .expect("validated execution index");
    let observed: Vec<_> = rows.iter().map(|row| row["id"].clone()).collect();
    if observed
        != validation["task_ids"]
            .as_array()
            .expect("validated TASK IDs")
            .to_vec()
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK document.",
            json!({"expected":validation["task_ids"],"observed":observed}),
        ));
    }
    let mut mismatches = serde_json::Map::new();
    for row in rows {
        let id = row["id"].as_str().expect("validated TASK ID");
        if row["instructions_sha256"] != validation["task_instructions_sha256"][id] {
            mismatches.insert(
                id.into(),
                json!({"expected":validation["task_instructions_sha256"][id],
                "observed":row["instructions_sha256"]}),
            );
        }
    }
    if !mismatches.is_empty() {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "execute_preflight_task_instructions_mismatch",
            "The execution index per-TASK instruction fingerprints are stale.",
            json!({"tasks":mismatches}),
        ));
    }
    let row = rows
        .iter()
        .find(|row| row["id"] == task_id)
        .expect("validated TASK set");
    if index.get("lock").is_some()
        || row["status"] != "pending"
        || row.get("latest_attempt").is_some()
    {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "handoff_task_already_started",
            "Preflight return requires an unlocked, initial pending TASK without Attempt history.",
            json!({}),
        ));
    }
    if row["skill_id"] != *skill_id
        || index["skill_selection_sha256"] != validation["skill_selection_sha256"]
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "handoff_skill_identity_mismatch",
            "Execution skill bindings or fingerprints do not match the validated source.",
            json!({}),
        ));
    }
    let skills: Vec<Value> = collection["skill_selection"]["skills"]
        .as_array()
        .expect("validated Plan skills")
        .iter()
        .filter(|skill| skill["id"] == *skill_id)
        .cloned()
        .collect();
    let fingerprint = work_operations::derivation::fingerprint::skill_selection(
        if skill_id.is_null() {
            "base_only"
        } else {
            "external_skills"
        },
        &skills,
    );
    let payload = json!({"summary":request["summary"],
        "confirmed_approach":request["confirmed_approach"],
        "requested_changes":request["requested_changes"],"preserve":request["preserve"],
        "affected_ids":request["affected_ids"],
        "validation_requirements":request["validation_requirements"]});
    let handoff = build_task_formal(
        repo,
        direction,
        validation["requirement_id"]
            .as_str()
            .expect("validated requirement"),
        &collection["artifacts"],
        &json!({"task_spec_id":validation["spec_id"],"task_id":task_id,
            "task_collection_sha256":validation["task_collection_sha256"],
            "task_index_sha256":validation["task_index_sha256"],
            "task_item_sha256":validation["task_item_sha256"][task_id],
            "task_instructions_sha256":validation["task_instructions_sha256"][task_id],
            "skill_id":skill_id,"execute_skill_selection_sha256":fingerprint,
            "execution_context":{"attempt":{"status":"not_created"},"phase":"preflight",
                "issue_type":"specification_defect","reason":request["reason"]}}),
        &payload,
    )?;
    let mut known: BTreeSet<String> = [
        "goals",
        "scope",
        "constraints",
        "dependencies",
        "risks",
        "milestones",
        "deliverables",
        "acceptance_criteria",
        "decisions",
        "changes",
    ]
    .into_iter()
    .flat_map(|group| {
        collection[group]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
    })
    .collect();
    for task in collection["tasks"].as_array().expect("validated TASKs") {
        known.insert(task["id"].as_str().expect("validated TASK ID").into());
        if task["id"] == task_id {
            for group in [
                "steps",
                "validations",
                "commands",
                "operations",
                "files",
                "inputs",
                "decisions",
                "risks",
                "acceptance_criteria",
            ] {
                for item in task[group].as_array().into_iter().flatten() {
                    if let Some(id) = item["id"].as_str() {
                        known.insert(id.into());
                    }
                }
            }
        }
    }
    for decision in collection["decisions"].as_array().into_iter().flatten() {
        if let Some(id) = decision["id"].as_str() {
            known.insert(id.into());
        }
    }
    let affected: Vec<String> = handoff["affected_ids"]
        .as_array()
        .expect("validated IDs")
        .iter()
        .map(|value| value.as_str().expect("validated ID").to_owned())
        .collect();
    require_known_affected_ids(&affected, &known, "handoff_unknown_affected_ids")
        .map_err(domain)?;
    Ok(handoff)
}

pub struct ClosedReturnInput<'a> {
    pub validation: &'a Value,
    pub index: &'a Value,
    pub attempt: &'a Value,
    pub attempt_raw: &'a [u8],
    pub current_execute_selection: &'a Value,
    pub direction: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub request: &'a Value,
}

pub fn build_closed_return(
    repo: &impl ArtifactPathRepository,
    input: &ClosedReturnInput<'_>,
) -> Result<Value, WorkError> {
    validate_preflight_return_request(input.request, input.direction)?;
    let validation = input.validation;
    let task_id = input.task_id;
    let skill_id = validation["task_skill_ids"].get(task_id).ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "handoff_task_not_found",
            "The explicitly selected TASK does not exist in the formal specification.",
            json!({"task_id":task_id}),
        )
    })?;
    let row = input.index["tasks"]
        .as_array()
        .expect("validated execution index")
        .iter()
        .find(|row| row["id"] == task_id)
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::WorkflowState,
                "record_begin_task_not_found",
                "The requested TASK is not present in the execution index.",
                json!({"task_id":task_id}),
            )
        })?;
    if input.index.get("lock").is_some() || row["latest_attempt"] != input.attempt_id {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "handoff_attempt_not_current",
            "The selected Attempt must be latest and the execution index unlocked.",
            json!({}),
        ));
    }
    let attempt = input.attempt;
    if !matches!(attempt["status"].as_str(), Some("stopped" | "blocked")) {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "handoff_attempt_not_closed",
            "A stopped or blocked Attempt is required.",
            json!({}),
        ));
    }
    let expected_index = json!({"requirement_id":validation["requirement_id"],
        "task_spec_id":validation["spec_id"],
        "task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_instructions_sha256":validation["instructions_sha256"],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"]});
    let actual_index = expected_index
        .as_object()
        .expect("index identity")
        .keys()
        .map(|field| (field.clone(), input.index[field].clone()))
        .collect::<serde_json::Map<_, _>>();
    if Value::Object(actual_index.clone()) != expected_index {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "record_begin_index_identity_mismatch",
            "The execution index does not match the formal TASK identity.",
            json!({"expected":expected_index,"actual":actual_index}),
        ));
    }
    let observed_ids: Vec<_> = input.index["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .map(|row| row["id"].clone())
        .collect();
    if observed_ids
        != validation["task_ids"]
            .as_array()
            .expect("validated TASK IDs")
            .to_vec()
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "record_begin_index_task_set_mismatch",
            "The execution index TASK set does not match the formal TASK.",
            json!({}),
        ));
    }
    if row["instructions_sha256"] != validation["task_instructions_sha256"][task_id] {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "record_begin_task_instructions_mismatch",
            "The target TASK instruction fingerprint is stale.",
            json!({}),
        ));
    }
    let expected_attempt = json!({"task_spec_id":validation["spec_id"],
        "task_id":task_id,"task_collection_sha256":validation["task_collection_sha256"],
        "task_index_sha256":validation["task_index_sha256"],
        "task_item_sha256":validation["task_item_sha256"][task_id],
        "task_instructions_sha256":validation["task_instructions_sha256"][task_id],
        "hierarchy_selection_sha256":validation["hierarchy_selection_sha256"]});
    let actual_attempt = expected_attempt
        .as_object()
        .expect("attempt identity")
        .keys()
        .map(|field| (field.clone(), attempt[field].clone()))
        .collect::<serde_json::Map<_, _>>();
    if Value::Object(actual_attempt.clone()) != expected_attempt {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "record_begin_attempt_identity_mismatch",
            "The active Attempt does not match the formal TASK identity.",
            json!({"expected":expected_attempt,"actual":actual_attempt}),
        ));
    }
    let blocked_type = matches!(
        attempt["final_type"].as_str(),
        Some("external_operation_failed" | "instructions_changed" | "specification_defect")
    );
    let expected_status = if attempt["status"] == "blocked" || blocked_type {
        "blocked"
    } else {
        "pending_retry"
    };
    if attempt["attempt_id"] != input.attempt_id
        || row["status"] != expected_status
        || row["status_reason"] != json!({"kind":"attempt","ref":input.attempt_id})
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "handoff_attempt_state_mismatch",
            "The index does not describe the selected closed Attempt.",
            json!({}),
        ));
    }
    let task = validation["collection_contract"]["tasks"]
        .as_array()
        .expect("validated TASKs")
        .iter()
        .find(|row| row["id"] == task_id)
        .expect("validated TASK ID");
    let current = input.current_execute_selection;
    if current["selected_paths"] != task["instruction_selection"]["selected_paths"]
        || current["resolved_paths"] != task["instruction_selection"]["resolved_paths"]
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "handoff_execute_instruction_hierarchy_mismatch",
            "The current Execute hierarchy does not match the target TASK.",
            json!({}),
        ));
    }
    if current["instructions_sha256"] != attempt["execute_instructions_sha256"] {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "handoff_execute_instructions_changed",
            "The Execute instruction fingerprint changed after Attempt start.",
            json!({"expected":attempt["execute_instructions_sha256"],
                "actual":current["instructions_sha256"]}),
        ));
    }
    let skills: Vec<Value> = validation["collection_contract"]["skill_selection"]["skills"]
        .as_array()
        .expect("validated skills")
        .iter()
        .filter(|row| row["id"] == *skill_id)
        .cloned()
        .collect();
    let fingerprint = work_operations::derivation::fingerprint::skill_selection(
        if skill_id.is_null() {
            "base_only"
        } else {
            "external_skills"
        },
        &skills,
    );
    if attempt["skill_id"] != *skill_id
        || row["skill_id"] != *skill_id
        || input.index["skill_selection_sha256"] != validation["skill_selection_sha256"]
        || attempt["execute_skill_selection_sha256"] != fingerprint
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "handoff_skill_identity_mismatch",
            "Execution skill bindings or fingerprints do not match the validated source.",
            json!({}),
        ));
    }
    let collection = &validation["collection_contract"];
    let request = input.request;
    let payload = json!({"summary":request["summary"],
        "confirmed_approach":request["confirmed_approach"],
        "requested_changes":request["requested_changes"],"preserve":request["preserve"],
        "affected_ids":request["affected_ids"],
        "validation_requirements":request["validation_requirements"]});
    let handoff = build_task_formal(
        repo,
        input.direction,
        validation["requirement_id"]
            .as_str()
            .expect("validated requirement"),
        &collection["artifacts"],
        &json!({"task_spec_id":validation["spec_id"],"task_id":task_id,
            "task_collection_sha256":validation["task_collection_sha256"],
            "task_index_sha256":validation["task_index_sha256"],
            "task_item_sha256":validation["task_item_sha256"][task_id],
            "task_instructions_sha256":validation["task_instructions_sha256"][task_id],
            "skill_id":skill_id,"execute_skill_selection_sha256":fingerprint,
            "execution_context":{"attempt":{"status":attempt["status"],"id":input.attempt_id},
                "phase":"execution","issue_type":"specification_defect","reason":request["reason"]},
            "attempt_sha256":work_operations::derivation::fingerprint::raw(input.attempt_raw)}),
        &payload,
    )?;
    let mut known: BTreeSet<String> = [
        "goals",
        "scope",
        "constraints",
        "dependencies",
        "risks",
        "milestones",
        "deliverables",
        "acceptance_criteria",
        "decisions",
        "changes",
    ]
    .into_iter()
    .flat_map(|group| {
        collection[group]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["id"].as_str().map(str::to_owned))
    })
    .collect();
    for task in collection["tasks"].as_array().expect("validated TASKs") {
        known.insert(task["id"].as_str().expect("validated TASK ID").into());
        if task["id"] == task_id {
            for group in [
                "steps",
                "validations",
                "commands",
                "operations",
                "files",
                "inputs",
                "decisions",
                "risks",
                "acceptance_criteria",
            ] {
                for item in task[group].as_array().into_iter().flatten() {
                    if let Some(id) = item["id"].as_str() {
                        known.insert(id.into());
                    }
                }
            }
        }
    }
    for decision in collection["decisions"].as_array().into_iter().flatten() {
        if let Some(id) = decision["id"].as_str() {
            known.insert(id.into());
        }
    }
    let affected: Vec<String> = handoff["affected_ids"]
        .as_array()
        .expect("validated IDs")
        .iter()
        .map(|value| value.as_str().expect("validated ID").to_owned())
        .collect();
    require_known_affected_ids(&affected, &known, "handoff_unknown_affected_ids")
        .map_err(domain)?;
    Ok(handoff)
}

pub fn verify_return_against_expected(
    repo: &impl ArtifactPathRepository,
    incoming: &Value,
    expected: &Value,
    direction: &str,
) -> Result<Value, WorkError> {
    let mut result = validate_task_handoff(repo, incoming)?;
    if direction != "execute_to_task" || incoming["direction"] != direction {
        return Err(WorkError::new(
            ExitCode::Contract,
            "handoff_direction_mismatch",
            "The incoming return direction does not match this receiver.",
            json!({}),
        ));
    }
    require_matching_source(incoming, expected).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    result["schema"] = json!("work-handoff-source-validation/v1");
    result["task_path"] = expected["artifacts"]["task"].clone();
    result["source"] = expected["source"].clone();
    let _: work_model::handoff::HandoffSourceValidation = serde_json::from_value(result.clone())
        .expect("handoff source validation matches its model");
    Ok(result)
}

fn build_task_formal(
    repo: &impl ArtifactPathRepository,
    direction: &str,
    requirement: &str,
    artifacts: &Value,
    source: &Value,
    payload: &Value,
) -> Result<Value, WorkError> {
    let handoff =
        build_formal_handoff(direction, requirement, artifacts, source, payload).map_err(domain)?;
    validate_task_handoff(repo, &handoff)?;
    Ok(handoff)
}
pub fn validate_task_handoff(
    repo: &impl ArtifactPathRepository,
    contract: &Value,
) -> Result<Value, WorkError> {
    let result = validate_handoff_structure(contract).map_err(domain)?;
    for field in ["source", "task", "execution"] {
        if let Some(path) = contract["artifacts"][field].as_str() {
            if path.is_empty()
                || path.starts_with('/')
                || path.contains('\\')
                || path.split('/').any(|part| matches!(part, "" | "." | ".."))
            {
                return Err(WorkError::new(
                    ExitCode::Contract,
                    "noncanonical_artifact_paths",
                    "Handoff artifact paths must use their normalized project-relative form.",
                    json!({"field":field}),
                ));
            }
        }
    }
    let requirement = contract["requirement_id"]
        .as_str()
        .expect("validated requirement");
    let artifacts = serde_json::from_value(contract["artifacts"].clone()).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_artifact_paths",
            "Task handoffs require Source, Task and Execution roots.",
            json!({}),
        )
    })?;
    work_operations::task::source::validate_artifacts(&artifacts, requirement).map_err(
        |error| {
            WorkError::new(
                ExitCode::Contract,
                error.reason_code,
                error.message,
                error.details,
            )
        },
    )?;
    repo.validate_paths(
        &requirement
            .parse::<RequirementId>()
            .expect("validated requirement"),
        &artifacts,
    )?;
    Ok(result)
}
