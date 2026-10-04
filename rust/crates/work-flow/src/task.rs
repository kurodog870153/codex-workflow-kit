//! Task command flows.

use serde_json::{Value, json};
use work_feature::artifact_paths::ArtifactPathRepository;
use work_feature::error::WorkError;
use work_feature::instruction::InstructionSourceRepository;
use work_feature::ports::SourceSnapshotReader;
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
    P: ArtifactPathRepository + SourceSnapshotReader,
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
    P: ArtifactPathRepository + SourceSnapshotReader,
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
    recovery: bool,
) -> Result<Value, WorkError>
where
    R: TaskAssemblyRepository,
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
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
            source_root: artifacts["source"].as_str().unwrap(),
            task_path: artifacts["task"].as_str().unwrap(),
            execution_dir: artifacts["execution"].as_str().unwrap(),
            recovery,
        },
    )?;
    created["approval_sha256"] = serde_json::json!(approved_sha256);
    Ok(created)
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
    P: ArtifactPathRepository + SourceSnapshotReader,
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
    work_feature::specification::migration_prepare::validate_semantic_request(input.request)?;
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
        let request = json!({"schema":"work-spec-migration-prepare-request", "mode":"revision",
            "requirement_id":"example", "sources":[{"path":"legacy.bin","raw_sha256":"a".repeat(64)}],
            "reason":"Reviewed revision", "edits":[], "semantic_decisions":[]});
        let result = migration_prepare(
            MigrationPrepareInput {
                raw: b"request",
                request: &request,
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
    #[test]
    fn unreviewed_migration_is_rejected_before_any_callback() {
        let error = migration_prepare(
            MigrationPrepareInput { raw:b"{}", request:&json!({"schema":"work-spec-migration-prepare-request", "mode":"reconstruction", "plan":{}}), date:"2026-10-04", output:Some("candidate.json") },
            |_, _| panic!("invalid request must not prepare"),
            |_| panic!("invalid request must not reconstruct"),
            |_| panic!("invalid request must not preview"),
            |_, _| panic!("invalid request must not write"),
        ).unwrap_err();
        assert_eq!(error.reason_code, "invalid_contract_value");
    }
}
