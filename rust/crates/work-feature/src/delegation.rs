//! Delegation role rules over resolved project identities.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::delegation::{build_envelope, validate_envelope, validation_result};
use work_operations::derivation::fingerprint;
use work_operations::execution::index::validate_execution_index;
use work_operations::identifiers::RequirementId;
use work_operations::progress::validate_progress;
use work_operations::protocol::{TASK_ID_PREFIX, valid_sha256};

use crate::error::{ExitCode, WorkError};

pub trait DelegationSourceRepository {
    fn resolve_project_path(&self, relative: &str) -> Result<(String, PathBuf), WorkError>;
    fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError>;
    fn canonical_project_root(&self) -> Result<String, WorkError>;
    fn canonical_skill_root(&self) -> Result<String, WorkError>;
}

fn boundary(message: &'static str) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        "delegation_boundary_mismatch",
        message,
        json!({}),
    )
}

/// Current validated inputs for Task-only delegation, supplied by project adapters.
pub trait TaskDelegationRepository {
    fn planning_context(&self, source: &Value) -> Result<(Value, Value), WorkError>;
    fn task_collection(&self, task_path: &str) -> Result<Value, WorkError>;
    fn source_bytes(&self, collection: &Value) -> Result<Value, WorkError>;
}

fn validate_task_source(context: &Value) -> Result<(), WorkError> {
    let required = [
        "requirement_id",
        "source",
        "artifacts",
        "hierarchy_selection",
        "skill_selection",
        "acceptance_criteria",
        "source_bytes",
    ];
    let object = context
        .as_object()
        .ok_or_else(|| boundary("A complete Task-owned Source context is required."))?;
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "A complete Task-owned Source context is required.",
        ));
    }
    let requirement = context["requirement_id"]
        .as_str()
        .ok_or_else(|| boundary("A requirement ID is required."))?;
    work_operations::task::source::validate_formal_context(context, requirement)
        .map_err(|_| boundary("Task source and confirmed choices are invalid."))?;
    match context["source"]["kind"].as_str() {
        Some("snapshot") => {
            let manifest = serde_json::from_value(context["source"]["manifest"].clone())
                .map_err(|_| boundary("The Source Snapshot is invalid."))?;
            let raw: Vec<u8> = serde_json::from_value(context["source_bytes"].clone())
                .map_err(|_| boundary("Original Source bytes are required."))?;
            work_operations::source_snapshot::validate(&manifest, Some(&raw))
                .map_err(|_| boundary("Original Source bytes differ from the Snapshot."))?;
        }
        Some("migration") if context["source_bytes"].is_null() => {}
        _ => {
            return Err(boundary(
                "Task context must retain its complete immutable source evidence.",
            ));
        }
    }
    Ok(())
}

fn validate_discussion_evidence(context: &Value) -> Result<(), WorkError> {
    for field in ["repository_evidence", "saved_discussion"] {
        if !context[field].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|text| !text.trim().is_empty()))
        }) {
            return Err(boundary("Task discussion evidence must be a string array."));
        }
    }
    Ok(())
}

fn validate_task_instruction(context: &Value, selected: &Value) -> Result<(), WorkError> {
    let instruction = &context["work_instruction_selection"];
    let _: work_model::instruction::InstructionSelection =
        serde_json::from_value(instruction.clone())
            .map_err(|_| boundary("Stored Work instruction selection is invalid."))?;
    if instruction["selected_paths"] != *selected
        || instruction["sources"]
            .as_array()
            .is_none_or(|rows| rows.is_empty())
        || !instruction["instructions_sha256"]
            .as_str()
            .is_some_and(valid_sha256)
    {
        return Err(boundary("Stored Work instruction selection is invalid."));
    }
    Ok(())
}

