//! Instruction command flows.

use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::WorkError;
use work_feature::instruction::{InstructionCatalogRepository, InstructionSourceRepository};

pub fn catalog(
    repository: &impl InstructionCatalogRepository,
    mode: &str,
) -> Result<Value, WorkError> {
    work_feature::instruction::catalog(repository, mode)
}

pub fn resolve(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    selected_paths: &[String],
    project_root: &Path,
) -> Result<Value, WorkError> {
    let hierarchy = work_feature::instruction::resolve_hierarchy(repository, mode, selected_paths)?;
    let mut result = serde_json::to_value(hierarchy).expect("hierarchy serializes");
    result["project_root"] = json!(project_root);
    Ok(result)
}

pub fn load(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    selected_paths: &[String],
    references: &[String],
) -> Result<Value, WorkError> {
    let source_set = work_feature::instruction::load(repository, mode, selected_paths, references)?;
    let sources: Vec<_> = source_set
        .sources
        .iter()
        .map(|source| &source.summary)
        .collect();
    Ok(work_model::instruction::verified::<
        work_model::instruction::InstructionsResponse,
    >(
        json!({"schema":"work-instructions/v1","mode":source_set.mode,
        "hierarchy":source_set.hierarchy,"sources":sources,
        "references":source_set.references,
        "instructions_sha256":source_set.instructions_sha256}),
    ))
}

pub fn select(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    selected_paths: &[String],
    references: &[String],
) -> Result<Value, WorkError> {
    let selection =
        work_feature::instruction::select(repository, mode, selected_paths, references)?;
    Ok(work_model::instruction::verified::<
        work_model::instruction::InstructionSelectionResponse,
    >(
        json!({"schema":"work-instruction-selection/v1","mode":mode,
        "instruction_selection":selection}),
    ))
}

pub fn impact(read: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    read()
}

pub fn task_select(
    repository: &impl InstructionSourceRepository,
    confirmed_hierarchy: &Value,
    selected_paths: &[String],
    references: &[String],
) -> Result<Value, WorkError> {
    let selection = work_feature::instruction::select_task(
        repository,
        confirmed_hierarchy,
        selected_paths,
        references,
    )?;
    Ok(work_model::instruction::verified::<
        work_model::instruction::InstructionSelectionResponse,
    >(
        json!({"schema":"work-instruction-selection/v1","mode":"task",
        "instruction_selection":selection}),
    ))
}

pub fn refresh_preview(
    preview: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    preview()
}

pub fn refresh_apply(apply: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    apply()
}

pub fn refresh_preview_all(
    preview: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    preview()
}

pub fn refresh_apply_all(
    apply: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    apply()
}

pub fn migration_preview(
    preview: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    preview()
}

pub fn migration_apply(
    apply: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    apply()
}
