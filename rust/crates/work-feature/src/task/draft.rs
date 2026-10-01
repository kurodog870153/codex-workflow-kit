//! Planning progress and draft source checks without filesystem state.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_model::task::draft::{TaskDraft, TaskPlanningIndex};
use work_operations::canonical::canonical_json;
use work_operations::derivation::fingerprint;
use work_operations::protocol::PLANNING_STATUSES;
use work_operations::task::TaskIssue;
use work_operations::task::candidate::validate_semantic_candidate;
use work_operations::task::draft::{
    validate_draft_instruction_selection, validate_planning_index, validate_task_draft,
};
use work_operations::task::draft_list::{affected_historical_draft_ids, prepare_list_change};
use work_operations::task::draft_source::{prepare_source_change, source_change_effects};

use crate::error::{ExitCode, WorkError};
use crate::hierarchy::validate_task_paths;
use crate::instruction::{InstructionSourceRepository, load};

fn domain(issue: TaskIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        issue.reason_code,
        issue.message,
        issue.details,
    )
}

fn canonical_planning_index(value: &Value) -> Vec<u8> {
    let index: TaskPlanningIndex =
        serde_json::from_value(value.clone()).expect("validated planning index matches Model");
    canonical_json(&serde_json::to_value(index).expect("planning index serializes"))
        .expect("planning index JSON serializes")
}

fn canonical_task_draft(value: &Value) -> Vec<u8> {
    let draft: TaskDraft =
        serde_json::from_value(value.clone()).expect("validated TASK draft matches Model");
    canonical_json(&serde_json::to_value(draft).expect("TASK draft serializes"))
        .expect("TASK draft JSON serializes")
}

fn source_domain(issue: TaskIssue) -> WorkError {
    let code = if issue.reason_code == "draft_sources_unchanged" {
        ExitCode::WorkflowState
    } else {
        ExitCode::Contract
    };
    WorkError::new(code, issue.reason_code, issue.message, issue.details)
}

fn list_domain(issue: TaskIssue) -> WorkError {
    let code = if matches!(
        issue.reason_code,
        "draft_content_integrity"
            | "draft_list_metadata_changed"
            | "draft_list_unchanged"
            | "draft_revision_conflict"
            | "draft_scope_changed"
            | "draft_selection_mismatch"
            | "draft_task_id_reused"
            | "draft_task_mismatch"
            | "invalid_new_draft_task"
            | "noncanonical_draft_storage"
    ) {
        ExitCode::WorkflowState
    } else {
        ExitCode::Contract
    };
    WorkError::new(code, issue.reason_code, issue.message, issue.details)
}

fn workflow(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::WorkflowState, reason, message, json!({}))
}

pub fn source_update_request(
    raw_request: &Value,
    expected_revision: u64,
) -> Result<String, WorkError> {
    let fields = raw_request.as_object().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        )
    })?;
    if fields.len() != 2 || !fields.contains_key("reason") || !fields.contains_key("selections") {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The source update request has missing or unknown fields.",
            json!({}),
        ));
    }
    let reason = raw_request["reason"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "empty_text_value",
                "A source update reason is required.",
                json!({}),
            )
        })?;
    if expected_revision < 1 {
        return Err(workflow(
            "invalid_expected_revision",
            "An existing planning revision is required.",
        ));
    }
    Ok(reason.to_owned())
}

pub fn source_update_selections(
    raw_request: &Value,
    previous: &Value,
    requirement_id: &str,
    expected_revision: u64,
) -> Result<BTreeMap<String, Value>, WorkError> {
    if previous["revision"].as_u64() != Some(expected_revision)
        || previous["requirement_id"] != requirement_id
    {
        return Err(workflow(
            "draft_revision_conflict",
            "Reload the planning index before updating sources.",
        ));
    }
    let selections: BTreeMap<String, Value> = raw_request["selections"]
        .as_object()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_object_fields",
                "Every active TASK requires an instruction selection.",
                json!({}),
            )
        })?
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let task_ids: BTreeSet<String> = previous["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .filter_map(|task| task["id"].as_str().map(str::to_owned))
        .collect();
    if selections.keys().cloned().collect::<BTreeSet<_>>() != task_ids {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "Every active TASK requires an instruction selection.",
            json!({}),
        ));
    }
    Ok(selections)
}

pub fn require_existing_revision(revision: u64) -> Result<(), WorkError> {
    if revision < 1 {
        return Err(workflow(
            "invalid_expected_revision",
            "An existing planning revision is required.",
        ));
    }
    Ok(())
}

pub fn validate_historical_index(
    index: &Value,
    requirement_id: &str,
    revision: u64,
) -> Result<(), WorkError> {
    validate_planning_index(index).map_err(domain)?;
    if index["requirement_id"] != requirement_id {
        return Err(workflow(
            "draft_requirement_mismatch",
            "The historical index belongs to another requirement.",
        ));
    }
    if index["revision"].as_u64() != Some(revision) {
        return Err(workflow(
            "draft_revision_conflict",
            "The historical index identifies another revision.",
        ));
    }
    Ok(())
}

pub fn validate_current_index(index: &Value, requirement_id: &str) -> Result<u64, WorkError> {
    validate_planning_index(index).map_err(domain)?;
    if index["requirement_id"] != requirement_id {
        return Err(workflow(
            "draft_requirement_mismatch",
            "The stored requirement does not match its directory.",
        ));
    }
    for task in index["tasks"].as_array().expect("validated planning tasks") {
        if task["status"] != "planned" && task.get("draft_ref").is_none() {
            return Err(workflow(
                "missing_stored_draft_reference",
                "A saved discussion requires a version reference.",
            ));
        }
    }
    Ok(index["revision"].as_u64().expect("validated revision"))
}

pub fn validate_current_index_history(raw: &[u8], history: &[u8]) -> Result<(), WorkError> {
    if history != raw {
        return Err(workflow(
            "draft_index_integrity",
            "The current index differs from its immutable history.",
        ));
    }
    Ok(())
}

