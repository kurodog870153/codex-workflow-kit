//! Delegation role rules over resolved project identities.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::delegation::{build_envelope, validate_envelope, validation_result};
use work_operations::execution::index::validate_execution_index;
use work_operations::identifiers::RequirementId;
use work_operations::plan::validation::validate_plan_structure;
use work_operations::progress::validate_progress;
use work_operations::protocol::{TASK_ID_PREFIX, valid_sha256};

use crate::error::{ExitCode, WorkError};
use crate::plan::PlanPathRepository;

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
        return Err(boundary("Only Plan and Task may restore discussion."));
    }
    let required = [
        "task_boundary",
        "skill_snapshot",
        "source_plan",
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
    let selected = &context["task_boundary"];
    let id = selected["id"].as_str().unwrap_or("");
    if id.len() != 8
        || !id.starts_with(TASK_ID_PREFIX)
        || !id.as_bytes()[5..].iter().all(u8::is_ascii_digit)
        || selected["title"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        || selected["goal"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
    {
        return Err(boundary(
            "Task refinement requires one identified TASK boundary.",
        ));
    }
    let skill = &context["skill_snapshot"];
    let skill_id = skill["id"]
        .as_str()
        .ok_or_else(|| boundary("Task refinement requires a selected skill."))?;
    let plan = &context["source_plan"];
    validate_plan_structure(plan)
        .map_err(|_| boundary("Task refinement requires a formal source Plan."))?;
    let skills = plan["skill_selection"]["skills"]
        .as_array()
        .ok_or_else(|| boundary("Supply a Work skill selection snapshot."))?;
    let decision = if skills.is_empty() {
        "base_only"
    } else {
        "external_skills"
    };
    if plan["skill_selection"]["selection_sha256"]
        != work_operations::skill::selection_sha256(decision, skills)
        || selected["skill_id"] != skill_id
        || !skills.contains(skill)
        || skill["mode_support"]["task"] == "unsupported"
        || skill["dependency_status"] != "available"
    {
        return Err(boundary(
            "Task refinement requires exactly its Plan-confirmed executable skill.",
        ));
    }
    if context["work_instruction_selection"]["sources"]
        .as_array()
        .is_none_or(|items| items.is_empty())
        || !context["work_instruction_selection"]["instructions_sha256"]
            .as_str()
            .is_some_and(valid_sha256)
    {
        return Err(boundary("Stored Work instruction selection is invalid."));
    }
    for field in ["repository_evidence", "saved_discussion"] {
        if !context[field].as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|text| !text.trim().is_empty()))
        }) {
            return Err(boundary(
                "Task skill discussion evidence must be a string array.",
            ));
        }
    }
    Ok(validation_result("task-skill", &mode, false))
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
        return Err(boundary("Only Plan and Task may restore discussion."));
    }
    let required = [
        "requirement_id",
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
    envelope: &Value,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    let (_, mode, context, resume) =
        validate_envelope(envelope, "execute", sender, project_root, skill_root)
            .map_err(|issue| boundary(issue.message))?;
    if resume {
        return Err(boundary("Only Plan and Task may restore discussion."));
    }
    let required = [
        "hierarchy_selection",
        "work_instruction_selection",
        "skill_selection",
        "target_task",
        "hierarchy_selection_sha256",
        "execute_skill_selection",
    ];
    let object = context.as_object().expect("validated delegation context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "workflow_context must contain exactly the required and optional fields.",
        ));
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
            != work_operations::hierarchy::selection_sha256(decision, &selected, entries, catalog)
        || context["hierarchy_selection_sha256"] != hierarchy["selection_sha256"]
    {
        return Err(boundary(
            "Execute must retain the target's exact skill and hierarchy identity.",
        ));
    }
    let instruction = &context["work_instruction_selection"];
    if instruction["selected_paths"] != hierarchy["selected_paths"]
        || instruction["sources"]
            .as_array()
            .is_none_or(|items| items.is_empty())
        || !instruction["instructions_sha256"]
            .as_str()
            .is_some_and(valid_sha256)
    {
        return Err(boundary("Stored Work instruction selection is invalid."));
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
            != work_operations::skill::selection_sha256(decision, selected_skills)
    {
        return Err(boundary(
            "Skill snapshot identities or selection fingerprint disagree.",
        ));
    }
    let task = &context["target_task"];
    let task_id = task["id"].as_str().unwrap_or("");
    if task_id.len() != 8
        || !task_id.starts_with(TASK_ID_PREFIX)
        || !task_id.as_bytes()[5..].iter().all(u8::is_ascii_digit)
        || task.get("skill_id").is_none()
    {
        return Err(boundary("Execute requires one explicit target TASK row."));
    }
    let matching: Vec<Value> = selected_skills
        .iter()
        .filter(|skill| skill["id"] == task["skill_id"])
        .cloned()
        .collect();
    let execute_skills = &context["execute_skill_selection"];
    let execute_decision = if matching.is_empty() {
        "base_only"
    } else {
        "external_skills"
    };
    if execute_skills["skills"] != json!(matching)
        || matching.len() != if task["skill_id"].is_null() { 0 } else { 1 }
        || execute_skills["decision"] != execute_decision
        || execute_skills["selection_sha256"]
            != work_operations::skill::selection_sha256(execute_decision, &matching)
    {
        return Err(boundary(
            "Execute must retain the target's exact skill and hierarchy identity.",
        ));
    }
    if matching.iter().any(|skill| {
        skill["mode_support"]["execute"] == "unsupported"
            || skill["dependency_status"] != "available"
    }) {
        return Err(boundary("Execute requires available, supported skills."));
    }
    Ok(validation_result("execute", &mode, false))
}

