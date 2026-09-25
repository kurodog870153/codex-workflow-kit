//! Plan command flows.

use serde_json::Value;
use work_feature::error::WorkError;
use work_feature::instruction::InstructionSourceRepository;
use work_feature::plan::{PlanPathRepository, PlanPreparedOutput};
use work_feature::skill::{SkillRoot, SkillSnapshotRepository};

pub fn semantic_prepare<H, S, P, O>(
    hierarchy: &H,
    skills: &S,
    paths: &P,
    output: &O,
    roots: &[SkillRoot],
    request: &Value,
    output_path: Option<&str>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
    O: PlanPreparedOutput,
{
    if let Some(path) = output_path {
        work_feature::plan::prepare_semantic_to_file(
            hierarchy, skills, paths, output, roots, request, path,
        )
    } else {
        work_feature::plan::prepare_semantic(hierarchy, skills, paths, roots, request)
    }
}

pub fn validate_bytes<H, S, P>(
    hierarchy: &H,
    skills: &S,
    paths: &P,
    roots: &[SkillRoot],
    raw: &[u8],
    plan_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    work_feature::plan::validate_plan_bytes(hierarchy, skills, paths, roots, raw, plan_path)
}

pub fn validate_file<H, S, P>(
    hierarchy: &H,
    skills: &S,
    paths: &P,
    roots: &[SkillRoot],
    plan_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    work_feature::plan::validate_plan_file(hierarchy, skills, paths, roots, plan_path)
}

pub fn create<H, S, P>(
    hierarchy: &H,
    skills: &S,
    paths: &P,
    roots: &[SkillRoot],
    raw: &[u8],
    plan_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    work_feature::plan::create_plan_bytes(hierarchy, skills, paths, roots, raw, plan_path)
}
