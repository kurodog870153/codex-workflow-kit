//! Plan preparation and validation over feature ports.

use serde_json::{Value, json};
use work_operations::canonical::{JsonContractIssue, parse_json_contract, sha256_hex};
use work_operations::identifiers::RequirementId;
use work_operations::instruction::selection as source_selection;
use work_operations::plan::render_plan_value;
use work_operations::plan::validation::validate_plan_structure;

use crate::error::{ExitCode, WorkError};
use crate::hierarchy::{
    build_selection as build_hierarchy_selection,
    validate_selection as validate_hierarchy_selection,
};
use crate::instruction::{
    InstructionSourceRepository, load as load_instructions, validate_work_selection_value,
};
use crate::skill::{
    SkillRoot, SkillSnapshotRepository, build_selection as build_skill_selection,
    validate_selection as validate_skill_selection,
};

pub trait PlanPathRepository {
    fn default_paths(&self, requirement_id: &RequirementId) -> Result<Value, WorkError>;
    fn validate_paths(
        &self,
        requirement_id: &RequirementId,
        artifacts: &Value,
        actual_plan_path: &str,
        allow_task_index: bool,
    ) -> Result<(), WorkError>;
    fn exists(&self, relative_path: &str) -> Result<bool, WorkError>;
    fn create_exclusive(&self, relative_path: &str, content: &[u8]) -> Result<(), WorkError>;
    fn read(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
}

pub fn default_artifact_paths(id: &RequirementId) -> [(&'static str, String); 3] {
    let id = id.as_str();
    [
        ("plan", format!("outputs/work/plans/{id}.json")),
        ("task", format!("outputs/work/tasks/{id}/index.json")),
        ("execution", format!("outputs/work/executions/{id}")),
    ]
}

pub trait PlanPreparedOutput {
    fn create_prepared_output(&self, path: &str, content: &[u8]) -> Result<(), WorkError>;
}

pub struct PlanValidationInput<'a> {
    pub raw: &'a [u8],
    pub actual_plan_path: &'a str,
    pub allow_task_index: bool,
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn text<'a>(value: &'a Value, location: &str) -> Result<&'a str, WorkError> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location": location}),
            )
        })
}

fn strings(value: &Value, location: &str) -> Result<Vec<String>, WorkError> {
    let values = value.as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_string_array",
            "A string array is required.",
            json!({"location": location}),
        )
    })?;
    values
        .iter()
        .map(|value| text(value, location).map(str::to_owned))
        .collect()
}

fn check_semantic_request(value: &Value) -> Result<(), WorkError> {
    let required = [
        "requirement_id",
        "title",
        "summary",
        "goals",
        "scope",
        "deliverables",
        "acceptance_criteria",
        "hierarchy_selection_request",
        "skill_selection_request",
        "references",
    ];
    let Some(object) = value.as_object() else {
        return Err(error(
            ExitCode::Contract,
            "expected_object",
            "A JSON object is required.",
            json!({"location": "plan_semantic_request"}),
        ));
    };
    let mut missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    missing.sort_unstable();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(error(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location": "plan_semantic_request", "missing": missing, "unknown": unknown}),
        ));
    }
    Ok(())
}