pub fn validate_artifact_editor(
    paths: &impl PlanPathRepository,
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
        return Err(boundary("Only Plan and Task may restore discussion."));
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
    ];
    let object = context.as_object().expect("validated context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "maintenance_context must contain exactly the required and optional fields.",
        ));
    }
    let id: RequirementId = context["requirement_id"]
        .as_str()
        .ok_or_else(|| boundary("requirement_id must be a nonempty string."))?
        .parse()
        .map_err(|_| boundary("The requirement ID is invalid."))?;
    let plan_path = context["artifacts"]["plan"]
        .as_str()
        .ok_or_else(|| boundary("The Plan artifact path is invalid."))?;
    paths.validate_paths(&id, &context["artifacts"], plan_path, true)?;
    if context["confirmed_request"]
        .as_object()
        .is_none_or(|value| value.is_empty())
    {
        return Err(boundary("confirmed_request must be a nonempty object."));
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
            != work_operations::hierarchy::selection_sha256(decision, &selected, entries, catalog)
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
            != work_operations::skill::selection_sha256(decision, selected_skills)
    {
        return Err(boundary(
            "Skill snapshot identities or selection fingerprint disagree.",
        ));
    }
    Ok(validation_result("artifact-editor", &mode, false))
}

pub fn validate_plan_role_name(role: &str) -> Result<(), WorkError> {
    if !matches!(role, "plan" | "task-coordinator") {
        return Err(boundary(
            "The expected sender cannot delegate to this role.",
        ));
    }
    Ok(())
}