pub fn draft_reference(index: &Value, task_id: &str) -> Result<(u64, String), WorkError> {
    let entry = index["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .find(|entry| entry["id"] == task_id)
        .filter(|entry| entry.get("draft_ref").is_some())
        .ok_or_else(|| {
            workflow(
                "draft_not_saved",
                "The requested TASK has no committed draft.",
            )
        })?;
    let reference = &entry["draft_ref"];
    Ok((
        reference["save_revision"]
            .as_u64()
            .expect("validated save revision"),
        reference["sha256"]
            .as_str()
            .expect("validated draft fingerprint")
            .to_owned(),
    ))
}

pub fn validate_draft_fingerprint(raw: &[u8], expected_sha256: &str) -> Result<(), WorkError> {
    if !fingerprint::verify_raw(raw, expected_sha256) {
        return Err(workflow(
            "draft_content_integrity",
            "The draft does not match its indexed fingerprint.",
        ));
    }
    Ok(())
}

pub fn validate_stored_draft(draft: &Value, index: &Value, task_id: &str) -> Result<(), WorkError> {
    if draft["task_id"] != task_id {
        return Err(workflow(
            "draft_task_mismatch",
            "The historical draft identifies a different TASK.",
        ));
    }
    validate_task_draft(draft, index).map_err(domain)?;
    Ok(())
}

pub fn source_check_entry<'a>(
    index: &'a Value,
    expected_revision: u64,
    task_id: &str,
) -> Result<&'a Value, WorkError> {
    if index["revision"].as_u64() != Some(expected_revision) {
        return Err(workflow(
            "draft_revision_conflict",
            "Reload the current planning index before checking sources.",
        ));
    }
    index["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .find(|entry| entry["id"] == task_id)
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "draft_task_not_in_index",
                "The selected TASK is absent from the planning index.",
                json!({}),
            )
        })
}

pub fn validate_save_request(request: &Value, expected_revision: u64) -> Result<(), WorkError> {
    let required = [
        "status",
        "notes",
        "confirmed_decisions",
        "tentative",
        "open_questions",
        "next_discussion_point",
    ];
    let fields = request.as_object().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        )
    })?;
    if fields.len() < required.len()
        || fields.len() > required.len() + 1
        || required.iter().any(|key| !fields.contains_key(*key))
        || fields
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "task_candidate")
    {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The draft save request has missing or unknown fields.",
            json!({}),
        ));
    }
    if let Some(candidate) = request.get("task_candidate") {
        validate_semantic_candidate(candidate, request["status"] == "refined").map_err(domain)?;
    }
    require_existing_revision(expected_revision)
}

pub fn validate_save_revision(
    current: &Value,
    expected_revision: u64,
    recover: bool,
) -> Result<u64, WorkError> {
    let revision = current["revision"].as_u64();
    if revision != Some(expected_revision) && !(recover && revision == Some(expected_revision + 1))
    {
        return Err(workflow(
            "draft_revision_conflict",
            "Reload the current planning index before saving or recovering.",
        ));
    }
    Ok(revision.expect("validated planning index"))
}

pub fn save_selection<'a>(
    previous: &'a Value,
    task_id: &str,
    selected_paths: Option<&[String]>,
    reference_names: Option<&[String]>,
) -> Result<(&'a Value, Vec<String>, Vec<String>), WorkError> {
    let entry = previous["tasks"]
        .as_array()
        .expect("validated planning index")
        .iter()
        .find(|entry| entry["id"] == task_id)
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "draft_task_not_in_index",
                "The selected TASK is absent from the planning index.",
                json!({}),
            )
        })?;
    let selected = resolve_instruction_selection(entry, selected_paths, reference_names)?;
    let paths = selected["selected_paths"]
        .as_array()
        .expect("validated selection")
        .iter()
        .map(|value| value.as_str().expect("validated path").to_owned())
        .collect();
    let references = selected["references"]
        .as_array()
        .expect("validated selection")
        .iter()
        .map(|value| value.as_str().expect("validated reference").to_owned())
        .collect();
    Ok((entry, paths, references))
}

