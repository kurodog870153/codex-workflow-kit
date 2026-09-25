//! Skill selection command flows.

use std::collections::HashSet;

use serde_json::Value;
use work_feature::error::WorkError;
pub use work_feature::skill::SkillRoot;
use work_feature::skill::{SkillCatalogRepository, SkillSnapshotRepository};

pub fn catalog(
    repository: &impl SkillCatalogRepository,
    disabled_sources: &HashSet<String>,
    excluded_names: &HashSet<String>,
) -> Result<Value, WorkError> {
    work_feature::skill::catalog(repository, disabled_sources, excluded_names)
}

pub fn snapshot(
    repository: &impl SkillSnapshotRepository,
    scope: &str,
    locator: &str,
    source: &str,
) -> Result<Value, WorkError> {
    repository.snapshot(scope, locator, source)
}

pub fn selection_build(
    repository: &impl SkillSnapshotRepository,
    roots: &[SkillRoot],
    request: &Value,
) -> Result<Value, WorkError> {
    work_feature::skill::build_selection(repository, roots, request)
}

pub fn selection_validate(
    repository: &impl SkillSnapshotRepository,
    roots: &[SkillRoot],
    request: &Value,
) -> Result<Value, WorkError> {
    work_feature::skill::validate_selection(repository, roots, request)
}
