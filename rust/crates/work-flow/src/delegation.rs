//! Delegation command flows.

use serde_json::{Value, json};
use work_feature::delegation::{self, DelegationSourceRepository};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::PlanPathRepository;

fn unknown_role() -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        "delegation_boundary_mismatch",
        "Unknown delegation role.",
        json!({}),
    )
}

pub fn build(
    source: &impl DelegationSourceRepository,
    paths: &impl PlanPathRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    match request["role"].as_str().unwrap_or("") {
        "plan" | "task-coordinator" => delegation::build_plan_role(source, request),
        "task-skill" => delegation::build_task_skill(source, request),
        "execute" => delegation::build_execute_role(source, request),
        "progress-saver" => delegation::build_progress_saver(source, request),
        "artifact-editor" => delegation::build_artifact_editor(source, paths, request),
        _ => Err(unknown_role()),
    }
}

pub fn validate(
    source: &impl DelegationSourceRepository,
    paths: &impl PlanPathRepository,
    request: &Value,
    role: &str,
    sender: &str,
) -> Result<Value, WorkError> {
    if matches!(role, "plan" | "task-coordinator") {
        delegation::validate_plan_role_name(role)?;
    }
    if !matches!(
        role,
        "plan"
            | "task-coordinator"
            | "task-skill"
            | "execute"
            | "progress-saver"
            | "artifact-editor"
    ) {
        return Err(unknown_role());
    }
    let project_root = source.canonical_project_root()?;
    let skill_root = source.canonical_skill_root()?;
    match role {
        "plan" | "task-coordinator" => {
            delegation::validate_plan_role(request, role, sender, &project_root, &skill_root)
        }
        "task-skill" => {
            delegation::validate_task_skill(request, sender, &project_root, &skill_root)
        }
        "execute" => delegation::validate_execute_role(request, sender, &project_root, &skill_root),
        "progress-saver" => {
            delegation::validate_progress_saver(request, sender, &project_root, &skill_root)
        }
        "artifact-editor" => {
            delegation::validate_artifact_editor(paths, request, sender, &project_root, &skill_root)
        }
        _ => unreachable!("known role"),
    }
}