pub fn validate_save_sources(
    previous: &Value,
    entry: &Value,
    checked: &Value,
) -> Result<(), WorkError> {
    if checked["source"] != previous["source"]
        || checked["instructions_sha256"] != entry["instructions_sha256"]
        || checked["skill_id"] != entry["skill_id"]
    {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "draft_source_drift",
            "The original save sources no longer match the validated sources.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn build_save_documents(
    previous: &Value,
    entry: &Value,
    checked: &Value,
    request: &Value,
    requirement_id: &str,
    task_id: &str,
    expected_revision: u64,
) -> (Value, Value) {
    let mut proposed = previous.clone();
    proposed["revision"] = json!(expected_revision + 1);
    proposed["current_task_id"] = json!(task_id);
    let target = proposed["tasks"]
        .as_array_mut()
        .expect("validated tasks")
        .iter_mut()
        .find(|row| row["id"] == task_id)
        .expect("selected TASK");
    target["status"] = request["status"].clone();
    target["instruction_selection"] = checked["instruction_selection"].clone();
    let mut draft = json!({"schema":"work-task-draft/v1",
        "requirement_id":requirement_id,"task_id":task_id,
        "revision":entry["draft_ref"]["revision"].as_u64().unwrap_or(0) + 1,
        "boundary_revision":entry["boundary_revision"],"source":previous["source"],
        "instructions_sha256":entry["instructions_sha256"]});
    for (key, value) in request.as_object().expect("validated request") {
        draft[key] = value.clone();
    }
    (proposed, draft)
}

pub trait TaskDraftHistoryRepository {
    fn read_historical_draft(
        &self,
        requirement_id: &str,
        save_revision: u64,
        task_id: &str,
    ) -> Result<Vec<u8>, WorkError>;
}

#[derive(Debug, Clone)]
pub struct PreparedListUpdate {
    pub index: Value,
    pub files: BTreeMap<String, Vec<u8>>,
    pub affected_task_ids: Vec<String>,
    pub revision: u64,
}

pub fn prepare_list_update(
    history: &impl TaskDraftHistoryRepository,
    previous: &Value,
    proposed: &Value,
    expected_revision: u64,
    reason: &str,
) -> Result<PreparedListUpdate, WorkError> {
    validate_planning_index(previous).map_err(domain)?;
    if expected_revision < 1 {
        return Err(workflow(
            "invalid_expected_revision",
            "A list update requires an existing index revision.",
        ));
    }
    if previous["revision"].as_u64() != Some(expected_revision) {
        return Err(workflow(
            "draft_revision_conflict",
            "Reload the planning index before updating its list.",
        ));
    }
    let requirement_id = previous["requirement_id"]
        .as_str()
        .expect("validated requirement");
    let affected_drafts = affected_historical_draft_ids(previous, proposed).map_err(domain)?;
    let mut drafts = BTreeMap::new();
    for task in previous["tasks"].as_array().expect("validated tasks") {
        if let Some(reference) = task.get("draft_ref") {
            let task_id = task["id"].as_str().expect("validated ID");
            if !affected_drafts.contains(task_id) {
                continue;
            }
            let revision = reference["save_revision"]
                .as_u64()
                .expect("validated revision");
            drafts.insert(
                task_id.into(),
                history.read_historical_draft(requirement_id, revision, task_id)?,
            );
        }
    }
    let prepared = prepare_list_change(previous, proposed, reason, &drafts).map_err(list_domain)?;
    let revision = prepared.index["revision"]
        .as_u64()
        .expect("validated revision");
    let raw = canonical_planning_index(&prepared.index);
    let request = canonical_json(
        &json!({"index":proposed,"reason":reason,"expected_revision":expected_revision}),
    )
    .expect("JSON value serializes");
    let mut files = BTreeMap::from([
        ("index.json".into(), raw.clone()),
        ("index-current.tmp".into(), raw),
        ("list-update.json".into(), request),
    ]);
    for (task_id, raw) in prepared.drafts {
        files.insert(format!("{task_id}.json"), raw);
    }
    Ok(PreparedListUpdate {
        index: prepared.index,
        files,
        affected_task_ids: prepared.affected_task_ids,
        revision,
    })
}

pub struct SourceUpdateRequest<'a> {
    pub new_source: &'a Value,
    pub selections: &'a BTreeMap<String, Value>,
    pub instruction_hashes: &'a BTreeMap<String, String>,
    pub expected_revision: u64,
    pub reason: &'a str,
    pub plan_path: &'a str,
}

pub fn prepare_source_update(
    history: &impl TaskDraftHistoryRepository,
    previous: &Value,
    request: &SourceUpdateRequest<'_>,
) -> Result<PreparedListUpdate, WorkError> {
    let new_source = request.new_source;
    let selections = request.selections;
    let instruction_hashes = request.instruction_hashes;
    let expected_revision = request.expected_revision;
    let reason = request.reason;
    let plan_path = request.plan_path;
    validate_planning_index(previous).map_err(domain)?;
    if expected_revision < 1 {
        return Err(workflow(
            "invalid_expected_revision",
            "An existing planning revision is required.",
        ));
    }
    if previous["revision"].as_u64() != Some(expected_revision) {
        return Err(workflow(
            "draft_revision_conflict",
            "Reload the planning index before updating sources.",
        ));
    }
    let effects =
        source_change_effects(previous, new_source, selections, instruction_hashes, reason)
            .map_err(source_domain)?;
    let requirement_id = previous["requirement_id"]
        .as_str()
        .expect("validated requirement");
    let mut drafts = BTreeMap::new();
    for row in previous["tasks"].as_array().expect("validated tasks") {
        let task_id = row["id"].as_str().expect("validated ID");
        if !effects.historical_drafts.contains(task_id) {
            continue;
        }
        let revision = row["draft_ref"]["save_revision"]
            .as_u64()
            .expect("validated revision");
        drafts.insert(
            task_id.into(),
            history.read_historical_draft(requirement_id, revision, task_id)?,
        );
    }
    let prepared = prepare_source_change(
        previous,
        new_source,
        selections,
        instruction_hashes,
        reason,
        &drafts,
    )
    .map_err(source_domain)?;
    let revision = prepared.index["revision"]
        .as_u64()
        .expect("validated revision");
    let raw = canonical_planning_index(&prepared.index);
    let request = canonical_json(&json!({"request":{"reason":reason,"selections":selections},"expected_revision":expected_revision,"plan_path":plan_path})).expect("JSON serializes");
    let mut files = BTreeMap::from([
        ("index.json".into(), raw.clone()),
        ("index-current.tmp".into(), raw),
        ("source-update.json".into(), request),
    ]);
    for (task_id, raw) in prepared.drafts {
        files.insert(format!("{task_id}.json"), raw);
    }
    Ok(PreparedListUpdate {
        index: prepared.index,
        files,
        affected_task_ids: prepared.affected_task_ids,
        revision,
    })
}

pub struct ValidatedSourceRefresh<'a> {
    pub previous: &'a Value,
    pub plan: &'a Value,
    pub plan_validation: &'a Value,
    pub selections: &'a BTreeMap<String, Value>,
    pub expected_revision: u64,
    pub reason: &'a str,
    pub plan_path: &'a str,
}