pub fn validate_plan_role(
    envelope: &Value,
    role: &str,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<Value, WorkError> {
    validate_plan_role_name(role)?;
    let (text, mode, context, resume) =
        validate_envelope(envelope, role, sender, project_root, skill_root)
            .map_err(|issue| boundary(issue.message))?;
    if resume {
        let object = context.as_object().expect("validated delegation context");
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
        let valid_request = words.len() == 2
            && words[0] == "resume"
            && words[1].parse::<RequirementId>().is_ok()
            && progress["requirement_id"] == words[1]
            && progress["mode"] == mode;
        if !valid_request {
            return Err(boundary(
                "Resume request, mode and saved discussion identity disagree.",
            ));
        }
        return Ok(validation_result(role, &mode, true));
    }
    let required: &[&str] = if role == "plan" {
        &[
            "hierarchy_selection",
            "work_instruction_selection",
            "skill_selection",
        ]
    } else {
        &[
            "hierarchy_selection",
            "work_instruction_selection",
            "skill_selection",
            "source_plan",
        ]
    };
    let object = context.as_object().expect("validated delegation context");
    if object.len() != required.len() || required.iter().any(|key| !object.contains_key(*key)) {
        return Err(boundary(
            "workflow_context must contain exactly the required and optional fields.",
        ));
    }
    let hierarchy = &context["hierarchy_selection"];
    let selected = hierarchy["selected_paths"]
        .as_array()
        .ok_or_else(|| boundary("selected_paths must be an array."))?;
    let selected: Vec<String> = selected
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
            != work_operations::hierarchy::selection_sha256(decision, &selected, entries, catalog)
    {
        return Err(boundary(
            "The stored hierarchy fingerprint disagrees with its fields.",
        ));
    }
    let instruction = &context["work_instruction_selection"];
    if instruction["selected_paths"] != hierarchy["selected_paths"]
        || instruction["sources"]
            .as_array()
            .is_none_or(|items| items.is_empty())
        || !instruction["references"].is_array()
        || !instruction["instructions_sha256"]
            .as_str()
            .is_some_and(valid_sha256)
    {
        return Err(boundary("Stored Work instruction selection is invalid."));
    }
    let skills = &context["skill_selection"];
    let selected_skills = skills["skills"]
        .as_array()
        .ok_or_else(|| boundary("Supply a Work skill selection snapshot."))?;
    let skill_decision = if selected_skills.is_empty() {
        "base_only"
    } else {
        "external_skills"
    };
    if skills["schema"] != "work-skill-selection/v1"
        || skills["decision"] != skill_decision
        || skills["selection_sha256"]
            != work_operations::skill::selection_sha256(skill_decision, selected_skills)
    {
        return Err(boundary(
            "Skill snapshot identities or selection fingerprint disagree.",
        ));
    }
    if role == "task-coordinator" {
        let plan = &context["source_plan"];
        validate_plan_structure(plan)
            .map_err(|_| boundary("Source Plan selections disagree with the envelope."))?;
        if plan["hierarchy_selection"] != *hierarchy || plan["skill_selection"] != *skills {
            return Err(boundary(
                "Source Plan selections disagree with the envelope.",
            ));
        }
    }
    Ok(validation_result(role, &mode, false))
}

pub fn build_task_skill(
    source_repository: &impl DelegationSourceRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let required = ["schema", "role", "request", "source_plan_path", "task_id"];
    if required.iter().any(|key| !object.contains_key(*key))
        || object.keys().any(|key| {
            !required.contains(&key.as_str())
                && !["mode", "repository_evidence", "saved_discussion"].contains(&key.as_str())
        })
    {
        return Err(boundary(
            "The selected role requires only its semantic source and decision fields.",
        ));
    }
    if request["schema"] != "work-delegation-build-request/v1"
        || request["role"] != "task-skill"
        || request.get("mode").is_some_and(|mode| mode != "task")
    {
        return Err(boundary("Task skill delegation must remain in Task mode."));
    }
    let text = request["request"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))?;
    let task_id = request["task_id"]
        .as_str()
        .ok_or_else(|| boundary("An explicit TASK ID is required."))?;
    let source = request["source_plan_path"]
        .as_str()
        .ok_or_else(|| boundary("A formal source Plan path is required."))?;
    let (normalized, plan_absolute) = source_repository.resolve_project_path(source)?;
    let plan_raw = source_repository.read_raw(&plan_absolute)?;
    let plan =
        parse_json_contract(&plan_raw).map_err(|_| boundary("The source Plan is invalid."))?;
    validate_plan_structure(&plan).map_err(|_| boundary("The source Plan is invalid."))?;
    if plan["artifacts"]["plan"] != normalized {
        return Err(boundary(
            "Source Plan path differs from its formal artifact routing.",
        ));
    }
    let task_path = plan["artifacts"]["task"]
        .as_str()
        .ok_or_else(|| boundary("The formal TASK path is invalid."))?;
    let (_, index_absolute) = source_repository.resolve_project_path(task_path)?;
    let index_raw = source_repository.read_raw(&index_absolute)?;
    let index = parse_json_contract(&index_raw)
        .map_err(|_| boundary("The formal TASK index is invalid."))?;
    let reference = index["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == task_id))
        .ok_or_else(|| boundary("The selected TASK is absent from the formal index."))?;
    let relative = reference["path"]
        .as_str()
        .ok_or_else(|| boundary("The formal TASK item path is invalid."))?;
    let directory = task_path
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let (_, item_absolute) =
        source_repository.resolve_project_path(&format!("{directory}/{relative}"))?;
    let item_raw = source_repository.read_raw(&item_absolute)?;
    let item =
        parse_json_contract(&item_raw).map_err(|_| boundary("The formal TASK item is invalid."))?;
    if item["id"] != task_id {
        return Err(boundary(
            "The TASK item identity differs from its index reference.",
        ));
    }
    let skill = plan["skill_selection"]["skills"]
        .as_array()
        .and_then(|skills| skills.iter().find(|skill| skill["id"] == item["skill_id"]))
        .ok_or_else(|| boundary("Task skill is not present in the formal Plan selection."))?;
    let context = json!({"task_boundary":{"id":item["id"],"title":item["title"],
            "goal":item["goal"],"skill_id":item["skill_id"]},
        "skill_snapshot":skill,"source_plan":plan,
        "work_instruction_selection":plan["work_instruction_selection"],
        "repository_evidence":request.get("repository_evidence").cloned().unwrap_or(json!([])),
        "saved_discussion":request.get("saved_discussion").cloned().unwrap_or(json!([]))});
    let project_root = source_repository.canonical_project_root()?;
    let skill_root = source_repository.canonical_skill_root()?;
    let envelope = build_envelope(
        "task-skill",
        "task",
        text,
        &project_root,
        &skill_root,
        &context,
    )
    .map_err(|issue| boundary(issue.message))?;
    validate_task_skill(&envelope, "task-coordinator", &project_root, &skill_root)?;
    for (path, expected) in [
        (&plan_absolute, &plan_raw),
        (&index_absolute, &index_raw),
        (&item_absolute, &item_raw),
    ] {
        if source_repository.read_raw(path).ok().as_ref() != Some(expected) {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "delegation_source_changed",
                "A formal source changed during delegation construction.",
                json!({"path":path}),
            ));
        }
    }
    Ok(envelope)
}

