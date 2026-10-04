//! Instruction refresh candidate construction over source and artifact ports.

use crate::error::{ExitCode, WorkError};
use crate::instruction::refresh::decide_refresh;
use crate::instruction::{
    InstructionSourceRepository, load, migration_manifest, task_document_selection,
};
use crate::workflow::WorkflowRoutingRepository;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::graph::{rebind_execution_index, rebind_task_index};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::identifiers::RequirementId;
use work_operations::instruction::{SourceSet, selection as source_selection};
use work_operations::instruction_refresh::{Compatibility, combined_compatibility};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

pub trait RefreshSnapshotRepository {
    fn discover_requirements(&self) -> Result<BTreeMap<String, Value>, WorkError>;
    fn default_paths(&self, id: &RequirementId) -> work_model::task::source::TaskArtifactPaths;
    fn source_evidence(&self, index: &Value) -> Result<BTreeMap<String, Vec<u8>>, WorkError>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
}

pub trait RefreshRoutingRepository: WorkflowRoutingRepository {
    fn recheck_sources(&self) -> Result<(), WorkError>;
}

pub struct RefreshCandidate {
    pub preview: Value,
    pub before: BTreeMap<String, Vec<u8>>,
    pub after: BTreeMap<String, Vec<u8>>,
    pub artifacts: Value,
    pub source_evidence: BTreeMap<String, Vec<u8>>,
    pub changed_source_names: BTreeSet<String>,
}

fn failure(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn read(
    repository: &impl RefreshSnapshotRepository,
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
        "item" => render_task(value, TaskDocumentKind::Item),
        "index" => render_task(value, TaskDocumentKind::Index),
        "execution" => render_execution_index(value),
        _ => unreachable!(),
    };
    bytes.map_err(|_| {
        failure(
            "invalid_contract_value",
            "The refreshed artifact cannot be rendered.",
        )
    })
}

fn strings(value: &Value) -> Result<Vec<String>, WorkError> {
    value
        .as_array()
        .ok_or_else(|| failure("invalid_string_array", "An instruction list is invalid."))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| failure("invalid_string_array", "An instruction list is invalid."))
        })
        .collect()
}

fn current_selection(
    source: &impl InstructionSourceRepository,
    mode: &str,
    stored: &Value,
    snapshots: &mut Vec<(String, Vec<String>, Vec<String>, String)>,
) -> Result<(Value, SourceSet), WorkError> {
    let paths = strings(&stored["selected_paths"])?;
    let refs = strings(&stored["references"])?;
    let loaded = load(source, mode, &paths, &refs)?;
    snapshots.push((mode.into(), paths, refs, loaded.instructions_sha256.clone()));
    let selection = serde_json::to_value(source_selection(&loaded)).map_err(|_| {
        failure(
            "invalid_contract_value",
            "The instruction selection cannot be serialized.",
        )
    })?;
    Ok((selection, loaded))
}

