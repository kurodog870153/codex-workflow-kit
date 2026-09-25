//! Task command flows.

use serde_json::Value;
use serde_json::json;
use work_feature::error::ExitCode;
use work_feature::error::WorkError;
use work_feature::instruction::InstructionSourceRepository;
use work_feature::plan::PlanPathRepository;
use work_feature::skill::{SkillRoot, SkillSnapshotRepository};
pub use work_feature::task::TaskCollectionRepository;
pub use work_feature::task::assembly::ProjectAssemblyInput;
use work_feature::task::assembly::{
    TaskAssemblyRepository, approved_contract, assemble_from_repository, render_approved_contract,
};
pub use work_feature::task::create::TaskCreateProjectInput;
use work_feature::task::create::{TaskCreationRepository, create_task_from_project};

pub fn create_task<H, S, P, T>(
    instructions: &H,
    skills: &S,
    paths: &P,
    storage: &T,
    skill_roots: &[SkillRoot],
    request: TaskCreateProjectInput<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
    T: TaskCreationRepository,
{
    create_task_from_project(instructions, skills, paths, storage, skill_roots, request)
}

pub fn assemble_task<R, H, S, P>(
    repository: &R,
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    request: ProjectAssemblyInput<'_>,
) -> Result<Value, WorkError>
where
    R: TaskAssemblyRepository,
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    assemble_from_repository(
        repository,
        instructions,
        skills,
        paths,
        skill_roots,
        request,
    )
}

pub struct DraftCreatePorts<'a, R, H, S, P, T> {
    pub repository: &'a R,
    pub instructions: &'a H,
    pub skills: &'a S,
    pub paths: &'a P,
    pub storage: &'a T,
    pub skill_roots: &'a [SkillRoot],
}

pub fn create_from_drafts<R, H, S, P, T>(
    ports: DraftCreatePorts<'_, R, H, S, P, T>,
    request: ProjectAssemblyInput<'_>,
    approved_sha256: &str,
) -> Result<Value, WorkError>
where
    R: TaskAssemblyRepository,
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
    T: TaskCreationRepository,
{
    let assembled = assemble_task(
        ports.repository,
        ports.instructions,
        ports.skills,
        ports.paths,
        ports.skill_roots,
        request,
    )?;
    let contract = approved_contract(&assembled, approved_sha256)?;
    let raw = render_approved_contract(&assembled, approved_sha256)?;
    let artifacts = &contract["artifacts"];
    let mut created = create_task(
        ports.instructions,
        ports.skills,
        ports.paths,
        ports.storage,
        ports.skill_roots,
        TaskCreateProjectInput {
            raw: &raw,
            plan_path: artifacts["plan"].as_str().unwrap(),
            task_path: artifacts["task"].as_str().unwrap(),
            execution_dir: artifacts["execution"].as_str().unwrap(),
            recovery: false,
        },
    )?;
    created["approval_sha256"] = serde_json::json!(approved_sha256);
    Ok(created)
}

pub fn diagnose(report: Value) -> Result<Value, WorkError> {
    if report["normal_use_allowed"] == false {
        Err(WorkError::new(
            ExitCode::Contract,
            "task_diagnostics_failed",
            "TASK validation is blocked; review the diagnostic report.",
            report,
        ))
    } else {
        Ok(report)
    }
}

pub fn repair(repair: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    repair()
}

pub fn repair_prepare(
    prepare: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    prepare()
}

pub fn validate_collection<H, S, P, R>(
    instructions: &H,
    skills: &S,
    paths: &P,
    repository: &R,
    skill_roots: &[SkillRoot],
    index_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
    R: work_feature::task::TaskCollectionRepository,
{
    work_feature::task::load_collection_with_file_state(
        instructions,
        skills,
        paths,
        repository,
        skill_roots,
        index_path,
        true,
    )
}

pub struct MigrationPrepareInput<'a> {
    pub raw: &'a [u8],
    pub request: &'a Value,
    pub date: &'a str,
    pub output: Option<&'a str>,
}

pub fn migration_prepare(
    input: MigrationPrepareInput<'_>,
    prepare_revision: impl FnOnce(&[u8], &str) -> Result<Value, WorkError>,
    prepare_reconstruction: impl FnOnce(&[u8]) -> Result<Value, WorkError>,
    preview: impl FnOnce(&Value) -> Result<Value, WorkError>,
    write: impl FnOnce(&str, &Value) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    let prepared = if input.request["mode"] == "revision" {
        prepare_revision(input.raw, input.date)?
    } else {
        prepare_reconstruction(input.raw)?
    };
    let checked = preview(&prepared)?;
    if let Some(path) = input.output {
        write(path, &prepared)?;
    }
    Ok(json!({"request":prepared,"preview":checked,"output_file":input.output}))
}

#[cfg(test)]
mod migration_flow_tests {
    use std::cell::RefCell;

    use serde_json::json;

    use super::{MigrationPrepareInput, migration_prepare};

    #[test]
    fn revision_previews_before_writing_and_skips_reconstruction() {
        let order = RefCell::new(Vec::new());
        let result = migration_prepare(
            MigrationPrepareInput {
                raw: b"request",
                request: &json!({"mode":"revision"}),
                date: "2026-09-27",
                output: Some("draft.json"),
            },
            |raw, date| {
                assert_eq!(raw, b"request");
                assert_eq!(date, "2026-09-27");
                order.borrow_mut().push("prepare");
                Ok(json!({"candidate":1}))
            },
            |_| panic!("revision must not reconstruct"),
            |candidate| {
                assert_eq!(candidate, &json!({"candidate":1}));
                order.borrow_mut().push("preview");
                Ok(json!({"ready":true}))
            },
            |path, candidate| {
                assert_eq!(path, "draft.json");
                assert_eq!(candidate, &json!({"candidate":1}));
                order.borrow_mut().push("write");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*order.borrow(), ["prepare", "preview", "write"]);
        assert_eq!(result["preview"], json!({"ready":true}));
    }
}