pub fn validate_task_coordinator(
    envelope: &Value,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    let (text, mode, context, resume) = validate_envelope(
        envelope,
        "task-coordinator",
        sender,
        project_root,
        skill_root,
    )
    .map_err(|issue| boundary(issue.message))?;
    if resume {
        let object = context.as_object().expect("validated context");
        if object.len() != 1 || !object.contains_key("saved_progress") {
            return Err(boundary(
                "resume_context must contain exactly saved_progress.",
            ));
        }
        let progress = &context["saved_progress"];
        validate_progress(progress).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        let words: Vec<_> = text.split_whitespace().collect();
        if words.len() != 2
            || words[0] != "resume"
            || words[1].parse::<RequirementId>().is_err()
            || progress["requirement_id"] != words[1]
            || progress["mode"] != "task"
        {
            return Err(boundary(
                "Resume request, mode and saved discussion identity disagree.",
            ));
        }
        work_operations::task::source::validate_planning_source(
            &progress["context"]["planning_source"],
            words[1],
        )
        .map_err(|_| boundary("Task resume requires its fixed Source."))?;
        return Ok(validation_result("task-coordinator", &mode, true));
    }
    let required = [
        "task_source",
        "work_instruction_selection",
        "repository_evidence",
        "saved_discussion",
    ];
    let object = context.as_object().expect("validated context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "workflow_context must contain exactly the required and optional fields.",
        ));
    }
    validate_task_source(&context["task_source"])?;
    validate_task_instruction(
        &context,
        &context["task_source"]["hierarchy_selection"]["selected_paths"],
    )?;
    validate_discussion_evidence(&context)?;
    Ok(validation_result("task-coordinator", &mode, false))
}

pub fn validate_task_skill(
    envelope: &Value,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    let (_, mode, context, resume) =
        validate_envelope(envelope, "task-skill", sender, project_root, skill_root)
            .map_err(|issue| boundary(issue.message))?;
    if resume {
        return Err(boundary("Only Task coordinator may restore discussion."));
    }
    let required = [
        "task_boundary",
        "skill_snapshot",
        "task_source",
        "work_instruction_selection",
        "repository_evidence",
        "saved_discussion",
    ];
    let object = context.as_object().expect("validated context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "task_skill_context must contain exactly the required and optional fields.",
        ));
    }
    validate_task_source(&context["task_source"])?;
    let selected = &context["task_boundary"];
    let item: work_model::task::item::TaskItem = serde_json::from_value(selected.clone())
        .map_err(|_| boundary("Task refinement requires one complete identified TASK boundary."))?;
    let raw = work_operations::task::ordering::render_task(
        selected,
        work_operations::task::ordering::TaskDocumentKind::Item,
    )
    .map_err(|_| boundary("The Task boundary is invalid."))?;
    work_operations::task::item::validate_task_item(selected, &raw, &item.id)
        .map_err(|_| boundary("The Task boundary is invalid."))?;
    let skill = &context["skill_snapshot"];
    if selected["skill_id"] != skill["id"]
        || !context["task_source"]["skill_selection"]["skills"]
            .as_array()
            .is_some_and(|rows| rows.contains(skill))
        || skill["mode_support"]["task"] == "unsupported"
        || skill["dependency_status"] != "available"
    {
        return Err(boundary(
            "Task refinement requires exactly its Task-confirmed executable skill.",
        ));
    }
    if context["work_instruction_selection"] != selected["instruction_selection"] {
        return Err(boundary(
            "Task instructions must match the selected Task boundary.",
        ));
    }
    validate_task_instruction(
        &context,
        &selected["instruction_selection"]["selected_paths"],
    )?;
    validate_discussion_evidence(&context)?;
    Ok(validation_result("task-skill", &mode, false))
}

fn task_build_request<'a>(
    request: &'a Value,
    role: &str,
    source_field: &str,
    with_task: bool,
) -> Result<&'a str, WorkError> {
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let mut required = vec!["schema", "role", "request", source_field];
    if with_task {
        required.push("task_id");
    }
    if required.iter().any(|key| !object.contains_key(*key))
        || object.keys().any(|key| {
            !required.contains(&key.as_str())
                && !["mode", "repository_evidence", "saved_discussion"].contains(&key.as_str())
        })
        || request["schema"] != "work-delegation-build-request/v1"
        || request["role"] != role
        || request.get("mode").is_some_and(|mode| mode != "task")
    {
        return Err(boundary(
            "The Task role requires only its Source and semantic decision fields.",
        ));
    }
    request["request"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))
}

