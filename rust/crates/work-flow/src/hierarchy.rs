//! Hierarchy command flows.

use std::path::Path;

use serde_json::Value;
use work_feature::error::WorkError;
use work_feature::hierarchy::{HierarchyCatalogRepository, build_selection, validate_selection};

pub fn resolve(
    mode: &str,
    selected_paths: &[String],
    project_root: &Path,
) -> Result<Value, WorkError> {
    work_feature::hierarchy::resolve(mode, selected_paths, project_root)
}

pub fn selection_build(
    catalog: &impl HierarchyCatalogRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    build_selection(catalog, request)
}

pub fn selection_validate(
    catalog: &impl HierarchyCatalogRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    validate_selection(catalog, request)
}