pub fn build_artifact_editor(
    source_repository: &impl DelegationSourceRepository,
    paths: &impl PlanPathRepository,
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
        "source_plan_path",
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
        .filter(|mode| matches!(*mode, "plan" | "task" | "execute"))
        .ok_or_else(|| boundary("Artifact editor requires an originating mode."))?;
    let text = request["request"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))?;
    let source = request["source_plan_path"]
        .as_str()
        .ok_or_else(|| boundary("A formal source Plan path is required."))?;
    let (normalized, absolute) = source_repository.resolve_project_path(source)?;
    let raw = source_repository.read_raw(&absolute)?;
    let plan = parse_json_contract(&raw).map_err(|_| boundary("The source Plan is invalid."))?;
    validate_plan_structure(&plan).map_err(|_| boundary("The source Plan is invalid."))?;
    if plan["artifacts"]["plan"] != normalized {
        return Err(boundary(
            "Source Plan path differs from its formal artifact routing.",
        ));
    }
    let context = json!({"requirement_id":plan["requirement_id"],
        "artifacts":plan["artifacts"], "confirmed_request":request["confirmed_request"],
        "decisions":request["decisions"],"affected_task_ids":request["affected_task_ids"],
        "hierarchy_selection":plan["hierarchy_selection"],
        "skill_selection":plan["skill_selection"],
        "repository_evidence":request.get("repository_evidence").cloned().unwrap_or(json!([])),
        "continuation_point":request["continuation_point"]});
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
    validate_artifact_editor(paths, &envelope, "parent", &project_root, &skill_root)?;
    if source_repository.read_raw(&absolute)? != raw {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "delegation_source_changed",
            "The source Plan changed during delegation construction.",
            json!({"path":normalized}),
        ));
    }
    Ok(envelope)
}