pub fn build_task_coordinator(
    repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    request: &Value,
) -> Result<Value, WorkError> {
    let text = task_build_request(request, "task-coordinator", "planning_source", false)?;
    let (source, instructions) = repository.planning_context(&request["planning_source"])?;
    let context = json!({"task_source":source,"work_instruction_selection":instructions,"repository_evidence":request.get("repository_evidence").cloned().unwrap_or(json!([])),"saved_discussion":request.get("saved_discussion").cloned().unwrap_or(json!([]))});
    let project = repository.canonical_project_root()?;
    let skill = repository.canonical_skill_root()?;
    let envelope = build_envelope("task-coordinator", "task", text, &project, &skill, &context)
        .map_err(|issue| boundary(issue.message))?;
    validate_task_coordinator(&envelope, "parent", &project, &skill)?;
    if repository.planning_context(&request["planning_source"])? != (source, instructions) {
        return Err(boundary(
            "The Task source changed during delegation construction.",
        ));
    }
    Ok(envelope)
}

pub fn validate_progress_saver(
    envelope: &Value,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    let (_, mode, context, resume) =
        validate_envelope(envelope, "progress-saver", sender, project_root, skill_root)
            .map_err(|issue| boundary(issue.message))?;
    if resume {
        return Err(boundary("Only Task may restore discussion."));
    }
    let required = [
        "requirement_id",
        "task_source",
        "content",
        "expected_revision",
        "continuation_point",
    ];
    let object = context.as_object().expect("validated context");
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "save_approval")
    {
        return Err(boundary(
            "maintenance_context must contain exactly the required and optional fields.",
        ));
    }
    let revision = context["expected_revision"]
        .as_u64()
        .ok_or_else(|| boundary("Expected saved revision must be a nonnegative integer."))?;
    let content = context["content"]
        .as_object()
        .ok_or_else(|| boundary("content must be an object."))?;
    let content_fields = [
        "title",
        "request",
        "current_task_id",
        "context",
        "source_status",
        "notes",
        "confirmed_decisions",
        "tentative",
        "open_questions",
        "next_discussion_point",
    ];
    if content.len() != content_fields.len()
        || content_fields
            .iter()
            .any(|field| !content.contains_key(*field))
    {
        return Err(boundary(
            "content must contain exactly the required progress fields.",
        ));
    }
    validate_task_source(&context["task_source"])?;
    let source = &context["task_source"];
    let planning = json!({"snapshot":source["source"]["manifest"],
        "artifacts":source["artifacts"],"hierarchy_selection":source["hierarchy_selection"],
        "skill_selection":source["skill_selection"],"acceptance_criteria":source["acceptance_criteria"]});
    if source["requirement_id"] != context["requirement_id"]
        || source["source"]["kind"] != "snapshot"
        || context["content"]["context"]["planning_source"] != planning
    {
        return Err(boundary("Saved discussion must retain its fixed Source."));
    }
    let mut candidate = Value::Object(content.clone());
    candidate["schema"] = json!("work-discussion-progress/v1");
    candidate["requirement_id"] = context["requirement_id"].clone();
    candidate["mode"] = json!(mode);
    candidate["revision"] = json!(revision + 1);
    candidate["status"] = json!("discussion_only");
    validate_progress(&candidate).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if context["continuation_point"]
        .as_str()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(boundary("continuation_point must be a nonempty string."));
    }
    if context.get("save_approval").is_some()
        && !context["save_approval"].as_str().is_some_and(valid_sha256)
    {
        return Err(boundary(
            "save_approval must be a lowercase SHA-256 digest.",
        ));
    }
    Ok(validation_result("progress-saver", &mode, false))
}