pub fn prepare_validated_source_refresh(
    instructions: &impl InstructionSourceRepository,
    history: &impl TaskDraftHistoryRepository,
    input: &ValidatedSourceRefresh<'_>,
) -> Result<PreparedListUpdate, WorkError> {
    validate_planning_index(input.previous).map_err(domain)?;
    let requirement_id = input.previous["requirement_id"]
        .as_str()
        .expect("validated requirement");
    if input.plan_validation["requirement_id"] != requirement_id {
        return Err(workflow(
            "draft_source_requirement_mismatch",
            "The validated Plan belongs to another requirement.",
        ));
    }
    let old_source = input.previous["source"]
        .as_object()
        .expect("validated source");
    let mut new_source = serde_json::Map::new();
    for key in old_source.keys() {
        let value = input
            .plan_validation
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "draft_source_drift",
                    "The validated Plan source fingerprint is missing.",
                    json!({"field":key}),
                )
            })?;
        new_source.insert(key.clone(), json!(value));
    }
    let skills = input.plan["skill_selection"]["skills"]
        .as_array()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_skill_selection",
                "The validated Plan skill selection is missing.",
                json!({}),
            )
        })?;
    let mut hashes = BTreeMap::new();
    for row in input.previous["tasks"].as_array().expect("validated tasks") {
        let task_id = row["id"].as_str().expect("validated ID");
        if let Some(skill_id) = row["skill_id"].as_str() {
            if !skills.iter().any(|skill| {
                skill["id"] == skill_id && skill["mode_support"]["task"] != "unsupported"
            }) {
                return Err(workflow(
                    "draft_skill_not_available",
                    "Resolve the TASK skill binding against the confirmed Plan before updating sources.",
                ));
            }
        }
        let selected = input.selections.get(task_id).ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_object_fields",
                "Every active TASK requires an instruction selection.",
                json!({"task_id":task_id}),
            )
        })?;
        work_operations::task::draft::validate_draft_instruction_selection(selected)
            .map_err(domain)?;
        let paths: Vec<String> = selected["selected_paths"]
            .as_array()
            .expect("validated paths")
            .iter()
            .map(|value| value.as_str().expect("validated path").to_owned())
            .collect();
        let references: Vec<String> = selected["references"]
            .as_array()
            .expect("validated references")
            .iter()
            .map(|value| value.as_str().expect("validated reference").to_owned())
            .collect();
        validate_task_paths(
            instructions,
            &paths,
            &input.plan["hierarchy_selection"],
            &format!("selections.{task_id}"),
        )?;
        hashes.insert(
            task_id.into(),
            load(instructions, "task", &paths, &references)?.instructions_sha256,
        );
    }
    prepare_source_update(
        history,
        input.previous,
        &SourceUpdateRequest {
            new_source: &Value::Object(new_source),
            selections: input.selections,
            instruction_hashes: &hashes,
            expected_revision: input.expected_revision,
            reason: input.reason,
            plan_path: input.plan_path,
        },
    )
}

#[derive(Debug, Clone)]
pub struct PreparedDraftSave {
    pub index: Value,
    pub index_raw: Vec<u8>,
    pub draft_raw: Option<Vec<u8>>,
    pub task_id: Option<String>,
    pub revision: u64,
}

pub fn prepare_save(
    index: &Value,
    expected_revision: u64,
    previous: Option<&Value>,
    draft: Option<&Value>,
) -> Result<PreparedDraftSave, WorkError> {
    let mut proposed = index.clone();
    validate_planning_index(&proposed).map_err(domain)?;
    if proposed["revision"].as_u64() != Some(expected_revision + 1)
        || previous
            .and_then(|value| value["revision"].as_u64())
            .unwrap_or(0)
            != expected_revision
    {
        return Err(workflow(
            "draft_revision_conflict",
            "The current index changed; reload before saving.",
        ));
    }
    if let Some(previous) = previous {
        validate_planning_index(previous).map_err(domain)?;
        if proposed.get("retired_task_ids") != previous.get("retired_task_ids") {
            return Err(workflow(
                "draft_scope_changed",
                "Discussion saving cannot change retired TASK identifiers.",
            ));
        }
    }
    let mut draft_raw = None;
    let mut task_id = None;
    match (previous, draft) {
        (None, Some(_)) => {
            return Err(workflow(
                "invalid_initial_draft_index",
                "Initialize the confirmed TASK list before saving discussions.",
            ));
        }
        (None, None) => {
            if proposed["tasks"]
                .as_array()
                .expect("validated tasks")
                .iter()
                .any(|entry| entry["status"] != "planned" || entry.get("draft_ref").is_some())
            {
                return Err(workflow(
                    "invalid_initial_draft_index",
                    "Initialize the confirmed TASK list before saving discussions.",
                ));
            }
        }
        (Some(_), None) => {
            return Err(workflow(
                "draft_required",
                "Updating an existing index requires one TASK discussion.",
            ));
        }
        (Some(previous), Some(draft)) => {
            let id = draft["task_id"].as_str().ok_or_else(|| {
                workflow(
                    "draft_task_not_in_index",
                    "The draft TASK must exist in the proposed index.",
                )
            })?;
            if proposed["source"] != previous["source"] {
                return Err(workflow(
                    "draft_scope_changed",
                    "This save cannot change the source snapshot or TASK list.",
                ));
            }
            let old_tasks = previous["tasks"].as_array().expect("validated tasks");
            let next_tasks = proposed["tasks"].as_array().expect("validated tasks");
            if old_tasks
                .iter()
                .map(|entry| &entry["id"])
                .collect::<Vec<_>>()
                != next_tasks
                    .iter()
                    .map(|entry| &entry["id"])
                    .collect::<Vec<_>>()
            {
                return Err(workflow(
                    "draft_scope_changed",
                    "This save cannot change the source snapshot or TASK list.",
                ));
            }
            let old = old_tasks
                .iter()
                .find(|entry| entry["id"] == id)
                .ok_or_else(|| {
                    workflow(
                        "draft_task_not_in_index",
                        "The draft TASK must exist in the proposed index.",
                    )
                })?;
            for (old_entry, new_entry) in old_tasks.iter().zip(next_tasks) {
                if old_entry["id"] != id && old_entry != new_entry {
                    return Err(workflow(
                        "draft_scope_changed",
                        "This save can change only the selected TASK.",
                    ));
                }
            }
            let target = proposed["tasks"]
                .as_array_mut()
                .expect("validated tasks")
                .iter_mut()
                .find(|entry| entry["id"] == id)
                .expect("same TASK IDs");
            if old.get("instruction_selection").is_some()
                && target.get("instruction_selection") != old.get("instruction_selection")
            {
                return Err(workflow(
                    "draft_selection_mismatch",
                    "Changing a saved instruction selection requires the source-update workflow.",
                ));
            }
            let old_draft_revision = old["draft_ref"]["revision"].as_u64().unwrap_or(0);
            if draft["revision"].as_u64() != Some(old_draft_revision + 1) {
                return Err(workflow(
                    "draft_revision_conflict",
                    "The draft must immediately follow its previous revision.",
                ));
            }
            let changed = [
                "title",
                "goal",
                "scope",
                "skill_id",
                "dependencies",
                "instructions_sha256",
            ]
            .iter()
            .any(|field| target[*field] != old[*field]);
            if target["boundary_revision"].as_u64()
                != old["boundary_revision"]
                    .as_u64()
                    .map(|revision| revision + u64::from(changed))
            {
                return Err(workflow(
                    "draft_boundary_conflict",
                    "Boundary changes require exactly one new boundary revision.",
                ));
            }
            target
                .as_object_mut()
                .expect("validated entry")
                .remove("draft_ref");
            validate_task_draft(draft, &proposed).map_err(domain)?;
            let raw = canonical_task_draft(draft);
            let reference = json!({"save_revision": proposed["revision"], "revision": draft["revision"], "sha256": fingerprint::raw(&raw)});
            proposed["tasks"]
                .as_array_mut()
                .expect("validated tasks")
                .iter_mut()
                .find(|entry| entry["id"] == id)
                .expect("same TASK IDs")["draft_ref"] = reference;
            draft_raw = Some(raw);
            task_id = Some(id.into());
        }
    }
    validate_planning_index(&proposed).map_err(domain)?;
    let raw = canonical_planning_index(&proposed);
    Ok(PreparedDraftSave {
        revision: expected_revision + 1,
        index: proposed,
        index_raw: raw,
        draft_raw,
        task_id,
    })
}