pub fn validate_plan<H, S, P>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    plan: &Value,
    input: PlanValidationInput<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let item_count = validate_plan_structure(plan).map_err(|issue| {
        error(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let requirement: RequirementId = plan["requirement_id"]
        .as_str()
        .unwrap_or("")
        .parse()
        .map_err(|issue: work_operations::identifiers::IdentifierIssue| {
            error(
                ExitCode::Contract,
                issue.reason_code(),
                "The requirement ID is invalid.",
                json!({}),
            )
        })?;
    paths.validate_paths(
        &requirement,
        &plan["artifacts"],
        input.actual_plan_path,
        input.allow_task_index,
    )?;
    validate_hierarchy_selection(hierarchy_repository, &plan["hierarchy_selection"])?;
    let selected_paths = strings(
        &plan["hierarchy_selection"]["selected_paths"],
        "hierarchy_selection.selected_paths",
    )?;
    let source_set = validate_work_selection_value(
        hierarchy_repository,
        "plan",
        &plan["work_instruction_selection"],
        &selected_paths,
        "work_instruction_selection",
    )?;
    validate_skill_selection(skill_repository, skill_roots, &plan["skill_selection"])?;
    let rendered = render_plan_value(plan).expect("JSON values serialize");
    if input.raw != rendered {
        return Err(error(
            ExitCode::Contract,
            "noncanonical_json_contract",
            "The JSON contract does not match the required canonical rendering.",
            json!({"source": input.actual_plan_path}),
        ));
    }
    let result = json!({"schema": "work-plan-validation/v1", "requirement_id": requirement.as_str(), "status": "confirmed", "plan_sha256": sha256_hex(input.raw), "hierarchy_selection_sha256": plan["hierarchy_selection"]["selection_sha256"], "work_instructions_sha256": source_set.instructions_sha256, "skill_selection_sha256": plan["skill_selection"]["selection_sha256"], "item_count": item_count});
    let _: work_model::plan::PlanValidation =
        serde_json::from_value(result.clone()).expect("validated Plan result matches its model");
    Ok(result)
}

pub fn prepare_semantic<H, S, P>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    request: &Value,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    check_semantic_request(request)?;
    let requirement: RequirementId = text(&request["requirement_id"], "requirement_id")?
        .parse()
        .map_err(|issue: work_operations::identifiers::IdentifierIssue| {
            error(
                ExitCode::Contract,
                issue.reason_code(),
                "The requirement ID is invalid.",
                json!({}),
            )
        })?;
    let title = text(&request["title"], "title")?;
    let summary = text(&request["summary"], "summary")?;
    let goals = strings(&request["goals"], "goals")?;
    let scope = strings(&request["scope"], "scope")?;
    let deliverables = strings(&request["deliverables"], "deliverables")?;
    let acceptances = strings(&request["acceptance_criteria"], "acceptance_criteria")?;
    let references = strings(&request["references"], "references")?;
    let hierarchy = build_hierarchy_selection(
        hierarchy_repository,
        &request["hierarchy_selection_request"],
    )?;
    let skill = build_skill_selection(
        skill_repository,
        skill_roots,
        &request["skill_selection_request"],
    )?;
    let _: work_model::plan::PlanSemanticRequest =
        serde_json::from_value(request.clone()).expect("validated Plan request matches its model");
    let selected_paths = strings(
        &hierarchy["selected_paths"],
        "hierarchy_selection.selected_paths",
    )?;
    let instructions = source_selection(&load_instructions(
        hierarchy_repository,
        "plan",
        &selected_paths,
        &references,
    )?);
    let artifacts = paths.default_paths(&requirement)?;
    let plan_path = artifacts["plan"].as_str().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_artifact_path",
            "Each artifact path must be a string.",
            json!({"field": "plan"}),
        )
    })?;
    if paths.exists(plan_path)? {
        return Err(error(
            ExitCode::WorkflowState,
            "plan_already_exists",
            "Initial preparation cannot revise an existing Plan.",
            json!({}),
        ));
    }
    let goal_ids: Vec<_> = (1..=goals.len())
        .map(|index| format!("GOAL-{index:03}"))
        .collect();
    let deliverable_ids: Vec<_> = (1..=deliverables.len())
        .map(|index| format!("DELIVERABLE-{index:03}"))
        .collect();
    let acceptance_ids: Vec<_> = (1..=acceptances.len())
        .map(|index| format!("ACCEPTANCE-{index:03}"))
        .collect();
    let plan = json!({
        "schema": "work-plan/v1", "requirement_id": requirement.as_str(), "status": "confirmed", "title": title, "summary": summary,
        "artifacts": artifacts, "hierarchy_selection": hierarchy, "work_instruction_selection": instructions, "skill_selection": skill,
        "goals": goals.iter().enumerate().map(|(index, statement)| json!({"id": goal_ids[index], "statement": statement})).collect::<Vec<_>>(),
        "scope": scope.iter().enumerate().map(|(index, statement)| json!({"id": format!("SCOPE-{:03}", index + 1), "kind": "in_scope", "statement": statement, "goal_ids": goal_ids})).collect::<Vec<_>>(),
        "deliverables": deliverables.iter().enumerate().map(|(index, statement)| json!({"id": deliverable_ids[index], "statement": statement, "goal_ids": goal_ids, "acceptance_ids": acceptance_ids})).collect::<Vec<_>>(),
        "acceptance_criteria": acceptances.iter().enumerate().map(|(index, statement)| json!({"id": acceptance_ids[index], "statement": statement, "deliverable_ids": deliverable_ids})).collect::<Vec<_>>(),
    });
    let rendered = render_plan_value(&plan).expect("JSON values serialize");
    let validation = validate_plan(
        hierarchy_repository,
        skill_repository,
        paths,
        skill_roots,
        &plan,
        PlanValidationInput {
            raw: &rendered,
            actual_plan_path: plan_path,
            allow_task_index: true,
        },
    )?;
    if paths.exists(plan_path)? {
        return Err(error(
            ExitCode::WorkflowState,
            "plan_prepare_target_changed",
            "The Plan target changed during preparation.",
            json!({}),
        ));
    }
    let result = json!({"schema": "work-plan-prepare/v1", "status": "prepared", "path": plan_path, "plan": plan, "validation": validation});
    let _: work_model::plan::PlanPrepare =
        serde_json::from_value(result.clone()).expect("prepared Plan matches its model");
    Ok(result)
}