pub fn validate_execute_role(
    repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    envelope: &Value,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    let (_, mode, context, resume) =
        validate_envelope(envelope, "execute", sender, project_root, skill_root)
            .map_err(|issue| boundary(issue.message))?;
    if resume {
        return Err(boundary("Only Task coordinator may restore discussion."));
    }
    let required = [
        "task_path",
        "task_source",
        "task_collection_sha256",
        "task_boundary",
        "target_task",
        "execution_index",
        "execution_index_sha256",
        "execute_skill_selection",
    ];
    let object = context.as_object().expect("validated context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "workflow_context must contain exactly the required and optional fields.",
        ));
    }
    let path = context["task_path"]
        .as_str()
        .ok_or_else(|| boundary("A formal Task path is required."))?;
    let id = context["task_boundary"]["id"]
        .as_str()
        .ok_or_else(|| boundary("An explicit TASK ID is required."))?;
    let expected = execute_context(repository, path, id)?;
    if context != expected {
        return Err(boundary(
            "Execute must retain the exact formal Task, Source and Execution evidence.",
        ));
    }
    let mut result = validation_result("execute", &mode, false);
    result["source_validation"] = json!("checked");
    Ok(result)
}

fn execute_context(
    repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    path: &str,
    id: &str,
) -> Result<Value, WorkError> {
    let validation = repository.task_collection(path)?;
    let collection = &validation["collection_contract"];
    if collection["artifacts"]["task"] != path {
        return Err(boundary(
            "Task path differs from its formal artifact routing.",
        ));
    }
    let item = collection["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == id))
        .ok_or_else(|| boundary("The selected TASK is absent from the formal collection."))?;
    let execution_path = collection["artifacts"]["execution"]
        .as_str()
        .ok_or_else(|| boundary("The execution path is invalid."))?;
    let (_, absolute) = repository.resolve_project_path(&format!("{execution_path}/index.json"))?;
    let raw = repository.read_raw(&absolute)?;
    let execution =
        parse_json_contract(&raw).map_err(|_| boundary("The execution index is invalid."))?;
    validate_execution_index(&execution, &raw)
        .map_err(|_| boundary("The execution index is invalid."))?;
    for key in [
        "task_collection_sha256",
        "task_index_sha256",
        "hierarchy_selection_sha256",
        "skill_selection_sha256",
    ] {
        if execution[key] != validation[key] {
            return Err(boundary(
                "Execution and formal Task collection bindings disagree.",
            ));
        }
    }
    if execution["requirement_id"] != collection["requirement_id"]
        || execution["task_spec_id"] != validation["spec_id"]
    {
        return Err(boundary("Execution and formal Task identities disagree."));
    }
    let target = execution["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == id))
        .ok_or_else(|| boundary("The selected TASK is absent from Execution."))?;
    let digest = &validation["task_item_sha256"][id];
    if target["skill_id"] != item["skill_id"]
        || target["task_item_sha256"] != *digest
        || target["instructions_sha256"] != validation["task_instructions_sha256"][id]
        || execution["task_instructions_sha256"] != validation["instructions_sha256"]
    {
        return Err(boundary(
            "Execution and formal TASK skill or item identity disagree.",
        ));
    }
    let skills: Vec<Value> = collection["skill_selection"]["skills"]
        .as_array()
        .expect("validated skills")
        .iter()
        .filter(|skill| skill["id"] == item["skill_id"])
        .cloned()
        .collect();
    if skills.len() != if item["skill_id"].is_null() { 0 } else { 1 }
        || skills.iter().any(|skill| {
            skill["mode_support"]["execute"] == "unsupported"
                || skill["dependency_status"] != "available"
        })
    {
        return Err(boundary(
            "Execute requires the target's confirmed available supported skill.",
        ));
    }
    let decision = if skills.is_empty() {
        "base_only"
    } else {
        "external_skills"
    };
    let bytes = repository.source_bytes(collection)?;
    let source = task_source_context(collection, bytes.clone());
    validate_task_source(&source)?;
    let context = json!({"task_path":path,"task_source":source,"task_collection_sha256":validation["task_collection_sha256"],"task_boundary":item,"target_task":target,"execution_index":execution,"execution_index_sha256":fingerprint::raw(&raw),"execute_skill_selection":{"schema":"work-skill-selection/v1","decision":decision,"skills":skills,"selection_sha256":fingerprint::skill_selection(decision,&skills)}});
    if repository.task_collection(path)? != validation
        || repository.source_bytes(collection)? != bytes
        || repository.read_raw(&absolute)? != raw
    {
        return Err(boundary(
            "A formal source changed during Execute delegation.",
        ));
    }
    Ok(context)
}

pub fn validate_artifact_editor(
    repository: &impl TaskDelegationRepository,
    envelope: &Value,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    let (_, mode, context, resume) = validate_envelope(
        envelope,
        "artifact-editor",
        sender,
        project_root,
        skill_root,
    )
    .map_err(|issue| boundary(issue.message))?;
    if resume {
        return Err(boundary("Only Task coordinator may restore discussion."));
    }
    let required = [
        "requirement_id",
        "artifacts",
        "confirmed_request",
        "decisions",
        "affected_task_ids",
        "hierarchy_selection",
        "skill_selection",
        "repository_evidence",
        "continuation_point",
        "task_source",
        "task_collection_sha256",
    ];
    let object = context.as_object().expect("validated context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "maintenance_context must contain exactly the required and optional fields.",
        ));
    }
    validate_task_source(&context["task_source"])?;
    let task_path = context["artifacts"]["task"]
        .as_str()
        .ok_or_else(|| boundary("A formal Task path is required."))?;
    let validation = repository.task_collection(task_path)?;
    let collection = &validation["collection_contract"];
    if context["requirement_id"] != collection["requirement_id"]
        || context["artifacts"] != collection["artifacts"]
        || context["hierarchy_selection"] != collection["hierarchy_selection"]
        || context["skill_selection"] != collection["skill_selection"]
        || context["task_collection_sha256"] != validation["task_collection_sha256"]
        || context["task_source"]
            != task_source_context(collection, repository.source_bytes(collection)?)
    {
        return Err(boundary(
            "Artifact editor must retain the exact validated Task collection and fixed Source.",
        ));
    }
    work_operations::specification::prepare::validate_prepare_request(
        &context["confirmed_request"],
    )
    .map_err(|_| {
        boundary("Editor candidates require a Task-only semantic specification request.")
    })?;
    if context["confirmed_request"]["requirement_id"] != collection["requirement_id"] {
        return Err(boundary(
            "Confirmed edits must identify the same Task requirement.",
        ));
    }
    let decisions = context["decisions"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| boundary("Artifact revision requires retained confirmed decisions."))?;
    for decision in decisions {
        if !(decision.as_object().is_some_and(|value| !value.is_empty())
            || decision
                .as_str()
                .is_some_and(|value| !value.trim().is_empty()))
        {
            return Err(boundary(
                "Artifact revision requires retained confirmed decisions.",
            ));
        }
    }
    let ids = context["affected_task_ids"]
        .as_array()
        .ok_or_else(|| boundary("affected_task_ids must be an array."))?;
    let mut seen = HashSet::new();
    for id in ids {
        let id = id
            .as_str()
            .ok_or_else(|| boundary("Affected TASK IDs must use TASK-nnn."))?;
        if id.len() != 8
            || !id.starts_with(TASK_ID_PREFIX)
            || !id.as_bytes()[5..].iter().all(u8::is_ascii_digit)
            || !seen.insert(id)
        {
            return Err(boundary(
                "Affected TASK IDs must be unique TASK-nnn values.",
            ));
        }
    }
    let active = collection["tasks"]
        .as_array()
        .expect("validated Task collection");
    if seen
        .iter()
        .any(|id| !active.iter().any(|task| task["id"] == *id))
    {
        return Err(boundary(
            "Affected TASK IDs must belong to the validated collection.",
        ));
    }
    let edits = context["confirmed_request"]["edits"]
        .as_array()
        .expect("validated semantic edits");
    for edit in edits {
        if edit["target"]["artifact"] == "task_item"
            && !seen.contains(edit["target"]["task_id"].as_str().unwrap_or(""))
        {
            return Err(boundary("Confirmed edits exceed the affected TASK scope."));
        }
    }
    if context["confirmed_request"].get("source_update").is_some()
        && active
            .iter()
            .any(|task| !seen.contains(task["id"].as_str().unwrap()))
    {
        return Err(boundary(
            "Source revisions require the complete affected TASK scope.",
        ));
    }
    if !context["repository_evidence"]
        .as_array()
        .is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|text| !text.trim().is_empty()))
        })
    {
        return Err(boundary("repository_evidence must be a string array."));
    }
    if context["continuation_point"]
        .as_str()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(boundary("continuation_point must be a nonempty string."));
    }
    let hierarchy = &context["hierarchy_selection"];
    let selected: Vec<String> = hierarchy["selected_paths"]
        .as_array()
        .ok_or_else(|| boundary("selected_paths must be an array."))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| boundary("selected_paths must be a string array."))
        })
        .collect::<Result<_, _>>()?;
    let entries = hierarchy["entries"]
        .as_array()
        .ok_or_else(|| boundary("Hierarchy entries must be an array."))?;
    let catalog = hierarchy["catalog_sha256"]
        .as_str()
        .ok_or_else(|| boundary("catalog_sha256 must be a lowercase SHA-256 digest."))?;
    let decision = hierarchy["decision"]
        .as_str()
        .ok_or_else(|| boundary("Supply the confirmed hierarchy snapshot."))?;
    if hierarchy["schema"] != "work-hierarchy-selection/v1"
        || hierarchy["selection_sha256"]
            != fingerprint::hierarchy_selection(decision, &selected, entries, catalog)
    {
        return Err(boundary(
            "The stored hierarchy fingerprint disagrees with its fields.",
        ));
    }
    let skills = &context["skill_selection"];
    let selected_skills = skills["skills"]
        .as_array()
        .ok_or_else(|| boundary("Supply a Work skill selection snapshot."))?;
    let decision = if selected_skills.is_empty() {
        "base_only"
    } else {
        "external_skills"
    };
    if skills["schema"] != "work-skill-selection/v1"
        || skills["decision"] != decision
        || skills["selection_sha256"]
            != work_operations::derivation::fingerprint::skill_selection(decision, selected_skills)
    {
        return Err(boundary(
            "Skill snapshot identities or selection fingerprint disagree.",
        ));
    }
    let mut result = validation_result("artifact-editor", &mode, false);
    result["source_validation"] = json!("checked");
    Ok(result)
}