pub fn resolve_instruction_selection(
    entry: &Value,
    selected_paths: Option<&[String]>,
    reference_names: Option<&[String]>,
) -> Result<Value, WorkError> {
    let stored = entry.get("instruction_selection");
    if let Some(value) = stored {
        validate_draft_instruction_selection(value).map_err(domain)?;
    }
    let Some(paths) = selected_paths else {
        if reference_names.is_some() {
            return Err(WorkError::new(
                ExitCode::Contract,
                "draft_selection_incomplete",
                "Explicit references require an explicit instruction path selection.",
                json!({}),
            ));
        }
        return stored.cloned().ok_or_else(|| {
            workflow(
                "draft_selection_required",
                "Confirm instruction paths and references for this legacy TASK before continuing.",
            )
        });
    };
    let explicit = json!({"selected_paths":paths,"references":reference_names.unwrap_or(&[])});
    validate_draft_instruction_selection(&explicit).map_err(domain)?;
    if stored.is_some_and(|value| value != &explicit) {
        return Err(workflow(
            "draft_selection_mismatch",
            "Changing a saved instruction selection requires the source-update workflow.",
        ));
    }
    Ok(explicit)
}

pub struct DraftSourceCheck<'a> {
    pub index: &'a Value,
    pub task_id: &'a str,
    pub expected_revision: u64,
    pub plan: &'a Value,
    pub plan_validation: &'a Value,
    pub selected_paths: Option<&'a [String]>,
    pub reference_names: Option<&'a [String]>,
}

pub fn check_validated_sources(
    instructions: &impl InstructionSourceRepository,
    input: &DraftSourceCheck<'_>,
) -> Result<Value, WorkError> {
    validate_planning_index(input.index).map_err(domain)?;
    if input.index["revision"].as_u64() != Some(input.expected_revision) {
        return Err(workflow(
            "draft_revision_conflict",
            "Reload the current planning index before checking sources.",
        ));
    }
    let entry = input.index["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .find(|entry| entry["id"] == input.task_id)
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "draft_task_not_in_index",
                "The selected TASK is absent from the planning index.",
                json!({}),
            )
        })?;
    let selected =
        resolve_instruction_selection(entry, input.selected_paths, input.reference_names)?;
    let requirement_id = input.index["requirement_id"]
        .as_str()
        .expect("validated ID");
    if input.plan_validation["requirement_id"] != requirement_id {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "draft_source_requirement_mismatch",
            "The Plan belongs to a different requirement.",
            json!({}),
        ));
    }
    let mismatches: Vec<&str> = input.index["source"]
        .as_object()
        .expect("validated source")
        .iter()
        .filter_map(|(field, stored)| {
            (input.plan_validation.get(field) != Some(stored)).then_some(field.as_str())
        })
        .collect();
    if !mismatches.is_empty() {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "draft_source_drift",
            "The saved planning sources differ from the validated Plan.",
            json!({"fields":mismatches,"task_id":input.task_id}),
        ));
    }
    let skill_id = &entry["skill_id"];
    if let Some(id) = skill_id.as_str() {
        let available = input.plan["skill_selection"]["skills"]
            .as_array()
            .is_some_and(|skills| {
                skills.iter().any(|skill| {
                    skill["id"] == id && skill["mode_support"]["task"] != "unsupported"
                })
            });
        if !available {
            return Err(WorkError::new(
                ExitCode::Contract,
                "draft_skill_not_available",
                "The TASK skill is not a Plan-confirmed Task-capable skill.",
                json!({}),
            ));
        }
    }
    let paths: Vec<String> = selected["selected_paths"]
        .as_array()
        .expect("validated paths")
        .iter()
        .map(|value| value.as_str().expect("validated path").to_owned())
        .collect();
    let references: Vec<String> = selected["references"]
        .as_array()
        .expect("validated references")
        .iter()
        .map(|value| value.as_str().expect("validated reference").to_owned())
        .collect();
    validate_task_paths(
        instructions,
        &paths,
        &input.plan["hierarchy_selection"],
        "draft.instruction_paths",
    )?;
    let loaded = load(instructions, "task", &paths, &references)?;
    if entry["instructions_sha256"] != loaded.instructions_sha256 {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "draft_instruction_drift",
            "The current TASK instruction fingerprint differs from the saved selection.",
            json!({"task_id":input.task_id}),
        ));
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskDraftSourceCheck,
    >(
        json!({"schema":"work-task-draft-source-check/v1","status":"valid",
        "requirement_id":requirement_id,"task_id":input.task_id,"revision":input.expected_revision,
        "source":input.index["source"],"skill_id":skill_id,
        "instructions_sha256":loaded.instructions_sha256,"instruction_selection":selected}),
    ))
}

