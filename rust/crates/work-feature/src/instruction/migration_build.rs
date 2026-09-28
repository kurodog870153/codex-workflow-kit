//! Instruction router migration over artifact and routing ports.

use crate::error::{ExitCode, WorkError};
use crate::instruction::migration::decide_migration;
use crate::instruction::migration_manifest;
use crate::workflow::WorkflowRoutingRepository;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::str::FromStr;
use work_model::task::index::TaskItemReference;
use work_operations::canonical::{canonical_sha256, parse_json_contract};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::identifiers::RequirementId;
use work_operations::plan::render_plan_value;
use work_operations::task::collection::collection_fingerprint_sha256;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

pub trait MigrationSnapshotRepository {
    fn default_paths(&self, id: &RequirementId) -> Vec<(String, String)>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
}

pub struct MigrationCandidate {
    pub preview: Value,
    pub before: BTreeMap<String, Vec<u8>>,
    pub after: BTreeMap<String, Vec<u8>>,
    pub artifacts: Value,
}

fn failure(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn read(
    repository: &impl MigrationSnapshotRepository,
    relative: &str,
) -> Result<(Vec<u8>, Value), WorkError> {
    let raw = repository.read(relative)?;
    let value = parse_json_contract(&raw).map_err(|_| {
        failure(
            "invalid_json_contract",
            "A formal artifact is not valid JSON.",
        )
    })?;
    Ok((raw, value))
}

fn render(value: &Value, kind: &str) -> Result<Vec<u8>, WorkError> {
    let bytes = match kind {
        "plan" => render_plan_value(value),
        "item" => render_task(value, TaskDocumentKind::Item),
        "index" => render_task(value, TaskDocumentKind::Index),
        "execution" => render_execution_index(value),
        _ => unreachable!(),
    };
    bytes.map_err(|_| {
        failure(
            "invalid_contract_value",
            "The migrated artifact cannot be rendered.",
        )
    })
}

fn hash(raw: &[u8]) -> Result<String, WorkError> {
    canonical_sha256(raw).map_err(|_| failure("invalid_utf8", "A formal artifact is not UTF-8."))
}

fn set_manifest(value: &mut Value, key: &str, manifest: Value) -> bool {
    if value[key]["routing_manifest"] == manifest {
        return false;
    }
    value[key]["routing_manifest"] = manifest;
    true
}

pub fn build_migration(
    repository: &impl MigrationSnapshotRepository,
    routing: &mut impl WorkflowRoutingRepository,
    requirement_id: &str,
) -> Result<MigrationCandidate, WorkError> {
    let id = RequirementId::from_str(requirement_id)
        .map_err(|_| failure("invalid_requirement_id", "The requirement ID is invalid."))?;
    let paths = repository.default_paths(&id);
    let artifacts = json!({"plan":paths[0].1,"task":paths[1].1,"execution":paths[2].1});
    let plan_relative = paths[0].1.as_str();
    let task_relative = paths[1].1.as_str();
    let execution_relative = format!("{}/index.json", paths[2].1);
    if !repository.exists(plan_relative)? {
        return Err(failure(
            "instruction_migration_plan_missing",
            "The requirement Plan does not exist.",
        ));
    }
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    let mut excluded = Vec::new();
    let mut counts = json!({"plans":0,"task_items":0,"task_indexes":0,"execution_indexes":0});

    let (plan_raw, mut plan) = read(repository, plan_relative)?;
    let manifest = migration_manifest(routing, "plan", "plan_confirmed", "prepare_plan", &plan)?;
    if set_manifest(&mut plan, "work_instruction_selection", manifest) {
        before.insert(plan_relative.to_owned(), plan_raw.clone());
        after.insert(plan_relative.to_owned(), render(&plan, "plan")?);
        counts["plans"] = json!(1);
    }
    let effective_plan = after
        .get(plan_relative)
        .cloned()
        .unwrap_or_else(|| plan_raw.clone());

    if repository.exists(task_relative)? {
        let (index_raw, mut index) = read(repository, task_relative)?;
        let base = task_relative.rsplit_once('/').map_or("", |(base, _)| base);
        let mut item_paths = BTreeMap::new();
        let references = index["tasks"]
            .as_array()
            .ok_or_else(|| {
                failure(
                    "invalid_task_index",
                    "The TASK index has no task references.",
                )
            })?
            .clone();
        for reference in &references {
            let task_id = reference["id"]
                .as_str()
                .ok_or_else(|| failure("invalid_task_index", "A TASK reference has no ID."))?;
            let item_path = reference["path"]
                .as_str()
                .ok_or_else(|| failure("invalid_task_index", "A TASK reference has no path."))?;
            let relative = format!("{base}/{item_path}");
            let (item_raw, mut item) = read(repository, &relative)?;
            let manifest =
                migration_manifest(routing, "task", "task_confirmed", "choose_task", &item)?;
            if set_manifest(&mut item, "instruction_selection", manifest) {
                before.insert(relative.clone(), item_raw);
                after.insert(relative.clone(), render(&item, "item")?);
                counts["task_items"] = json!(counts["task_items"].as_u64().unwrap() + 1);
            }
            item_paths.insert(task_id.to_owned(), relative);
        }
        let manifest =
            migration_manifest(routing, "task", "task_confirmed", "confirm_review", &index)?;
        if set_manifest(&mut index, "instruction_selection", manifest)
            || counts["plans"] != 0
            || counts["task_items"] != 0
        {
            index["source_plan"]["canonical_sha256"] = json!(hash(&effective_plan)?);
            for reference in index["tasks"].as_array_mut().ok_or_else(|| {
                failure(
                    "invalid_task_index",
                    "The TASK index has no task references.",
                )
            })? {
                let task_id = reference["id"]
                    .as_str()
                    .ok_or_else(|| failure("invalid_task_index", "A TASK reference has no ID."))?;
                let relative = &item_paths[task_id];
                let raw = if let Some(raw) = after.get(relative) {
                    raw.clone()
                } else {
                    read(repository, relative)?.0
                };
                reference["canonical_sha256"] = json!(hash(&raw)?);
            }
            before.insert(task_relative.to_owned(), index_raw.clone());
            after.insert(task_relative.to_owned(), render(&index, "index")?);
            counts["task_indexes"] = json!(1);
        }
        if repository.exists(&execution_relative)? {
            let (execution_raw, mut execution) = read(repository, &execution_relative)?;
            validate_execution_index(&execution, &execution_raw).map_err(|issue| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            })?;
            if !execution["lock"].is_null()
                || execution["tasks"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|row| row["status"] == "in_progress"))
            {
                excluded
                    .push(json!({"path":execution_relative,"reason":"active_attempt_snapshot"}));
            } else {
                let manifest = migration_manifest(
                    routing,
                    "execute",
                    "execution_bound",
                    "select_task_for_execution",
                    &execution,
                )?;
                execution["instruction_selection_manifest"] = manifest;
                if let Some(index_new) = after.get(task_relative) {
                    let index_sha = hash(index_new)?;
                    execution["task_index_sha256"] = json!(index_sha);
                    execution["task_collection_sha256"] = json!(collection_fingerprint_sha256(
                        &index_sha,
                        &index["tasks"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|reference| serde_json::from_value::<TaskItemReference>(
                                reference.clone()
                            )
                            .expect("validated TASK reference"))
                            .collect::<Vec<_>>()
                    ));
                    for reference in index["tasks"].as_array().unwrap() {
                        let task_id = &reference["id"];
                        if let Some(row) = execution["tasks"]
                            .as_array_mut()
                            .and_then(|rows| rows.iter_mut().find(|row| row["id"] == *task_id))
                        {
                            row["task_item_sha256"] = reference["canonical_sha256"].clone();
                        }
                    }
                }
                let candidate = render(&execution, "execution")?;
                if candidate != execution_raw {
                    before.insert(execution_relative.clone(), execution_raw);
                    after.insert(execution_relative.clone(), candidate);
                    counts["execution_indexes"] = json!(1);
                }
            }
        }
    }
    let decision = decide_migration(requirement_id, before, after, excluded, counts)?;
    Ok(MigrationCandidate {
        preview: decision.preview,
        before: decision.before,
        after: decision.after,
        artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_operations::routing::RoutingRequest;

    struct MissingPlan;

    impl MigrationSnapshotRepository for MissingPlan {
        fn default_paths(&self, _: &RequirementId) -> Vec<(String, String)> {
            vec![
                ("plan".into(), "plan.json".into()),
                ("task".into(), "task/index.json".into()),
                ("execution".into(), "execution".into()),
            ]
        }

        fn exists(&self, relative: &str) -> Result<bool, WorkError> {
            assert_eq!(relative, "plan.json");
            Ok(false)
        }

        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("missing Plan must stop before any artifact read")
        }
    }

    struct UnusedRouting;

    impl WorkflowRoutingRepository for UnusedRouting {
        fn route(&mut self, _: &RoutingRequest<'_>) -> Result<Value, WorkError> {
            panic!("missing Plan must stop before routing")
        }
    }

    #[test]
    fn missing_plan_stops_before_read_or_routing_ports() {
        let error = build_migration(&MissingPlan, &mut UnusedRouting, "example")
            .err()
            .expect("missing Plan must fail");
        assert_eq!(error.reason_code, "instruction_migration_plan_missing");
    }
}