pub fn build_execute_role(
    source_repository: &impl DelegationSourceRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let required = ["schema", "role", "request", "source_plan_path", "task_id"];
    if object.len() < required.len()
        || required.iter().any(|field| !object.contains_key(*field))
        || object
            .keys()
            .any(|field| !required.contains(&field.as_str()) && field != "mode")
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
    let task_id = request["task_id"]
        .as_str()
        .ok_or_else(|| boundary("An explicit TASK ID is required."))?;
    let plan_path = request["source_plan_path"]
        .as_str()
        .ok_or_else(|| boundary("A formal source Plan path is required."))?;
    let (normalized_plan, plan_absolute) = source_repository.resolve_project_path(plan_path)?;
    let plan_raw = source_repository.read_raw(&plan_absolute)?;
    let plan =
        parse_json_contract(&plan_raw).map_err(|_| boundary("The source Plan is invalid."))?;
    validate_plan_structure(&plan).map_err(|_| boundary("The source Plan is invalid."))?;
    if plan["artifacts"]["plan"] != normalized_plan {
        return Err(boundary(
            "Source Plan path differs from its formal artifact routing.",
        ));
    }
    let task_path = plan["artifacts"]["task"]
        .as_str()
        .ok_or_else(|| boundary("The formal TASK path is invalid."))?;
    let (_, task_absolute) = source_repository.resolve_project_path(task_path)?;
    let task_raw = source_repository.read_raw(&task_absolute)?;
    let task_index = parse_json_contract(&task_raw)
        .map_err(|_| boundary("The formal TASK index is invalid."))?;
    let reference = task_index["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == task_id))
        .ok_or_else(|| boundary("The selected TASK is absent from the formal index."))?;
    let relative = reference["path"]
        .as_str()
        .ok_or_else(|| boundary("The formal TASK item path is invalid."))?;
    let directory = task_path
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let (_, item_absolute) =
        source_repository.resolve_project_path(&format!("{directory}/{relative}"))?;
    let item_raw = source_repository.read_raw(&item_absolute)?;
    let item =
        parse_json_contract(&item_raw).map_err(|_| boundary("The formal TASK item is invalid."))?;
    if item["id"] != task_id {
        return Err(boundary(
            "The TASK item identity differs from its index reference.",
        ));
    }
    let execution_path = plan["artifacts"]["execution"]
        .as_str()
        .ok_or_else(|| boundary("The execution path is invalid."))?;
    let (_, execution_absolute) =
        source_repository.resolve_project_path(&format!("{execution_path}/index.json"))?;
    let execution_raw = source_repository.read_raw(&execution_absolute)?;
    let execution = parse_json_contract(&execution_raw)
        .map_err(|_| boundary("The execution index is invalid."))?;
    validate_execution_index(&execution, &execution_raw)
        .map_err(|_| boundary("The execution index is invalid."))?;
    let target = execution["tasks"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == task_id))
        .ok_or_else(|| boundary("Execution TASK and formal TASK skill identity disagree."))?;
    if target["skill_id"] != item["skill_id"] {
        return Err(boundary(
            "Execution TASK and formal TASK skill identity disagree.",
        ));
    }
    let selected_skills: Vec<Value> = plan["skill_selection"]["skills"]
        .as_array()
        .ok_or_else(|| boundary("Supply a Work skill selection snapshot."))?
        .iter()
        .filter(|skill| skill["id"] == target["skill_id"])
        .cloned()
        .collect();
    let skill_decision = if selected_skills.is_empty() {
        "base_only"
    } else {
        "external_skills"
    };
    let context = json!({
        "hierarchy_selection":plan["hierarchy_selection"],
        "work_instruction_selection":plan["work_instruction_selection"],
        "skill_selection":plan["skill_selection"],
        "target_task":target,
        "hierarchy_selection_sha256":plan["hierarchy_selection"]["selection_sha256"],
        "execute_skill_selection":{"schema":"work-skill-selection/v1",
            "decision":skill_decision,"skills":selected_skills,
            "selection_sha256":work_operations::skill::selection_sha256(skill_decision, &selected_skills)},
    });
    let project_root = source_repository.canonical_project_root()?;
    let skill_root = source_repository.canonical_skill_root()?;
    let envelope = build_envelope(
        "execute",
        "execute",
        text,
        &project_root,
        &skill_root,
        &context,
    )
    .map_err(|issue| boundary(issue.message))?;
    validate_envelope(&envelope, "execute", "parent", &project_root, &skill_root)
        .map_err(|issue| boundary(issue.message))?;
    validate_execute_role(&envelope, "parent", &project_root, &skill_root)?;
    for (path, expected) in [
        (&plan_absolute, &plan_raw),
        (&task_absolute, &task_raw),
        (&item_absolute, &item_raw),
        (&execution_absolute, &execution_raw),
    ] {
        if source_repository.read_raw(path).ok().as_ref() != Some(expected) {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "delegation_source_changed",
                "A formal source changed during delegation construction.",
                json!({"path":path}),
            ));
        }
    }
    Ok(envelope)
}