pub fn build_refresh(
    repository: &impl RefreshSnapshotRepository,
    source: &impl InstructionSourceRepository,
    routing: &mut impl RefreshRoutingRepository,
    requirement_id: &str,
) -> Result<RefreshCandidate, WorkError> {
    let id = RequirementId::from_str(requirement_id)
        .map_err(|_| failure("invalid_requirement_id", "The requirement ID is invalid."))?;
    let discovered = repository.discover_requirements()?;
    let artifacts = discovered.get(requirement_id).cloned().unwrap_or_else(|| {
        let paths = repository.default_paths(&id);
        json!(paths)
    });
    let task_relative = artifacts["task"]
        .as_str()
        .ok_or_else(|| failure("invalid_artifact_paths", "The TASK path is invalid."))?;
    let execution_dir = artifacts["execution"]
        .as_str()
        .ok_or_else(|| failure("invalid_artifact_paths", "The execution path is invalid."))?;
    let execution_relative = format!("{execution_dir}/index.json");
    if !repository.exists(task_relative)? {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_refresh_task_missing",
            "The formal Task collection does not exist.",
            json!({"requirement_id":requirement_id}),
        ));
    }
    let mut snapshots = Vec::new();
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    let mut blocked = Vec::new();
    let mut changed_names = BTreeSet::new();
    let mut counts = json!({"task_items":0,"task_indexes":0,"execution_indexes":0});

    let (_, baseline_index) = read(repository, task_relative)?;
    let source_evidence = repository.source_evidence(&baseline_index)?;

    if repository.exists(task_relative)? {
        let (index_raw, mut index) = read(repository, task_relative)?;
        let base = task_relative.rsplit_once('/').map_or("", |(base, _)| base);
        let references = index["tasks"]
            .as_array()
            .ok_or_else(|| {
                failure(
                    "invalid_task_index",
                    "The TASK index has no task references.",
                )
            })?
            .clone();
        let mut item_paths = BTreeMap::new();
        let mut loaded_sets = Vec::new();
        for reference in &references {
            let task_id = reference["id"]
                .as_str()
                .ok_or_else(|| failure("invalid_task_index", "A TASK reference has no ID."))?;
            let item_path = reference["path"]
                .as_str()
                .ok_or_else(|| failure("invalid_task_index", "A TASK reference has no path."))?;
            let relative = format!("{base}/{item_path}");
            let (item_raw, mut item) = read(repository, &relative)?;
            let stored = &item["instruction_selection"];
            let (mut current, loaded) = current_selection(source, "task", stored, &mut snapshots)?;
            loaded_sets.push(loaded);
            current["routing_manifest"] =
                migration_manifest(routing, "task", "task_confirmed", "choose_task", &item)?;
            let (state, changed) = combined_compatibility(
                stored,
                current["sources"].as_array().unwrap(),
                &current["routing_manifest"],
            );
            changed_names.extend(changed);
            if state == Compatibility::ReviewRequired {
                blocked.push(json!({"path":relative,"reason":"compatibility_revision_changed"}));
            } else if state == Compatibility::Refreshable {
                item["instruction_selection"] = current;
                before.insert(relative.clone(), item_raw);
                after.insert(relative.clone(), render(&item, "item")?);
                counts["task_items"] = json!(counts["task_items"].as_u64().unwrap() + 1);
            }
            item_paths.insert(task_id.to_owned(), relative);
        }
        let mut current = task_document_selection(&loaded_sets)?;
        current["routing_manifest"] =
            migration_manifest(routing, "task", "task_confirmed", "confirm_review", &index)?;
        let (state, changed) = combined_compatibility(
            &index["instruction_selection"],
            current["sources"].as_array().unwrap(),
            &current["routing_manifest"],
        );
        changed_names.extend(changed);
        if state == Compatibility::ReviewRequired {
            blocked.push(json!({"path":task_relative,"reason":"routing_manifest_changed"}));
        }
        if blocked.is_empty() && (!after.is_empty() || state == Compatibility::Refreshable) {
            index["instruction_selection"] = current;
            let item_bytes = item_paths
                .iter()
                .map(|(task_id, relative)| {
                    Ok((
                        task_id.clone(),
                        if let Some(raw) = after.get(relative) {
                            raw.clone()
                        } else {
                            read(repository, relative)?.0
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, WorkError>>()?;
            let index_bytes = rebind_task_index(&mut index, &item_bytes).map_err(|issue| {
                failure(issue.reason_code(), "TASK bindings cannot be derived.")
            })?;
            before.insert(task_relative.to_owned(), index_raw);
            after.insert(task_relative.to_owned(), index_bytes);
            counts["task_indexes"] = json!(1);
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
                    blocked.push(
                        json!({"path":execution_relative,"reason":"active_attempt_snapshot"}),
                    );
                } else {
                    execution["instruction_selection_manifest"] = migration_manifest(
                        routing,
                        "execute",
                        "execution_bound",
                        "select_task_for_execution",
                        &execution,
                    )?;
                    rebind_execution_index(
                        &after[task_relative],
                        &index,
                        &item_bytes,
                        &mut execution,
                    )
                    .map_err(|issue| {
                        failure(issue.reason_code(), "Execute bindings cannot be derived.")
                    })?;
                    before.insert(execution_relative.clone(), execution_raw);
                    after.insert(execution_relative.clone(), render(&execution, "execution")?);
                    counts["execution_indexes"] = json!(1);
                }
            }
        }
    }
    let decision = decide_refresh(
        requirement_id,
        before,
        after,
        blocked,
        &changed_names,
        counts,
        &source_evidence,
    )?;
    if repository.source_evidence(&baseline_index)? != source_evidence {
        return Err(failure(
            "source_refresh_source_changed",
            "The immutable requirement Source changed during validation.",
        ));
    }
    routing.recheck_sources()?;
    for (mode, paths, refs, digest) in snapshots {
        if load(source, &mode, &paths, &refs)?.instructions_sha256 != digest {
            return Err(failure(
                "instruction_source_changed",
                "An instruction source changed during validation.",
            ));
        }
    }
    Ok(RefreshCandidate {
        preview: decision.preview,
        before: decision.before,
        after: decision.after,
        artifacts,
        source_evidence,
        changed_source_names: changed_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy::HierarchyCatalogRepository;
    use work_operations::hierarchy::{CrossModeCatalog, Hierarchy};
    use work_operations::routing::RoutingRequest;

    struct MissingTask;

    impl RefreshSnapshotRepository for MissingTask {
        fn discover_requirements(&self) -> Result<BTreeMap<String, Value>, WorkError> {
            Ok(BTreeMap::new())
        }
        fn default_paths(&self, _: &RequirementId) -> work_model::task::source::TaskArtifactPaths {
            work_model::task::source::TaskArtifactPaths {
                source: "sources/example".into(),
                task: "task/index.json".into(),
                execution: "execution".into(),
            }
        }
        fn source_evidence(&self, _: &Value) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
            panic!("missing Task must stop before Source evidence")
        }
        fn exists(&self, relative: &str) -> Result<bool, WorkError> {
            assert_eq!(relative, "task/index.json");
            Ok(false)
        }
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("missing Task must stop before read")
        }
    }

    struct UnusedSource;

    impl HierarchyCatalogRepository for UnusedSource {
        fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
            panic!("missing Task must stop before hierarchy")
        }
        fn mode_paths(&self, _: &str) -> Result<Vec<String>, WorkError> {
            panic!("missing Task must stop before hierarchy")
        }
    }
    impl InstructionSourceRepository for UnusedSource {
        fn load_sources(
            &self,
            _: &str,
            _: &Hierarchy,
            _: &[String],
        ) -> Result<SourceSet, WorkError> {
            panic!("missing Task must stop before sources")
        }
    }

    struct UnusedRouting;
    impl WorkflowRoutingRepository for UnusedRouting {
        fn route(&mut self, _: &RoutingRequest<'_>) -> Result<Value, WorkError> {
            panic!("missing Task must stop before routing")
        }
    }
    impl RefreshRoutingRepository for UnusedRouting {
        fn recheck_sources(&self) -> Result<(), WorkError> {
            panic!("missing Task must stop before routing recheck")
        }
    }

    #[test]
    fn missing_task_stops_before_source_and_routing_ports() {
        let error = build_refresh(&MissingTask, &UnusedSource, &mut UnusedRouting, "example")
            .err()
            .expect("missing Task must fail");
        assert_eq!(error.reason_code, "source_refresh_task_missing");
    }
}