pub fn status(
    requirement_id: &str,
    index: Option<&Value>,
    selected_task_id: Option<&str>,
    discussion: Option<&Value>,
    recovery_required: bool,
) -> Result<Value, WorkError> {
    let Some(index) = index else {
        if selected_task_id.is_some() && !recovery_required {
            return Err(WorkError::new(
                ExitCode::Contract,
                "draft_task_not_in_index",
                "No planning index exists for the selected TASK.",
                json!({}),
            ));
        }
        return Ok(
            json!({"schema":"work-task-draft-status/v1","requirement_id":requirement_id,
            "status":if recovery_required {"recovery_required"} else {"not_initialized"},
            "revision":null,"current_task_id":null,"selected_task_id":null,
            "counts":{"planned":0,"in_progress":0,"refined":0,"needs_review":0},"tasks":[],"discussion":null,
            "next_action":if recovery_required {"inspect_recovery"} else {"confirm_task_list"},
            "required_checks":if recovery_required {vec![]} else {vec!["plan validate"]},"requires_user_confirmation":true,
            "source_validation":"not_checked","assembly_validation":"not_performed","instruction_selection":null,
            "selection_confirmation_required":false}),
        );
    };
    validate_planning_index(index).map_err(domain)?;
    if index["requirement_id"] != requirement_id {
        return Err(workflow(
            "draft_requirement_mismatch",
            "The stored requirement does not match its directory.",
        ));
    }
    let selected_id = selected_task_id.or_else(|| index["current_task_id"].as_str());
    let tasks = index["tasks"].as_array().expect("validated planning tasks");
    let selected =
        selected_id.and_then(|task_id| tasks.iter().find(|entry| entry["id"] == task_id));
    if selected_task_id.is_some() && selected.is_none() {
        return Err(WorkError::new(
            ExitCode::Contract,
            "draft_task_not_in_index",
            "The selected TASK is absent from the planning index.",
            json!({}),
        ));
    }
    let mut counts = serde_json::Map::new();
    for state in PLANNING_STATUSES {
        counts.insert(
            state.into(),
            json!(
                tasks
                    .iter()
                    .filter(|entry| entry["status"] == state)
                    .count()
            ),
        );
    }
    let mut result = json!({"schema":"work-task-draft-status/v1","requirement_id":requirement_id,"status":"saved",
        "revision":index["revision"],"current_task_id":index["current_task_id"],"selected_task_id":selected_id,
        "counts":counts,"tasks":tasks,"discussion":null,"next_action":"confirm_task_list","required_checks":["plan validate"],
        "requires_user_confirmation":true,"source_validation":"not_checked","assembly_validation":"not_performed",
        "instruction_selection":selected.and_then(|entry| entry.get("instruction_selection")).cloned(),
        "selection_confirmation_required":selected.is_some_and(|entry| entry.get("instruction_selection").is_none())});
    if recovery_required {
        result["status"] = json!("recovery_required");
        result["next_action"] = json!("inspect_recovery");
        result["required_checks"] = json!([]);
        return Ok(result);
    }
    if let Some(selected) = selected {
        if selected.get("draft_ref").is_some() {
            let discussion = discussion.ok_or_else(|| {
                workflow(
                    "missing_stored_draft_reference",
                    "A saved discussion requires a version reference.",
                )
            })?;
            validate_task_draft(discussion, index).map_err(domain)?;
            let mut summary = serde_json::Map::new();
            for key in [
                "task_id",
                "revision",
                "status",
                "notes",
                "confirmed_decisions",
                "tentative",
                "open_questions",
                "next_discussion_point",
            ] {
                summary.insert(key.into(), discussion[key].clone());
            }
            summary.insert(
                "has_task_candidate".into(),
                json!(discussion.get("task_candidate").is_some()),
            );
            result["discussion"] = Value::Object(summary);
        }
    }
    if counts["refined"] == tasks.len() {
        result["next_action"] = json!("assemble_for_review");
        result["required_checks"] = json!(["task draft-assemble"]);
    } else if selected.is_none() || selected.is_some_and(|entry| entry["status"] == "refined") {
        result["next_action"] = json!("choose_task");
        result["required_checks"] = json!(["plan validate", "task draft-check"]);
    } else {
        result["next_action"] = json!(match selected.expect("selected TASK")["status"].as_str() {
            Some("planned") => "confirm_start",
            Some("in_progress") => "confirm_resume",
            Some("needs_review") => "confirm_review",
            _ => unreachable!("validated planning status"),
        });
        result["required_checks"] = json!(["plan validate", "task draft-check"]);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy::HierarchyCatalogRepository;
    use work_operations::hierarchy::{CrossModeCatalog, Hierarchy};
    use work_operations::instruction::SourceSet;

    struct DraftInstructions;

    impl HierarchyCatalogRepository for DraftInstructions {
        fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
            unreachable!("task source checks do not request a plan catalog")
        }

        fn mode_paths(&self, _mode: &str) -> Result<Vec<String>, WorkError> {
            Ok(vec!["general".into()])
        }
    }

    impl InstructionSourceRepository for DraftInstructions {
        fn load_sources(
            &self,
            mode: &str,
            hierarchy: &Hierarchy,
            references: &[String],
        ) -> Result<SourceSet, WorkError> {
            Ok(SourceSet {
                mode: mode.into(),
                hierarchy: hierarchy.clone(),
                sources: vec![],
                references: references.to_vec(),
                instructions_sha256: "d".repeat(64),
            })
        }
    }

    struct NoHistory;

    impl TaskDraftHistoryRepository for NoHistory {
        fn read_historical_draft(&self, _: &str, _: u64, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("A planned TASK has no historical discussion")
        }
    }

    #[test]
    fn list_update_prepares_exact_history_file_set() {
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Before","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let mut proposed = previous.clone();
        proposed["revision"] = json!(2);
        proposed["tasks"][0]["title"] = json!("After");
        let prepared =
            prepare_list_update(&NoHistory, &previous, &proposed, 1, "Review changed title.")
                .unwrap();
        assert_eq!(prepared.revision, 2);
        assert_eq!(prepared.affected_task_ids, ["TASK-001"]);
        assert_eq!(prepared.index["tasks"][0]["boundary_revision"], 2);
        assert_eq!(
            prepared
                .files
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["index-current.tmp", "index.json", "list-update.json"]
        );
        assert_eq!(
            prepared.files["index.json"],
            prepared.files["index-current.tmp"]
        );
        assert_eq!(
            crate::error::ExitCode::WorkflowState,
            prepare_list_update(&NoHistory, &previous, &proposed, 0, "Review changed title.")
                .unwrap_err()
                .exit_code
        );
        let mut unchanged = previous.clone();
        unchanged["revision"] = json!(2);
        assert_eq!(
            prepare_list_update(&NoHistory, &previous, &unchanged, 1, "Review.")
                .unwrap_err()
                .reason_code,
            "draft_list_unchanged"
        );
        let mut forged = proposed.clone();
        forged["tasks"][0]["status"] = json!("needs_review");
        assert_eq!(
            prepare_list_update(&NoHistory, &previous, &forged, 1, "Review.")
                .unwrap_err()
                .reason_code,
            "draft_list_metadata_changed"
        );
        let mut selection_changed = proposed.clone();
        selection_changed["tasks"][0]["instruction_selection"] =
            json!({"selected_paths":[],"references":[]});
        assert_eq!(
            prepare_list_update(&NoHistory, &previous, &selection_changed, 1, "Review.")
                .unwrap_err()
                .reason_code,
            "draft_selection_mismatch"
        );
    }

    #[test]
    fn empty_progress_and_planned_action_match_python() {
        let empty = status("example", None, None, None, false).unwrap();
        assert_eq!(empty["status"], "not_initialized");
        assert_eq!(empty["next_action"], "confirm_task_list");
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let planned = status("example", Some(&index), None, None, false).unwrap();
        assert_eq!(planned["next_action"], "confirm_start");
        assert_eq!(planned["selection_confirmation_required"], true);
        assert_eq!(
            status("example", Some(&index), None, None, true).unwrap()["next_action"],
            "inspect_recovery"
        );
        let initialized = prepare_save(&index, 0, None, None).unwrap();
        assert_eq!(initialized.revision, 1);
        assert!(initialized.draft_raw.is_none());
        let mut next = initialized.index.clone();
        next["revision"] = json!(2);
        next["tasks"][0]["status"] = json!("in_progress");
        let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001","revision":1,
            "boundary_revision":1,"source":index["source"],"instructions_sha256":"d".repeat(64),"status":"in_progress",
            "notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue discussion."});
        let saved = prepare_save(&next, 1, Some(&index), Some(&draft)).unwrap();
        assert_eq!(
            saved.index["tasks"][0]["draft_ref"]["sha256"],
            work_operations::derivation::fingerprint::raw(saved.draft_raw.as_ref().unwrap())
        );
        assert_eq!(
            prepare_save(&next, 0, Some(&index), Some(&draft))
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
    }

    #[test]
    fn status_planned_selection_and_missing_current_match_python() {
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[
                {"id":"TASK-001","title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                 "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)},
                {"id":"TASK-002","title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                 "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let planned = status("example", Some(&index), None, None, false).unwrap();
        assert_eq!(planned["next_action"], "confirm_start");
        assert_eq!(planned["selected_task_id"], "TASK-001");
        assert_eq!(planned["counts"]["planned"], 2);
        assert_eq!(planned["source_validation"], "not_checked");
        assert!(
            planned["required_checks"]
                .as_array()
                .unwrap()
                .contains(&json!("task draft-check"))
        );
        assert_eq!(planned["instruction_selection"], Value::Null);
        assert_eq!(planned["selection_confirmation_required"], true);

        let mut selected = index.clone();
        let selection =
            json!({"selected_paths":["web/backend"],"references":["task.general.task-records"]});
        selected["tasks"][0]["instruction_selection"] = selection.clone();
        let saved = status("example", Some(&selected), None, None, false).unwrap();
        assert_eq!(saved["instruction_selection"], selection);
        assert_eq!(saved["selection_confirmation_required"], false);
        assert_eq!(saved["source_validation"], "not_checked");

        selected["current_task_id"] = Value::Null;
        let unselected = status("example", Some(&selected), None, None, false).unwrap();
        assert_eq!(unselected["next_action"], "choose_task");
        assert_eq!(unselected["selected_task_id"], Value::Null);
        assert_eq!(
            status("example", Some(&selected), Some("TASK-999"), None, false)
                .unwrap_err()
                .reason_code,
            "draft_task_not_in_index"
        );
    }

    #[test]
    fn status_saved_and_refined_actions_match_python() {
        let mut index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":3,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[
                {"id":"TASK-001","title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                 "dependencies":[],"status":"in_progress","boundary_revision":1,"instructions_sha256":"d".repeat(64),
                 "draft_ref":{"save_revision":2,"revision":1,"sha256":"e".repeat(64)}},
                {"id":"TASK-002","title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                 "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let mut discussion = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001","revision":1,
            "boundary_revision":1,"source":index["source"],"instructions_sha256":"d".repeat(64),"status":"in_progress",
            "notes":["具體討論"],"confirmed_decisions":[],"tentative":[],"open_questions":["Which test?"],
            "next_discussion_point":"Confirm test."});
        let progress = status("example", Some(&index), None, Some(&discussion), false).unwrap();
        assert_eq!(progress["next_action"], "confirm_resume");
        assert_eq!(
            progress["discussion"]["open_questions"],
            json!(["Which test?"])
        );
        assert_eq!(
            progress["discussion"]["next_discussion_point"],
            "Confirm test."
        );

        index["tasks"][0]["status"] = json!("needs_review");
        discussion["status"] = json!("needs_review");
        let review = status("example", Some(&index), None, Some(&discussion), false).unwrap();
        assert_eq!(review["next_action"], "confirm_review");
        assert_eq!(
            review["discussion"]["open_questions"],
            json!(["Which test?"])
        );

        index["tasks"][0]["status"] = json!("refined");
        discussion["status"] = json!("refined");
        discussion["open_questions"] = json!([]);
        discussion["next_discussion_point"] = Value::Null;
        let refined = status("example", Some(&index), None, Some(&discussion), false).unwrap();
        assert_eq!(refined["next_action"], "choose_task");
        assert_eq!(refined["selected_task_id"], "TASK-001");
        let next = status("example", Some(&index), Some("TASK-002"), None, false).unwrap();
        assert_eq!(next["next_action"], "confirm_start");
        assert_eq!(next["current_task_id"], "TASK-001");

        index["tasks"][1]["status"] = json!("refined");
        let complete = status("example", Some(&index), None, Some(&discussion), false).unwrap();
        assert_eq!(complete["next_action"], "assemble_for_review");
        assert_eq!(complete["required_checks"], json!(["task draft-assemble"]));
        assert_eq!(complete["discussion"]["has_task_candidate"], false);
        assert_eq!(complete["assembly_validation"], "not_performed");
    }

    #[test]
    fn draft_source_check_respects_saved_selection_and_live_fingerprints() {
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64),
            "instruction_selection":{"selected_paths":[],"references":[]}}]});
        let plan =
            json!({"skill_selection":{"skills":[]},"hierarchy_selection":{"selected_paths":[]}});
        let validation = json!({"requirement_id":"example","plan_sha256":"a".repeat(64),
            "hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let mut input = DraftSourceCheck {
            index: &index,
            task_id: "TASK-001",
            expected_revision: 1,
            plan: &plan,
            plan_validation: &validation,
            selected_paths: None,
            reference_names: None,
        };
        let result = check_validated_sources(&DraftInstructions, &input).unwrap();
        assert_eq!(result["status"], "valid");
        assert_eq!(
            result["instruction_selection"],
            index["tasks"][0]["instruction_selection"]
        );
        input.task_id = "TASK-999";
        assert_eq!(
            check_validated_sources(&DraftInstructions, &input)
                .unwrap_err()
                .reason_code,
            "draft_task_not_in_index"
        );
        input.task_id = "TASK-001";
        input.expected_revision = 2;
        assert_eq!(
            check_validated_sources(&DraftInstructions, &input)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        input.expected_revision = 1;
        let conflicting = ["web".into()];
        input.selected_paths = Some(&conflicting);
        assert_eq!(
            check_validated_sources(&DraftInstructions, &input)
                .unwrap_err()
                .reason_code,
            "draft_selection_mismatch"
        );
        input.selected_paths = None;
        let mut legacy = index.clone();
        legacy["tasks"][0]
            .as_object_mut()
            .unwrap()
            .remove("instruction_selection");
        input.index = &legacy;
        assert_eq!(
            check_validated_sources(&DraftInstructions, &input)
                .unwrap_err()
                .reason_code,
            "draft_selection_required"
        );
        input.index = &index;
        let mut stale_validation = validation.clone();
        stale_validation["plan_sha256"] = json!("0".repeat(64));
        input.plan_validation = &stale_validation;
        let drift = check_validated_sources(&DraftInstructions, &input).unwrap_err();
        assert_eq!(drift.reason_code, "draft_source_drift");
        assert_eq!(drift.details["fields"], json!(["plan_sha256"]));
        let mut skill_stale_validation = validation.clone();
        skill_stale_validation["skill_selection_sha256"] = json!("0".repeat(64));
        input.plan_validation = &skill_stale_validation;
        let drift = check_validated_sources(&DraftInstructions, &input).unwrap_err();
        assert_eq!(drift.reason_code, "draft_source_drift");
        assert_eq!(drift.details["fields"], json!(["skill_selection_sha256"]));
        input.plan_validation = &validation;
        let mut referenced = index.clone();
        referenced["tasks"][0]["instruction_selection"]["references"] =
            json!(["task.general.task-records"]);
        input.index = &referenced;
        let selected = check_validated_sources(&DraftInstructions, &input).unwrap();
        assert_eq!(
            selected["instruction_selection"]["references"],
            json!(["task.general.task-records"])
        );
        assert_eq!(selected["instructions_sha256"], "d".repeat(64));
        let mut unavailable = index.clone();
        unavailable["tasks"][0]["skill_id"] = json!("unknown");
        input.index = &unavailable;
        assert_eq!(
            check_validated_sources(&DraftInstructions, &input)
                .unwrap_err()
                .reason_code,
            "draft_skill_not_available"
        );
        let mut stale_instruction = index.clone();
        stale_instruction["tasks"][0]["instructions_sha256"] = json!("0".repeat(64));
        input.index = &stale_instruction;
        assert_eq!(
            check_validated_sources(&DraftInstructions, &input)
                .unwrap_err()
                .reason_code,
            "draft_instruction_drift"
        );
    }

    #[test]
    fn draft_instruction_selection_preserves_saved_order_and_requires_complete_choice() {
        let entry = json!({"instruction_selection":{"selected_paths":["web/backend","web/frontend"],"references":["second","first"]}});
        let mut result = resolve_instruction_selection(&entry, None, None).unwrap();
        assert_eq!(result, entry["instruction_selection"]);
        result["references"]
            .as_array_mut()
            .unwrap()
            .push(json!("third"));
        assert_eq!(
            entry["instruction_selection"]["references"],
            json!(["second", "first"])
        );

        let paths = ["web/backend".into(), "web/frontend".into()];
        let references = ["second".into(), "first".into()];
        assert_eq!(
            resolve_instruction_selection(&entry, Some(&paths), Some(&references)).unwrap(),
            entry["instruction_selection"]
        );
        assert_eq!(
            resolve_instruction_selection(&entry, Some(&paths), None)
                .unwrap_err()
                .reason_code,
            "draft_selection_mismatch"
        );

        let legacy = json!({});
        assert_eq!(
            resolve_instruction_selection(&legacy, None, None)
                .unwrap_err()
                .reason_code,
            "draft_selection_required"
        );
        assert_eq!(
            resolve_instruction_selection(&legacy, Some(&[]), None).unwrap(),
            json!({"selected_paths":[],"references":[]})
        );
        assert_eq!(
            resolve_instruction_selection(&legacy, None, Some(&[]))
                .unwrap_err()
                .reason_code,
            "draft_selection_incomplete"
        );
    }
}