pub fn build_task_skill(
    repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    request: &Value,
) -> Result<Value, WorkError> {
    let text = task_build_request(request, "task-skill", "task_path", true)?;
    let path = request["task_path"]
        .as_str()
        .ok_or_else(|| boundary("An explicit Task path is required."))?;
    let id = request["task_id"]
        .as_str()
        .ok_or_else(|| boundary("An explicit TASK ID is required."))?;
    let validation = repository.task_collection(path)?;
    let collection = &validation["collection_contract"];
    let mut item = collection["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == id))
        .ok_or_else(|| boundary("The selected TASK is absent from the formal collection."))?
        .clone();
    item["schema"] = json!("work-task-item/v1");
    let skill = collection["skill_selection"]["skills"]
        .as_array()
        .and_then(|rows| rows.iter().find(|skill| skill["id"] == item["skill_id"]))
        .ok_or_else(|| {
            boundary("The selected TASK requires an explicitly confirmed executable skill.")
        })?;
    let bytes = repository.source_bytes(collection)?;
    let source = task_source_context(collection, bytes.clone());
    let context = json!({"task_boundary":item,"skill_snapshot":skill,"task_source":source,"work_instruction_selection":item["instruction_selection"],"repository_evidence":request.get("repository_evidence").cloned().unwrap_or(json!([])),"saved_discussion":request.get("saved_discussion").cloned().unwrap_or(json!([]))});
    let project = repository.canonical_project_root()?;
    let work = repository.canonical_skill_root()?;
    let envelope = build_envelope("task-skill", "task", text, &project, &work, &context)
        .map_err(|issue| boundary(issue.message))?;
    validate_task_skill(&envelope, "task-coordinator", &project, &work)?;
    if repository.task_collection(path)? != validation
        || repository.source_bytes(collection)? != bytes
    {
        return Err(boundary(
            "The Task source changed during delegation construction.",
        ));
    }
    Ok(envelope)
}

pub fn task_source_context(collection: &Value, bytes: Value) -> Value {
    json!({"requirement_id":collection["requirement_id"],"source":collection["source"],"artifacts":collection["artifacts"],"hierarchy_selection":collection["hierarchy_selection"],"skill_selection":collection["skill_selection"],"acceptance_criteria":collection["acceptance_criteria"],"source_bytes":bytes})
}

pub fn build_artifact_editor(
    source_repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    request: &Value,
) -> Result<Value, WorkError> {
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let required = [
        "schema",
        "role",
        "mode",
        "request",
        "task_path",
        "confirmed_request",
        "decisions",
        "affected_task_ids",
        "continuation_point",
    ];
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "repository_evidence")
    {
        return Err(boundary(
            "The selected role requires only its semantic source and decision fields.",
        ));
    }
    if request["schema"] != "work-delegation-build-request/v1"
        || request["role"] != "artifact-editor"
    {
        return Err(boundary("Unknown delegation role."));
    }
    let mode = request["mode"]
        .as_str()
        .filter(|mode| matches!(*mode, "task" | "execute"))
        .ok_or_else(|| boundary("Artifact editor requires an originating mode."))?;
    let text = request["request"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))?;
    let task_path = request["task_path"]
        .as_str()
        .ok_or_else(|| boundary("A formal Task path is required."))?;
    let validation = source_repository.task_collection(task_path)?;
    let collection = &validation["collection_contract"];
    let bytes = source_repository.source_bytes(collection)?;
    let context = json!({"requirement_id":collection["requirement_id"],
        "artifacts":collection["artifacts"], "confirmed_request":request["confirmed_request"],
        "decisions":request["decisions"],"affected_task_ids":request["affected_task_ids"],
        "hierarchy_selection":collection["hierarchy_selection"],
        "skill_selection":collection["skill_selection"],
        "repository_evidence":request.get("repository_evidence").cloned().unwrap_or(json!([])),
        "continuation_point":request["continuation_point"],"task_source":task_source_context(collection, bytes.clone()),"task_collection_sha256":validation["task_collection_sha256"]});
    let project_root = source_repository.canonical_project_root()?;
    let skill_root = source_repository.canonical_skill_root()?;
    let envelope = build_envelope(
        "artifact-editor",
        mode,
        text,
        &project_root,
        &skill_root,
        &context,
    )
    .map_err(|issue| boundary(issue.message))?;
    validate_artifact_editor(
        source_repository,
        &envelope,
        "parent",
        &project_root,
        &skill_root,
    )?;
    if source_repository.task_collection(task_path)? != validation
        || source_repository.source_bytes(collection)? != bytes
    {
        return Err(boundary(
            "The Task collection or immutable Source changed during editor construction.",
        ));
    }
    Ok(envelope)
}