pub fn build_plan_role(
    source_repository: &impl DelegationSourceRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    let role = request["role"]
        .as_str()
        .ok_or_else(|| boundary("Unknown delegation role."))?;
    if !matches!(role, "plan" | "task-coordinator") {
        return Err(boundary(
            "The selected role requires a different formal source.",
        ));
    }
    let object = request
        .as_object()
        .ok_or_else(|| boundary("The build request must be an object."))?;
    let required = ["schema", "role", "request", "source_plan_path"];
    if object.len() < required.len()
        || required.iter().any(|field| !object.contains_key(*field))
        || object
            .keys()
            .any(|field| !required.contains(&field.as_str()) && field != "mode")
    {
        return Err(boundary(
            "The selected role requires only its semantic source and decision fields.",
        ));
    }
    let mode = if role == "plan" { "plan" } else { "task" };
    if request["schema"] != "work-delegation-build-request/v1"
        || request.get("mode").is_some_and(|value| value != mode)
    {
        return Err(boundary("The role determines its originating mode."));
    }
    let text = request["request"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| boundary("request must be a nonempty string."))?;
    let selected = request["source_plan_path"]
        .as_str()
        .ok_or_else(|| boundary("A formal source Plan path is required."))?;
    let (normalized, absolute) = source_repository.resolve_project_path(selected)?;
    let raw = source_repository.read_raw(&absolute)?;
    let plan = parse_json_contract(&raw).map_err(|_| boundary("The source Plan is invalid."))?;
    validate_plan_structure(&plan).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if plan["artifacts"]["plan"] != normalized {
        return Err(boundary(
            "Source Plan path differs from its formal artifact routing.",
        ));
    }
    let mut context = json!({
        "hierarchy_selection":plan["hierarchy_selection"],
        "work_instruction_selection":plan["work_instruction_selection"],
        "skill_selection":plan["skill_selection"],
    });
    if role == "task-coordinator" {
        context["source_plan"] = plan.clone();
    }
    let project_root = source_repository.canonical_project_root()?;
    let skill_root = source_repository.canonical_skill_root()?;
    let envelope = build_envelope(role, mode, text, &project_root, &skill_root, &context)
        .map_err(|issue| boundary(issue.message))?;
    validate_envelope(&envelope, role, "parent", &project_root, &skill_root)
        .map_err(|issue| boundary(issue.message))?;
    validate_plan_role(&envelope, role, "parent", &project_root, &skill_root)?;
    if source_repository.read_raw(&absolute)? != raw {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "delegation_source_changed",
            "The source Plan changed during delegation construction.",
            json!({"path":normalized}),
        ));
    }
    Ok(envelope)
}

pub fn build_progress_saver(
    source_repository: &impl DelegationSourceRepository,
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
        .filter(|mode| matches!(*mode, "plan" | "task"))
        .ok_or_else(|| boundary("Progress saver requires Plan or Task mode."))?;
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
    let mut context = json!({"requirement_id":progress["requirement_id"],
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