pub fn prepare_semantic_to_file<H, S, P, O>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    output: &O,
    skill_roots: &[SkillRoot],
    request: &Value,
    output_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
    O: PlanPreparedOutput,
{
    let prepared = prepare_semantic(
        hierarchy_repository,
        skill_repository,
        paths,
        skill_roots,
        request,
    )?;
    let rendered = render_plan_value(&prepared["plan"]).expect("JSON values serialize");
    output.create_prepared_output(output_path, &rendered)?;
    Ok(prepared)
}

pub fn create_plan<H, S, P>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    plan: &Value,
    actual_plan_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let rendered = render_plan_value(plan).expect("JSON values serialize");
    let allow_task_index = plan["artifacts"]["task"]
        .as_str()
        .is_some_and(|path| path.ends_with("/index.json"));
    let validation = validate_plan(
        hierarchy_repository,
        skill_repository,
        paths,
        skill_roots,
        plan,
        PlanValidationInput {
            raw: &rendered,
            actual_plan_path,
            allow_task_index,
        },
    )?;
    if paths.exists(actual_plan_path)? {
        return Err(error(
            ExitCode::WorkflowState,
            "plan_already_exists",
            "The Plan target already exists; plan create never overwrites it.",
            json!({"path": actual_plan_path}),
        ));
    }
    paths.create_exclusive(actual_plan_path, &rendered)?;
    let stored = paths.read(actual_plan_path)?;
    if stored != rendered {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "plan_post_write_mismatch",
            "The stored Plan does not match the validated canonical Plan.",
            json!({"path": actual_plan_path}),
        ));
    }
    let mut result = validation;
    result["schema"] = json!("work-plan-create/v1");
    result["path"] = json!(actual_plan_path);
    let _: work_model::plan::PlanCreate =
        serde_json::from_value(result.clone()).expect("created Plan result matches its model");
    Ok(result)
}

fn parse_plan(raw: &[u8], source: &str) -> Result<Value, WorkError> {
    parse_json_contract(raw).map_err(|issue| match issue {
        JsonContractIssue::InvalidUtf8(offset) => error(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source": source, "byte_offset": offset}),
        ),
        JsonContractIssue::MultipleBom => error(
            ExitCode::InputFormat,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            json!({"source": source}),
        ),
        JsonContractIssue::DuplicateKey(key) => error(
            ExitCode::InputFormat,
            "duplicate_json_key",
            "The JSON contract contains a duplicate key.",
            json!({"key": key}),
        ),
        JsonContractIssue::InvalidConstant(value) => error(
            ExitCode::InputFormat,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            json!({"value": value}),
        ),
        JsonContractIssue::InvalidJson { line, column } => error(
            ExitCode::InputFormat,
            "invalid_json_contract",
            "The JSON contract is invalid.",
            json!({"line": line, "column": column}),
        ),
        JsonContractIssue::NotObject => error(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        ),
    })
}

pub fn validate_plan_bytes<H, S, P>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    raw: &[u8],
    actual_plan_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let plan = parse_plan(raw, actual_plan_path)?;
    let allow_task_index = plan["artifacts"]["task"]
        .as_str()
        .is_some_and(|path| path.ends_with("/index.json"));
    validate_plan(
        hierarchy_repository,
        skill_repository,
        paths,
        skill_roots,
        &plan,
        PlanValidationInput {
            raw,
            actual_plan_path,
            allow_task_index,
        },
    )
}

pub fn validate_plan_file<H, S, P>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    relative_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let raw = paths.read(relative_path)?;
    validate_plan_bytes(
        hierarchy_repository,
        skill_repository,
        paths,
        skill_roots,
        &raw,
        relative_path,
    )
}

pub fn create_plan_bytes<H, S, P>(
    hierarchy_repository: &H,
    skill_repository: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    raw: &[u8],
    actual_plan_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let plan = parse_plan(raw, actual_plan_path)?;
    create_plan(
        hierarchy_repository,
        skill_repository,
        paths,
        skill_roots,
        &plan,
        actual_plan_path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_plan_parser_rejects_nested_duplicate_keys() {
        assert_eq!(
            parse_plan(b"{\"goals\": [{\"id\": 1, \"id\": 2}]}", "test")
                .unwrap_err()
                .reason_code,
            "duplicate_json_key"
        );
        assert_eq!(
            parse_plan(b"[]", "test").unwrap_err().reason_code,
            "json_contract_not_object"
        );
    }
}