pub fn build_execute_role(
    repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    request: &Value,
) -> Result<Value, WorkError> {
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let required = ["schema", "role", "request", "task_path", "task_id"];
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "mode")
    {
        return Err(boundary(
            "The selected role requires only its semantic source and decision fields.",
        ));
    }
    if request["schema"] != "work-delegation-build-request/v1"
        || request["role"] != "execute"
        || request.get("mode").is_some_and(|mode| mode != "execute")
    {
        return Err(boundary("The role determines its originating mode."));
    }
    let text = request["request"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))?;
    let path = request["task_path"]
        .as_str()
        .ok_or_else(|| boundary("An explicit Task path is required."))?;
    let id = request["task_id"]
        .as_str()
        .ok_or_else(|| boundary("An explicit TASK ID is required."))?;
    let context = execute_context(repository, path, id)?;
    let project = repository.canonical_project_root()?;
    let work = repository.canonical_skill_root()?;
    let envelope = build_envelope("execute", "execute", text, &project, &work, &context)
        .map_err(|issue| boundary(issue.message))?;
    validate_execute_role(repository, &envelope, "parent", &project, &work)?;
    Ok(envelope)
}

pub fn build_progress_saver(
    source_repository: &(impl DelegationSourceRepository + TaskDelegationRepository),
    request: &Value,
) -> Result<Value, WorkError> {
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let required = [
        "schema",
        "role",
        "mode",
        "request",
        "source_progress_path",
        "content",
        "continuation_point",
    ];
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "save_approval")
    {
        return Err(boundary(
            "The selected role requires only its semantic source and decision fields.",
        ));
    }
    if request["schema"] != "work-delegation-build-request/v1"
        || request["role"] != "progress-saver"
    {
        return Err(boundary("Unknown delegation role."));
    }
    let mode = request["mode"]
        .as_str()
        .filter(|mode| *mode == "task")
        .ok_or_else(|| boundary("Progress saver requires Task mode."))?;
    let text = request["request"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))?;
    let source = request["source_progress_path"]
        .as_str()
        .ok_or_else(|| boundary("A saved discussion path is required."))?;
    let (_, absolute) = source_repository.resolve_project_path(source)?;
    let raw = source_repository.read_raw(&absolute)?;
    let progress =
        parse_json_contract(&raw).map_err(|_| boundary("The saved discussion is invalid."))?;
    validate_progress(&progress).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if progress["mode"] != mode {
        return Err(boundary(
            "Saved discussion mode differs from the delegated mode.",
        ));
    }
    let planning = &progress["context"]["planning_source"];
    let (task_source, instructions) = source_repository.planning_context(planning)?;
    if task_source["requirement_id"] != progress["requirement_id"]
        || request["content"]["context"]["planning_source"] != *planning
    {
        return Err(boundary("Saved discussion must retain its fixed Source."));
    }
    let mut context = json!({"task_source":task_source,"requirement_id":progress["requirement_id"],
        "content":request["content"],"expected_revision":progress["revision"],
        "continuation_point":request["continuation_point"]});
    if let Some(approval) = request.get("save_approval") {
        context["save_approval"] = approval.clone();
    }
    let project_root = source_repository.canonical_project_root()?;
    let skill_root = source_repository.canonical_skill_root()?;
    let envelope = build_envelope(
        "progress-saver",
        mode,
        text,
        &project_root,
        &skill_root,
        &context,
    )
    .map_err(|issue| boundary(issue.message))?;
    validate_progress_saver(&envelope, "parent", &project_root, &skill_root)?;
    if source_repository.planning_context(planning)? != (task_source, instructions) {
        return Err(boundary(
            "The fixed Source changed during progress delegation.",
        ));
    }
    if source_repository.read_raw(&absolute)? != raw {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "delegation_source_changed",
            "The saved discussion changed during delegation construction.",
            json!({"path":source}),
        ));
    }
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::{DelegationSourceRepository, build_progress_saver};
    use crate::error::{ExitCode, WorkError};

    struct UnavailableSource;

    impl DelegationSourceRepository for UnavailableSource {
        fn resolve_project_path(&self, _: &str) -> Result<(String, PathBuf), WorkError> {
            Err(WorkError::new(
                ExitCode::IoFailure,
                "source_unavailable",
                "The source is unavailable.",
                json!({}),
            ))
        }

        fn read_raw(&self, _: &Path) -> Result<Vec<u8>, WorkError> {
            unreachable!("path resolution fails first")
        }

        fn canonical_project_root(&self) -> Result<String, WorkError> {
            unreachable!("path resolution fails first")
        }

        fn canonical_skill_root(&self) -> Result<String, WorkError> {
            unreachable!("path resolution fails first")
        }
    }

    impl super::TaskDelegationRepository for UnavailableSource {
        fn planning_context(
            &self,
            _: &serde_json::Value,
        ) -> Result<(serde_json::Value, serde_json::Value), WorkError> {
            unreachable!("path resolution fails first")
        }
        fn task_collection(&self, _: &str) -> Result<serde_json::Value, WorkError> {
            unreachable!("path resolution fails first")
        }
        fn source_bytes(&self, _: &serde_json::Value) -> Result<serde_json::Value, WorkError> {
            unreachable!("path resolution fails first")
        }
    }

    #[test]
    fn progress_builder_propagates_source_port_failure_after_request_validation() {
        let request = json!({
            "schema":"work-delegation-build-request/v1",
            "role":"progress-saver",
            "mode":"task",
            "request":"Save progress.",
            "source_progress_path":"outputs/work/progress/example/task/progress.json",
            "content":{},
            "continuation_point":"Continue",
        });
        assert_eq!(
            build_progress_saver(&UnavailableSource, &request)
                .unwrap_err()
                .reason_code,
            "source_unavailable"
        );
    }
}
