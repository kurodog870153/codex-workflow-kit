//! Read-only source and routing comparison over current artifact ports.

use crate::error::{ExitCode, WorkError};
use crate::instruction::{
    InstructionSourceRepository, load, migration_manifest, task_document_selection,
};
use crate::workflow::WorkflowRoutingRepository;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::execution::index::validate_execution_index;
use work_operations::identifiers::RequirementId;
use work_operations::instruction::{SourceSet, selection as source_selection};

pub trait ImpactSnapshotRepository {
    fn discover_requirements(&self) -> Result<BTreeMap<String, Value>, WorkError>;
    fn default_paths(&self, id: &RequirementId) -> work_model::task::source::TaskArtifactPaths;
    fn source_evidence(&self, index: &Value) -> Result<BTreeMap<String, Vec<u8>>, WorkError>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
}

pub trait ImpactRoutingRepository: WorkflowRoutingRepository {
    fn recheck_sources(&self) -> Result<(), WorkError>;
}

fn failure(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn read(
    repository: &impl ImpactSnapshotRepository,
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

/// Compare current instruction inputs with a persisted current collection.
/// This assessment has no after-state, approval, or publication candidate.
pub fn inspect_source_impact(
    repository: &impl ImpactSnapshotRepository,
    source: &impl InstructionSourceRepository,
    routing: &mut impl ImpactRoutingRepository,
    requirement_id: &str,
) -> Result<(Value, BTreeSet<String>), WorkError> {
    let id = RequirementId::from_str(requirement_id)
        .map_err(|_| failure("invalid_requirement_id", "The requirement ID is invalid."))?;
    let discovered = repository.discover_requirements()?;
    let artifacts = discovered
        .get(requirement_id)
        .cloned()
        .unwrap_or_else(|| json!(repository.default_paths(&id)));
    let path = artifacts["task"]
        .as_str()
        .ok_or_else(|| failure("invalid_artifact_paths", "A current Task path is required."))?;
    let (raw, index) = read(repository, path)?;
    work_operations::task::index::validate_task_index(&index, &raw, path)
        .map_err(|issue| failure(issue.reason_code, issue.message))?;
    let proof = repository.source_evidence(&index)?;
    let base = path.rsplit_once('/').map_or("", |(base, _)| base);
    let mut changed = BTreeSet::new();
    let mut files = Vec::new();
    let mut sets = Vec::new();
    let mut snapshots = Vec::new();
    for reference in index["tasks"].as_array().expect("validated references") {
        let relative = format!(
            "{base}/{}",
            reference["path"].as_str().expect("validated path")
        );
        let (item_raw, item) = read(repository, &relative)?;
        work_operations::task::item::validate_task_item(
            &item,
            &item_raw,
            reference["id"].as_str().expect("validated ID"),
        )
        .map_err(|issue| failure(issue.reason_code, issue.message))?;
        if fingerprint::canonical(&item_raw)
            .map_err(|_| failure("invalid_utf8", "The Task item is invalid."))?
            != reference["canonical_sha256"]
        {
            return Err(failure(
                "task_item_fingerprint_mismatch",
                "A Task item does not match the index.",
            ));
        }
        let stored = &item["instruction_selection"];
        let (mut current, loaded) = current_selection(source, "task", stored, &mut snapshots)?;
        current["routing_manifest"] =
            migration_manifest(routing, "task", "task_confirmed", "choose_task", &item)?;
        if *stored != current {
            files.push(json!({"path":relative,"status":"stale","required_action":"revise"}));
            for row in current["sources"].as_array().expect("current sources") {
                if !stored["sources"]
                    .as_array()
                    .expect("validated sources")
                    .contains(row)
                {
                    changed.insert(
                        row["logical_name"]
                            .as_str()
                            .expect("source name")
                            .to_owned(),
                    );
                }
            }
        }
        sets.push(loaded);
    }
    let mut current = task_document_selection(&sets)?;
    current["routing_manifest"] =
        migration_manifest(routing, "task", "task_confirmed", "confirm_review", &index)?;
    if index["instruction_selection"] != current {
        files.push(json!({"path":path,"status":"stale","required_action":"revise"}));
    }
    let execution_dir = artifacts["execution"]
        .as_str()
        .ok_or_else(|| failure("invalid_artifact_paths", "An execution path is required."))?;
    let execution_path = format!("{execution_dir}/index.json");
    if repository.exists(&execution_path)? {
        let (execution_raw, execution) = read(repository, &execution_path)?;
        validate_execution_index(&execution, &execution_raw)
            .map_err(|issue| failure(issue.reason_code, issue.message))?;
        let manifest = migration_manifest(
            routing,
            "execute",
            "execution_bound",
            "select_task_for_execution",
            &execution,
        )?;
        if execution["instruction_selection_manifest"] != manifest {
            files.push(json!({"path":execution_path,"status":"stale","required_action":"revise"}));
        }
    }
    if repository.source_evidence(&index)? != proof {
        return Err(failure(
            "instruction_source_changed",
            "The immutable Source changed during assessment.",
        ));
    }
    routing.recheck_sources()?;
    for (mode, paths, refs, digest) in snapshots {
        if load(source, &mode, &paths, &refs)?.instructions_sha256 != digest {
            return Err(failure(
                "instruction_source_changed",
                "An instruction source changed during assessment.",
            ));
        }
    }
    Ok((
        json!({"requirement_id":requirement_id,"status":if files.is_empty() {"valid"} else {"stale"},
        "changed_sources":changed.len(),"files":files,"blocked":[]}),
        changed,
    ))
}
